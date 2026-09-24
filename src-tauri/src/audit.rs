//! Self-audit surface — the "floor" half of containment + receipts.
//!
//! `csp_guard()` / `cap_guard()` in the test suite already prove the packaged
//! CSP and capability allowlist as *test* receipts. This module surfaces the
//! same two facts as a *runtime* receipt, so the diagnostics pane can show a
//! person what the app is actually constrained by, right now.
//!
//! Both are embedded at compile time from the real config files, so the pane
//! can never drift from what shipped: if someone loosens the CSP or widens the
//! capability surface, this renders the loosened values, and the guards in CI
//! fail on the same commit.

use serde_json::{json, Value};

/// The packaged CSP, compiled in from the shipping config.
const PACKAGED_CONF: &str = include_str!("../tauri.conf.json");
/// The capability surface, compiled in from the shipping config.
const CAPABILITY: &str = include_str!("../capabilities/default.json");

/// Directives whose absence means the floor has been lowered. Mirrors
/// `CSP_HARDENING` in tests/test_desktop_ui.py so test and runtime agree.
const REQUIRED_HARDENING: &[&str] = &[
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
    "frame-ancestors 'none'",
];

/// Pull the `csp` string out of tauri.conf.json without a schema dependency.
fn csp_directives() -> (String, Vec<String>) {
    let conf: Value = serde_json::from_str(PACKAGED_CONF).unwrap_or(Value::Null);
    let csp = conf
        .get("app")
        .and_then(|a| a.get("security"))
        .and_then(|s| s.get("csp"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let directives = csp
        .split(';')
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect();
    (csp, directives)
}

/// The capability file's window list + permission allowlist.
fn capability_surface() -> (Vec<String>, Vec<String>) {
    let cap: Value = serde_json::from_str(CAPABILITY).unwrap_or(Value::Null);
    let windows = cap
        .get("windows")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let permissions = cap
        .get("permissions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    (windows, permissions)
}

/// The full self-audit payload: security floor + a verdict on whether the
/// floor is intact, with the specific missing directives called out.
pub fn self_audit() -> Value {
    let (csp, directives) = csp_directives();
    let missing: Vec<&str> = REQUIRED_HARDENING
        .iter()
        .filter(|d| !csp.contains(**d))
        .copied()
        .collect();
    let (windows, permissions) = capability_surface();
    let wildcards: Vec<&String> = permissions.iter().filter(|p| p.contains('*')).collect();

    let intact = missing.is_empty() && wildcards.is_empty() && windows == ["main"];

    json!({
        "csp": csp,
        "csp_directives": directives,
        "required_hardening": REQUIRED_HARDENING,
        "missing_hardening": missing,
        "capability": {
            "windows": windows,
            "permissions": permissions,
            "wildcards": wildcards,
        },
        "floor_intact": intact,
        "verdict": if intact {
            "floor intact — CSP hardening present, capability surface least-privilege (one window, no wildcards)"
        } else if !missing.is_empty() {
            "FLOOR LOWERED — CSP hardening directives missing"
        } else if !wildcards.is_empty() {
            "FLOOR LOWERED — capability permission uses a wildcard"
        } else {
            "FLOOR LOWERED — capability windows widened beyond 'main'"
        },
    })
}

// ---------------------------------------------------------------- command

#[tauri::command]
pub fn grove_self_audit() -> Value {
    self_audit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_floor_is_intact() {
        let v = self_audit();
        assert_eq!(
            v.get("floor_intact").and_then(Value::as_bool),
            Some(true),
            "self-audit should report an intact floor on the shipped config: {v}"
        );
    }

    #[test]
    fn every_hardening_directive_is_present_in_csp() {
        let (csp, _) = csp_directives();
        for d in REQUIRED_HARDENING {
            assert!(csp.contains(d), "packaged CSP missing {d}");
        }
    }

    #[test]
    fn capability_is_one_window_no_wildcards() {
        let (windows, permissions) = capability_surface();
        assert_eq!(windows, vec!["main".to_string()]);
        for p in permissions {
            assert!(!p.contains('*'), "wildcard permission: {p}");
        }
    }
}
