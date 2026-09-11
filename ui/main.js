/* Thin webview controller: all agent work happens in the CLI child process.
   Uses the global Tauri bridge (withGlobalTauri) — no bundler required.

   Transports:
   * ACP (preferred): `spruce-grove --acp` — structured JSON-RPC stream with
     message deltas, thinking, tool calls, per-turn token usage, and
     session/load durability across relaunches.
   * Legacy fallback: headless `-p` line scraping (grove_send).

   Extras riding the stream:
   * Live Look-in panel — browser-tool screenshots render as they are taken
     (paths detected in tool_call payloads, read via grove_read_image_base64).
   * Mid-run steering — sending text while a turn runs cancels the turn and
     redirects within the SAME session (context preserved); voice works too
     because dictation fills the same prompt box. */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const els = {
  cwd: document.getElementById("cwd"),
  cliVersion: document.getElementById("cli-version"),
  mode: document.getElementById("mode"),
  transcript: document.getElementById("transcript"),
  empty: document.getElementById("empty-state"),
  prompt: document.getElementById("prompt"),
  send: document.getElementById("send"),
  cancel: document.getElementById("cancel"),
  mic: document.getElementById("mic"),
  status: document.getElementById("status"),
  lookinBtn: document.getElementById("lookin"),
  lookinPanel: document.getElementById("lookin-panel"),
  lookinImg: document.getElementById("lookin-img"),
  lookinLog: document.getElementById("lookin-log"),
  lookinClose: document.getElementById("lookin-close"),
};

const state = {
  busy: false,
  resume: false, // legacy path: first prompt starts fresh; later turns quick-resume
  pendingSteer: null,
};

const acp = { ready: false, sessionId: null, model: null, cwd: null, turnActive: false };
const toolEls = new Map(); // toolCallId -> { box, chip, pre }

function sessionKey(cwd) {
  return `grove.session.${cwd}`;
}

/* ------------------------------- utilities ------------------------------- */

async function initCwd() {
  const saved = localStorage.getItem("grove.cwd");
  if (saved) {
    els.cwd.value = saved;
    return;
  }
  try {
    els.cwd.value = await invoke("grove_default_cwd");
  } catch {
    els.cwd.value = defaultCwd();
  }
}

function defaultCwd() {
  return "/Users/tygranlund/SPRUCE-GROVE-OS";
}

function setBusy(busy, note) {
  state.busy = busy;
  els.send.disabled = busy && !acp.ready; // with ACP, sending = steering
  els.cancel.disabled = !busy;
  els.status.textContent = note;
  els.status.classList.toggle("busy", busy);
}

function scrollDown() {
  els.transcript.scrollTop = els.transcript.scrollHeight;
}

function addMessage(kind, title) {
  els.empty?.remove();
  const box = document.createElement("div");
  box.className = `msg ${kind}`;
  const who = document.createElement("div");
  who.className = "who";
  who.textContent = title;
  const pre = document.createElement("pre");
  box.append(who, pre);
  els.transcript.appendChild(box);
  scrollDown();
  return pre;
}

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
  scrollDown();
}

/* ------------------------------ ACP streaming ---------------------------- */

let acpAgentPre = null;
let acpThoughtPre = null;

function acpEnsureAgentPre() {
  if (!acpAgentPre) {
    acpThoughtPre = null;
    acpAgentPre = addMessage("agent", `grove · ${acp.model || "agent"}`);
  }
  return acpAgentPre;
}

function acpAppendText(text) {
  if (!text) return;
  const span = document.createElement("span");
  span.textContent = text;
  acpEnsureAgentPre().appendChild(span);
  scrollDown();
}

function acpAppendThought(text) {
  if (!text) return;
  if (!acpThoughtPre) {
    acpThoughtPre = addMessage("thought", "cedar · thinking");
  }
  const span = document.createElement("span");
  span.textContent = text;
  acpThoughtPre.appendChild(span);
  scrollDown();
}

