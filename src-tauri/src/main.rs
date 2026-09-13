//! Thin Tauri shell that drives the groomed `spruce-grove` CLI as a library.
//!
//! The desktop app owns no agent logic: every prompt spawns the CLI in
//! headless mode (`-p`) inside the chosen working directory, streams stdout
//! and stderr back to the webview as events, and reports the exit status.
//! Conversation continuity rides the CLI's own `--quick-resume` scoping.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

mod acp;

struct ActiveRun(Mutex<Option<Child>>);

#[derive(Clone, Serialize)]
struct LinePayload {
    stream: &'static str,
    line: String,
}

#[derive(Clone, Serialize)]
struct ExitPayload {
    code: Option<i32>,
    ok: bool,
}

/// Resolve how to launch the CLI. `GROVE_CLI` (space-separated prefix, e.g.
/// `uv run --directory /path/to/repo spruce-grove`) wins; then a bare
/// `spruce-grove` on PATH; finally the in-house fork checkout.
fn cli_command() -> (String, Vec<String>) {
    if let Ok(prefix) = std::env::var("GROVE_CLI") {
        let mut parts = prefix.split_whitespace().map(str::to_string);
        if let Some(program) = parts.next() {
            return (program, parts.collect());
        }
    }
    // newest first: the uv tool install tracks PyPI releases; the fork venv
    // may lag releases behind. Path order matters for --acp dialect drift.
    if let Ok(home) = std::env::var("HOME") {
        let uv_tool = format!("{home}/.local/bin/spruce-grove");
        if std::path::Path::new(&uv_tool).exists() {
            return (uv_tool, Vec::new());
        }
        let fork_venv = format!("{home}/SPRUCE-GROVE-OS/.venv/bin/spruce-grove");
        if std::path::Path::new(&fork_venv).exists() {
            return (fork_venv, Vec::new());
        }
    }
    ("spruce-grove".to_string(), Vec::new())
}

/// Strip ANSI escape sequences so the webview renders plain text.
/// Handles CSI (`ESC [ ... final-byte`) and OSC (`ESC ] ... BEL | ESC \`).
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters/intermediates, then one final byte @..~
            Some('[') => {
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: ends at BEL or ST (ESC \)
            Some(']') => {
                let mut prev_esc = false;
                for n in chars.by_ref() {
                    if n == '\u{7}' || (prev_esc && n == '\\') {
                        break;
                    }
                    prev_esc = n == '\u{1b}';
                }
            }
            // Two-byte escapes (ESC + one byte): swallowed by the match itself
            _ => {}
        }
    }
    out
}

#[tauri::command]
fn grove_default_cwd() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let fork = format!("{home}/SPRUCE-GROVE-OS");
    if std::path::Path::new(&fork).is_dir() {
        fork
    } else {
        home
    }
}

#[tauri::command]
fn grove_version() -> Result<String, String> {
    let (program, prefix) = cli_command();
    let output = Command::new(&program)
        .args(&prefix)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot launch {program}: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Err(format!(
            "no version output (stderr: {})",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    } else {
        Ok(text)
    }
}

/// Pump a child stream to the webview, one event per line.
fn spawn_reader<R>(app: AppHandle, reader: R, tag: &'static str)
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) => {
                    let _ = app.emit(
                        "grove://line",
                        LinePayload {
                            stream: tag,
                            line: strip_ansi(&line),
                        },
                    );
                }
                Err(_) => break,
            }
        }
    });
}

#[tauri::command]
fn grove_send(
    app: AppHandle,
    state: State<'_, ActiveRun>,
    prompt: String,
    cwd: String,
    resume: bool,
) -> Result<(), String> {
    {
        let guard = state.0.lock().map_err(|_| "state poisoned")?;
        if guard.is_some() {
            return Err("a run is already active; cancel it first".to_string());
        }
    }

    let (program, prefix) = cli_command();
    let mut cmd = Command::new(&program);
    cmd.args(&prefix);
    if resume {
        cmd.arg("--quick-resume").arg(&cwd);
    }
    let mut child = cmd
        .arg("--prompt")
        .arg(&prompt)
        .arg("--disable-ask-user-question")
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot launch {program}: {e}"))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    *state.0.lock().map_err(|_| "state poisoned")? = Some(child);

    spawn_reader(app.clone(), stdout, "stdout");
    spawn_reader(app.clone(), stderr, "stderr");

    std::thread::spawn(move || {
        let state_handle = app.state::<ActiveRun>();
        let status = {
            let mut guard = match state_handle.0.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            guard.as_mut().and_then(|child| child.wait().ok())
        };
        if let Ok(mut guard) = state_handle.0.lock() {
            *guard = None;
        }
        let (code, ok) = match status {
            Some(s) => (s.code(), s.success()),
            None => (None, false),
        };
        let _ = app.emit("grove://exit", ExitPayload { code, ok });
    });

    Ok(())
}

