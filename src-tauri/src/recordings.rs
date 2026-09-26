//! The recording library: a session of takes, kept on disk.
//!
//! A **recording** is one editing session — a name, a summary, and an ordered
//! list of **takes** (each an audio blob plus its transcript). A take can be
//! added, re-recorded, transcribed again, reordered, renamed, or deleted, and
//! the summary is explicitly **locked** once the operator says the session is
//! final. Locking is a statement, not a permission system: the app still
//! refetches whatever the operator asks for, but the UI treats a locked
//! recording as read-only and the overview can then show what was actually
//! committed.
//!
//! Everything lives under `~/.spruce_grove/desktop/recordings/`:
//!
//! ```text
//! recordings/
//!   index.json                       # every recording's metadata
//!   <id>/take-000.webm               # the audio blobs, in take order
//!   <id>/final.wav                   # written by splice()
//! ```
//!
//! The desktops never re-implement audio: `splice()` shells out to ffmpeg's
//! concat demuxer (the same lossless PCM stitch Mockingbird uses).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One captured take: audio + what we made of it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Take {
    /// File name inside the recording's directory (never a path).
    pub file: String,
    /// Container hint from the recorder, for honest extension naming.
    pub mime: String,
    pub created_ms: u64,
    /// Seconds, as reported by the recorder when it stopped.
    pub duration_s: f64,
    /// The transcript, editable in place.
    pub transcript: String,
}

/// A named editing session — an ordered list of takes plus its summary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub name: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// The operator's overall summary of the session.
    pub summary: String,
    /// Locked means "this is what was committed" — the UI goes read-only.
    pub locked: bool,
    /// HMAC of the committed body, written when the record is locked. A locked
    /// record whose tag no longer matches was edited outside the app.
    #[serde(default)]
    pub commit_tag: Option<String>,
    /// Derived on load, never persisted: is a locked record's tag stale? True
    /// means "someone changed the file since it was committed" (tamper-evident,
    /// not tamper-proof — see studio_auth.rs).
    #[serde(default, skip_deserializing)]
    pub tampered: bool,
    pub takes: Vec<Take>,
}

impl Recording {
    /// The bytes the commit tag covers: everything that IS the record. Excludes
    /// the tag itself and the derived flag, so the HMAC is stable.
    fn commit_body(&self) -> String {
        let view = serde_json::json!({
            "id": self.id,
            "name": self.name,
            "summary": self.summary,
            "created_ms": self.created_ms,
            "takes": self.takes,
        });
        view.to_string()
    }

    /// Recompute `tampered`. Only a *locked* record under an owner PIN can be
    /// tampered with: with no PIN, "locked" is pure UI state and we claim
    /// nothing. Under a PIN, a locked record whose tag is missing or stale
    /// means the file was edited outside the app. `guard` is resolved once by
    /// the caller so a whole index costs one Keychain read, not one per record.
    fn restamp(&mut self, guard: Option<&crate::studio_auth::OwnerGuard>) {
        self.tampered = if !self.locked {
            false
        } else {
            match (guard, self.commit_tag.as_deref()) {
                (Some(g), Some(tag)) => !g.matches(&self.commit_body(), tag),
                // locked under a PIN but never vouched for
                (Some(_), None) => true,
                // no PIN: no key to vouch with, claim nothing
                (None, _) => false,
            }
        };
    }
}

/// A partial update; every field is optional so one command covers the lot.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Patch {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub locked: Option<bool>,
    /// Replace one take's transcript.
    pub take_index: Option<usize>,
    pub transcript: Option<String>,
}

