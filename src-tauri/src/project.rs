//! Core Project View — the directory's own truth, read straight from the
//! backend, never duplicated.
//!
//! The rule: "we're in a directory; there should be a core project view
//! with the tracker of everything that's going on... a direct reference of
//! what's already in the beads default database."
//!
//! So this module is a *reader*. It shells out to `bd` (the same Dolt-backed
//! CLI the repo already uses) and lists the project's own docs. It stores
//! nothing of its own — no cache, no parallel database, no copy. If `bd` says
//! it, the pane says it; if `bd` is absent or the dir isn't a bead store, the
//! pane says that plainly instead of inventing a view.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inspector::run_with_timeout;

const BD_TIMEOUT: Duration = Duration::from_secs(8);

/// Governance / process documents the project view surfaces. These are the
/// "ways of working" the repo already holds; the view is a window, not a
/// second copy. Missing files are reported as absent, never stubbed.
///
/// `(name, label, family)` — `family` lets one category hold several docs
/// without the view treating them as rivals for a single slot.
const DOC_CANDIDATES: &[(&str, &str, &str)] = &[
    ("GOVERNANCE.md", "governance", "governance"),
    ("SOVEREIGNTY.md", "sovereignty", "governance"),
    (
        "docs/SOVEREIGNTY-EXECUTION.md",
        "sovereignty: execution board",
        "governance",
    ),
    (
        "docs/SOVEREIGNTY-INFRASTRUCTURE.md",
        "sovereignty: infrastructure",
        "governance",
    ),
    ("PLAN.md", "plan", "plan"),
    ("ETHOS.md", "ethos", "ethos"),
    ("BRAND.md", "brand", "brand"),
    ("AGENTS.md", "agents / ways of working", "ways of working"),
    ("CHANGELOG.md", "changelog", "history"),
    ("BUILD-LOG.md", "build log", "history"),
    ("PROVENANCE.md", "provenance", "history"),
    ("MASTER-CLASS.md", "master class", "ways of working"),
    ("docs/DOMAIN-LAUNCH.md", "domain launch", "operations"),
    ("docs/DEPENDENCY-EXIT.md", "dependency exit", "operations"),
    ("docs/COMPAT-EXIT.md", "compatibility exit", "operations"),
];

/// Suffixes that mark a document as part of the governance surface, wherever
/// it lives. This is the *convention*: a new `docs/ADR-007-listen-ports.md`
/// or `docs/BRD.md` is surfaced by NAMING, with no code change here — which is
/// the whole point, since a fixed candidate list means every new governance
/// doc needs a Rust edit before anyone can see it.
const DOC_SUFFIXES: &[&str] = &[
    "ADR.md",
    "BRD.md",
    "GOVERNANCE.md",
    "PIPELINE.md",
    "PLAN.md",
    "PROVENANCE.md",
    "RFC.md",
    "SOVEREIGNTY.md",
];

/// Where convention-discovered documents are looked for, repo-root relative.
/// Shallow and explicit: a deep walk would pull test fixtures and vendored
/// trees into the governance surface.
const DOC_SEARCH_DIRS: &[&str] = &["", "docs", "docs/adr", "docs/rfc"];

/// Directories never worth searching for governance docs.
const DOC_SKIP_DIRS: &[&str] = &[
    ".git",
    ".venv",
    "node_modules",
    "target",
    "site-packages",
    "dist",
    "build",
];

