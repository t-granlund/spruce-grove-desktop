/* ============================================================================
   studio.js — the recording library.

   One editing session ("recording") is an ordered list of takes. Each take is
   audio + its transcript. You can play the session, seek, edit a take's text,
   delete a take, reorder takes and *splice* them into a single master, write a
   summary, and finally **lock** the record — which is the "committed overview".

   The audio work (storing blobs, stitching PCM) lives in Rust; this file only
   presents it. Keeping it that way means the studio has no ffmpeg knowledge
   and the library survives a reload.
============================================================================ */

const studio = {
  open: false,
  rec: null,          // the Recording currently open
  audio: null,        // the spliced master, as an object URL
  audioBytes: null,   // the same master, kept for the waveform (see drawWaveform)
  audioEl: null,
  busy: false,
};

const S = (id) => document.getElementById(id);

function studioStatus(text) {
  const el = S("studio-status");
  if (el) el.textContent = text || "";
}

function fmtTime(sec) {
  if (!isFinite(sec) || sec < 0) return "0:00";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return m + ":" + String(s).padStart(2, "0");
}

/* ------------------------------- library -------------------------------- */

async function studioRefresh() {
  let list = [];
  try {
    list = await invoke("grove_recordings_list");
  } catch (err) {
    studioStatus("library unavailable: " + String(err).split("\n")[0]);
    return;
  }
  const host = S("studio-recordings");
  host.textContent = "";
  if (!list.length) {
    const p = document.createElement("p");
    p.className = "studio-note";
    p.textContent = "Nothing recorded yet.";
    host.appendChild(p);
    return;
  }
  for (const r of list) {
    const row = document.createElement("button");
    row.className = "studio-row" + (studio.rec && studio.rec.id === r.id ? " on" : "");
    row.dataset.id = r.id;
    const name = document.createElement("span");
    name.className = "sr-name";
    name.textContent = r.name;
    const meta = document.createElement("span");
    meta.className = "sr-meta";
    const secs = r.takes.reduce((a, t) => a + (t.duration_s || 0), 0);
    meta.textContent =
      r.takes.length + (r.takes.length === 1 ? " take" : " takes") +
      " · " + fmtTime(secs) + (r.locked ? " · locked" : "");
    row.append(name, meta);
    row.addEventListener("click", () => studioOpen(r.id));
    host.appendChild(row);
  }
}

/* -------------------------------- open ---------------------------------- */

async function studioOpen(id) {
  if (studio.busy) return;
  studio.busy = true;
  studioStatus("opening…");
  try {
    studio.rec = await invoke("grove_recording_get", { id });
    S("studio-empty").classList.add("hidden");
    S("studio-open").classList.remove("hidden");
    renderRecording();
    await loadMaster();
    await studioRefresh();
    studioStatus("");
  } catch (err) {
    studioStatus("could not open: " + String(err).split("\n")[0]);
  } finally {
    studio.busy = false;
  }
}

/* Load the spliced master into an <audio>, if one has been written. The blob
   is decoded via a blob: URL, which the CSP's media-src allows. */
async function loadMaster() {
  releaseMaster();
  if (!studio.rec) return;
  const hasMaster = studio.rec.takes.length > 0;
  if (!hasMaster) return;
  try {
    const bytes = await invoke("grove_recording_audio", {
      id: studio.rec.id,
      file: "final.wav",
    });
    const view = new Uint8Array(bytes);
    const blob = new Blob([view], { type: "audio/wav" });
    studio.audio = URL.createObjectURL(blob);
    // Keep the raw bytes: the waveform decodes these directly. Re-fetching the
    // blob: URL is blocked by the packaged CSP (connect-src has no blob:), which
    // is exactly what made every waveform read "audio could not be decoded".
    studio.audioBytes = view;
    const el = new Audio();
    el.src = studio.audio;
    el.preload = "metadata";
    el.addEventListener("ended", () => setPlayLabel(false));
    el.addEventListener("timeupdate", () => {
      if (!el.duration) return;
      S("studio-time").textContent = fmtTime(el.currentTime) + " / " + fmtTime(el.duration);
      S("studio-seek").value = String(Math.round((el.currentTime / el.duration) * 1000));
    });
    el.addEventListener("loadedmetadata", () => {
      S("studio-time").textContent = "0:00 / " + fmtTime(el.duration);
    });
    studio.audioEl = el;
    S("studio-dur").textContent = "master spliced";
  } catch (_) {
    // no master yet: that is the normal state before the first splice
    studio.audioEl = null;
    studio.audio = null;
    studio.audioBytes = null;
    S("studio-dur").textContent = "no master yet — splice to hear the whole session";
    S("studio-time").textContent = "0:00 / 0:00";
  }
  await drawWaveform();
}