/// Where the library lives. `GROVE_RECORDINGS_DIR` wins when set so tests (and
/// anyone scripting the desktop) can point the library somewhere disposable
/// without touching `$HOME`.
fn root() -> PathBuf {
    if let Ok(dir) = std::env::var("GROVE_RECORDINGS_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home)
        .join(".spruce_grove")
        .join("desktop")
        .join("recordings")
}

fn index_path() -> PathBuf {
    root().join("index.json")
}

/// A recording id must be a plain token: it becomes a directory name, and a
/// crafted id must never be able to step outside the library root.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A take file must be a bare file name — no separators, no traversal.
fn valid_take_file(file: &str) -> bool {
    !file.is_empty()
        && file.len() <= 128
        && !file.contains('/')
        && !file.contains('\\')
        && file != ".."
        && file != "."
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mint_id() -> String {
    format!("rec-{:x}", now_ms())
}

fn load_index() -> Result<Vec<Recording>, String> {
    let path = index_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read index: {e}"))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut all: Vec<Recording> =
        serde_json::from_str(&text).map_err(|e| format!("index.json is not valid: {e}"))?;
    // Freshly recompute tamper state on every load: a locked record read back
    // from disk is only trusted if its tag still matches the body. Resolve the
    // owner key ONCE for the whole index (a Keychain read is a subprocess).
    let guard = crate::studio_auth::OwnerGuard::load();
    for rec in &mut all {
        rec.restamp(guard.as_ref());
    }
    Ok(all)
}

/// Write the index atomically: a crash mid-write must never eat the library.
fn save_index(all: &[Recording]) -> Result<(), String> {
    let dir = root();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create library: {e}"))?;
    let text = serde_json::to_string_pretty(all).map_err(|e| format!("encode index: {e}"))?;
    let tmp = dir.join("index.json.tmp");
    {
        let mut fh = std::fs::File::create(&tmp).map_err(|e| format!("write index: {e}"))?;
        fh.write_all(text.as_bytes())
            .map_err(|e| format!("write index: {e}"))?;
        fh.sync_all().map_err(|e| format!("sync index: {e}"))?;
    }
    std::fs::rename(&tmp, index_path()).map_err(|e| format!("replace index: {e}"))
}

fn extension_for(mime: &str) -> &'static str {
    let m = mime.to_ascii_lowercase();
    if m.contains("mp4") || m.contains("aac") {
        "m4a"
    } else if m.contains("ogg") {
        "ogg"
    } else if m.contains("wav") {
        "wav"
    } else {
        "webm"
    }
}

fn find<'a>(all: &'a mut [Recording], id: &str) -> Result<&'a mut Recording, String> {
    if !valid_id(id) {
        return Err(format!("bad recording id: {id}"));
    }
    all.iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("no recording {id}"))
}

// --------------------------------- commands -------------------------------

/// Append a freshly recorded take, creating the recording on first use.
///
/// Returns the take so the caller can render it immediately.
pub fn save_take(
    bytes: &[u8],
    mime: &str,
    duration_s: f64,
    transcript: &str,
    recording_id: Option<&str>,
    name: Option<&str>,
) -> Result<(Recording, Take), String> {
    if bytes.is_empty() {
        return Err("recording came back empty — nothing to save".into());
    }
    let mut all = load_index()?;
    let id = match recording_id {
        Some(id) => {
            find(&mut all, id)?;
            id.to_string()
        }
        None => mint_id(),
    };
    let dir = root().join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create recording dir: {e}"))?;

    // take numbering follows the directory, so a deleted take never collides
    let index = all
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.takes.len())
        .unwrap_or(0);
    let file = format!("take-{index:03}.{}", extension_for(mime));
    std::fs::write(dir.join(&file), bytes).map_err(|e| format!("write take: {e}"))?;

    let take = Take {
        file,
        mime: mime.to_string(),
        created_ms: now_ms(),
        duration_s: if duration_s.is_finite() && duration_s >= 0.0 {
            duration_s
        } else {
            0.0
        },
        transcript: transcript.trim().to_string(),
    };

    let recording = match all.iter_mut().find(|r| r.id == id) {
        Some(r) => {
            r.takes.push(take.clone());
            r.updated_ms = now_ms();
            r.clone()
        }
        None => {
            let rec = Recording {
                id: id.clone(),
                name: name
                    .map(str::to_string)
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| default_name(&take)),
                created_ms: now_ms(),
                updated_ms: now_ms(),
                summary: String::new(),
                locked: false,
                commit_tag: None,
                tampered: false,
                takes: vec![take.clone()],
            };
            all.push(rec.clone());
            rec
        }
    };

    all.sort_by_key(|r| std::cmp::Reverse(r.updated_ms));
    save_index(&all)?;
    Ok((recording, take))
}

