//! Minimal ACP (Agent Client Protocol) client: newline-delimited JSON-RPC
//! over the agent's stdio.
//!
//! Speaks exactly the dialect `spruce-grove --acp` serves (probed live;
//! see tests/acp_probe.py and the recorded fixtures):
//!
//! * client -> agent requests: `initialize`, `session/new`, `session/prompt`
//! * client -> agent notifications: `session/cancel`
//! * agent -> client requests: `session/request_permission` (auto-allowed,
//!   logged), anything else gets a method-not-found error so the turn never
//!   hangs
//! * agent -> client notifications: `session/update` with variants
//!   `agent_message_chunk`, `agent_thought_chunk`, `tool_call`,
//!   `tool_call_update`, `plan`, `available_commands_update`, `current_mode`
//!
//! Deliberately dependency-free (serde_json only): the protocol surface we
//! use is small, and owning the router makes the contract tests honest.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub const PROTOCOL_VERSION: u64 = 1;

/// Everything the UI learns from the agent flows through this one channel.
#[derive(Clone, serde::Serialize)]
pub struct AcpEvent {
    pub kind: String, // ready | chunk | thought | tool | permission | plan | commands | turn-end | error
    pub data: Value,
}

/// Managed app state: at most one live ACP session. Replacing the value
/// drops (and kills) any previous agent child.
pub struct AcpState(pub Arc<Mutex<Option<AcpConnection>>>);

impl AcpState {
    pub fn new() -> Self {
        AcpState(Arc::new(Mutex::new(None)))
    }
}

pub struct AcpConnection {
    pub child: Child,
    pub stdin: Arc<Mutex<ChildStdin>>,
    pub session_id: String,
    pub model: Option<String>,
    next_id: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
}

impl Drop for AcpConnection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------- helpers
// Pure functions first: the contract tests live on these.

/// Parse one ndjson line into a JSON-RPC message, if it is one.
pub fn parse_line(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

/// Classify an incoming message for the router.
pub enum Incoming {
    /// A response to one of our requests (matches by id).
    Response(u64, Value),
    /// A request FROM the agent (method + id present) — must be answered.
    AgentRequest(u64, String, Value),
    /// A notification (method, no id) — e.g. session/update.
    Notification(String, Value),
    /// Unparseable/irrelevant.
    Ignore,
}

pub fn classify(msg: &Value) -> Incoming {
    let id = msg.get("id").and_then(|v| v.as_u64());
    let method = msg.get("method").and_then(|v| v.as_str());

    // Responses carry an id plus result/error (method is absent in strict
    // JSON-RPC; tolerate its presence for lenient peers).
    if let Some(id) = id {
        if msg.get("result").is_some() || msg.get("error").is_some() {
            return Incoming::Response(id, msg.clone());
        }
        if let Some(method) = method {
            return Incoming::AgentRequest(
                id,
                method.to_string(),
                msg.get("params").cloned().unwrap_or(Value::Null),
            );
        }
    }

    if let Some(method) = method {
        if id.is_none() {
            return Incoming::Notification(
                method.to_string(),
                msg.get("params").cloned().unwrap_or(Value::Null),
            );
        }
    }

    Incoming::Ignore
}

/// The `sessionUpdate` variant discriminator of a session/update payload.
pub fn update_kind(params: &Value) -> Option<&str> {
    params.get("update")?.get("sessionUpdate")?.as_str()
}

/// Concatenate text deltas of one chunk variant into UI-appendable text.
pub fn chunk_text(params: &Value) -> String {
    params
        .get("update")
        .and_then(|u| u.get("content"))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Pick the first allow-ish option id from a permission request, else the
/// first option at all. Headless precedent: `-p` runs are always EXTREME.
pub fn first_allow_option(params: &Value) -> Option<String> {
    let options = params.get("options")?.as_array()?;
    let allow = options
        .iter()
        .find(|o| {
            matches!(
                o.get("kind").and_then(|k| k.as_str()),
                Some("allow_once") | Some("allow_always")
            )
        })
        .or_else(|| options.first())?;
    option_id(allow)
}

fn option_id(option: &Value) -> Option<String> {
    match option.get("optionId") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string().trim_matches('"').to_string()),
        None => None,
    }
}