function releaseMaster() {
  if (studio.audioEl) {
    try { studio.audioEl.pause(); } catch (_) { /* ignore */ }
  }
  if (studio.audio) URL.revokeObjectURL(studio.audio);
  studio.audioEl = null;
  studio.audio = null;
  studio.audioBytes = null;
}

function setPlayLabel(playing) {
  S("studio-play").textContent = playing ? "pause" : "play";
}

/* ------------------------------- waveform -------------------------------- */

/* Decode the master with the Web Audio API and draw an RMS envelope. This is
   presentation only — no audio is modified here.

   Decode the bytes we already hold; never re-fetch the blob: URL. The packaged
   CSP's connect-src permits only `ipc:`/`http://ipc.localhost`, so fetching a
   blob: URL is blocked and every waveform fell into the catch ("audio could not
   be decoded") even though the WAV was perfectly valid. decodeAudioData wants a
   detached ArrayBuffer, so hand it a copy. */
async function drawWaveform() {
  const host = S("studio-wave");
  host.textContent = "";
  if (!studio.audioBytes || !studio.audioBytes.length) return;
  try {
    const buf = studio.audioBytes.slice().buffer;
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    const audio = await ctx.decodeAudioData(buf);
    const data = audio.getChannelData(0);
    const W = 640, H = 64, BARS = 96;
    const canvas = document.createElement("canvas");
    canvas.width = W; canvas.height = H;
    canvas.className = "studio-canvas";
    const g = canvas.getContext("2d");
    g.fillStyle = "rgba(234,228,211,.10)";
    g.fillRect(0, 0, W, H);
    const step = Math.floor(data.length / BARS) || 1;
    for (let i = 0; i < BARS; i++) {
      let peak = 0;
      for (let j = 0; j < step; j++) {
        const v = Math.abs(data[i * step + j] || 0);
        if (v > peak) peak = v;
      }
      const h = Math.max(2, Math.round(peak * (H - 6)));
      g.fillStyle = "#E4AA71";
      g.fillRect(i * (W / BARS) + 1, (H - h) / 2, Math.max(1, W / BARS - 2), h);
    }
    host.appendChild(canvas);
    await ctx.close();
  } catch (_) {
    const p = document.createElement("p");
    p.className = "studio-note";
    p.textContent = "waveform unavailable (audio could not be decoded)";
    host.appendChild(p);
  }
}

/* -------------------------------- render --------------------------------- */

function renderRecording() {
  const r = studio.rec;
  if (!r) return;
  S("studio-name").value = r.name;
  S("studio-name").disabled = r.locked;
  S("studio-summary").value = r.summary || "";
  S("studio-summary").disabled = r.locked;
  S("studio-lock-btn").textContent = r.locked ? "unlock" : "lock the record";
  S("studio-lock").textContent = r.locked ? "locked — this is the committed record" : "";
  S("studio-lock").dataset.state = r.locked ? "on" : "";
  // tamper banner: the file changed under a committed record (see studio_auth)
  const tamper = S("studio-tamper");
  if (r.tampered) {
    tamper.textContent =
      "⚠ this committed record was changed outside the app — its signature no longer matches. " +
      "Unlock with the owner PIN to re-commit it deliberately.";
    tamper.classList.remove("hidden");
  } else {
    tamper.classList.add("hidden");
  }
  S("studio-count").textContent = "(" + r.takes.length + ")";

  const host = S("studio-takes");
  host.textContent = "";
  const disabled = r.locked;
  r.takes.forEach((take, i) => {
    const card = document.createElement("div");
    card.className = "take";
    card.dataset.index = String(i);

    const head = document.createElement("div");
    head.className = "take-head";
    const num = document.createElement("span");
    num.className = "take-num";
    num.textContent = String(i + 1).padStart(2, "0");
    const dur = document.createElement("span");
    dur.className = "take-dur";
    dur.textContent = fmtTime(take.duration_s);
    const spacer = document.createElement("span");
    spacer.className = "take-spacer";
    head.append(num, dur, spacer);

    if (!disabled) {
      const up = document.createElement("button");
      up.className = "take-btn";
      up.textContent = "↑";
      up.title = "move earlier";
      up.disabled = i === 0;
      up.addEventListener("click", () => reorder(i, i - 1));
      const down = document.createElement("button");
      down.className = "take-btn";
      down.textContent = "↓";
      down.title = "move later";
      down.disabled = i === r.takes.length - 1;
      down.addEventListener("click", () => reorder(i, i + 1));
      const del = document.createElement("button");
      del.className = "take-btn danger";
      del.textContent = "×";
      del.title = "delete this take";
      del.addEventListener("click", () => dropTake(i));
      head.append(up, down, del);
    }

    const ta = document.createElement("textarea");
    ta.className = "take-text";
    ta.rows = 2;
    ta.value = take.transcript;
    ta.disabled = disabled;
    ta.placeholder = "(no transcript yet)";
    // edit in place: save on blur, and on Cmd/Ctrl+Enter
    ta.addEventListener("blur", () => saveTranscript(i, ta.value));
    ta.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && (ev.metaKey || ev.ctrlKey)) {
        ev.preventDefault();
        ta.blur();
      }
    });

    card.append(head, ta);
    host.appendChild(card);
  });
}

