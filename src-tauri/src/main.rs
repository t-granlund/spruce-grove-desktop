//! Thin Tauri shell that drives the groomed `spruce-grove` CLI as a library.
//!
//! The desktop app owns no agent logic: every prompt spawns the CLI in
//! headless mode (`-p`) inside the chosen working directory, streams stdout
//! and stderr back to the webview as events, and reports the exit status.
//! Conversation continuity rides the CLI's own `--quick-resume` scoping.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

mod acp;
mod audit;
mod inspector;
mod ledger;
mod project;
mod recordings;
mod studio_auth;

/// One process-wide lock for tests that mutate environment variables (library
/// and auth dirs). Env vars are global, so every test module must share this
/// one guard — per-module mutexes would let two suites stomp each other.
#[cfg(test)]
pub(crate) static TEST_ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// Where the grove's own source checkout lives, most-canonical first.
///
/// `GROVE_REPO` lets a developer point the shell at a fork; otherwise the
/// canonical clone location wins, with the historical `~/SPRUCE-GROVE-OS`
/// kept as a last-resort fallback for machines that still have one.
fn grove_repo_dirs() -> Vec<String> {
    let mut dirs = Vec::new();
    if let Ok(explicit) = std::env::var("GROVE_REPO") {
        if !explicit.trim().is_empty() {
            dirs.push(explicit);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(format!("{home}/dev/SPRUCE-GROVE-OS"));
        dirs.push(format!("{home}/SPRUCE-GROVE-OS"));
    }
    dirs
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
        // Fall back to a source checkout only if it actually has a CLI.
        for repo in grove_repo_dirs() {
            let fork_venv = format!("{repo}/.venv/bin/spruce-grove");
            if std::path::Path::new(&fork_venv).exists() {
                return (fork_venv, Vec::new());
            }
        }
    }
    ("spruce-grove".to_string(), Vec::new())
}

/// Strip ANSI escape sequences so the webview renders plain text.
/// Handles CSI (`ESC [ ... final-byte`) and OSC (`ESC ] ... BEL | ESC \`).
pub(crate) fn strip_ansi(input: &str) -> String {
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
    for repo in grove_repo_dirs() {
        if std::path::Path::new(&repo).is_dir() {
            return repo;
        }
    }
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

/// Per-business shell profile: reads `<cwd>/.spruce_grove/shell.json` and
/// returns it verbatim (or null when absent/invalid). The UI wears it: brand
/// name, accent color, quick prompts, agent roster notes. The machine stays
/// generic; the workspace makes it theirs.
#[tauri::command]
fn grove_shell_profile(cwd: String) -> Result<Option<String>, String> {
    let path = std::path::Path::new(&cwd)
        .join(".spruce_grove")
        .join("shell.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok(None), // no profile: stock grove, honestly
    };
    // validate it parses before handing it to the UI (fail honest, not partial)
    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("shell.json invalid: {e}"))?;
    Ok(Some(parsed.to_string()))
}

#[tauri::command]
fn grove_version() -> Result<String, String> {
    let (program, prefix) = cli_command();
    let output = Command::new(&program)
        .args(&prefix)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot launch {program}: {e}"))?;
    // `spruce-grove --version` writes OSC color escapes ahead of the version
    // string (same boot palette as --acp). Leaving them in made the sidebar
    // readout show raw `[11;#1a1b26m[10;#c0caf5m…` garbage on the projector.
    let raw = String::from_utf8_lossy(&output.stdout);
    let text = strip_ansi(&raw).trim().to_string();
    if text.is_empty() {
        Err(format!(
            "no version output (stderr: {})",
            strip_ansi(&String::from_utf8_lossy(&output.stderr)).trim()
        ))
    } else {
        Ok(text)
    }
}

