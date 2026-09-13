/* ==================================================================
   Spruce Grove desktop shell controller.

   Left sidebar: session history (resume via session/load) + directory
   memory. Main column: live ACP stream (chunks / thinking / tool cards
   with status chips / per-turn usage), dictation loop, mid-run steering,
   Live Look-in panel. Legacy line-mode fallback preserved.

   Zero dependencies; the Tauri global bridge is the only boundary.
   ================================================================== */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const els = {
  cwd: document.getElementById("cwd"),
  mode: document.getElementById("mode"),
  transcript: document.getElementById("transcript"),
  prompt: document.getElementById("prompt"),
  send: document.getElementById("send"),
  cancel: document.getElementById("cancel"),
  mic: document.getElementById("mic"),
  status: document.getElementById("status"),
  cliVersion: document.getElementById("cli-version"),
  lookin: document.getElementById("lookin"),
  lookinPanel: document.getElementById("lookin-panel"),
  lookinImg: document.getElementById("lookin-img"),
  lookinLog: document.getElementById("lookin-log"),
  lookinClose: document.getElementById("lookin-close"),
  sessionList: document.getElementById("session-list"),
  dirList: document.getElementById("dir-list"),
  newChat: document.getElementById("new-chat"),
  sidebar: document.getElementById("sidebar"),
  sidebarToggle: document.getElementById("sidebar-toggle"),
  sidebarOpen: document.getElementById("sidebar-open"),
};

const state = { busy: false, resumeFlag: false, pendingSteer: null };
const acp = {
  ready: false, sessionId: null, model: null, cwd: null,
  turnActive: false, firstPrompt: null, lastEventAt: 0,
};
const toolEls = new Map();
let sessions = [];

/* ============================ session store ======================= */

const STORE_KEY = "grove.sessions.v1";

function loadSessions() {
  try { sessions = JSON.parse(localStorage.getItem(STORE_KEY) || "[]"); }
  catch { sessions = []; }
  if (!Array.isArray(sessions)) sessions = [];
}

function saveSessions() {
  localStorage.setItem(STORE_KEY, JSON.stringify(sessions.slice(0, 40)));
}

function upsertSession(id, cwd, title) {
  if (!id || !cwd) return;
  const found = sessions.find((s) => s.id === id);
  if (found) {
    found.ts = Date.now();
    // first real prompt replaces the placeholder "new session" title
    if (title && (!found.title || found.title === "new session")) found.title = title;
    found.cwd = cwd;
  } else {
    sessions.unshift({ id, cwd, title: title || "new session", ts: Date.now() });
  }
  sessions.sort((a, b) => b.ts - a.ts);
  saveSessions();
  renderSidebar();
}

function dropSession(id) {
  sessions = sessions.filter((s) => s.id !== id);
  saveSessions();
  renderSidebar();
}

function sessionsForDir(cwd) {
  return sessions.filter((s) => s.cwd === cwd);
}

function relTime(ts) {
  const m = Math.round((Date.now() - ts) / 60000);
  if (m < 1) return "now";
  if (m < 60) return m + "m ago";
  const h = Math.round(m / 60);
  if (h < 24) return h + "h ago";
  return Math.round(h / 24) + "d ago";
}

function baseName(p) { return (p || "").replace(/\/+$/, "").split("/").pop() || p; }

/* ============================ sidebar render ====================== */

let activeSessionId = null;