/* -------------------------------- actions -------------------------------- */

async function saveTranscript(index, text) {
  if (!studio.rec || studio.rec.locked) return;
  if ((studio.rec.takes[index] || {}).transcript === text.trim()) return;
  try {
    studio.rec = await invoke("grove_recording_patch", {
      id: studio.rec.id,
      takeIndex: index,
      transcript: text,
    });
    studioStatus("take " + (index + 1) + " saved");
  } catch (err) {
    studioStatus("could not save: " + String(err).split("\n")[0]);
  }
}

async function dropTake(index) {
  if (!studio.rec || studio.rec.locked) return;
  try {
    studio.rec = await invoke("grove_recording_drop_take", {
      id: studio.rec.id,
      takeIndex: index,
    });
    renderRecording();
    await loadMaster();
    await studioRefresh();
    studioStatus("take deleted");
  } catch (err) {
    studioStatus("could not delete: " + String(err).split("\n")[0]);
  }
}

/* Reorder is local until you splice: you are arranging an edit, not rewriting
   history. The order is sent to Rust only when "splice to master" runs. */
let pendingOrder = null;
function reorder(from, to) {
  if (!studio.rec || studio.rec.locked) return;
  if (!pendingOrder) pendingOrder = studio.rec.takes.map((_, i) => i);
  const [moved] = pendingOrder.splice(from, 1);
  pendingOrder.splice(to, 0, moved);
  // reflect the new order immediately in the UI
  const byOld = new Map(studio.rec.takes.map((t, i) => [i, t]));
  studio.rec.takes = pendingOrder.map((i) => byOld.get(i));
  renderRecording();
  studioStatus("order changed — press splice to make it real");
}

async function doSplice() {
  if (!studio.rec) return;
  const order = pendingOrder || studio.rec.takes.map((_, i) => i);
  studioStatus("splicing with ffmpeg…");
  try {
    studio.rec = await invoke("grove_recording_splice", { id: studio.rec.id, order });
    pendingOrder = null;
    renderRecording();
    await loadMaster();
    await studioRefresh();
    studioStatus("master spliced — " + studio.rec.takes.length + " takes in order");
  } catch (err) {
    studioStatus("splice failed: " + String(err).split("\n")[0]);
  }
}

/* ------------------------------ owner PIN -------------------------------- */
/*
   Locking is free; unlocking is the owner's call. The first lock on a library
   with no PIN prompts to set one (owner-PIN modal). After that, unlocking a
   committed record requires that PIN — and the Rust side enforces it, so a
   hand-edit to index.json cannot quietly reopen the record.
*/

let pinFlow = null; // { mode: "set" | "unlock" } while the modal is up

function showPinModal(mode) {
  pinFlow = { mode };
  const set = mode === "set";
  S("studio-pin-title").textContent = set ? "set owner PIN" : "owner PIN";
  S("studio-pin-hint").textContent = set
    ? "Locks are free, but unlocking will require this PIN. It is stored in your macOS Keychain — never in the record file. Choose at least 4 characters."
    : "This record is committed. Enter the owner PIN to unlock and edit it.";
  S("studio-pin-input").value = "";
  S("studio-pin-err").textContent = "";
  S("studio-pin-ok").textContent = set ? "set PIN" : "unlock";
  S("studio-pin").classList.remove("hidden");
  S("studio-pin-input").focus();
}

function hidePinModal() {
  pinFlow = null;
  S("studio-pin").classList.add("hidden");
  S("studio-pin-input").value = "";
}

async function doPinConfirm() {
  if (!pinFlow) return;
  const pin = S("studio-pin-input").value;
  if (!pin) {
    S("studio-pin-err").textContent = "enter a PIN";
    return;
  }
  const mode = pinFlow.mode;
  try {
    if (mode === "set") {
      await invoke("grove_studio_set_pin", { pin });
      hidePinModal();
      // now actually perform the lock the user asked for
      await commitLock();
    } else {
      studio.rec = await invoke("grove_recording_unlock", { id: studio.rec.id, pin });
      hidePinModal();
      renderRecording();
      await studioRefresh();
      studioStatus("unlocked for editing");
      recordUnlock(studio.rec.id);
    }
  } catch (err) {
    S("studio-pin-err").textContent = String(err).split("\n")[0] || "could not verify";
  }
}

