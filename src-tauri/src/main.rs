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
    if let Ok(home) = std::env::var("HOME") {
        let local = format!("{home}/SPRUCE-GROVE-OS/.venv/bin/spruce-grove");
        if std::path::Path::new(&local).exists() {
            return (local, Vec::new());
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
fn grove_save_recording(bytes: Vec<u8>) -> Result<String, String> {
    let dir = std::env::temp_dir().join("spruce-grove-dictation");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("clock: {e}"))?
        .as_millis();
    let path = dir.join(format!("dictation-{stamp}.webm"));
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

fn main() {
    tauri::Builder::default()
        .manage(ActiveRun(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            grove_default_cwd,
            grove_version,
            grove_send,
            grove_cancel,
            grove_save_recording,
            grove_transcribe
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