function renderSidebar() {
  // sessions (newest first, per current dir first)
  const cwd = els.cwd.value.trim();
  const sorted = [...sessions].sort((a, b) => {
    const pa = a.cwd === cwd ? 0 : 1;
    const pb = b.cwd === cwd ? 0 : 1;
    return pa - pb || b.ts - a.ts;
  });
  els.sessionList.innerHTML = "";
  for (const s of sorted.slice(0, 14)) {
    const item = document.createElement("button");
    item.className = "sess-item" + (s.id === activeSessionId ? " active" : "");
    const title = document.createElement("span");
    title.className = "s-title";
    title.textContent = s.title || "session";
    const meta = document.createElement("span");
    meta.className = "s-meta";
    meta.textContent = baseName(s.cwd) + " · " + relTime(s.ts);
    const del = document.createElement("span");
    del.className = "s-del";
    del.textContent = "×";
    del.title = "forget this session";
    del.addEventListener("click", (ev) => {
      ev.stopPropagation();
      dropSession(s.id);
    });
    item.append(title, meta, del);
    item.addEventListener("click", () => openSession(s));
    els.sessionList.appendChild(item);
  }
  if (!sorted.length) {
    const none = document.createElement("div");
    none.className = "s-meta";
    none.style.padding = "6px";
    none.textContent = "no sessions yet";
    els.sessionList.appendChild(none);
  }

  // directories (distinct, most recent first)
  const seen = new Map();
  for (const s of sessions) { if (!seen.has(s.cwd)) seen.set(s.cwd, s.ts); }
  els.dirList.innerHTML = "";
  for (const [dir, ts] of [...seen.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8)) {
    const item = document.createElement("button");
    item.className = "dir-item" + (dir === cwd ? " active" : "");
    item.textContent = baseName(dir);
    item.title = dir;
    item.addEventListener("click", () => {
      els.cwd.value = dir;
      const last = sessionsForDir(dir)[0];
      startAcp(dir, last ? last.id : null, true);
    });
    els.dirList.appendChild(item);
  }
}

/* ============================ ACP session ========================= */

async function startAcp(cwd, resumeId, forceRestart) {
  els.mode.textContent = "connecting…";
  els.mode.dataset.state = "connecting";
  acp.ready = false;
  acp.turnActive = false;
  try {
    const res = await invoke("grove_acp_start", {
      cwd,
      resume: resumeId && sessions.some((s) => s.id === resumeId) ? resumeId : null,
    });
    acp.sessionId = res.sessionId;
    acp.model = res.model;
    acp.cwd = cwd;
    acp.ready = true;
    activeSessionId = res.sessionId;
    if (!acp.firstPrompt) acp.firstPrompt = null;
    localStorage.setItem("grove.cwd", cwd);
    upsertSession(res.sessionId, cwd, res.resumed ? null : "new session");
    els.mode.textContent = `ACP · ${res.model || "live"}${res.resumed ? " · resumed" : ""}`;
    els.mode.dataset.state = "acp";
    els.status.textContent = res.resumed
      ? "grove ready — previous session resumed"
      : "grove ready — structured live session";
    renderSidebar();
    els.prompt.focus();
  } catch (err) {
    acp.ready = false;
    els.mode.textContent = "legacy line mode";
    els.mode.dataset.state = "legacy";
    els.status.textContent = `ACP unavailable (${String(err).slice(0, 80)}) — using line mode`;
  }
}

/* ============================ utilities =========================== */

function setBusy(busy, note) {
  state.busy = busy;
  els.send.disabled = busy && !acp.ready;
  els.cancel.disabled = !busy;
  els.status.textContent = note;
  els.status.classList.toggle("busy", busy);
}

function nearBottom(el) {
  return el.scrollHeight - el.scrollTop - el.clientHeight < 120;
}
function scrollDown(force) {
  const t = els.transcript;
  if (force || nearBottom(t)) t.scrollTop = t.scrollHeight;
}

function addMessage(kind, title) {
  document.getElementById("empty-state")?.remove();
  const box = document.createElement("div");
  box.className = "msg " + kind;
  const who = document.createElement("div");
  who.className = "who";
  who.textContent = title;
  const pre = document.createElement("pre");
  box.append(who, pre);
  els.transcript.appendChild(box);
  scrollDown(true);
  return pre;
}