function acpToolCard(update) {
  const id = update.toolCallId || update.title || JSON.stringify(update).slice(0, 40);
  let card = toolEls.get(id);
  if (!card) {
    els.empty?.remove();
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
  scrollDown();
}

/* ---------------------------- live look-in panel ------------------------- */

const LOOKIN_ACTION = /browser|navigate|click|fill|type|upload|screenshot|snapshot|scroll|press|select/i;
const SHOT_PATH = /screenshot_path\\?"?\s*:\s*\\?"([^"\\]+?\.(?:png|jpe?g|webp))/;

function lookinLog(text) {
  if (!els.lookinLog) return;
  const line = document.createElement("div");
  line.className = "lookin-line";
  line.textContent = text;
  els.lookinLog.prepend(line);
  while (els.lookinLog.children.length > 30) {
    els.lookinLog.lastChild.remove();
  }
}

function lookinOpen() {
  els.lookinPanel?.classList.remove("hidden");
}

async function lookinShowScreenshot(rawPath) {
  let path = rawPath;
  try {
    path = JSON.parse(`"${rawPath}"`); // unescape \\uXXXX etc.
  } catch {
    /* keep raw */
  }
  try {
    const dataUrl = await invoke("grove_read_image_base64", { path });
    els.lookinImg.src = dataUrl;
    els.lookinImg.dataset.path = path;
    lookinOpen();
  } catch {
    lookinLog(`screenshot not readable yet: ${path.split("/").pop()}`);
  }
}

function handleToolForLookin(update) {
  if (!update) return;
  const title = String(update.title || "");
  if (LOOKIN_ACTION.test(title)) {
    lookinLog(`${update.status || "…"} · ${title}`);
  }
  const blob = JSON.stringify(update);
  const m = blob.match(SHOT_PATH);
  if (m && m[1]) {
    lookinLog(`screenshot captured`);
    lookinShowScreenshot(m[1]);
  }
}

/* ------------------------------ ACP events ------------------------------- */

async function handleAcpEvent(event) {
  const { kind, data } = event.payload;
  switch (kind) {
    case "ready": {
      acp.ready = true;
      acp.sessionId = data.sessionId;
      acp.model = data.model;
      acp.cwd = els.cwd.value.trim();
      localStorage.setItem(sessionKey(acp.cwd), acp.sessionId);
      els.mode.textContent = `ACP · ${data.model || "live"}${data.resumed ? " · resumed" : ""}`;
      els.mode.dataset.state = "acp";
      els.status.textContent = data.resumed
        ? "grove ready — previous session resumed"
        : "grove ready — structured live session";
      break;
    }
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
        line.textContent = (data.params.toolCall?.title || data.params.toolCall || "")
          .toString()
          .slice(0, 160);
      }
      break;
    case "error":
      if (data && data.message) {
        els.status.textContent = data.message;
      }
      break;
    case "turn-end": {
      const usage = data?.result?.usage || null;
      const toks = usage ? ` · ${usage.totalTokens.toLocaleString()} tok` : "";
      const why = data?.result?.stopReason || (data?.ok === false ? "error" : "done");
      if (data && data.ok === false) {
        addMessage("agent", "grove · error").textContent = String(data.error || "turn failed");
      }
      acpAgentPre = null;
      acpThoughtPre = null;
      acp.turnActive = false;
      toolEls.clear();
      if (acp.sessionId && acp.cwd) {
        localStorage.setItem(sessionKey(acp.cwd), acp.sessionId);
      }
      if (state.pendingSteer && acp.ready) {
        const steer = state.pendingSteer;
        state.pendingSteer = null;
        setBusy(false, "steering Cedar back on course…");
        setTimeout(() => {
          els.prompt.value = steer;
          sendPrompt();
        }, 500);
        return;
      }
      setBusy(false, `done${toks} · ${why}`);
      break;
    }
    default:
      break;
  }
}

async function startAcp(cwd) {
  els.mode.textContent = "connecting…";
  els.mode.dataset.state = "connecting";
  const resume = localStorage.getItem(sessionKey(cwd)) || null;
  try {
    const res = await invoke("grove_acp_start", { cwd, resume });
    acp.sessionId = res.sessionId;
    acp.model = res.model;
    acp.cwd = cwd;
    acp.ready = true;
    localStorage.setItem(sessionKey(cwd), res.sessionId);
    els.mode.textContent = `ACP · ${res.model || "live"}${res.resumed ? " · resumed" : ""}`;
    els.mode.dataset.state = "acp";
    els.status.textContent = res.resumed
      ? "grove ready — previous session resumed"
      : "grove ready — structured live session";
  } catch (err) {
    acp.ready = false;
    els.mode.textContent = "legacy line mode";
    els.mode.dataset.state = "legacy";
    els.status.textContent = `ACP unavailable (${String(err).slice(0, 80)}) — using line mode`;
  }
}

/* ------------------------------ send / cancel ---------------------------- */

async function sendPrompt() {
  const prompt = els.prompt.value.trim();
  const cwd = els.cwd.value.trim();

  // Mid-run steer (ACP only): cancel the turn, redirect within the session.
  if (prompt && state.busy && acp.ready && acp.sessionId && acp.turnActive) {
    pendingSteerFallbackArm();
    state.pendingSteer = prompt;
    els.prompt.value = "";
    els.status.textContent = "steer queued — redirecting Cedar…";
    invoke("grove_acp_cancel", { sessionId: acp.sessionId }).catch(() => {});
    return;
  }

  if (!prompt || !cwd || state.busy) return;

  localStorage.setItem("grove.cwd", cwd);
  addMessage("user", "you");
  els.transcript.lastElementChild.querySelector("pre").textContent = prompt;
  currentAgentPre = null;
  acpAgentPre = null;
  acpThoughtPre = null;
  els.prompt.value = "";

  // ACP path: restart the session if the working dir moved out from under it.
  if (acp.ready && cwd !== acp.cwd) {
    acp.ready = false;
    await startAcp(cwd);
  }

  if (acp.ready && acp.sessionId) {
    setBusy(true, "Cedar is working — streaming live…");
    acp.turnActive = true;
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
    await invoke("grove_send", { prompt, cwd, resume: state.resume });
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
    els.cliVersion.textContent = `cli: ${v}`;
  } catch (err) {
    els.cliVersion.textContent = "cli: unavailable";
    els.cliVersion.title = String(err);
  }
}

listen("grove://line", (event) => {
  appendLine(event.payload.stream, event.payload.line);
});

listen("grove://exit", (event) => {
  const { code, ok } = event.payload;
  state.resume = true;
  setBusy(false, ok ? "idle" : `exit code ${code ?? "unknown"}`);
});

listen("grove://acp", handleAcpEvent);

els.send.addEventListener("click", sendPrompt);
els.cancel.addEventListener("click", cancelRun);
els.prompt.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    sendPrompt();
  }
});
els.lookinBtn?.addEventListener("click", () => els.lookinPanel.classList.toggle("hidden"));
els.lookinClose?.addEventListener("click", () => els.lookinPanel.classList.add("hidden"));