/// Documents the repo's own naming convention marks as governance, sorted.
///
/// Searches a shallow, explicit set of directories rather than walking the
/// tree, so vendored code and test fixtures can never masquerade as governance.
/// A missing directory is simply no results.
fn discover_governance_docs(cwd: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for dir in DOC_SEARCH_DIRS {
        let base = if dir.is_empty() {
            PathBuf::from(cwd)
        } else {
            Path::new(cwd).join(dir)
        };
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue; // absent directory is not an error
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".md") || !entry.path().is_file() {
                continue;
            }
            let upper = name.to_uppercase();
            let matched = DOC_SUFFIXES.iter().any(|suffix| {
                // Accept both the bare form (`ADR.md`) and the numbered form
                // (`ADR-007-listen-ports.md`), which is how these are usually
                // filed. Compare stems, so the ".md" is stripped once, from
                // the right place, rather than uppercased and matched against.
                let stem = suffix.trim_end_matches(".md").to_uppercase();
                let file_stem = upper.trim_end_matches(".MD");
                file_stem == stem || file_stem.starts_with(&format!("{stem}-"))
            });
            if !matched {
                continue;
            }
            let rel = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            found.push(rel);
        }
    }
    found.sort();
    found.dedup();
    found
}

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

    // Project docs — the governance / plan / pipeline surface. The known
    // candidates first, then anything the repo's own naming convention marks
    // as governance, so a new ADR/BRD/PIPELINE doc appears on its own.
    let mut docs: Vec<Value> = DOC_CANDIDATES
        .iter()
        .map(|(rel, label, family)| {
            let present = Path::new(cwd).join(rel).is_file();
            json!({
                "path": rel, "label": label, "family": family,
                "present": present, "discovered": false,
            })
        })
        .collect();

    let known: std::collections::HashSet<&str> =
        DOC_CANDIDATES.iter().map(|(rel, _, _)| *rel).collect();
    for rel in discover_governance_docs(cwd) {
        if known.contains(rel.as_str()) {
            continue; // already listed above, with a curated label
        }
        let label = rel
            .rsplit('/')
            .next()
            .unwrap_or(rel.as_str())
            .trim_end_matches(".md")
            .replace(['-', '_'], " ")
            .to_lowercase();
        let family = if rel.to_lowercase().contains("adr") {
            "decisions"
        } else if rel.to_lowercase().contains("brd") {
            "requirements"
        } else if rel.to_lowercase().contains("pipeline") {
            "pipeline"
        } else {
            "governance"
        };
        docs.push(json!({
            "path": rel, "label": label, "family": family,
            "present": true, "discovered": true,
        }));
    }

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
        for (rel, _, _) in DOC_CANDIDATES {
            assert!(seen.insert(*rel), "duplicate doc candidate: {rel}");
        }
    }

    #[test]
    fn every_candidate_declares_a_family() {
        for (rel, label, family) in DOC_CANDIDATES {
            assert!(!family.is_empty(), "{rel} has no family");
            assert!(!label.is_empty(), "{rel} has no label");
        }
    }

    /// Build a temp tree without pulling in a dev-dependency.
    fn temp_tree(files: &[&str]) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "grove-project-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for rel in files {
            let path = base.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "# doc").unwrap();
        }
        base
    }

    #[test]
    fn convention_finds_numbered_adrs_and_brds() {
        let base = temp_tree(&[
            "docs/ADR-007-listen-ports.md",
            "docs/BRD.md",
            "docs/PIPELINE.md",
            "docs/random-notes.md", // must NOT be picked up
        ]);
        let found = discover_governance_docs(base.to_str().unwrap());
        assert!(found.contains(&"docs/ADR-007-listen-ports.md".to_string()));
        assert!(found.contains(&"docs/BRD.md".to_string()));
        assert!(found.contains(&"docs/PIPELINE.md".to_string()));
        assert!(
            !found.iter().any(|f| f.contains("random-notes")),
            "a non-governance doc leaked in: {found:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn convention_ignores_directories_that_do_not_exist() {
        let base = temp_tree(&["docs/ADR.md"]);
        // docs/adr and docs/rfc are absent; that must simply yield nothing.
        assert_eq!(
            discover_governance_docs(base.to_str().unwrap()),
            vec!["docs/ADR.md".to_string()]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn convention_does_not_recurse_into_vendored_trees() {
        let base = temp_tree(&[
            "node_modules/pkg/BRD.md",
            "target/debug/ADR.md",
            "docs/PLAN.md",
        ]);
        let found = discover_governance_docs(base.to_str().unwrap());
        assert_eq!(found, vec!["docs/PLAN.md".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn convention_results_are_sorted_and_deduped() {
        let base = temp_tree(&["docs/BRD.md", "docs/ADR.md", "PLAN.md"]);
        let found = discover_governance_docs(base.to_str().unwrap());
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted);
        let mut deduped = found.clone();
        deduped.dedup();
        assert_eq!(found, deduped);
        let _ = std::fs::remove_dir_all(&base);
    }
}