/* Flush the summary, then lock. Never run while unlocking. */
async function commitLock() {
  try {
    await invoke("grove_recording_patch", {
      id: studio.rec.id,
      summary: S("studio-summary").value,
    });
    studio.rec = await invoke("grove_recording_patch", {
      id: studio.rec.id,
      locked: true,
    });
    renderRecording();
    await studioRefresh();
    studioStatus("locked — the record is committed");
  } catch (err) {
    studioStatus("could not lock: " + String(err).split("\n")[0]);
  }
}

async function toggleLock() {
  if (!studio.rec) return;
  if (studio.rec.locked) {
    // unlocking always needs the owner PIN
    showPinModal("unlock");
    return;
  }
  // first-ever lock offers to set a PIN; without one, target="_free" mode
  let hasPin = false;
  try { hasPin = await invoke("grove_studio_has_pin"); } catch (_) {}
  if (!hasPin) {
    showPinModal("set");
    return;
  }
  await commitLock();
}

/* Record the unlock in the audit ledger (receipts, not a gate). */
function recordUnlock(id) {
  try {
    invoke("grove_ledger_record", {
      kind: "studio",
      cwd: "",
      summary: "unlocked committed record " + id,
      outcome: "owner-pin",
    }).catch(() => {});
  } catch (_) { /* ledger is best-effort */ }
}

async function doExport() {
  if (!studio.rec) return;
  try {
    const path = await invoke("grove_recording_export", { id: studio.rec.id });
    studioStatus("exported to " + path);
  } catch (err) {
    studioStatus("export failed: " + String(err).split("\n")[0]);
  }
}

async function saveName() {
  if (!studio.rec || studio.rec.locked) return;
  const name = S("studio-name").value.trim();
  if (!name || name === studio.rec.name) return;
  try {
    studio.rec = await invoke("grove_recording_patch", { id: studio.rec.id, name });
    await studioRefresh();
  } catch (err) {
    studioStatus("could not rename: " + String(err).split("\n")[0]);
  }
}

async function saveSummary() {
  if (!studio.rec || studio.rec.locked) return;
  try {
    studio.rec = await invoke("grove_recording_patch", {
      id: studio.rec.id,
      summary: S("studio-summary").value,
    });
    studioStatus("summary saved");
  } catch (err) {
    studioStatus("could not save summary: " + String(err).split("\n")[0]);
  }
}

/* ------------------------------- open/close ------------------------------ */

async function openStudio() {
  studio.open = true;
  S("studio-overlay").classList.remove("hidden");
  await studioRefresh();
  if (studio.rec) await studioOpen(studio.rec.id);
}

function closeStudio() {
  studio.open = false;
  S("studio-overlay").classList.add("hidden");
  if (studio.audioEl) { try { studio.audioEl.pause(); } catch (_) {} }
  setPlayLabel(false);
}

/* -------------------------------- wiring --------------------------------- */

function studioWire() {
  S("studio-close").addEventListener("click", closeStudio);
  S("studio-play").addEventListener("click", () => {
    const el = studio.audioEl;
    if (!el) { studioStatus("no master yet — splice first"); return; }
    if (el.paused) { el.play().then(() => setPlayLabel(true)).catch(() => {}); }
    else { el.pause(); setPlayLabel(false); }
  });
  S("studio-seek").addEventListener("input", (ev) => {
    const el = studio.audioEl;
    if (!el || !el.duration) return;
    el.currentTime = (Number(ev.target.value) / 1000) * el.duration;
  });
  S("studio-splice").addEventListener("click", doSplice);
  S("studio-lock-btn").addEventListener("click", toggleLock);
  S("studio-pin-ok").addEventListener("click", doPinConfirm);
  S("studio-pin-cancel").addEventListener("click", hidePinModal);
  S("studio-pin-input").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") { ev.preventDefault(); doPinConfirm(); }
  });
  S("studio-export").addEventListener("click", doExport);
  S("studio-name").addEventListener("blur", saveName);
  S("studio-summary").addEventListener("blur", saveSummary);
  S("studio-record-more").addEventListener("click", () => {
    closeStudio();
    // the composer's mic starts a new take; it will attach to the open session
    if (window.groveRecordMore) window.groveRecordMore();
  });
  document.addEventListener("keydown", (ev) => {
    if (ev.key !== "Escape" || !studio.open) return;
    // the PIN modal owns Escape while it is up (closing it, not the studio)
    if (pinFlow) { hidePinModal(); return; }
    closeStudio();
  });
}

studioWire();

/* Exposed so main.js can open the studio after a take lands, and so "record
   another take" can attach the next one to the session that is open. */
window.groveStudio = {
  open: openStudio,
  refresh: studioRefresh,
  currentId: () => (studio.rec ? studio.rec.id : null),
};
