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
  status: document.getElementById("status"),
};

const state = {
  busy: false,
  resume: false, // first prompt starts fresh; later turns quick-resume
};

els.cwd.value =
  localStorage.getItem("grove.cwd") || `${location.host ? "" : ""}${"~"}`;
els.cwd.value = localStorage.getItem("grove.cwd") || defaultCwd();

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

initCwd();
refreshVersion();