/* legacy line-mode buffering */
let currentAgentPre = null;
let lastStreamWasErr = false;
function appendLine(stream, line) {
  if (!currentAgentPre || lastStreamWasErr !== (stream === "stderr")) {
    currentAgentPre = addMessage("agent", stream === "stderr" ? "grove · cli stderr" : "grove");
    lastStreamWasErr = stream === "stderr";
  }
  const span = document.createElement("span");
  if (stream === "stderr") span.className = "err";
  span.textContent = line + "\n";
  currentAgentPre.appendChild(span);
  scrollDown(true);
}

/* ============================ ACP stream ========================== */

let acpAgentPre = null;
let acpThoughtPre = null;
let waitingEl = null;

function clearWaiting() {
  waitingEl?.remove();
  waitingEl = null;
}
function showWaiting() {
  if (waitingEl || !acp.turnActive) return;
  clearWaiting();
  waitingEl = document.createElement("div");
  waitingEl.className = "waiting";
  for (let i = 0; i < 3; i++) waitingEl.appendChild(document.createElement("i"));
  els.transcript.appendChild(waitingEl);
  scrollDown(true);
}

function acpAgentPre_() {
  if (!acpAgentPre) {
    acpThoughtPre = null;
    clearWaiting();
    acpAgentPre = addMessage("agent", "grove · " + (acp.model || "agent"));
  }
  return acpAgentPre;
}
function acpAppendText(text) {
  if (!text) return;
  const span = document.createElement("span");
  span.textContent = text;
  acpAgentPre_().appendChild(span);
  scrollDown(false);
}
function acpAppendThought(text) {
  if (!text) return;
  if (!acpThoughtPre) {
    acpAgentPre = null;
    clearWaiting();
    acpThoughtPre = addMessage("thought", "cedar · thinking");
  }
  const span = document.createElement("span");
  span.textContent = text;
  acpThoughtPre.appendChild(span);
  scrollDown(false);
}

function acpToolCard(update) {
  const id = update.toolCallId || update.title || JSON.stringify(update).slice(0, 40);
  let card = toolEls.get(id);
  if (!card) {
    clearWaiting();
    document.getElementById("empty-state")?.remove();
    const box = document.createElement("div");
    box.className = "msg tool";
    const head = document.createElement("div");
    head.className = "tool-head";
    const title = document.createElement("span");
    title.className = "tool-title";
    title.textContent = update.title || update.kind || "tool";
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.textContent = update.status || "pending";
    head.append(title, chip);
    const pre = document.createElement("pre");
    box.append(head, pre);
    els.transcript.appendChild(box);
    card = { box, chip, pre };
    toolEls.set(id, card);
  }
  card.chip.textContent = update.status || card.chip.textContent;
  card.chip.dataset.status = update.status || "pending";
  const input = update.rawInput ?? update.input;
  if (input && !card.pre.dataset.hasInput) {
    card.pre.textContent = JSON.stringify(input).slice(0, 300);
    card.pre.dataset.hasInput = "1";
  }
  scrollDown(false);
}

/* ============================ look-in ============================= */

