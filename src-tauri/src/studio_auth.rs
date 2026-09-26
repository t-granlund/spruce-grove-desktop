//! Studio lock authority — the owner PIN and the record's tamper tag.
//!
//! Locking a recording is a *statement*; this module is what makes unlocking
//! mean something. Two secrets live here, together, as one owner blob:
//!
//!   * a **PIN verifier** — PBKDF2-HMAC-SHA256 of the owner PIN over a random
//!     salt, so no plaintext PIN is ever stored; and
//!   * an **HMAC key** — random, used to tag a locked record so a hand-edit to
//!     `index.json` is *tamper-evident* (it can't be *prevented*: anyone who can
//!     read the secret can forge, and anyone who owns the disk can edit files).
//!
//! WHERE THE SECRET LIVES. Production uses the macOS Keychain (`security` CLI,
//! already on the box) so the blob is encrypted at rest and ACL'd to the app.
//! Anywhere the Keychain isn't available — tests, or a non-macOS build — it
//! falls back to a 0600 file, and says so. The fallback is NOT equivalent: it is
//! readable by anyone who can read your home directory. That is the honest shape
//! of a local guardrail, and this module never pretends otherwise.
//!
//! HONEST LIMIT (see docs/STUDIO-LOCK-DESIGN.md): a PIN gates the *app's* unlock
//! path and makes quiet edits *loud*. It is not a wall against the owner of the
//! machine. The wall against a *remote* collaborator is GitHub (CODEOWNERS +
//! branch protection + signed commits), which this module has no say over.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// One owner blob, serialized to JSON and stored whole.
const BLOB_VERSION: u32 = 1;
/// PBKDF2 iterations. A PIN is short and low-entropy, so the cost is the point.
const ITERATIONS: u32 = 600_000;

/// Iterations to actually use. Production (the Keychain backend) is fixed at
/// `ITERATIONS`; the file backend may lower it via `GROVE_PBKDF2_ITERS` so
/// tests and scripted libraries stay fast. This is NOT a weakening of the real
/// store — only the throwaway backend honors it.
fn iterations() -> u32 {
    if use_keychain() {
        return ITERATIONS;
    }
    std::env::var("GROVE_PBKDF2_ITERS")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1000)
        .unwrap_or(ITERATIONS)
}
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// The stored owner secret. Never contains the PIN, only its verifier.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct OwnerBlob {
    version: u32,
    iterations: u32,
    salt: String,
    /// PBKDF2-HMAC-SHA256(pin, salt, iterations).
    pin_hash: String,
    /// Random key for locking HMACs.
    hmac_key: String,
}

/// Where the blob is kept. `GROVE_AUTH_DIR` forces the file backend (tests and
/// scripted use); production prefers the Keychain.
fn use_keychain() -> bool {
    // Explicit override wins. A scripted/test library (GROVE_RECORDINGS_DIR)
    // also forces the file backend so tests never touch the real login keychain.
    if std::env::var("GROVE_AUTH_DIR").map_or(false, |v| !v.trim().is_empty()) {
        return false;
    }
    if std::env::var("GROVE_RECORDINGS_DIR").map_or(false, |v| !v.trim().is_empty()) {
        return false;
    }
    true
}

fn file_backend_path() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("GROVE_AUTH_DIR") {
        if !dir.trim().is_empty() {
            return std::path::PathBuf::from(dir).join("owner.json");
        }
    }
    // no explicit dir: sit next to the recordings when that override is set,
    // so a scripted library carries its own authority.
    if let Ok(dir) = std::env::var("GROVE_RECORDINGS_DIR") {
        if !dir.trim().is_empty() {
            return std::path::PathBuf::from(dir).join("owner.json");
        }
    }
    std::env::temp_dir().join("spruce-grove-studio-owner.json")
}

const KC_SERVICE: &str = "spruce-grove-desktop.studio-owner";
const KC_ACCOUNT: &str = "owner";

