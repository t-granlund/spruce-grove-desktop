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
    let gh = ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "gh"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists() || *p == "gh");
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

/// Open a path in Finder/manager (or reveal its parent). Paths only.
#[tauri::command]
pub fn grove_open_path(path: String) -> Result<(), String> {
    if path.contains("..") {
        return Err("path traversal is not a link".into());
    }
    Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("open: {e}"))?;
    Ok(())
}

/// Open a URL in the default browser. https only — no scheme games.
#[tauri::command]
pub fn grove_open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links open from the inspector".into());
    }
    Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("open: {e}"))?;
    Ok(())
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
}