/* ------------------- dictation (mic -> local whisper -> prompt) ----------- */

const dictation = { recording: false, recorder: null, chunks: [], stream: null };

async function toggleDictation() {
  if (dictation.recording) {
    dictation.recorder?.stop();
    return;
  }
  try {
    dictation.stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (err) {
    els.status.textContent = `mic unavailable: ${err.name || err}`;
    return;
  }
  dictation.chunks = [];
  dictation.recorder = new MediaRecorder(dictation.stream);
  dictation.recorder.ondataavailable = (e) => {
    if (e.data && e.data.size) dictation.chunks.push(e.data);
  };
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
  const blob = new Blob(dictation.chunks, {
    type: dictation.recorder?.mimeType || "audio/webm",
  });
  dictation.recorder = null;
  try {
    const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
    const file = await invoke("grove_save_recording", { bytes });
    els.status.textContent = "transcribing on-device (local whisper)...";
    const text = await invoke("grove_transcribe", { file });
    els.prompt.value = els.prompt.value ? `${els.prompt.value}\n${text}` : text;
    els.status.textContent = "transcript in the prompt — edit it, then send";
    els.prompt.focus();
  } catch (err) {
    els.status.textContent = `dictation failed: ${String(err).split("\n")[0]}`;
  } finally {
    els.mic.disabled = false;
    els.mic.textContent = "record";
  }
}

els.mic.addEventListener("click", toggleDictation);

initCwd()
  .then(() => startAcp(els.cwd.value.trim()))
  .then(() => refreshVersion());