/// First line of the transcript, trimmed — a name the operator would recognise.
fn default_name(take: &Take) -> String {
    let first = take
        .transcript
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return "untitled recording".to_string();
    }
    let mut name: String = first.chars().take(48).collect();
    if first.chars().count() > 48 {
        name.push('…');
    }
    name
}

pub fn list() -> Result<Vec<Recording>, String> {
    let mut all = load_index()?;
    all.sort_by_key(|r| std::cmp::Reverse(r.updated_ms));
    Ok(all)
}

pub fn get(id: &str) -> Result<Recording, String> {
    let mut all = load_index()?;
    Ok(find(&mut all, id)?.clone())
}

/// Apply a partial update (rename, summary, lock, or a transcript edit).
///
/// LOCKING is free and anyone may do it — locking is the safe direction, and a
/// locked record is tagged with an HMAC so a later out-of-band edit is visible.
/// UNLOCKING is the owner's call: with an owner PIN set, this path REFUSES
/// `locked:false`; use `unlock()` with the PIN instead.
pub fn update(id: &str, patch: Patch) -> Result<Recording, String> {
    let mut all = load_index()?;
    let rec = find(&mut all, id)?;
    if rec.locked {
        // Unlock is free UNLESS an owner PIN is set — with no PIN, "locked" is
        // pure UI state and requiring a PIN would brick the record. With a PIN,
        // unlocking must go through `unlock()`.
        if patch.locked == Some(false) && !crate::studio_auth::has_pin() {
            rec.locked = false;
            rec.updated_ms = now_ms();
            let out = rec.clone();
            save_index(&all)?;
            return Ok(out);
        }
        if patch.locked == Some(false) {
            return Err("unlock requires the owner PIN — use the unlock command".into());
        }
        return Err("this recording is locked — unlock it to edit".into());
    }
    if let Some(name) = patch.name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            rec.name = trimmed.to_string();
        }
    }
    if let Some(summary) = patch.summary {
        rec.summary = summary;
    }
    if patch.locked == Some(true) {
        // Commit: freeze the body and tag it. A PIN is required so the tag has
        // a key; without one, locking stays pure UI state (tag None).
        if crate::studio_auth::has_pin() {
            let tag = crate::studio_auth::record_tag(&rec.commit_body())?;
            rec.commit_tag = Some(tag);
        }
        rec.locked = true;
    }
    if let (Some(index), Some(text)) = (patch.take_index, patch.transcript) {
        let take = rec
            .takes
            .get_mut(index)
            .ok_or_else(|| format!("no take {index} in {id}"))?;
        take.transcript = text.trim().to_string();
    }
    rec.updated_ms = now_ms();
    rec.restamp(crate::studio_auth::OwnerGuard::load().as_ref());
    let out = rec.clone();
    save_index(&all)?;
    Ok(out)
}

/// Unlock a committed record with the owner PIN. Refused when no PIN is set
/// (nothing to verify against) or the PIN is wrong. Every success clears the
/// tag so the next lock re-tags a fresh body.
pub fn unlock(id: &str, pin: &str) -> Result<Recording, String> {
    if !crate::studio_auth::has_pin() {
        return Err("no owner PIN is set — set one to lock records".into());
    }
    if !crate::studio_auth::verify_pin(pin) {
        return Err("wrong PIN".into());
    }
    let mut all = load_index()?;
    let rec = find(&mut all, id)?;
    if !rec.locked {
        return Err("this recording is not locked".into());
    }
    rec.locked = false;
    rec.commit_tag = None;
    rec.tampered = false;
    rec.updated_ms = now_ms();
    let out = rec.clone();
    save_index(&all)?;
    Ok(out)
}