/// Extract the session id + current model from a session/new result.
pub fn parse_session_new(result: &Value) -> (Option<String>, Option<String>) {
    let session_id = result
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let model = result
        .get("configOptions")
        .and_then(|c| c.as_array())
        .and_then(|opts| {
            opts.iter()
                .find(|o| o.get("currentValue").is_some_and(|v| v.is_string()))
        })
        .and_then(|o| o.get("currentValue"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    (session_id, model)
}

// ---------------------------------------------------------------- session

fn write_msg(stdin: &Arc<Mutex<ChildStdin>>, msg: &Value) -> Result<(), String> {
    let mut w = stdin.lock().map_err(|_| "stdin poisoned")?;
    w.write_all(msg.to_string().as_bytes())
        .and_then(|_| w.write_all(b"\n"))
        .and_then(|_| w.flush())
        .map_err(|e| format!("write: {e}"))
}

/// Spawn `spruce-grove --acp`, run the initialize + (load|new) handshake,
/// start the reader loop, and install the connection into app state.
/// Returns (sessionId, currentModel).
///
/// `resume` carries a previous session id (from a dead process): it is
/// loaded with `session/load` so the conversation survives relaunches
/// (durability proven by tests/acp_probe.py --lifecycle). On load failure
/// we fall back to a fresh `session/new` — a dead session must never
/// brick the app.
pub fn start(
    state: &AcpState,
    cwd: &str,
    resume: Option<String>,
    app: &AppHandle,
) -> Result<(String, Option<String>, bool), String> {
    let (program, prefix) = crate::cli_command();
    let mut child = Command::new(&program)
        .args(&prefix)
        .arg("--acp")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot launch {program} --acp: {e}"))?;

    let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or("no stdin")?));
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let next_id = Arc::new(AtomicU64::new(1));
    let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Reader loop: route every line, emit UI events, auto-answer agent
    // requests so a turn can never hang on an unanswered permission.
    let pending_reader = pending.clone();
    let next_id_reader = next_id.clone();
    let stdin_reader = stdin.clone();
    let app_reader = app.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let Some(msg) = parse_line(&line) else { continue };
            match classify(&msg) {
                Incoming::Response(id, value) => {
                    if let Some(tx) = pending_reader.lock().unwrap().remove(&id) {
                        let _ = tx.send(value);
                    }
                }
                Incoming::AgentRequest(rid, method, params) => {
                    let is_permission = method == "session/request_permission";
                    let outcome = if is_permission {
                        json!({ "outcome": {
                            "outcome": "selected",
                            "optionId": first_allow_option(&params),
                        } })
                    } else {
                        json!({ "error": "method not supported by this client" })
                    };
                    if is_permission {
                        let _ = app_reader.emit(
                            "grove://acp",
                            AcpEvent {
                                kind: "permission".into(),
                                data: json!({ "method": method, "params": params }),
                            },
                        );
                    }
                    let reply = json!({ "jsonrpc": "2.0", "id": rid, "result": outcome });
                    let _ = write_msg(&stdin_reader, &reply);
                }
                Incoming::Notification(method, params) => {
                    if method == "session/update" {
                        let raw_kind = update_kind(&params).unwrap_or("unknown").to_string();
                        let kind = match raw_kind.as_str() {
                            "agent_message_chunk" => "chunk",
                            "agent_thought_chunk" => "thought",
                            "tool_call" | "tool_call_update" => "tool",
                            "plan" => "plan",
                            "available_commands_update" => "commands",
                            other => other,
                        };
                        let _ = app_reader.emit(
                            "grove://acp",
                            AcpEvent { kind: kind.into(), data: params },
                        );
                    }
                }
                Incoming::Ignore => {}
            }
        }
    });

    let mut conn = AcpConnection {
        child,
        stdin: stdin.clone(),
        session_id: String::new(),
        model: None,
        next_id: next_id.clone(),
        pending: pending.clone(),
    };

    // Handshake helpers route through the same pending map the reader uses.
    let call = |method: &str, params: Value, timeout: u64| -> Result<Value, String> {
        let rid = next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        pending.lock().unwrap().insert(rid, tx);
        let req = json!({ "jsonrpc": "2.0", "id": rid, "method": method, "params": params });
        write_msg(&stdin, &req)?;
        rx.recv_timeout(std::time::Duration::from_secs(timeout))
            .map_err(|_| format!("{method} timed out after {timeout}s"))
            .and_then(|msg| {
                if let Some(err) = msg.get("error") {
                    Err(format!("{method} error: {err}"))
                } else {
                    Ok(msg.get("result").cloned().unwrap_or(Value::Null))
                }
            })
    };

    call(
        "initialize",
        json!({ "protocolVersion": PROTOCOL_VERSION, "clientCapabilities": {} }),
        30,
    )?;
    let mut resumed = false;
    let previous_session = resume.as_deref().filter(|s| !s.is_empty());
    // A load can "succeed" at the JSON-RPC level yet carry no sessionId when
    // the CLI does not know the id (e.g. a session stored by an older CLI
    // release). Only a response containing sessionId counts as resumed;
    // anything else falls back to a fresh session/new on the same wire.
    let load_attempt = previous_session
        .map(|prev| {
            call(
                "session/load",
                json!({ "cwd": cwd, "sessionId": prev, "mcpServers": [] }),
                30,
            )
        })
        .unwrap_or_else(|| Err("no previous session".into()));
    let new = match load_attempt.ok().filter(|v| v.pointer("/result/sessionId").is_some()) {
        Some(loaded) => {
            resumed = true;
            loaded
        }
        None => {
            if let Some(prev) = previous_session {
                let _ = app.emit(
                    "grove://acp",
                    AcpEvent {
                        kind: "error".into(),
                        data: json!({ "message": format!(
                            "could not resume {prev} -- starting a fresh session"
                        ) }),
                    },
                );
            }
            call("session/new", json!({ "cwd": cwd, "mcpServers": [] }), 30)?
        }
    };
    let (session_id, model) = parse_session_new(&new);
    let session_id = session_id.ok_or("session/new returned no sessionId")?;

    conn.session_id = session_id.clone();
    conn.model = model.clone();

    // Install into app state; dropping any previous connection kills it.
    match state.0.lock() {
        Ok(mut guard) => {
            *guard = Some(conn);
        }
        Err(_) => return Err("ACP state poisoned".into()),
    }

    app.emit(
        "grove://acp",
        AcpEvent {
            kind: "ready".into(),
            data: json!({ "sessionId": session_id, "model": model, "resumed": resumed }),
        },
    )
    .ok();

    Ok((session_id, model, resumed))
}

