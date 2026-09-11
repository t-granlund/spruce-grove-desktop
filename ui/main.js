/* Thin webview controller: all agent work happens in the CLI child process.
   Uses the global Tauri bridge (withGlobalTauri) — no bundler required. */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const els = {
  cwd: document.getElementById("cwd"),
  cliVersion: document.getElementById("cli-version"),
  transcript: document.getElementById("transcript"),
  empty: document.getElementById("empty-state"),
  prompt: document.getElementById("prompt"),
  send: document.getElementById("send"),
  cancel: document.getElementById("cancel"),
  mic: document.getElementById("mic"),
  status: document.getElementById("status"),
};

const state = {
  busy: false,
  resume: false, // first prompt starts fresh; later turns quick-resume
};

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
  // The webview cannot read $HOME; seed with the fork checkout and let the
  // user retarget. Todo: expose a proper directory picker command.
  return "/Users/tygranlund/SPRUCE-GROVE-OS";
}

function setBusy(busy, note) {
  state.busy = busy;
  els.send.disabled = busy;
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

async function sendPrompt() {
  const prompt = els.prompt.value.trim();
  const cwd = els.cwd.value.trim();
  if (!prompt || !cwd || state.busy) return;

  localStorage.setItem("grove.cwd", cwd);
  addMessage("user", "you");
  els.transcript.lastElementChild.querySelector("pre").textContent = prompt;
  currentAgentPre = null;
  els.prompt.value = "";
  setBusy(true, "grove is thinking...");

  try {
    await invoke("grove_send", { prompt, cwd, resume: state.resume });
  } catch (err) {
    addMessage("agent", "grove · launch error").textContent = String(err);
    setBusy(false, "idle");
  }
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
  state.resume = true; // later turns continue this conversation
  setBusy(false, ok ? "idle" : `exit code ${code ?? "unknown"}`);
});

els.send.addEventListener("click", sendPrompt);
els.cancel.addEventListener("click", () => invoke("grove_cancel").catch(() => {}));
els.prompt.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    sendPrompt();
  }
});

/* ---------------- dictation (mic -> local whisper -> prompt) ----------------
   The pause-edit-adapt loop, desktop edition: record, stop, and the
   transcript lands in the EDITABLE prompt box — tweak it, then send. All
   transcription is on-device via the CLI's --transcribe verb (Mockingbird
   plugin's whisper rig); the shell only ferries bytes. */

const dictation = { recording: false, recorder: null, chunks: [], stream: null };

async function toggleDictation() {
  if (dictation.recording) {
    dictation.recorder?.stop(); // onstop -> finishDictation
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
  els.status.textContent = "recording — click stop when the thought is out";
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

initCwd();
refreshVersion();