/// Which grove sessions are open on this machine, as the CLI sees them.
///
/// The shell deliberately does **not** reimplement process inspection: the
/// CLI owns the definition of a live session (an open terminal that has not
/// been closed out) and the knowledge of which saved session each one maps
/// to. This forwards `--console --json` so the sidebar and the `/console`
/// panel can never disagree.
///
/// Returns the parsed array directly (not a JSON string) so the webview gets
/// objects without a second decode.
#[tauri::command]
fn grove_console_sessions() -> Result<serde_json::Value, String> {
    let (program, prefix) = cli_command();
    let output = Command::new(&program)
        .args(&prefix)
        .arg("--console")
        .arg("--json")
        .output()
        .map_err(|e| format!("cannot launch {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "console query failed (stderr: {})",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    // The grove emits terminal colour sequences during import, so the JSON is
    // not the whole of stdout. Strip escapes BEFORE locating the array: an
    // OSC sequence happens to be bracket-free, but a CSI one (`ESC [ 0 m`) is
    // not, and `find('[')` would then split a colour code in half.
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    let start = text.find('[').ok_or("console output had no JSON array")?;
    serde_json::from_str(&text[start..]).map_err(|e| format!("bad console JSON: {e}"))
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

/// Directory listing for the in-webview working-dir picker. The shell stays
/// thin: path in, immediate subdirectories out (parent included so the UI can
/// climb). Symlinks that resolve to directories count; broken ones are
/// skipped honestly. Dotfiles are hidden — same default as the Finder.
#[tauri::command]
fn grove_list_dirs(path: String) -> Result<serde_json::Value, String> {
    // sanity cap: a listing of 50k rows would wedge the webview, not help it
    const MAX_ENTRIES: usize = 500;
    let p = std::path::Path::new(&path);
    let mut dirs: Vec<String> = Vec::new();
    let mut total = 0usize;
    for entry in std::fs::read_dir(p).map_err(|e| format!("read {path}: {e}"))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // one unreadable entry must not kill the listing
        };
        // metadata() follows symlinks; broken links just don't list
        let Ok(md) = entry.metadata() else { continue };
        if !md.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        total += 1;
        if dirs.len() < MAX_ENTRIES {
            dirs.push(name);
        }
    }
    dirs.sort();
    let parent = p.parent().map(|x| x.to_string_lossy().to_string());
    Ok(serde_json::json!({
        "path": path, "parent": parent, "dirs": dirs, "total": total
    }))
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
    // `--transcribe` writes the same OSC boot palette ahead of its text that
    // `--version` does. Without stripping, the palette lands verbatim in the
    // prompt (the dry run showed `]11;#1a1b26]10;#c0caf5…` in the transcript).
    let raw = String::from_utf8_lossy(&output.stdout);
    let text = strip_ansi(&raw).trim().to_string();
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

/// Bridge self-check ack: the webview calls this when the boot-time
/// `grove://bridge-probe` event arrives. The marker file lets the shell
/// (and CI scripts) prove the event bridge end-to-end without a human
/// click — a missing file means capabilities or the event plugin are
/// broken and every streamed update is being silently dropped.
/// Boot breadcrumb: the webview calls this at key boot steps (script loaded,
/// listeners registered, listen rejected) and the shell drops a marker file.
/// Lets CI/scripts see how far the packaged app actually got without a
/// human watching the window. Note rides in the file, name in the filename.
#[tauri::command]
fn grove_boot_marker(name: String, note: Option<String>) -> Result<(), String> {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = std::env::temp_dir().join(format!("sg-boot-{safe}.txt"));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::fs::write(&path, format!("{stamp} {}", note.unwrap_or_default()))
        .map_err(|e| format!("boot marker: {e}"))
}

/// Marker path for THIS process: per-PID so a previous instance's ack can
/// never short-circuit a later probe's retry loop.
fn bridge_probe_marker() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("spruce-grove-bridge-probe-{}", std::process::id()))
}