/// Delete a whole recording and everything it owns on disk.
pub fn delete(id: &str) -> Result<(), String> {
    let mut all = load_index()?;
    if !valid_id(id) {
        return Err(format!("bad recording id: {id}"));
    }
    let before = all.len();
    all.retain(|r| r.id != id);
    if all.len() == before {
        return Err(format!("no recording {id}"));
    }
    save_index(&all)?;
    let dir = root().join(id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove recording dir: {e}"))?;
    }
    Ok(())
}

/// Read one take's audio back out (for playback in the webview).
pub fn read_audio(id: &str, file: &str) -> Result<Vec<u8>, String> {
    if !valid_id(id) {
        return Err(format!("bad recording id: {id}"));
    }
    if !valid_take_file(file) {
        return Err(format!("bad take file: {file}"));
    }
    let path = root().join(id).join(file);
    std::fs::read(&path).map_err(|e| format!("read take audio: {e}"))
}

/// Delete one take (and its blob), keeping the rest of the session intact.
pub fn delete_take(id: &str, take_index: usize) -> Result<Recording, String> {
    let mut all = load_index()?;
    let rec = find(&mut all, id)?;
    if rec.locked {
        return Err("this recording is locked — unlock it to edit".into());
    }
    if take_index >= rec.takes.len() {
        return Err(format!("no take {take_index} in {id}"));
    }
    let removed = rec.takes.remove(take_index);
    rec.updated_ms = now_ms();
    let out = rec.clone();
    save_index(&all)?;
    let blob = root().join(id).join(&removed.file);
    let _ = std::fs::remove_file(blob); // best effort: the index is the truth
    Ok(out)
}

