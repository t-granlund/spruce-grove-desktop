//! Agent action ledger — the "receipts" half of containment + receipts.
//!
//! Every action the agent takes through the desktop shell is recorded here as
//! one append-only JSONL row in the app data dir: what, when, where, outcome.
//! The inspector renders the tail; nothing else reads it. This is a *receipt*,
//! not a gate — it never blocks an action, it only refuses to forget one.
//!
//! Honest scope: the shell sees the actions that flow through it (ACP tool
//! calls, permission decisions, prompt sends, cancellations). It does NOT
//! inspect files the CLI writes internally. We record what we actually know
//! and say so, rather than implying a coverage we don't have.

use serde_json::{json, Value};

use crate::inspector::{chrono_now, data_dir};

const LEDGER_FILE: &str = "action-ledger.jsonl";
/// Keep the ledger bounded: a long-lived install must not grow without limit.
/// 2000 rows is weeks of heavy use; the tail is what the UI shows anyway.
const LEDGER_KEEP: usize = 2000;

/// One recorded action. `kind` is the coarse class (prompt | tool | permission
/// | file | shell | cancel | error), `summary` is the human line, `outcome` is
/// free text (e.g. "completed", "auto-allowed", "denied", "failed").
pub(crate) struct Action<'a> {
    pub kind: &'a str,
    pub cwd: &'a str,
    pub summary: &'a str,
    pub outcome: &'a str,
}

/// Append one row. Best-effort: a ledger write must never break the action it
/// records, so every failure path returns the error but callers may ignore it.
pub(crate) fn record(action: Action<'_>) -> Result<(), String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("ledger dir: {e}"))?;
    let path = dir.join(LEDGER_FILE);

    // Read-modify-write to enforce the cap. Cheap at this size and keeps the
    // file a plain JSONL trail a human can `cat`/`tail -f` without tooling.
    let mut rows: Vec<Value> = std::fs::read_to_string(&path)
        .map(|raw| {
            raw.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default();

    rows.push(json!({
        "at": chrono_now(),
        "kind": clamp(action.kind, 24),
        "cwd": clamp(action.cwd, 160),
        "summary": clamp(action.summary, 240),
        "outcome": clamp(action.outcome, 80),
    }));
    if rows.len() > LEDGER_KEEP {
        let drop = rows.len() - LEDGER_KEEP;
        rows.drain(0..drop);
    }
    let mut out = rows
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    out.push('\n');
    std::fs::write(&path, out).map_err(|e| format!("ledger write: {e}"))
}

/// The most recent `limit` rows, newest last (the UI reverses for display).
pub(crate) fn tail(limit: usize) -> Vec<Value> {
    let path = data_dir().join(LEDGER_FILE);
    let mut rows: Vec<Value> = std::fs::read_to_string(&path)
        .map(|raw| {
            raw.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default();
    if rows.len() > limit {
        let drop = rows.len() - limit;
        rows.drain(0..drop);
    }
    rows
}

fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------- commands

/// Record an action from the webview (ACP tool calls, permission decisions,
/// prompt sends, cancellations). The shell is the single writer so the file
/// stays append-only and well-ordered.
#[tauri::command]
pub fn grove_ledger_record(
    kind: String,
    cwd: String,
    summary: String,
    outcome: Option<String>,
) -> Result<(), String> {
    record(Action {
        kind: &kind,
        cwd: &cwd,
        summary: &summary,
        outcome: outcome.as_deref().unwrap_or(""),
    })
}

/// Read the ledger tail for the inspector.
#[tauri::command]
pub fn grove_ledger_tail(limit: Option<usize>) -> Value {
    let limit = limit.unwrap_or(120).min(LEDGER_KEEP);
    json!({ "entries": tail(limit), "path": data_dir().join(LEDGER_FILE).to_string_lossy() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_truncates_with_ellipsis() {
        assert_eq!(clamp("abc", 10), "abc");
        assert_eq!(clamp("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn clamp_counts_chars_not_bytes() {
        // multibyte must not split a codepoint
        let s = "héllo wörld";
        assert_eq!(clamp(s, 5).chars().count(), 5);
    }
}
