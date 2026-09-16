//! Inspector backend: repository + GitHub state for the working directory.
//!
//! Read-only git plumbing and `gh` queries, each with a hard timeout so a
//! wedged network call can never wedge the drawer. Pure parsers live here
//! too, unit-tested, so dialect drift fails in `cargo test` first.
//!
//! Zero new dependencies: git ships with the OS toolchain (/usr/bin/git),
//! gh is located at the usual Homebrew paths, and timeouts ride
//! `try_wait` polling — no wait-timeout crate.

use serde_json::{json, Value};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const GH_TIMEOUT: Duration = Duration::from_secs(10);
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Run a command to completion with a hard deadline. Returns None on
/// spawn failure, timeout, or unreadable output — callers decide what
/// "absent" renders as. stdout is capped at 256KB (git/gh never approach
/// it; a runaway pipe must not balloon memory).
fn run_with_timeout(
    program: &str,
    args: &[&str],
    cwd: &str,
    timeout: Duration,
) -> Option<std::process::Output> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(256 * 1024).read_to_end(&mut buf);
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.take(64 * 1024).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => return None,
        }
    };
    let out = out_handle.join().ok()?;
    let err = err_handle.join().ok()?;
    Some(std::process::Output {
        status,
        stdout: out,
        stderr: err,
    })
}

fn string_out(out: &Option<std::process::Output>) -> Option<String> {
    let out = out.as_ref()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Parse an origin remote URL into "owner/repo". Handles the three shapes
/// GitHub actually serves: https, scp-like ssh, and ssh:// scheme.
pub fn parse_remote_repo(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    // https://github.com/owner/repo  |  ssh://git@github.com/owner/repo
    if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
    {
        let mut parts = rest.splitn(3, '/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return None;
        }
        return Some(format!("{owner}/{repo}"));
    }
    None
}

/// Parse `git log --pretty=%h%x1f%s%x1f%cr%x1e` into rows.
pub fn parse_git_log(raw: &str) -> Vec<Value> {
    raw.split('\u{1e}')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let mut parts = entry.split('\u{1f}');
            let hash = parts.next()?.trim();
            let subject = parts.next()?.trim();
            let when = parts.next().unwrap_or("").trim();
            if hash.is_empty() || subject.is_empty() {
                return None;
            }
            Some(json!({ "hash": hash, "subject": subject, "when": when }))
        })
        .collect()
}

/// Parse `git status --porcelain` into (first entries, total count).
pub fn parse_status_lines(raw: &str, max: usize) -> (Vec<Value>, usize) {
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let total = lines.len();
    let rows = lines
        .iter()
        .take(max)
        .map(|l| {
            let status = l.get(0..2).unwrap_or("??").trim().to_string();
            let path = l.get(3..).unwrap_or("").trim().to_string();
            json!({ "status": status, "path": path })
        })
        .collect();
    (rows, total)
}

/// Pull requests + workflow runs via gh, or an honest "no gh" payload.
fn gh_state(cwd: &str, repo: &str) -> Value {
    let gh = gh_candidates()
        .into_iter()
        .find(|p| *p == "gh" || std::path::Path::new(p).exists());
    let Some(gh) = gh else {
        return json!({ "gh_ok": false, "note": "gh not installed" });
    };
    let prs = run_with_timeout(
        gh,
        &["pr", "list", "--limit", "4", "--json", "number,title,url"],
        cwd,
        GH_TIMEOUT,
    )
    .and_then(|o| string_out(&Some(o)))
    .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let runs = run_with_timeout(
        gh,
        &[
            "run",
            "list",
            "--limit",
            "4",
            "--json",
            "displayTitle,url,status,conclusion",
        ],
        cwd,
        GH_TIMEOUT,
    )
    .and_then(|o| string_out(&Some(o)))
    .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    match (&prs, &runs) {
        (Some(_), _) | (_, Some(_)) => json!({
            "gh_ok": true, "repo": repo,
            "prs": prs.unwrap_or(Value::Array(vec![])),
            "runs": runs.unwrap_or(Value::Array(vec![])),
        }),
        _ => json!({ "gh_ok": false, "note": "gh unavailable or not authed" }),
    }
}