/// Send a user prompt; returns immediately. The turn streams `chunk`,
/// `thought`, `tool` ... events and ends with a `turn-end` event carrying
/// stopReason + token usage.
pub fn prompt(state: &AcpState, session_id: &str, text: &str, app: &AppHandle) -> Result<(), String> {
    let (stdin, next_id, pending) = {
        let guard = state.0.lock().map_err(|_| "ACP state poisoned")?;
        let conn = guard.as_ref().ok_or("no active ACP session")?;
        if conn.session_id != session_id {
            return Err("session id mismatch".into());
        }
        (conn.stdin.clone(), conn.next_id.clone(), conn.pending.clone())
    };

    let rid = next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    pending.lock().unwrap().insert(rid, tx);
    let req = json!({
        "jsonrpc": "2.0", "id": rid, "method": "session/prompt",
        "params": { "sessionId": session_id,
                    "prompt": [ { "type": "text", "text": text } ] }
    });
    write_msg(&stdin, &req)?;

    let app_turn = app.clone();
    std::thread::spawn(move || match rx.recv() {
        Ok(msg) => {
            let payload = if let Some(err) = msg.get("error") {
                json!({ "ok": false, "error": err })
            } else {
                json!({ "ok": true, "result": msg.get("result") })
            };
            let _ = app_turn.emit(
                "grove://acp",
                AcpEvent { kind: "turn-end".into(), data: payload },
            );
        }
        Err(_) => {
            let _ = app_turn.emit(
                "grove://acp",
                AcpEvent {
                    kind: "turn-end".into(),
                    data: json!({ "ok": false, "error": "turn dropped" }),
                },
            );
        }
    });
    Ok(())
}

/// Hard stop: drop the connection (its Drop kills the child CLI) so a
/// wedged turn can never wedge the shell. The pending prompt's response
/// channel resolves as "turn dropped", which the UI handles as a failed
/// turn and recovers from.
pub fn kill(state: &AcpState) -> bool {
    state
        .0
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
        .is_some()
}