#[tauri::command]
fn grove_bridge_probe_ack() -> Result<String, String> {
    let path = bridge_probe_marker();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::fs::write(&path, format!("ok {stamp}")).map_err(|e| format!("probe write: {e}"))?;
    Ok("acked".into())
}

fn main() {
    tauri::Builder::default()
        .manage(ActiveRun(Mutex::new(None)))
        .manage(acp::AcpState::new())
        .setup(|app| {
            // fire the bridge probe until acked: the webview may still be
            // loading (blocking font fetches) when the first ping lands, so
            // a one-shot probe would be a race, not an instrument
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let marker = bridge_probe_marker();
                for attempt in 0..30 {
                    if marker.exists() {
                        break;
                    }
                    let _ = tauri::Emitter::emit(&handle, "grove://bridge-probe", attempt);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            grove_default_cwd,
            grove_shell_profile,
            grove_console_sessions,
            grove_version,
            grove_send,
            grove_cancel,
            grove_save_recording,
            grove_transcribe,
            grove_read_image_base64,
            grove_list_dirs,
            grove_acp_start,
            grove_acp_prompt,
            grove_acp_cancel,
            grove_acp_kill,
            grove_bridge_probe_ack,
            grove_boot_marker,
            inspector::grove_git_state,
            inspector::grove_open_path,
            inspector::grove_open_url,
            inspector::grove_settings_get,
            inspector::grove_settings_set,
            inspector::grove_note_error,
            inspector::grove_repo_access,
            inspector::grove_diagnostics,
            ledger::grove_ledger_record,
            ledger::grove_ledger_tail,
            project::grove_project_state,
            audit::grove_self_audit,
            recordings::grove_library_takes,
            recordings::grove_recordings_list,
            recordings::grove_recording_get,
            recordings::grove_recording_audio,
            recordings::grove_recording_patch,
            recordings::grove_recording_unlock,
            recordings::grove_studio_has_pin,
            recordings::grove_studio_set_pin,
            recordings::grove_recording_drop,
            recordings::grove_recording_drop_take,
            recordings::grove_recording_splice,
            recordings::grove_recording_export
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

    /// The real `spruce-grove --version` line: an OSC color palette, then the
    /// version, on one line. Without stripping, the sidebar cli: readout showed
    /// `[11;#1a1b26m[10;#c0caf5m…` on the projector (caught in the dry run).
    #[test]
    fn strip_ansi_handles_osc_version_preamble() {
        let line = "\u{1b}]11;#1a1b26\u{7}\u{1b}]10;#c0caf5\u{7}\u{1b}]4;0;#15161e\u{7}1.0.76";
        assert_eq!(strip_ansi(line), "1.0.76");
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

    /// `--transcribe` prepends the same OSC boot palette as `--version`. The
    /// transcript must arrive clean; the dry run showed the palette verbatim in
    /// the prompt. This pins the strip the command now applies.
    #[test]
    fn transcribe_output_is_ansi_free() {
        let line = "\u{1b}]11;#1a1b26\u{7}\u{1b}]10;#c0caf5\u{7}Launch the desktop dictation loop.";
        assert_eq!(
            strip_ansi(line).trim(),
            "Launch the desktop dictation loop."
        );
    }

    #[test]
    fn list_dirs_sorts_hides_dotfiles_and_reports_parent() {
        let root = std::env::temp_dir().join(format!("grove-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::create_dir_all(root.join("zeta")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::write(root.join("file.txt"), b"not a dir").unwrap();

        let out = grove_list_dirs(root.to_string_lossy().to_string()).unwrap();
        assert_eq!(out["dirs"].clone(), serde_json::json!(["alpha", "zeta"]));
        assert_eq!(out["total"].as_u64(), Some(2));
        assert_eq!(
            out["parent"].clone(),
            serde_json::json!(root.parent().map(|p| p.to_string_lossy().to_string()))
        );

        // bad path: honest error, not a silent empty list
        assert!(grove_list_dirs("/definitely/not/a/grove/path".into()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