/// Everything the inspector's repo/github sections render, in one call.
pub fn repo_state(cwd: &str) -> Result<Value, String> {
    let branch = run_with_timeout(
        "git",
        &["rev-parse", "--abbrev-ref", "HEAD"],
        cwd,
        GIT_TIMEOUT,
    )
    .and_then(|o| string_out(&Some(o)))
    .ok_or("not a git repository (or git unavailable)")?;

    let raw = run_with_timeout("git", &["status", "--porcelain"], cwd, GIT_TIMEOUT)
        .and_then(|o| string_out(&Some(o)))
        .unwrap_or_default();
    let (dirty, dirty_total) = parse_status_lines(&raw, 8);

    let log_raw = run_with_timeout(
        "git",
        &["log", "-6", "--pretty=%h%x1f%s%x1f%cr%x1e"],
        cwd,
        GIT_TIMEOUT,
    )
    .and_then(|o| string_out(&Some(o)))
    .unwrap_or_default();
    let commits = parse_git_log(&log_raw);

    let remote = run_with_timeout("git", &["remote", "get-url", "origin"], cwd, GIT_TIMEOUT)
        .and_then(|o| string_out(&Some(o)))
        .unwrap_or_default();
    let parsed = parse_remote_repo(&remote);

    let github = match &parsed {
        Some(repo) => gh_state(cwd, repo),
        None => json!({ "gh_ok": false, "note": "no github origin remote" }),
    };
    let base = parsed
        .as_ref()
        .map(|r| format!("https://github.com/{r}"))
        .unwrap_or_default();

    Ok(json!({
        "branch": branch,
        "dirty": dirty,
        "dirty_total": dirty_total,
        "commits": commits,
        "repo": parsed,
        "urls": if parsed.is_some() { json!({
            "repo": base,
            "commits": format!("{base}/commits"),
            "actions": format!("{base}/actions"),
            "pulls": format!("{base}/pulls"),
            "issues": format!("{base}/issues"),
        }) } else { json!(null) },
        "github": github,
    }))
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn grove_git_state(cwd: String) -> Result<Value, String> {
    repo_state(&cwd)
}

// ------------------------------------------------------------ platform
// The shell runs on macOS today and must run on Windows 11 and current
// stable Linux. Every OS-flavored decision lives here, behind cfg.

fn data_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    #[cfg(target_os = "macos")]
    {
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("SpruceGroveDesktop")
    }
    #[cfg(target_os = "windows")]
    {
        std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or(home))
            .join("SpruceGroveDesktop")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(home).join(".config"));
        base.join("SpruceGroveDesktop")
    }
}

fn open_target(target: &str, is_url: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn().map(|_| ()).map_err(|e| format!("open: {e}"))
    }
    #[cfg(target_os = "windows")]
    {
        if is_url {
            Command::new("cmd")
                .args(["/C", "start", "", target])
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("start: {e}"))
        } else {
            Command::new("explorer").arg(target).spawn().map(|_| ()).map_err(|e| format!("explorer: {e}"))
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn().map(|_| ()).map_err(|e| format!("xdg-open: {e}"))
    }
}

fn gh_candidates() -> Vec<&'static str> {
    #[cfg(target_os = "macos")]
    {
        vec!["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "gh"]
    }
    #[cfg(target_os = "windows")]
    {
        vec!["gh", "C:\\Program Files\\GitHub CLI\\gh.exe"]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec!["/usr/bin/gh", "/usr/local/bin/gh", "gh"]
    }
}

