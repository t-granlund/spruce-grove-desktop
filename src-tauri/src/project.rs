//! Core Project View — the directory's own truth, read straight from the
//! backend, never duplicated.
//!
//! Tyler's rule: "we're in a directory; there should be a core project view
//! with the tracker of everything that's going on... a direct reference of
//! what's already in the beads default database."
//!
//! So this module is a *reader*. It shells out to `bd` (the same Dolt-backed
//! CLI the repo already uses) and lists the project's own docs. It stores
//! nothing of its own — no cache, no parallel database, no copy. If `bd` says
//! it, the pane says it; if `bd` is absent or the dir isn't a bead store, the
//! pane says that plainly instead of inventing a view.

use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

use crate::inspector::run_with_timeout;

const BD_TIMEOUT: Duration = Duration::from_secs(8);

/// Governance / process documents the project view surfaces. These are the
/// "ways of working" the repo already holds; the view is a window, not a
/// second copy. Missing files are reported as absent, never stubbed.
const DOC_CANDIDATES: &[(&str, &str)] = &[
    ("GOVERNANCE.md", "governance"),
    ("SOVEREIGNTY.md", "sovereignty"),
    ("docs/SOVEREIGNTY-EXECUTION.md", "sovereignty: execution board"),
    ("docs/SOVEREIGNTY-INFRASTRUCTURE.md", "sovereignty: infrastructure"),
    ("PLAN.md", "plan"),
    ("ETHOS.md", "ethos"),
    ("BRAND.md", "brand"),
    ("AGENTS.md", "agents / ways of working"),
    ("CHANGELOG.md", "changelog"),
    ("BUILD-LOG.md", "build log"),
    ("PROVENANCE.md", "provenance"),
    ("MASTER-CLASS.md", "master class"),
    ("docs/DOMAIN-LAUNCH.md", "domain launch"),
    ("docs/DEPENDENCY-EXIT.md", "dependency exit"),
    ("docs/COMPAT-EXIT.md", "compatibility exit"),
];

/// `bd` resolution: the uv-installed wrapper first, then PATH. Mirrors the CLI
/// resolution order in main.rs so the shell and the tracker agree.
fn bd_candidates() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        v.push(format!("{home}/.local/bin/bd"));
    }
    v.push("bd".to_string());
    v
}

/// Run `bd` with `args` in `cwd`. Returns (program_used, output) or None.
fn bd_run(cwd: &str, args: &[&str]) -> Option<(String, std::process::Output)> {
    for program in bd_candidates() {
        let is_path = program.contains('/');
        if is_path && !Path::new(&program).exists() {
            continue;
        }
        if let Some(out) = run_with_timeout(&program, args, cwd, BD_TIMEOUT) {
            return Some((program, out));
        }
    }
    None
}

/// Does this directory hold a beads store? (`.beads/` present.) The default
/// database lives there; presence is the honest gate for "this is a tracker
/// directory."
pub fn is_beads_dir(cwd: &str) -> bool {
    Path::new(cwd).join(".beads").is_dir()
}

/// Read the directory's beads issue tracker, straight from `bd`.
pub fn project_state(cwd: &str) -> Value {
    if !is_beads_dir(cwd) {
        return json!({
            "is_beads": false,
            "note": "no .beads/ in this directory — not a bead store",
        });
    }

    let Some((bd_program, out)) = bd_run(cwd, &["list", "--all", "--format", "json"]) else {
        return json!({
            "is_beads": true,
            "bd_found": false,
            "note": "bd CLI not found (looked in ~/.local/bin and PATH)",
        });
    };

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return json!({
            "is_beads": true,
            "bd_found": true,
            "ok": false,
            "note": if err.is_empty() { "bd list failed".to_string() } else { err },
        });
    }

    let raw = String::from_utf8_lossy(&out.stdout);
    let issues: Vec<Value> = serde_json::from_str(raw.trim()).unwrap_or_default();

    // Counts by status, plus the ready set the room cares about most.
    let mut open = 0usize;
    let mut in_progress = 0usize;
    let mut blocked = 0usize;
    let mut closed = 0usize;
    let mut deferred = 0usize;
    let mut rows: Vec<Value> = Vec::new();
    for issue in &issues {
        let status = issue.get("status").and_then(Value::as_str).unwrap_or("");
        match status {
            "open" => open += 1,
            "in_progress" => in_progress += 1,
            "blocked" => blocked += 1,
            "closed" => closed += 1,
            "deferred" => deferred += 1,
            _ => {}
        }
        rows.push(json!({
            "id": issue.get("id").cloned().unwrap_or(Value::Null),
            "title": issue.get("title").cloned().unwrap_or(Value::Null),
            "status": status,
            "priority": issue.get("priority").cloned().unwrap_or(Value::Null),
            "issue_type": issue.get("issue_type").cloned().unwrap_or(Value::Null),
            "updated_at": issue.get("updated_at").cloned().unwrap_or(Value::Null),
        }));
    }
    // Sort: in-progress, blocked, open, then closed/deferred; P ascending.
    let rank = |s: &str| match s {
        "in_progress" => 0,
        "blocked" => 1,
        "open" => 2,
        "deferred" => 3,
        "closed" => 4,
        _ => 5,
    };
    rows.sort_by(|a, b| {
        let sa = rank(a.get("status").and_then(Value::as_str).unwrap_or(""));
        let sb = rank(b.get("status").and_then(Value::as_str).unwrap_or(""));
        sa.cmp(&sb).then_with(|| {
            let pa = a.get("priority").and_then(Value::as_i64).unwrap_or(9);
            let pb = b.get("priority").and_then(Value::as_i64).unwrap_or(9);
            pa.cmp(&pb)
        })
    });

    // Project docs — the governance / plan / pipeline surface.
    let docs: Vec<Value> = DOC_CANDIDATES
        .iter()
        .map(|(rel, label)| {
            let present = Path::new(cwd).join(rel).is_file();
            json!({ "path": rel, "label": label, "present": present })
        })
        .collect();

    json!({
        "is_beads": true,
        "bd_found": true,
        "bd_program": bd_program,
        "ok": true,
        "cwd": cwd,
        "counts": {
            "open": open, "in_progress": in_progress, "blocked": blocked,
            "closed": closed, "deferred": deferred, "total": issues.len(),
        },
        "issues": rows,
        "docs": docs,
    })
}

// ---------------------------------------------------------------- commands

/// The Core Project View payload: the directory's tracker (a direct read of
/// the beads default database) plus its governance/plan documents.
#[tauri::command]
pub fn grove_project_state(cwd: String) -> Value {
    project_state(&cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_beads_dir_reports_honestly() {
        let v = project_state("/");
        assert_eq!(v.get("is_beads").and_then(Value::as_bool), Some(false));
        assert!(v.get("note").is_some());
    }

    #[test]
    fn doc_candidates_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (rel, _) in DOC_CANDIDATES {
            assert!(seen.insert(*rel), "duplicate doc candidate: {rel}");
        }
    }
}