const LOOKIN_ACTION = /browser|navigate|click|fill|type|upload|screenshot|snapshot|scroll|press|select/i;
const SHOT_PATH = /screenshot_path\\?"?\s*:\s*\\?"([^"\\]+?\.(?:png|jpe?g|webp))/;

function lookinLog(text) {
  const line = document.createElement("div");
  line.className = "lookin-line";
  line.textContent = text;
  els.lookinLog.prepend(line);
  while (els.lookinLog.children.length > 30) els.lookinLog.lastChild.remove();
}
function lookinOpen() { els.lookinPanel.classList.remove("hidden"); }

async function lookinShowScreenshot(rawPath) {
  let path = rawPath;
  try { path = JSON.parse('"' + rawPath + '"'); } catch { /* keep raw */ }
  try {
    const dataUrl = await invoke("grove_read_image_base64", { path });
    els.lookinImg.src = dataUrl;
    els.lookinImg.dataset.path = path;
    lookinOpen();
  } catch {
    lookinLog("screenshot not readable yet: " + path.split("/").pop());
  }
}

function handleToolForLookin(update) {
  if (!update) return;
  const title = String(update.title || "");
  if (LOOKIN_ACTION.test(title)) lookinLog((update.status || "…") + " · " + title);
  const m = JSON.stringify(update).match(SHOT_PATH);
  if (m && m[1]) { lookinLog("screenshot captured"); lookinShowScreenshot(m[1]); }
}

/* ============================ ACP events ========================== */

async function handleAcpEvent(event) {
  acp.lastEventAt = Date.now();
  const { kind, data } = event.payload;
  switch (kind) {
    case "ready":
      // handled by startAcp; kept for parity with backend emits
      break;
    case "chunk":
      acpAppendText(data && data.update ? (data.update.content || {}).text : "");
      break;
    case "thought":
      acpAppendThought(data && data.update ? (data.update.content || {}).text : "");
      break;
    case "tool":
      if (data && data.update) {
        acpToolCard(data.update);
        handleToolForLookin(data.update);
      }
      break;
    case "permission":
      if (data && data.params) {
        const line = addMessage("permission", "permission · auto-allowed");
        line.textContent = String(
          (data.params.toolCall && data.params.toolCall.title) || data.params.toolCall || ""
        ).slice(0, 160);
      }
      break;
    case "error":
      if (data && data.message) els.status.textContent = data.message;
      break;
    case "turn-end": {
      const usage = data && data.result && data.result.usage;
      const toks = usage ? " · " + Number(usage.totalTokens || 0).toLocaleString() + " tok" : "";
      const why = (data && data.result && data.result.stopReason) || (data && data.ok === false ? "error" : "done");
      if (data && data.ok === false) {
        addMessage("agent", "grove · error").textContent = String(data.error || "turn failed");
      }
      acpAgentPre = null;
      acpThoughtPre = null;
      clearWaiting();
      acp.turnActive = false;
      toolEls.clear();
      if (acp.sessionId && acp.cwd) {
        localStorage.setItem("grove.session." + acp.cwd, acp.sessionId);
      }
      if (state.pendingSteer && acp.ready) {
        const steer = state.pendingSteer;
        state.pendingSteer = null;
        setBusy(false, "steering Cedar back on course…");
        setTimeout(() => { els.prompt.value = steer; sendPrompt(); }, 500);
        return;
      }
      setBusy(false, `done${toks} · ${why}`);
      els.prompt.focus();
      break;
    }
    default:
      break;
  }
}

/* ============================ stall watchdog ====================== */
/* The CLI can wedge on a turn (observed: ACP glue deadlock while the model
   socket sat in CLOSE_WAIT). A wedged turn must never wedge the shell:
   after 75s of silence, warn; after 80s, cancel, then hard-kill the CLI and
   restart the session (resume falls back to fresh automatically). */
const STALL_WARN_MS = 75000;
const STALL_KILL_MS = 80000;
let stallState = null; // null | "warned"

setInterval(() => {
  if (!acp.turnActive || !acp.ready) { stallState = null; return; }
  const quiet = Date.now() - acp.lastEventAt;
  if (quiet < STALL_WARN_MS) {
    if (stallState === "warned") {
      stallState = null;
      setBusy(true, "Cedar is working — streaming live…");
    }
    return;
  }
  if (quiet < STALL_KILL_MS) {
    if (stallState !== "warned") {
      stallState = "warned";
      setBusy(true, `no stream activity for ${Math.round(quiet / 1000)}s — watching…`);
    }
    return;
  }
  // hard recovery
  const dead = acp.sessionId;
  stallState = null;
  addMessage("agent", "grove · stall watchdog").textContent =
    "The CLI stopped streaming for over a minute (observed wedge). " +
    "Restarting the session — your history is kept in the sidebar.";
  setBusy(false, "recovering from stalled turn…");
  acp.turnActive = false;
  invoke("grove_acp_cancel", { sessionId: dead }).catch(() => {});
  setTimeout(async () => {
    invoke("grove_acp_kill").catch(() => {});
    acp.ready = false;
    const cwd = els.cwd.value.trim();
    setTimeout(() => startAcp(cwd, dead, true), 400);
  }, 1200);
}, 1000);

/* ============================ send / cancel ======================= */

async function sendPrompt() {
  const prompt = els.prompt.value.trim();
  const cwd = els.cwd.value.trim();

  // mid-run steer: cancel the turn, redirect within the SAME session
  if (prompt && state.busy && acp.ready && acp.sessionId && acp.turnActive) {
    state.pendingSteer = prompt;
    els.prompt.value = "";
    els.status.textContent = "steer queued — redirecting Cedar…";
    invoke("grove_acp_cancel", { sessionId: acp.sessionId }).catch(() => {});
    pendingSteerFallbackArm();
    return;
  }

  if (!prompt || !cwd || state.busy) return;

  localStorage.setItem("grove.cwd", cwd);
  const userPre = addMessage("user", "you");
  userPre.textContent = prompt;
  currentAgentPre = null;
  acpAgentPre = null;
  acpThoughtPre = null;
  els.prompt.value = "";
  if (acp.firstPrompt === null) acp.firstPrompt = prompt;

  if (acp.ready && cwd !== acp.cwd) {
    // directory moved: fresh session in the new dir (history stays per-dir)
    const prior = sessionsForDir(cwd)[0];
    await startAcp(cwd, prior ? prior.id : null, true);
  }

  if (acp.ready && acp.sessionId) {
    setBusy(true, "Cedar is working — streaming live…");
    acp.turnActive = true;
    acp.lastEventAt = Date.now();
    acp.firstPrompt = acp.firstPrompt || prompt;
    upsertSession(acp.sessionId, cwd, acp.firstPrompt);
    showWaiting();
    try {
      await invoke("grove_acp_prompt", { sessionId: acp.sessionId, text: prompt });
    } catch (err) {
      addMessage("agent", "grove · error").textContent = String(err);
      setBusy(false, "idle");
    }
    return;
  }

  setBusy(true, "grove is thinking...");
  try {
    await invoke("grove_send", { prompt, cwd, resume: state.resumeFlag });
  } catch (err) {
    addMessage("agent", "grove · launch error").textContent = String(err);
    setBusy(false, "idle");
  }
}

function pendingSteerFallbackArm() {
  setTimeout(() => {
    if (!state.pendingSteer) return;
    const steer = state.pendingSteer;
    state.pendingSteer = null;
    acp.turnActive = false;
    setBusy(false, "steering (direct)…");
    els.prompt.value = steer;
    sendPrompt();
  }, 1200);
}

async function cancelRun() {
  if (acp.ready && acp.sessionId) {
    invoke("grove_acp_cancel", { sessionId: acp.sessionId }).catch(() => {});
    return;
  }
  invoke("grove_cancel").catch(() => {});
}

async function refreshVersion() {
  try {
    const v = await invoke("grove_version");
    els.cliVersion.textContent = "cli: " + v;
    els.cliVersion.title = v;
  } catch (err) {
    els.cliVersion.textContent = "cli: unavailable";
    els.cliVersion.title = String(err);
  }
}

/* ============================ session actions ===================== */

async function openSession(s) {
  els.cwd.value = s.cwd;
  els.transcript.innerHTML = "";
  currentAgentPre = null; acpAgentPre = null; acpThoughtPre = null;
  const empty = document.createElement("div");
  empty.className = "empty";
  empty.innerHTML = '<p class="empty-title">Resuming session…</p><p>' +
    "Loading the conversation the agent kept across the restart.</p>";
  els.transcript.appendChild(empty);
  activeSessionId = s.id;
  await startAcp(s.cwd, s.id, true);
}

async function newChat() {
  const cwd = els.cwd.value.trim();
  activeSessionId = null;
  acp.firstPrompt = null;
  els.transcript.innerHTML = "";
  currentAgentPre = null; acpAgentPre = null; acpThoughtPre = null;
  const empty = document.createElement("div");
  empty.className = "empty";
  empty.innerHTML = '<p class="empty-title">Fresh ground.</p><p>New session in ' +
    (baseName(cwd) || "the working directory") + ".</p>";
  els.transcript.appendChild(empty);
  await startAcp(cwd, null, true);
}

/* ============================ event wiring ======================== */

listen("grove://line", (event) => appendLine(event.payload.stream, event.payload.line));
listen("grove://exit", (event) => {
  const { code, ok } = event.payload;
  state.resumeFlag = true;
  setBusy(false, ok ? "idle" : "exit code " + (code ?? "unknown"));
});
listen("grove://acp", handleAcpEvent);

els.send.addEventListener("click", sendPrompt);
els.cancel.addEventListener("click", cancelRun);
els.newChat.addEventListener("click", newChat);
els.prompt.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    sendPrompt();
  }
});
els.cwd.addEventListener("change", () => {
  const cwd = els.cwd.value.trim();
  if (!cwd) return;
  const last = sessionsForDir(cwd)[0];
  startAcp(cwd, last ? last.id : null, true);
});
els.lookin.addEventListener("click", () => els.lookinPanel.classList.toggle("hidden"));
els.lookinClose.addEventListener("click", () => els.lookinPanel.classList.add("hidden"));
els.sidebarToggle.addEventListener("click", () => {
  els.sidebar.classList.add("collapsed");
  els.sidebarOpen.classList.remove("hidden");
});
els.sidebarOpen.addEventListener("click", () => {
  els.sidebar.classList.remove("collapsed");
  els.sidebarOpen.classList.add("hidden");
});