/// Reorder takes and stitch them into one master WAV.
///
/// Order is the splice. ffmpeg's concat demuxer does the audio work; the takes
/// are renumbered so `take-000` is always the first thing you hear.
pub fn splice(id: &str, order: &[usize]) -> Result<Recording, String> {
    let mut all = load_index()?;
    let rec = find(&mut all, id)?;
    if rec.locked {
        return Err("this recording is locked — unlock it to edit".into());
    }
    if order.len() != rec.takes.len() {
        return Err(format!(
            "splice order has {} entries but there are {} takes",
            order.len(),
            rec.takes.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for &i in order {
        if i >= rec.takes.len() {
            return Err(format!("splice order references take {i}"));
        }
        if !seen.insert(i) {
            return Err("splice order repeats a take".into());
        }
    }

    let dir = root().join(id);
    let reordered: Vec<Take> = order.iter().map(|&i| rec.takes[i].clone()).collect();
    let staged = dir.join("splice");
    std::fs::create_dir_all(&staged).map_err(|e| format!("create splice dir: {e}"))?;

    // Copy each take under its new number so the directory mirrors the order.
    let mut list_lines = String::new();
    let mut new_takes = Vec::new();
    for (slot, take) in reordered.iter().enumerate() {
        let ext = take
            .file
            .rsplit_once('.')
            .map(|(_, e)| e.to_string())
            .unwrap_or_else(|| "webm".to_string());
        let new_file = format!("take-{slot:03}.{ext}");
        let src = dir.join(&take.file);
        let dst = staged.join(&new_file);
        std::fs::copy(&src, &dst).map_err(|e| format!("stage take {slot}: {e}"))?;
        list_lines.push_str(&format!("file 'splice/{}'\n", new_file));
        let mut t = take.clone();
        t.file = new_file;
        new_takes.push(t);
    }
    std::fs::write(dir.join("splice.txt"), &list_lines)
        .map_err(|e| format!("write concat list: {e}"))?;

    let master = dir.join("final.wav");
    let ffmpeg = ffmpeg_binary()?;
    let out = std::process::Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
        ])
        .arg("-i")
        .arg("splice.txt")
        .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", "-y"])
        .arg(&master)
        // the concat list names its entries relative to the differ's cwd, and
        // the output path is absolute — running from `dir` is what makes both
        // resolve. (Absolute paths gotcha: ffmpeg's -safe 0 only covers the
        // list file, not the entries inside it.)
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if !out.status.success() || !master.exists() {
        let detail = String::from_utf8_lossy(&out.stderr);
        return Err(format!("splice failed: {}", detail.trim().to_string()));
    }

    // The library now points at the new numbering; drop the splice staging.
    rec.takes = new_takes;
    rec.updated_ms = now_ms();
    let result = rec.clone();
    save_index(&all)?;
    let _ = std::fs::remove_dir_all(&staged);

    // Remove any now-orphaned takes (those the reorder replaced).
    let keep: std::collections::HashSet<&str> =
        result.takes.iter().map(|t| t.file.as_str()).collect();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("take-") && !keep.contains(name.as_str()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(result)
}

/// Write the session's transcript + summary as markdown; returns the path.
pub fn export(id: &str) -> Result<String, String> {
    let rec = get(id)?;
    let dir = root().join(&rec.id);
    let mut body = String::new();
    body.push_str(&format!("# {}\n\n", rec.name));
    if rec.locked {
        body.push_str("_locked — this is the committed record_\n\n");
    }
    if !rec.summary.trim().is_empty() {
        body.push_str("## Summary\n\n");
        body.push_str(rec.summary.trim());
        body.push_str("\n\n");
    }
    body.push_str("## Transcript\n\n");
    for (i, take) in rec.takes.iter().enumerate() {
        body.push_str(&format!(
            "**take {} — {:.1}s**\n\n{}\n\n",
            i + 1,
            take.duration_s,
            take.transcript.trim()
        ));
    }
    let path = dir.join("export.md");
    std::fs::write(&path, body).map_err(|e| format!("write export: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

fn ffmpeg_binary() -> Result<String, String> {
    for candidate in [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ] {
        if Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }
    Err("ffmpeg not found — splice needs it (brew install ffmpeg)".into())
}

// --------------------------------- commands --------------------------------

#[tauri::command]
pub fn grove_library_takes(
    bytes: Vec<u8>,
    mime: Option<String>,
    duration_s: Option<f64>,
    transcript: Option<String>,
    recording_id: Option<String>,
    name: Option<String>,
) -> Result<Recording, String> {
    let rec = save_take(
        &bytes,
        mime.as_deref().unwrap_or("audio/webm"),
        duration_s.unwrap_or(0.0),
        transcript.as_deref().unwrap_or(""),
        recording_id.as_deref(),
        name.as_deref(),
    )?
    .0;
    Ok(rec)
}

#[tauri::command]
pub fn grove_recordings_list() -> Result<Vec<Recording>, String> {
    list()
}

#[tauri::command]
pub fn grove_recording_get(id: String) -> Result<Recording, String> {
    get(&id)
}

#[tauri::command]
pub fn grove_recording_audio(id: String, file: String) -> Result<Vec<u8>, String> {
    read_audio(&id, &file)
}

#[tauri::command]
pub fn grove_recording_patch(
    id: String,
    name: Option<String>,
    summary: Option<String>,
    locked: Option<bool>,
    take_index: Option<usize>,
    transcript: Option<String>,
) -> Result<Recording, String> {
    update(
        &id,
        Patch {
            name,
            summary,
            locked,
            take_index,
            transcript,
        },
    )
}

#[tauri::command]
pub fn grove_recording_unlock(id: String, pin: String) -> Result<Recording, String> {
    unlock(&id, &pin)
}

/// Is an owner PIN set? The UI shows the "set a PIN" flow when false.
#[tauri::command]
pub fn grove_studio_has_pin() -> bool {
    crate::studio_auth::has_pin()
}

/// Set the owner PIN once. Refused if one already exists.
#[tauri::command]
pub fn grove_studio_set_pin(pin: String) -> Result<(), String> {
    crate::studio_auth::set_pin(&pin)
}

#[tauri::command]
pub fn grove_recording_drop(id: String) -> Result<(), String> {
    delete(&id)
}

#[tauri::command]
pub fn grove_recording_drop_take(id: String, take_index: usize) -> Result<Recording, String> {
    delete_take(&id, take_index)
}

#[tauri::command]
pub fn grove_recording_splice(id: String, order: Vec<usize>) -> Result<Recording, String> {
    splice(&id, &order)
}

#[tauri::command]
pub fn grove_recording_export(id: String) -> Result<String, String> {
    export(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate-wide env guard: these tests point the library at a temp dir
    /// through process-global env vars, so they must serialize with the auth
    /// module's tests too (both mutate the same vars).
    use crate::TEST_ENV_GUARD;
    static ENV_GUARD: &std::sync::Mutex<()> = &TEST_ENV_GUARD;

    fn tmp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sg-rec-{tag}-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("GROVE_RECORDINGS_DIR", &dir) };
        // Point auth at the same temp dir, so these tests use the file backend
        // and never touch the real login keychain.
        unsafe { std::env::set_var("GROVE_AUTH_DIR", &dir) };
        // keep PBKDF2 cheap on the throwaway backend (production is unaffected)
        unsafe { std::env::set_var("GROVE_PBKDF2_ITERS", "1000") };
        dir
    }

    #[test]
    fn ids_are_simple_tokens_only() {
        assert!(valid_id("rec-abc_123"));
        assert!(!valid_id(""));
        assert!(!valid_id("../escape"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id("has space"));
        assert!(!valid_id(&"x".repeat(65)));
    }

    #[test]
    fn take_files_cannot_traverse() {
        assert!(valid_take_file("take-000.webm"));
        assert!(!valid_take_file("../index.json"));
        assert!(!valid_take_file("sub/dir.webm"));
        assert!(!valid_take_file(".."));
        assert!(!valid_take_file(""));
    }

    #[test]
    fn extension_follows_the_container() {
        assert_eq!(extension_for("audio/webm"), "webm");
        assert_eq!(extension_for("audio/mp4"), "m4a");
        assert_eq!(extension_for("audio/ogg;codecs=opus"), "ogg");
        assert_eq!(extension_for("audio/wav"), "wav");
        assert_eq!(extension_for(""), "webm");
    }

    #[test]
    fn default_name_is_the_first_real_line() {
        let take = Take {
            file: "take-000.webm".into(),
            mime: "audio/webm".into(),
            created_ms: 0,
            duration_s: 1.0,
            transcript: "\n\n  Plan the Thursday deck  \nmore text".into(),
        };
        assert_eq!(default_name(&take), "Plan the Thursday deck");
    }

    #[test]
    fn default_name_survives_silence() {
        let take = Take {
            file: "t".into(),
            mime: String::new(),
            created_ms: 0,
            duration_s: 0.0,
            transcript: "   \n  ".into(),
        };
        assert_eq!(default_name(&take), "untitled recording");
    }

    /// End-to-end against a real temp HOME: save, patch, export, delete.
    #[test]
    fn library_round_trip() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tmp_home("roundtrip");

        let (rec, take) = save_take(
            b"fake-audio-bytes",
            "audio/webm",
            2.5,
            "First thought about the plan.",
            None,
            None,
        )
        .expect("save");
        assert_eq!(rec.takes.len(), 1);
        assert_eq!(take.file, "take-000.webm");
        assert!(!rec.locked);

        let id = rec.id.clone();
        // the blob is really on disk and reads back byte-identical
        let got = read_audio(&id, "take-000.webm").expect("read audio");
        assert_eq!(got, b"fake-audio-bytes");

        // append a second take to the same recording
        let (rec2, t2) = save_take(
            b"more",
            "audio/webm",
            1.5,
            "Second thought.",
            Some(&id),
            None,
        )
        .expect("append");
        assert_eq!(rec2.takes.len(), 2);
        assert_eq!(t2.file, "take-001.webm");

        // patch: rename + transcript edit
        let patched = update(
            &id,
            Patch {
                name: Some("Thursday prep".into()),
                take_index: Some(0),
                transcript: Some("Edited first thought.".into()),
                ..Default::default()
            },
        )
        .expect("patch");
        assert_eq!(patched.name, "Thursday prep");
        assert_eq!(patched.takes[0].transcript, "Edited first thought.");

        // lock, then a further edit must be refused
        update(
            &id,
            Patch {
                locked: Some(true),
                ..Default::default()
            },
        )
        .expect("lock");
        assert!(update(
            &id,
            Patch {
                name: Some("nope".into()),
                ..Default::default()
            }
        )
        .is_err());

        // unlock, then drop a take
        update(
            &id,
            Patch {
                locked: Some(false),
                ..Default::default()
            },
        )
        .expect("unlock");
        let after = delete_take(&id, 0).expect("drop take");
        assert_eq!(after.takes.len(), 1);
        assert_eq!(after.takes[0].transcript, "Second thought.");

        // export writes markdown next to the recording
        let path = export(&id).expect("export");
        let body = std::fs::read_to_string(&path).expect("read export");
        assert!(body.contains("Second thought."));
        assert!(body.contains("## Transcript"));

        // delete removes the recording from the index
        delete(&id).expect("delete");
        assert!(list().expect("list").is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    /// With an owner PIN set: locking tags the record, the plain patch path
    /// refuses to unlock, a wrong PIN is refused, and the right PIN unlocks.
    #[test]
    fn pin_gates_unlock_when_set() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tmp_home("pin-gate");
        crate::studio_auth::set_pin("246810").expect("set pin");

        let (rec, _) = save_take(b"a", "audio/webm", 1.0, "hello", None, None).expect("save");
        let id = rec.id.clone();

        let locked = update(
            &id,
            Patch {
                locked: Some(true),
                ..Default::default()
            },
        )
        .expect("lock");
        assert!(locked.locked);
        assert!(locked.commit_tag.is_some(), "locking under a PIN must tag");
        assert!(!locked.tampered);

        // the patch path refuses to unlock while a PIN exists
        assert!(update(
            &id,
            Patch {
                locked: Some(false),
                ..Default::default()
            }
        )
        .is_err());

        // wrong PIN refused, record stays locked
        assert!(unlock(&id, "000000").is_err());
        assert!(get(&id).expect("get").locked);

        // right PIN unlocks and clears the tag
        let open = unlock(&id, "246810").expect("unlock");
        assert!(!open.locked);
        assert!(open.commit_tag.is_none());

        let _ = std::fs::remove_dir_all(&home);
    }

    /// Tamper-evidence: a locked record edited on disk (behind the app's back)
    /// reads back as `tampered`, and the change survives a fresh load.
    #[test]
    fn out_of_band_edit_is_detected() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tmp_home("tamper");
        crate::studio_auth::set_pin("246810").expect("set pin");

        let (rec, _) = save_take(b"a", "audio/webm", 1.0, "original", None, None).expect("save");
        let id = rec.id.clone();
        update(
            &id,
            Patch {
                locked: Some(true),
                ..Default::default()
            },
        )
        .expect("lock");
        assert!(!get(&id).expect("get").tampered);

        // hand-edit the on-disk body, as a text editor would
        let path = index_path();
        let raw = std::fs::read_to_string(&path).unwrap();
        let edited = raw.replace("original", "EDITED OUT OF BAND");
        assert_ne!(raw, edited);
        std::fs::write(&path, edited).unwrap();

        let after = get(&id).expect("get");
        assert!(after.locked, "still marked locked");
        assert!(after.tampered, "a changed body must be flagged tampered");

        let _ = std::fs::remove_dir_all(&home);
    }

    /// Regression guard: with NO owner PIN, lock must stay pure UI state, so a
    /// plain unlock still works (requiring a PIN would brick the record).
    #[test]
    fn no_pin_keeps_unlock_free() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tmp_home("no-pin");
        assert!(!crate::studio_auth::has_pin());

        let (rec, _) = save_take(b"a", "audio/webm", 1.0, "x", None, None).expect("save");
        let id = rec.id.clone();
        let locked = update(
            &id,
            Patch {
                locked: Some(true),
                ..Default::default()
            },
        )
        .expect("lock");
        assert!(locked.locked);
        assert!(locked.commit_tag.is_none(), "no PIN -> no tag");

        let open = update(
            &id,
            Patch {
                locked: Some(false),
                ..Default::default()
            },
        )
        .expect("free unlock without a PIN");
        assert!(!open.locked);

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The splice path with real audio: two WAV takes stitched into one master,
    /// renumbered, and the stale blobs cleaned up. Skipped without ffmpeg.
    #[test]
    fn splice_stitches_real_audio_in_order() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let ffmpeg = match ffmpeg_binary() {
            Ok(f) => f,
            Err(_) => return, // no ffmpeg on this box: nothing to prove here
        };
        let home = tmp_home("real-splice");

        // two distinguishable tones, so order is observable
        let mk = |freq: u32, out: &Path| {
            let status = std::process::Command::new(&ffmpeg)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
                .arg(format!("sine=frequency={freq}:duration=1"))
                .args(["-ar", "16000", "-ac", "1"])
                .arg(out)
                .status()
                .expect("run ffmpeg");
            assert!(status.success());
        };
        let a = home.join("a.wav");
        let b = home.join("b.wav");
        mk(440, &a);
        mk(880, &b);

        let (rec, _) = save_take(
            &std::fs::read(&a).unwrap(),
            "audio/wav",
            1.0,
            "take A",
            None,
            Some("splice test"),
        )
        .unwrap();
        let (rec, _) = save_take(
            &std::fs::read(&b).unwrap(),
            "audio/wav",
            1.0,
            "take B",
            Some(&rec.id),
            None,
        )
        .unwrap();
        assert_eq!(rec.takes.len(), 2);

        // reverse the order and stitch
        let spliced = splice(&rec.id, &[1, 0]).expect("splice real audio");
        assert_eq!(spliced.takes.len(), 2);
        assert_eq!(spliced.takes[0].transcript, "take B");
        assert_eq!(spliced.takes[1].transcript, "take A");
        assert_eq!(spliced.takes[0].file, "take-000.wav");

        // the master really exists and is ~2 s of PCM
        let master = root().join(&rec.id).join("final.wav");
        assert!(master.exists(), "final.wav was not written");
        let bytes = std::fs::metadata(&master).unwrap().len();
        // 2 s * 16000 Hz * 2 bytes/sample = 64000, plus a WAV header
        assert!(
            (60_000..70_000).contains(&bytes),
            "final.wav is {bytes} bytes, expected roughly 64k"
        );

        // the old blobs are gone; the new numbering is what remains
        let dir = root().join(&rec.id);
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"take-000.wav".to_string()));
        assert!(names.contains(&"take-001.wav".to_string()));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn splice_order_must_be_a_permutation() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tmp_home("splice");
        let (rec, _) = save_take(b"a", "audio/webm", 1.0, "one", None, None).unwrap();
        let (rec, _) = save_take(b"b", "audio/webm", 1.0, "two", Some(&rec.id), None).unwrap();
        assert_eq!(rec.takes.len(), 2);
        // wrong length
        assert!(splice(&rec.id, &[0]).is_err());
        // repeat
        assert!(splice(&rec.id, &[0, 0]).is_err());
        // out of range
        assert!(splice(&rec.id, &[0, 9]).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