/// A human-readable OS line for diagnostics: name, version, arch.
fn os_line() -> String {
    let arch = std::env::consts::ARCH;
    #[cfg(target_os = "macos")]
    {
        let sw = run_with_timeout("sw_vers", &[], "/", Duration::from_secs(3))
            .and_then(|o| string_out(&Some(o)))
            .unwrap_or_default();
        let mut version = String::from("macOS");
        let mut build = String::new();
        for line in sw.lines() {
            if line.starts_with("ProductVersion:") {
                version = format!("macOS {}", line.trim_start_matches("ProductVersion:").trim());
            }
            if line.starts_with("BuildVersion:") {
                build = line.trim_start_matches("BuildVersion:").trim().to_string();
            }
        }
        if build.is_empty() { version } else { format!("{version} ({build})") }
    }
    #[cfg(target_os = "windows")]
    {
        format!("Windows ({arch})")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let rel = run_with_timeout("uname", &["-r"], "/", Duration::from_secs(3))
            .and_then(|o| string_out(&Some(o)))
            .unwrap_or_default();
        if rel.is_empty() { format!("Linux ({arch})") } else { format!("Linux {rel} ({arch})") }
    }
}

// ------------------------------------------------------------ settings

pub fn default_settings() -> Value {
    json!({
        "version": 1,
        "default_cwd": "",
        "inspector_auto_open": false,
        "error_report_level": "errors",
        "watched_repos": [
            "t-granlund/spruce-grove-os",
            "t-granlund/spruce-grove-desktop"
        ],
        "personas": [],
        "active_persona": null
    })
}

/// Light, honest validation: object, version 1, personas well-formed.
/// The point is catching a hand-edited file early, not a schema engine.
pub fn validate_settings(v: &Value) -> Result<(), String> {
    let obj = v.as_object().ok_or("settings must be an object")?;
    if obj.get("version").and_then(Value::as_i64) != Some(1) {
        return Err("settings.version must be 1".into());
    }
    if let Some(personas) = obj.get("personas") {
        let arr = personas.as_array().ok_or("personas must be an array")?;
        for p in arr {
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .ok_or("each persona needs a string name")?;
            if name.trim().is_empty() {
                return Err("persona name cannot be empty".into());
            }
        }
    }
    Ok(())
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

#[tauri::command]
pub fn grove_settings_get() -> Value {
    match std::fs::read_to_string(settings_path()) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => v,
            Err(_) => default_settings(), // corrupt file: defaults, honestly
        },
        Err(_) => default_settings(),
    }
}

#[tauri::command]
pub fn grove_settings_set(settings: Value) -> Result<Value, String> {
    validate_settings(&settings)?;
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("data dir: {e}"))?;
    let path = settings_path();
    std::fs::write(&path, serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?)
        .map_err(|e| format!("write settings: {e}"))?;
    Ok(settings)
}

// ---------------------------------------------------------- repo access

/// Live GitHub answer to "who can touch these repos": viewerPermission per
/// repo, straight from gh (ADMIN > MAINTAIN > WRITE > TRIAGE > READ). No
/// invented roles — GitHub's own verdict for the authenticated account.
#[tauri::command]
pub fn grove_repo_access(repos: Option<Vec<String>>) -> Value {
    let repos = repos.unwrap_or_else(|| {
        default_settings()["watched_repos"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default()
    });
    let gh = gh_candidates().into_iter().find(|p| {
        *p == "gh" || std::path::Path::new(p).exists()
    });
    let Some(gh) = gh else {
        return json!({ "gh_ok": false, "note": "gh not installed", "repos": [] });
    };
    let login = run_with_timeout(gh, &["api", "user", "--jq", ".login"], "/", GH_TIMEOUT)
        .and_then(|o| string_out(&Some(o)))
        .unwrap_or_default();
    let rows: Vec<Value> = repos
        .iter()
        .map(|repo| {
            let out = run_with_timeout(
                gh,
                &["repo", "view", repo, "--json", "viewerPermission"],
                "/",
                GH_TIMEOUT,
            )
            .and_then(|o| string_out(&Some(o)))
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
            match out {
                Some(v) => json!({
                    "repo": repo,
                    "ok": true,
                    "permission": v.get("viewerPermission").cloned().unwrap_or(Value::Null),
                }),
                None => json!({ "repo": repo, "ok": false, "permission": null }),
            }
        })
        .collect();
    json!({
        "gh_ok": !login.is_empty(),
        "login": login,
        "repos": rows,
    })
}

// ---------------------------------------------------------- diagnostics

/// What the shell knows about itself, for the diagnostics tab: platform,
/// engine seams (bridge probe, boot breadcrumbs), process facts. All read
/// locally; nothing here leaves the machine.
#[tauri::command]
pub fn grove_diagnostics() -> Value {
    let tmp = std::env::temp_dir();
    let pid = std::process::id();
    let marker = |name: &str| tmp.join(name).exists();
    json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": os_line(),
        "arch": std::env::consts::ARCH,
        "pid": pid,
        "tmpdir": tmp.to_string_lossy(),
        "data_dir": data_dir().to_string_lossy(),
        "bridge_probe_acked": marker(format!("spruce-grove-bridge-probe-{pid}").as_str()),
        "boot_mainjs": marker("sg-boot-mainjs-loaded"),
        "boot_listen_ok": marker("sg-boot-listen-ok"),
    })
}