/// Best-effort cancel of the in-flight turn.
pub fn cancel(state: &AcpState, session_id: &str) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "ACP state poisoned")?;
    let conn = guard.as_ref().ok_or("no active ACP session")?;
    let note = json!({
        "jsonrpc": "2.0", "method": "session/cancel",
        "params": { "sessionId": session_id }
    });
    write_msg(&conn.stdin, &note)
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_routes_all_three_shapes() {
        let resp = json!({"jsonrpc":"2.0","id":7,"result":{"stopReason":"end_turn"}});
        assert!(matches!(classify(&resp), Incoming::Response(7, _)));

        let req = json!({"jsonrpc":"2.0","id":3,"method":"session/request_permission","params":{}});
        assert!(matches!(classify(&req), Incoming::AgentRequest(3, m, _) if m == "session/request_permission"));

        let note = json!({"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk"}}});
        assert!(matches!(classify(&note), Incoming::Notification(m, _) if m == "session/update"));
    }

    #[test]
    fn chunk_text_extracts_deltas() {
        let params = json!({"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ACP ONLINE."}}});
        assert_eq!(chunk_text(&params), "ACP ONLINE.");
    }

    #[test]
    fn first_allow_prefers_allow_kind() {
        let params = json!({"options":[
            {"optionId":"deny1","kind":"reject_once"},
            {"optionId":"ok1","kind":"allow_once"},
            {"optionId":"ok2","kind":"allow_always"}
        ]});
        assert_eq!(first_allow_option(&params).as_deref(), Some("ok1"));
        let none = json!({"options":[{"optionId":"only","kind":"reject_once"}]});
        assert_eq!(first_allow_option(&none).as_deref(), Some("only"));
    }

    #[test]
    fn parse_session_new_reads_id_and_model() {
        let result = json!({
            "sessionId": "sess_123",
            "configOptions": [ {"currentValue": "syn:large:text", "options": []} ]
        });
        let (sid, model) = parse_session_new(&result);
        assert_eq!(sid.as_deref(), Some("sess_123"));
        assert_eq!(model.as_deref(), Some("syn:large:text"));
    }

    /// Replay the recorded probe session (fixtures/acp/session.jsonl) through
    /// the router: the contract between the CLI's --acp dialect and this
    /// client, pinned so protocol drift fails loudly. Regenerate with
    /// `python3 tests/acp_probe.py`.
    #[test]
    fn replay_probe_fixture() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let path = std::path::Path::new(&manifest)
            .join("../fixtures/acp/session.jsonl");
        let Ok(content) = std::fs::read_to_string(path) else {
            panic!("fixture missing - run python3 tests/acp_probe.py to record it");
        };

        let mut responses = 0usize;
        let mut updates: HashMap<String, usize> = HashMap::new();
        let mut saw_protocol_version = false;
        for line in content.lines() {
            let Some(msg) = parse_line(line) else {
                panic!("fixture line is not valid JSON: {line}");
            };
            match classify(&msg) {
                Incoming::Response(_, ref value) => {
                    responses += 1;
                    if value
                        .pointer("/result/protocolVersion")
                        .is_some_and(|v| v.as_u64() == Some(1))
                    {
                        saw_protocol_version = true;
                    }
                }
                Incoming::Notification(method, ref params) => {
                    if method == "session/update" {
                        let kind = update_kind(params).unwrap_or("unknown").to_string();
                        *updates.entry(kind).or_insert(0) += 1;
                    }
                }
                Incoming::AgentRequest(..) => {
                    panic!("no-tools fixture must not contain agent requests");
                }
                Incoming::Ignore => panic!("fixture line routed as Ignore"),
            }
        }

        assert!(saw_protocol_version, "initialize response missing");
        assert!(responses >= 3, "expected >= 3 responses, got {responses}");
        assert!(
            updates.get("agent_message_chunk").copied().unwrap_or(0) >= 1,
            "fixture lost its message chunks: {updates:?}"
        );
        assert!(
            updates.get("available_commands_update").copied().unwrap_or(0) >= 1,
            "fixture lost available_commands_update"
        );
    }

    #[test]
    fn parse_line_handles_noise() {
        assert!(parse_line("").is_none());
        assert!(parse_line("not json at all").is_none());
        assert!(parse_line("{\"jsonrpc\":\"2.0\",\"method\":\"x\"}").is_some());
    }
}