/* ============================ dictation =========================== */

const dictation = { recording: false, recorder: null, chunks: [], stream: null };

async function toggleDictation() {
  if (dictation.recording) { dictation.recorder?.stop(); return; }
  try {
    dictation.stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (err) {
    els.status.textContent = "mic unavailable: " + (err.name || err);
    return;
  }
  dictation.chunks = [];
  dictation.recorder = new MediaRecorder(dictation.stream);
  dictation.recorder.ondataavailable = (e) => { if (e.data && e.data.size) dictation.chunks.push(e.data); };
  dictation.recorder.onstop = finishDictation;
  dictation.recorder.start();
  dictation.recording = true;
  els.mic.textContent = "stop";
  els.mic.classList.add("rec");
  els.status.textContent = state.busy
    ? "recording a steer — stop to redirect Cedar"
    : "recording — click stop when the thought is out";
}

async function finishDictation() {
  dictation.recording = false;
  els.mic.classList.remove("rec");
  els.mic.textContent = "…";
  els.mic.disabled = true;
  dictation.stream?.getTracks().forEach((t) => t.stop());
  dictation.stream = null;
  const blob = new Blob(dictation.chunks, { type: dictation.recorder?.mimeType || "audio/webm" });
  dictation.recorder = null;
  try {
    const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
    const file = await invoke("grove_save_recording", { bytes });
    els.status.textContent = "transcribing on-device (local whisper)...";
    const text = await invoke("grove_transcribe", { file });
    els.prompt.value = els.prompt.value ? els.prompt.value + "\n" + text : text;
    els.status.textContent = "transcript in the prompt — edit it, then send";
    els.prompt.focus();
  } catch (err) {
    els.status.textContent = "dictation failed: " + String(err).split("\n")[0];
  } finally {
    els.mic.disabled = false;
    els.mic.textContent = "record";
  }
}

els.mic.addEventListener("click", toggleDictation);

/* ============================ boot ================================ */

async function initCwd() {
  const saved = localStorage.getItem("grove.cwd");
  if (saved) { els.cwd.value = saved; return; }
  try { els.cwd.value = await invoke("grove_default_cwd"); }
  catch { els.cwd.value = "/Users/tygranlund/SPRUCE-GROVE-OS"; }
}

loadSessions();
initCwd()
  .then(() => {
    const cwd = els.cwd.value.trim();
    const last = sessionsForDir(cwd)[0];
    return startAcp(cwd, last ? last.id : null, false);
  })
  .then(() => refreshVersion());