/// Open a path in Finder/manager (or reveal its parent). Paths only.
#[tauri::command]
pub fn grove_open_path(path: String) -> Result<(), String> {
    if path.contains("..") {
        return Err("path traversal is not a link".into());
    }
    open_target(&path, false)
}

/// Open a URL in the default browser. https only — no scheme games.
#[tauri::command]
pub fn grove_open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links open from the inspector".into());
    }
    open_target(&url, true)
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remote_repo_handles_all_github_shapes() {
        assert_eq!(
            parse_remote_repo("https://github.com/t-granlund/spruce-grove-desktop.git"),
            Some("t-granlund/spruce-grove-desktop".into())
        );
        assert_eq!(
            parse_remote_repo("git@github.com:t-granlund/spruce-grove-desktop.git"),
            Some("t-granlund/spruce-grove-desktop".into())
        );
        assert_eq!(
            parse_remote_repo("ssh://git@github.com/t-granlund/spruce-grove-desktop"),
            Some("t-granlund/spruce-grove-desktop".into())
        );
        assert_eq!(parse_remote_repo("https://gitlab.com/a/b.git"), None);
        assert_eq!(parse_remote_repo("https://github.com/onlyowner"), None);
    }

    #[test]
    fn parse_git_log_reads_unit_separator_rows() {
        let raw = "abc1234\u{1f}fix the thing\u{1f}2 hours ago\u{1e}\ndef5678\u{1f}init\u{1f}3 days ago\u{1e}\n";
        let rows = parse_git_log(raw);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["hash"], "abc1234");
        assert_eq!(rows[0]["subject"], "fix the thing");
        assert_eq!(rows[0]["when"], "2 hours ago");
        assert_eq!(parse_git_log(""), Vec::<Value>::new());
    }

    #[test]
    fn parse_status_lines_caps_and_counts() {
        let raw = " M src/a.rs\n?? new.txt\nA  b.txt\n";
        let (rows, total) = parse_status_lines(raw, 2);
        assert_eq!(total, 3);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["path"], "src/a.rs");
        assert_eq!(rows[1]["status"], "??");
    }

    #[test]
    fn settings_validation_enforces_shape() {
        let good = default_settings();
        assert!(validate_settings(&good).is_ok());

        let mut persona = default_settings();
        persona["personas"] = json!([{ "name": "builder", "grants": { "dictation": true } }]);
        assert!(validate_settings(&persona).is_ok());

        let mut no_version = default_settings();
        no_version["version"] = json!(2);
        assert!(validate_settings(&no_version).is_err());

        let mut empty_name = default_settings();
        empty_name["personas"] = json!([{ "name": "  " }]);
        assert!(validate_settings(&empty_name).is_err());

        let mut bad_personas = default_settings();
        bad_personas["personas"] = json!("not-an-array");
        assert!(validate_settings(&bad_personas).is_err());

        assert!(validate_settings(&json!("garbage")).is_err());
    }
}