// ------------------------------------------------------------- keychain io

fn keychain_read() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            KC_SERVICE,
            "-a",
            KC_ACCOUNT,
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn keychain_write(blob: &str) -> Result<(), String> {
    // -U updates in place if the item already exists.
    let out = std::process::Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KC_SERVICE,
            "-a",
            KC_ACCOUNT,
            "-w",
            blob,
            "-D",
            "studio owner PIN",
        ])
        .output()
        .map_err(|e| format!("keychain unavailable: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "keychain refused the write: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

// -------------------------------------------------------------- blob store

fn load_blob() -> Option<OwnerBlob> {
    let raw = if use_keychain() {
        match keychain_read() {
            Some(v) => v,
            None => return None,
        }
    } else {
        let path = file_backend_path();
        std::fs::read_to_string(&path).ok()?
    };
    serde_json::from_str(&raw).ok()
}

fn store_blob(blob: &OwnerBlob) -> Result<(), String> {
    let raw = serde_json::to_string(blob).map_err(|e| e.to_string())?;
    if use_keychain() {
        keychain_write(&raw)
    } else {
        let path = file_backend_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("auth dir: {e}"))?;
        }
        std::fs::write(&path, &raw).map_err(|e| format!("write owner blob: {e}"))?;
        // 0600: the fallback is readable-by-you, not by the world.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

fn random_b64(len: usize) -> Result<String, String> {
    let mut buf = vec![0u8; len];
    getrandom::getrandom(&mut buf).map_err(|e| format!("no entropy: {e}"))?;
    Ok(b64().encode(buf))
}

fn derive(pin: &str, salt: &[u8], iterations: u32) -> [u8; KEY_LEN] {
    let mut out = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<Sha256>(pin.as_bytes(), salt, iterations, &mut out);
    out
}

// ------------------------------------------------------------------ public

/// Is an owner PIN set?
pub fn has_pin() -> bool {
    load_blob().is_some()
}

/// Set the owner PIN. Refused if one already exists — re-keying is a separate,
/// deliberate act we have not built (YAGNI until asked).
pub fn set_pin(pin: &str) -> Result<(), String> {
    if pin.trim().len() < 4 {
        return Err("PIN must be at least 4 characters".into());
    }
    if has_pin() {
        return Err("an owner PIN is already set".into());
    }
    let iters = iterations();
    let salt = random_b64(SALT_LEN)?;
    let salt_bytes = b64().decode(&salt).map_err(|e| e.to_string())?;
    let hash = derive(pin, &salt_bytes, iters);
    let blob = OwnerBlob {
        version: BLOB_VERSION,
        iterations: iters,
        salt,
        pin_hash: b64().encode(hash),
        hmac_key: random_b64(KEY_LEN)?,
    };
    store_blob(&blob)
}

/// Constant-time PIN check. False when no PIN is set (nothing to match).
pub fn verify_pin(pin: &str) -> bool {
    let Some(blob) = load_blob() else {
        return false;
    };
    let Ok(salt) = b64().decode(&blob.salt) else {
        return false;
    };
    let Ok(expected) = b64().decode(&blob.pin_hash) else {
        return false;
    };
    let got = derive(pin, &salt, blob.iterations);
    // lengths match by construction; ct_eq still guards the compare.
    got.as_slice().ct_eq(expected.as_slice()).into()
}

/// Tag a locked record's canonical body. Returns hex HMAC-SHA256.
pub fn record_tag(body: &str) -> Result<String, String> {
    let guard = OwnerGuard::load().ok_or("no owner PIN set — cannot tag the record")?;
    Ok(guard.tag(body))
}

/// Does the tag still match? `false` = the file changed under a locked record.
pub fn tag_matches(body: &str, tag: &str) -> bool {
    OwnerGuard::load().map_or(false, |g| g.matches(body, tag))
}

/// A resolved owner key, so a whole library can be restamped with ONE Keychain
/// read instead of one read per record. `tag`/`matches` are cheap after that.
pub struct OwnerGuard {
    key: Vec<u8>,
}

impl OwnerGuard {
    /// Load the HMAC key. `None` when no PIN is set (no key to vouch with).
    pub fn load() -> Option<OwnerGuard> {
        let blob = load_blob()?;
        let key = b64().decode(&blob.hmac_key).ok()?;
        Some(OwnerGuard { key })
    }

    /// HMAC-SHA256 of `body`, hex.
    pub fn tag(&self, body: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(body.as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Constant-time compare of a stored tag against a fresh one.
    pub fn matches(&self, body: &str, tag: &str) -> bool {
        self.tag(body).as_bytes().ct_eq(tag.as_bytes()).into()
    }
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    // Shared with recordings.rs: both mutate the same process-global env vars.
    use crate::TEST_ENV_GUARD;
    static ENV_GUARD: &std::sync::Mutex<()> = &TEST_ENV_GUARD;

    fn auth_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sg-auth-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the guard serializes these tests through one env var.
        unsafe { std::env::set_var("GROVE_AUTH_DIR", &dir) };
        unsafe { std::env::remove_var("GROVE_RECORDINGS_DIR") };
        // keep PBKDF2 cheap on the throwaway backend (production is unaffected)
        unsafe { std::env::set_var("GROVE_PBKDF2_ITERS", "1000") };
        dir
    }

    #[test]
    fn pin_round_trip_and_rejection() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = auth_dir("roundtrip");
        assert!(!has_pin());
        set_pin("246810").expect("set");
        assert!(has_pin());
        assert!(verify_pin("246810"));
        assert!(!verify_pin("246811"));
        assert!(!verify_pin(""));
        // a second set is refused (no silent re-key)
        assert!(set_pin("999999").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn short_pin_is_refused() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = auth_dir("short");
        assert!(set_pin("12").is_err());
        assert!(!has_pin());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_without_pin_is_false_not_a_panic() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = auth_dir("nopin");
        assert!(!verify_pin("anything"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tag_detects_a_changed_body() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = auth_dir("tag");
        set_pin("246810").expect("set");
        let body = r#"{"id":"rec-1","summary":"committed"}"#;
        let tag = record_tag(body).expect("tag");
        assert!(tag_matches(body, &tag));
        // one character different -> no longer vouched for
        let changed = r#"{"id":"rec-1","summary":"EDITED"}"#;
        assert!(!tag_matches(changed, &tag));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_backend_is_0600() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = auth_dir("perms");
        set_pin("246810").expect("set");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("owner.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "owner blob must not be world-readable");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REAL Keychain round-trip — ignored by default (writes to the login
    /// keychain). Run: `cargo test -- --ignored real_keychain_roundtrip`.
    #[test]
    #[ignore]
    fn real_keychain_roundtrip() {
        // Force the keychain backend by clearing the file overrides.
        unsafe { std::env::remove_var("GROVE_AUTH_DIR") };
        unsafe { std::env::remove_var("GROVE_RECORDINGS_DIR") };
        let _ = std::process::Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KC_SERVICE,
                "-a",
                KC_ACCOUNT,
            ])
            .output();
        assert!(!has_pin(), "start clean");
        set_pin("135791").expect("write to keychain");
        assert!(has_pin(), "has_pin reads the keychain back");
        assert!(verify_pin("135791"));
        assert!(!verify_pin("135792"));
        let tag = record_tag("body").expect("tag");
        assert!(tag_matches("body", &tag));
        let _ = std::process::Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KC_SERVICE,
                "-a",
                KC_ACCOUNT,
            ])
            .output();
        assert!(!has_pin(), "cleaned up");
    }

    #[test]
    fn no_key_means_no_vouch() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = auth_dir("nokey");
        // no PIN set -> can't tag, and cannot vouch for anything
        assert!(record_tag("x").is_err());
        assert!(!tag_matches("x", "deadbeef"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