#[tauri::command]
fn grove_cancel(state: State<'_, ActiveRun>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|_| "state poisoned")?;
    match guard.as_mut() {
        Some(child) => {
            let _ = child.kill();
            Ok(())
        }
        None => Err("no active run".to_string()),
    }
}

/// Persist a dictation recording (webm/opus bytes from MediaRecorder) to a
/// temp file and return its path. The webview cannot touch the filesystem
/// itself and the shell stays thin: bytes in, path out.
#[tauri::command]
fn grove_save_recording(bytes: Vec<u8>, mime: Option<String>) -> Result<String, String> {
    let dir = std::env::temp_dir().join("spruce-grove-dictation");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("clock: {e}"))?
        .as_millis();
    // name the container honestly: WKWebView engines differ (Chromium emits
    // webm, Safari/WKWebView mp4). ffmpeg sniffs content anyway, but a true
    // extension keeps debugging sane.
    let ext = match mime.as_deref().map(str::to_ascii_lowercase) {
        Some(m) if m.contains("mp4") => "mp4",
        Some(m) if m.contains("ogg") => "ogg",
        _ => "webm",
    };
    let path = dir.join(format!("dictation-{stamp}.{ext}"));
    std::fs::write(&path, bytes).map_err(|e| format!("write: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// Transcribe a recorded file through the CLI's own rig
/// (`spruce-grove --transcribe <file>` — the Mockingbird plugin's headless
/// verb). The shell never re-implements whisper: it launches the same
/// resolution order as grove_send and returns the transcript text.
#[tauri::command]
fn grove_transcribe(file: String) -> Result<String, String> {
    let (program, prefix) = cli_command();
    let output = Command::new(&program)
        .args(&prefix)
        .arg("--transcribe")
        .arg(&file)
        .output()
        .map_err(|e| format!("cannot launch {program}: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(if err.trim().is_empty() {
            format!("transcribe failed ({})", output.status)
        } else {
            err.trim().to_string()
        });
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Err("transcript came back empty".to_string())
    } else {
        Ok(text)
    }
}

// ---------------------------- look-in panel media ------------------------

/// Read a screenshot/image from disk as a data URL for the Live Look-in
/// panel. Browser tools (spruce_grove_screenshots_* tempdirs) save PNGs and
/// report the path in their tool output; the panel polls tool_call updates
/// for those paths and calls this. Extension-whitelisted, size-capped.
#[tauri::command]
fn grove_read_image_base64(path: String) -> Result<String, String> {
    use base64::Engine as _;
    let p = std::path::Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        other => return Err(format!("unsupported image type: {other}")),
    };
    let bytes = std::fs::read(p).map_err(|e| format!("read {path}: {e}"))?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("image too large".into());
    }
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

// ---------------------------- ACP (structured live sessions) -------------

#[tauri::command]
fn grove_acp_start(
    state: State<'_, acp::AcpState>,
    app: AppHandle,
    cwd: String,
    resume: Option<String>,
) -> Result<serde_json::Value, String> {
    let (session_id, model, resumed) = acp::start(&state, &cwd, resume, &app)?;
    Ok(serde_json::json!({
        "sessionId": session_id, "model": model, "resumed": resumed
    }))
}

#[tauri::command]
fn grove_acp_prompt(
    state: State<'_, acp::AcpState>,
    app: AppHandle,
    session_id: String,
    text: String,
) -> Result<(), String> {
    acp::prompt(&state, &session_id, &text, &app)
}

/// Hard-restart hook for the UI stall watchdog: kill the wedged CLI.
#[tauri::command]
fn grove_acp_kill(state: State<'_, acp::AcpState>) -> bool {
    acp::kill(&state)
}

#[tauri::command]
fn grove_acp_cancel(state: State<'_, acp::AcpState>, session_id: String) -> Result<(), String> {
    acp::cancel(&state, &session_id)
}

fn main() {
    tauri::Builder::default()
        .manage(ActiveRun(Mutex::new(None)))
        .manage(acp::AcpState::new())
        .invoke_handler(tauri::generate_handler![
            grove_default_cwd,
            grove_version,
            grove_send,
            grove_cancel,
            grove_save_recording,
            grove_transcribe,
            grove_read_image_base64,
            grove_acp_start,
            grove_acp_prompt,
            grove_acp_cancel,
            grove_acp_kill
        ])
        .run(tauri::generate_context!())
        .expect("error while running spruce-grove desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_sgr_sequences() {
        assert_eq!(strip_ansi("\u{1b}[32mOK\u{1b}[0m plain"), "OK plain");
    }

    #[test]
    fn strip_ansi_keeps_plain_text_untouched() {
        let input = "brand-personal-guard: OK -- no leaks";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn strip_ansi_drops_mid_line_color_switches() {
        assert_eq!(strip_ansi("a\u{1b}[1;33mb\u{1b}[0mc"), "abc");
    }
}
