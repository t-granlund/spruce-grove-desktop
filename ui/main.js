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

/* flight recorder: how far did boot actually get in the packaged webview? */
const bootMark = (name, note) => invoke("grove_boot_marker", { name, note }).catch(() => {});
bootMark("mainjs-loaded");

const els = {
  cwd: document.getElementById("cwd"),
  cwdBrowse: document.getElementById("cwd-browse"),
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
  inspector: document.getElementById("inspector"),
  inspectorToggle: document.getElementById("inspector-toggle"),
  inspRefresh: document.getElementById("insp-refresh"),
  inspClose: document.getElementById("insp-close"),
  sessionList: document.getElementById("session-list"),
  liveList: document.getElementById("live-list"),
  liveCount: document.getElementById("live-count"),
  dirList: document.getElementById("dir-list"),
  newChat: document.getElementById("new-chat"),
  sidebar: document.getElementById("sidebar"),
  sidebarToggle: document.getElementById("sidebar-toggle"),
  sidebarOpen: document.getElementById("sidebar-open"),
  engineCheck: document.getElementById("engine-check"),
};

const state = { busy: false, resumeFlag: false, pendingSteer: null };
const acp = {
  ready: false, sessionId: null, model: null, cwd: null,
  turnActive: false, firstPrompt: null, lastEventAt: 0,
  ledgerSeen: new Set(), // tool-call ids already written to the action ledger
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

/* ===================== live sessions (from the CLI) ================= */
/* The shell does not inspect processes itself -- `grove_console_sessions`
   forwards `spruce-grove --console --json`, so this list and the `/console`
   panel can never disagree about what is running. */

let liveSessions = [];
let liveTimer = null;

function ageLabel(seconds) {
  if (seconds == null) return "?";
  if (seconds < 3600) return Math.floor(seconds / 60) + "m";
  if (seconds < 86400) return Math.floor(seconds / 3600) + "h";
  return Math.floor(seconds / 86400) + "d";
}

async function refreshLiveSessions() {
  try {
    liveSessions = await invoke("grove_console_sessions");
  } catch {
    // A CLI that cannot answer must not empty the sidebar of history --
    // just show that we could not see, rather than showing "nothing".
    liveSessions = null;
  }
  renderLive();
}

function renderLive() {
  const list = els.liveList;
  const count = els.liveCount;
  if (!list) return;
  list.innerHTML = "";

  if (liveSessions === null) {
    count.textContent = "";
    list.appendChild(el2("div", "s-meta live-empty", "view unavailable"));
    return;
  }
  count.textContent = liveSessions.length ? String(liveSessions.length) : "";
  if (!liveSessions.length) {
    list.appendChild(el2("div", "s-meta live-empty", "none"));
    return;
  }

  for (const s of liveSessions) {
    const item = el2("div", "sess-item live-item" + (s.terminal_open ? "" : " orphan"));
    item.setAttribute("role", "button");
    item.tabIndex = 0;
    item.title = [
      "pid " + s.pid,
      s.tty || "piped",
      s.cwd || "",
      s.session_name || "(unidentified session)",
    ]
      .filter(Boolean)
      .join("\n");

    item.appendChild(el2("span", "s-title", s.title || "unidentified session"));
    item.appendChild(
      el2(
        "span",
        "s-meta",
        (s.terminal_open ? "" : "orphan · ") +
          baseName(s.cwd) +
          " · pid " +
          s.pid +
          " · " +
          ageLabel(s.uptime_seconds)
      )
    );

    // Attaching can only happen for a session we can name, and only when the
    // shell is idle -- one conversation at a time, as elsewhere.
    if (s.session_name && s.cwd) {
      item.addEventListener("click", () => {
        if (acp.ready && acp.turnActive) return;
        els.cwd.value = s.cwd;
        upsertSession(s.session_name, s.cwd, s.title || "session");
        renderSidebar();
        startAcp(s.cwd, s.session_name);
      });
    }
    list.appendChild(item);
  }
}

function startLivePolling() {
  refreshLiveSessions();
  if (liveTimer) clearInterval(liveTimer);
  // Slow enough to be free, fast enough to feel live.
  liveTimer = setInterval(refreshLiveSessions, 5000);
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
    // role=button div, not <button>: the destructive × must be a real button
    // nested inside, and a <button> may not contain another <button>.
    const item = document.createElement("div");
    item.className = "sess-item" + (s.id === activeSessionId ? " active" : "");
    item.setAttribute("role", "button");
    item.tabIndex = 0;
    const label = "resume session: " + (s.title || "session");
    item.setAttribute("aria-label", label);
    if (s.id === activeSessionId) item.setAttribute("aria-current", "true");
    const title = document.createElement("span");
    title.className = "s-title";
    title.textContent = s.title || "session";
    const meta = document.createElement("span");
    meta.className = "s-meta";
    meta.textContent = baseName(s.cwd) + " · " + relTime(s.ts);
    const del = document.createElement("button");
    del.className = "s-del";
    del.textContent = "×";
    del.title = "forget this session";
    del.setAttribute("aria-label", "forget session: " + (s.title || "session"));
    del.addEventListener("click", (ev) => {
      ev.stopPropagation();
      dropSession(s.id);
    });
    item.append(title, meta, del);
    item.addEventListener("click", () => openSession(s));
    // keyboard activation for the row button (Enter / Space), guarded so
    // keypresses aimed at the nested delete button don't double-fire
    item.addEventListener("keydown", (ev) => {
      if (ev.target !== item) return;
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        openSession(s);
      }
    });
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
      startAcp(dir, last ? last.id : null);
    });
    els.dirList.appendChild(item);
  }
}

/* ============================ ACP session ========================= */

async function startAcp(cwd, resumeId, { create = false } = {}) {
  els.mode.textContent = "connecting…";
  els.mode.dataset.state = "connecting";
  acp.ready = false;
  acp.turnActive = false;
  // the highlight follows the session we are opening, not the one we left:
  // clear it up front so a slow or failed start can't leave a stale ember row
  activeSessionId = null;
  renderSidebar();
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
    localStorage.setItem("grove.cwd", cwd);
    const prev = resumeId ? sessions.find((s) => s.id === resumeId) : null;
    if (prev && res.sessionId !== resumeId) {
      // a revival minted a new session id: carry the old row's title,
      // drop the old row — restarts must not litter the sidebar
      upsertSession(res.sessionId, cwd, prev.title || null);
      dropSession(resumeId);
    } else if (create && !res.resumed) {
      // only a deliberate "new chat" may stamp a placeholder row. Boot
      // auto-start, dir switches and resumes touch no row: sessions are
      // recorded when a prompt is actually sent (sendPrompt upserts with
      // the real first prompt), never as launch residue.
      upsertSession(res.sessionId, cwd, "new session");
    } else if (res.resumed && !sessions.some((s) => s.id === res.sessionId)) {
      // resumed a session the store lost (cleared localStorage): adopt it
      upsertSession(res.sessionId, cwd, null);
    }
    els.mode.textContent = `ACP · ${res.model || "live"}${res.resumed ? " · resumed" : ""}`;
    els.mode.dataset.state = "acp";
    els.status.textContent = res.resumed
      ? "grove ready — previous session resumed"
      : "grove ready — structured live session";
    loadShellProfile(cwd);
    renderSidebar();
    els.prompt.focus();
  } catch (err) {
    acp.ready = false;
    els.mode.textContent = "legacy line mode";
    els.mode.dataset.state = "legacy";
    els.status.textContent = `ACP unavailable (${String(err).slice(0, 80)}) — using line mode`;
    renderSidebar();
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

/* Interruptions must outlive the 200ms status blip: steer and cancel get a
   persistent transcript marker (the .msg.steer ember voice), so a steered
   or canceled turn is readable in the record hours later, not just live. */
function addInterruption(who, text) {
  return addMessage("steer", who).textContent = text;
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

/* ============================ inspector =========================== */
/* Right rail: repository state (live git), GitHub (live gh), and this
   session's turns. Everything read-only; every row with a URL opens in
   the browser, every file row opens with the default app. */

const turns = [];
let inspectorOpen = false;

function pushTurn(text) {
  // a fresh turn closes any turn left running (e.g. a canceled steer)
  turns.forEach((t) => { if (t.state === "running") t.state = "canceled"; });
  turns.unshift({ text: text.slice(0, 64), at: Date.now(), state: "running" });
  if (turns.length > 12) turns.length = 12;
  renderTurns();
}

function finishTurn(data) {
  const t = turns.find((x) => x.state === "running");
  if (!t) return;
  t.state = data && data.ok === false ? "failed" : "done";
  t.secs = Math.round((Date.now() - t.at) / 1000);
  t.tokens = data && data.result && data.result.usage ? data.result.usage.totalTokens : null;
  if (t.state === "failed") counters.turnsFailed++; else counters.turnsDone++;
  if (t.tokens) counters.tokens += t.tokens;
  renderTurns();
  if (inspectorOpen) loadInspector();
}

const TURN_GLYPH = { running: "●", done: "✓", failed: "✗", canceled: "·" };

function renderTurns() {
  const el0 = document.getElementById("insp-turns");
  if (!el0) return;
  el0.textContent = "";
  if (!turns.length) {
    el0.appendChild(el2("div", "insp-empty", "no turns yet"));
    return;
  }
  for (const t of turns) {
    const row = el2("div", "insp-row insp-turn");
    const glyph = el2("span", "insp-glyph g-" + t.state, TURN_GLYPH[t.state] || "·");
    const body = el2("span", "insp-main", t.text);
    const meta = el2("span", "insp-meta");
    const parts = [];
    if (t.secs != null) parts.push(t.secs + "s");
    if (t.tokens) parts.push(Number(t.tokens).toLocaleString() + " tok");
    meta.textContent = parts.join(" · ");
    row.append(glyph, body, meta);
    el0.appendChild(row);
  }
}

/* tiny element helper (named to dodge the .who `el` in older scopes) */
function el2(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}

function inspList(container, rows, render, emptyLabel) {
  container.textContent = "";
  if (!rows || !rows.length) {
    container.appendChild(el2("div", "insp-empty", emptyLabel || "—"));
    return;
  }
  for (const r of rows) container.appendChild(render(r));
}

function runGlyph(r) {
  if (r.conclusion === "success") return ["✓", "g-done"];
  if (r.conclusion === "failure" || r.conclusion === "timed_out") return ["✗", "g-failed"];
  if (r.conclusion === "skipped" || r.conclusion === "cancelled") return ["−", "g-neutral"];
  return ["●", "g-running"]; // queued / in_progress
}

function renderInspector(st) {
  const branch = document.getElementById("insp-branch");
  branch.textContent = "";
  branch.append(
    el2("span", "insp-branch-name", "⎇ " + (st.branch || "?")),
    st.dirty_total
      ? el2("span", "insp-dirty-badge", st.dirty_total + " uncommitted")
      : el2("span", "insp-dirty-badge clean", "clean"),
  );

  inspList(document.getElementById("insp-commits"), st.commits, (c) => {
    const row = el2("button", "insp-row insp-click");
    if (st.repo) row.dataset.url = `https://github.com/${st.repo}/commit/${c.hash}`;
    row.title = c.subject;
    row.append(
      el2("span", "insp-hash", c.hash),
      el2("span", "insp-main", c.subject),
      el2("span", "insp-meta", c.when || ""),
    );
    return row;
  });

  const dirtyEl = document.getElementById("insp-dirty");
  inspList(dirtyEl, st.dirty, (f) => {
    const row = el2("button", "insp-row insp-click");
    row.dataset.path = f.path;
    row.title = "open " + f.path;
    row.append(el2("span", "insp-hash", f.status), el2("span", "insp-main", f.path));
    return row;
  });
  if (!st.dirty_total) {
    dirtyEl.textContent = "";
    dirtyEl.appendChild(el2("div", "insp-empty", "working tree clean"));
  }

  const gh = st.github || {};
  const links = document.getElementById("insp-gh-links");
  links.textContent = "";
  if (st.urls) {
    for (const [label, url] of Object.entries(st.urls)) {
      const a = el2("button", "insp-link", label);
      a.dataset.url = url;
      a.title = url;
      links.appendChild(a);
    }
  } else {
    links.appendChild(el2("span", "insp-empty", "no github origin"));
  }

  const prEmpty = gh.gh_ok ? "no open pull requests" : (gh.note || "gh unavailable");
  inspList(document.getElementById("insp-prs"), gh.gh_ok ? gh.prs : null, (p) => {
    const row = el2("button", "insp-row insp-click");
    row.dataset.url = p.url;
    row.title = p.title;
    const state = p.isDraft ? "draft" : String(p.state || "open").toLowerCase();
    row.append(
      el2("span", "insp-hash", "#" + p.number),
      el2("span", "insp-main", p.title),
      el2("span", "insp-chip chip-" + state, state),
    );
    return row;
  }, prEmpty);

  const runEmpty = gh.gh_ok ? "no recent runs" : "—";
  inspList(document.getElementById("insp-runs"), gh.gh_ok ? gh.runs : null, (r) => {
    const row = el2("button", "insp-row insp-click");
    if (r.url) row.dataset.url = r.url;
    const [glyphText, glyphClass] = runGlyph(r);
    const glyph = el2("span", "insp-glyph " + glyphClass, glyphText);
    row.append(glyph, el2("span", "insp-main", r.displayTitle || "run"));
    let dur = "";
    if (r.createdAt && r.updatedAt) {
      const ms = new Date(r.updatedAt) - new Date(r.createdAt);
      if (Number.isFinite(ms) && ms > 0) dur = Math.round(ms / 1000) + "s";
    }
    const bits = [r.headBranch, dur].filter(Boolean);
    if (bits.length) row.appendChild(el2("span", "insp-meta", bits.join(" · ")));
    return row;
  }, runEmpty);
}

async function loadInspector() {
  const cwd = els.cwd.value.trim();
  if (!cwd) return;
  els.inspRefresh.classList.add("spinning");
  try {
    renderInspector(await invoke("grove_git_state", { cwd }));
    document.getElementById("insp-updated").textContent =
      "updated " + new Date().toTimeString().slice(0, 8);
  } catch (err) {
    const branch = document.getElementById("insp-branch");
    branch.textContent = "";
    branch.appendChild(el2("span", "insp-empty", String(err).split("\n")[0]));
    ["insp-commits", "insp-dirty", "insp-prs", "insp-runs"].forEach((id) => {
      const n = document.getElementById(id);
      n.textContent = "";
      n.appendChild(el2("div", "insp-empty", "—"));
    });
    const links = document.getElementById("insp-gh-links");
    links.textContent = "";
    links.appendChild(el2("span", "insp-empty", "no github origin"));
  } finally {
    els.inspRefresh.classList.remove("spinning");
  }
  renderTurns();
}

function toggleInspector(force) {
  const opening = force != null ? force : !inspectorOpen;
  if (opening && !personaAllows("inspector")) {
    els.status.textContent = "persona \"" + (activePersona()?.name || "?") + "\" may not open the inspector";
    return;
  }
  inspectorOpen = opening;
  els.inspector.classList.toggle("hidden", !inspectorOpen);
  els.inspectorToggle.classList.toggle("active", inspectorOpen);
  els.inspectorToggle.setAttribute("aria-pressed", String(inspectorOpen));
  localStorage.setItem("grove.inspector", inspectorOpen ? "1" : "");
  if (inspectorOpen) loadInspector();
}

/* Deck hook: the presentation (or any embedder) can ask the app to open the
   receipts — inspector on a given pane — by posting a message. Inert in the
   packaged app, where nothing posts it. Lets a live demo land on the project
   tracker / self-audit surfaces without the presenter hunting for ⌘I. */
window.addEventListener("message", (ev) => {
  const d = ev && ev.data;
  if (!d) return;
  if (d.type === "grove-show-receipts") {
    toggleInspector(true);
    const tab = d.tab === "project" ? "project" : "diag";
    const el = document.querySelector(".insp-tab[data-tab='" + tab + "']");
    if (el) selectInspTab(el);
  } else if (d.type === "grove-hide-receipts") {
    toggleInspector(false);
  }
});

els.inspector.addEventListener("click", (ev) => {
  const t = ev.target.closest("[data-url],[data-path]");
  if (!t) return;
  if (t.dataset.url) invoke("grove_open_url", { url: t.dataset.url }).catch(() => {});
  else if (t.dataset.path) invoke("grove_open_path", { path: t.dataset.path }).catch(() => {});
});
els.inspectorToggle.addEventListener("click", () => toggleInspector());
els.inspRefresh.addEventListener("click", () => {
  loadInspector();
  // refresh whichever non-default panes are the ones on screen
  const active = document.querySelector(".insp-tab.active");
  if (active?.dataset.tab === "project") refreshProject();
  if (active?.dataset.tab === "diag") refreshDiag();
});
els.inspClose.addEventListener("click", () => toggleInspector(false));
document.addEventListener("keydown", (ev) => {
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "i" && !ev.shiftKey) {
    ev.preventDefault();
    toggleInspector();
  }
});

/* ============================ diagnostics ========================== */
/* The inspector's second pane: platform, engine seams, analytics, and
   the session's error ring — the oversight half of "full oversight". */

let diagVisible = false;

function kvRow(k, v) {
  const row = el2("div", "kv");
  row.append(el2("span", "k", k), el2("span", "v", String(v)));
  return row;
}

function renderDiag(d) {
  if (!d) return;
  const platform = document.getElementById("diag-platform");
  platform.textContent = "";
  platform.append(
    kvRow("os", d.os), kvRow("arch", d.arch),
    kvRow("app", "v" + d.app_version), kvRow("pid", d.pid),
    kvRow("cli", (els.cliVersion.textContent || "").replace(/^cli:\s*/, "")),
  );
  const engine = document.getElementById("diag-engine");
  engine.textContent = "";
  engine.append(
    kvRow("event bridge", d.bridge_probe_acked ? "live (probe acked)" : "DEAD — no probe ack"),
    kvRow("webview scripts", d.boot_mainjs ? "loaded" : "not seen"),
    kvRow("event listeners", d.boot_listen_ok ? "registered" : "not seen"),
    kvRow("acp session", acp.sessionId ? acp.sessionId.slice(0, 16) : "none"),
    kvRow("acp model", acp.model || "—"),
  );
  const analytics = document.getElementById("diag-analytics");
  analytics.textContent = "";
  analytics.append(
    kvRow("turns done", counters.turnsDone),
    kvRow("turns failed", counters.turnsFailed),
    kvRow("tokens (session)", counters.tokens.toLocaleString()),
    kvRow("self-heals", counters.heals),
    kvRow("watchdog fires", counters.watchdog),
    kvRow("errors (ring)", errorRing.length),
    kvRow("errors (persisted)", (d.persisted_errors || []).length),
  );
  const errs = document.getElementById("diag-errors");
  errs.textContent = "";
  if (!errorRing.length) {
    errs.appendChild(el2("div", "insp-empty", "clean — nothing to report"));
  }
  for (const e of errorRing) {
    const row = el2("div", "insp-row");
    row.append(
      el2("span", "insp-meta", e.at),
      el2("span", "insp-hash", e.source),
      el2("span", "insp-main", e.message),
    );
    errs.appendChild(row);
  }
  // the durable trail: what previous sessions left in the data dir
  const persisted = document.getElementById("diag-persisted");
  persisted.textContent = "";
  const past = d.persisted_errors || [];
  if (!past.length) {
    persisted.appendChild(el2("div", "insp-empty", "no persisted errors yet"));
  }
  for (const e of past.slice(0, 12)) {
    const row = el2("div", "insp-row");
    row.append(
      el2("span", "insp-meta", String(e.at || "").replace("T", " ").replace("Z", "")),
      el2("span", "insp-hash", e.source),
      el2("span", "insp-main", e.message),
    );
    persisted.appendChild(row);
  }
}

async function refreshDiag() {
  try { renderDiag(await invoke("grove_diagnostics")); }
  catch (err) { noteError("diagnostics", err); }
  refreshAudit();
}

/* ---- self-audit + action ledger: the containment + receipts surface ---- */

function renderAudit(a) {
  const floor = document.getElementById("diag-floor");
  if (!floor || !a) return;
  floor.textContent = "";
  floor.append(
    kvRow("floor", a.floor_intact ? "INTACT" : "LOWERED"),
    kvRow("csp hardening", (a.required_hardening || []).length - (a.missing_hardening || []).length
      + "/" + (a.required_hardening || []).length + " directives"),
    kvRow("capability window", (a.capability && a.capability.windows || []).join(", ") || "—"),
    kvRow("capability perms", ((a.capability && a.capability.permissions) || []).join(", ") || "—"),
  );
  if (a.missing_hardening && a.missing_hardening.length) {
    floor.append(kvRow("missing", a.missing_hardening.join("; ")));
  }
}

async function refreshAudit() {
  try { renderAudit(await invoke("grove_self_audit")); }
  catch (err) { noteError("self-audit", err); }
  try {
    const led = await invoke("grove_ledger_tail", { limit: 60 });
    renderLedger(led);
  } catch (err) { noteError("ledger", err); }
}

function renderLedger(led) {
  const box = document.getElementById("diag-ledger");
  if (!box) return;
  box.textContent = "";
  const entries = (led && led.entries) || [];
  if (!entries.length) {
    box.appendChild(el2("div", "insp-empty", "no actions recorded yet"));
    return;
  }
  // newest first — the tail reversed for reading
  for (const e of entries.slice().reverse()) {
    const row = el2("div", "insp-row");
    row.append(
      el2("span", "insp-meta", String(e.at || "").replace("T", " ").replace("Z", "")),
      el2("span", "insp-hash", e.kind || ""),
      el2("span", "insp-main", (e.summary || "") + (e.outcome ? " · " + e.outcome : "")),
    );
    box.appendChild(row);
  }
}

/* Records one action to the ledger. Best-effort: never block the action it
   describes, never surface a ledger failure as a run failure. */
function recordAction(kind, summary, outcome) {
  const cwd = (acp && acp.cwd) || state.cwd || "";
  invoke("grove_ledger_record", { kind, cwd, summary: String(summary).slice(0, 240), outcome })
    .catch(() => {});
}

/* ---- core project view: the directory's own tracker + governance ---- */

function renderProject(p) {
  const cwd = document.getElementById("proj-cwd");
  const counts = document.getElementById("proj-counts");
  const issues = document.getElementById("proj-issues");
  const docs = document.getElementById("proj-docs");
  if (!cwd) return;
  cwd.textContent = p.cwd || (acp && acp.cwd) || state.cwd || "—";
  counts.textContent = "";
  issues.textContent = "";
  docs.textContent = "";

  if (!p.is_beads) {
    counts.appendChild(el2("div", "insp-empty", p.note || "not a bead store"));
    return;
  }
  if (!p.bd_found || p.ok === false) {
    counts.appendChild(el2("div", "insp-empty", p.note || "bd unavailable"));
    return;
  }
  const c = p.counts || {};
  counts.append(
    kvRow("in progress", c.in_progress ?? 0),
    kvRow("blocked", c.blocked ?? 0),
    kvRow("open", c.open ?? 0),
    kvRow("closed", c.closed ?? 0),
    kvRow("total", c.total ?? 0),
    kvRow("source", "bd list · beads default db"),
  );

  const rankClass = (s) => s === "in_progress" ? "run" : s === "blocked" ? "bad" : s === "closed" ? "ok" : "idle";
  for (const it of (p.issues || [])) {
    const row = el2("div", "insp-row");
    row.append(
      el2("span", "insp-meta " + rankClass(it.status), it.status || ""),
      el2("span", "insp-hash", "P" + (it.priority ?? "?")),
      el2("span", "insp-main", (it.id ? it.id.replace(/^.*-/, "") + " · " : "") + (it.title || "")),
    );
    row.title = it.id || "";
    issues.appendChild(row);
  }
  if (!(p.issues || []).length) issues.appendChild(el2("div", "insp-empty", "tracker empty"));

  for (const d of (p.docs || [])) {
    const row = el2("div", "insp-row");
    const open = el2("span", "insp-main", d.label + "  " + d.path);
    if (d.present) {
      open.classList.add("proj-doc");
      open.title = "open in default app";
      open.addEventListener("click", () => {
        const base = (acp && acp.cwd) || state.cwd || "";
        invoke("grove_open_path", { path: base.replace(/\/$/, "") + "/" + d.path })
          .catch((err) => noteError("open doc", err));
      });
    } else {
      open.classList.add("insp-empty");
    }
    row.append(el2("span", "insp-hash", d.present ? "open" : "absent"), open);
    docs.appendChild(row);
  }
}

async function refreshProject() {
  const cwd = (acp && acp.cwd) || state.cwd || "";
  try { renderProject(await invoke("grove_project_state", { cwd })); }
  catch (err) { noteError("project", err); }
}

document.getElementById("diag-copy").addEventListener("click", () => {
  const platform = [...document.querySelectorAll("#diag-platform .kv")]
    .map((r) => "- " + r.children[0].textContent + ": " + r.children[1].textContent).join("\n");
  const engine = [...document.querySelectorAll("#diag-engine .kv")]
    .map((r) => "- " + r.children[0].textContent + ": " + r.children[1].textContent).join("\n");
  const analytics = [...document.querySelectorAll("#diag-analytics .kv")]
    .map((r) => "- " + r.children[0].textContent + ": " + r.children[1].textContent).join("\n");
  const errors = errorRing.length
    ? errorRing.map((e) => "- " + e.at + " [" + e.source + "] " + e.message).join("\n")
    : "- none";
  const report = "# Spruce Grove desktop — session report\n\n## Platform\n" + platform +
    "\n\n## Engine\n" + engine + "\n\n## Analytics\n" + analytics +
    "\n\n## Errors\n" + errors + "\n";
  navigator.clipboard.writeText(report)
    .then(() => els.status.textContent = "diagnostics report copied to clipboard")
    .catch(() => els.status.textContent = "clipboard unavailable");
});

document.getElementById("diag-data-dir").addEventListener("click", () => {
  invoke("grove_diagnostics")
    .then((d) => invoke("grove_open_path", { path: d.data_dir }))
    .then(() => els.status.textContent = "opened data dir")
    .catch((err) => { els.status.textContent = String(err).split("\n")[0]; noteError("diagnostics", err); });
});

/* inspector tabs — role=tablist semantics, arrow keys walk the pair */
function selectInspTab(tab) {
  document.querySelectorAll(".insp-tab").forEach((t) => {
    t.classList.toggle("active", t === tab);
    t.setAttribute("aria-selected", String(t === tab));
    t.tabIndex = t === tab ? 0 : -1;
  });
  document.getElementById("insp-pane-repo").classList.toggle("hidden", tab.dataset.tab !== "repo");
  document.getElementById("insp-pane-project").classList.toggle("hidden", tab.dataset.tab !== "project");
  document.getElementById("insp-pane-diag").classList.toggle("hidden", tab.dataset.tab !== "diag");
  if (tab.dataset.tab === "project") refreshProject();
  if (tab.dataset.tab === "diag") refreshDiag();
}

document.querySelectorAll(".insp-tab").forEach((tab) => {
  tab.addEventListener("click", () => selectInspTab(tab));
  tab.addEventListener("keydown", (ev) => {
    if (ev.key !== "ArrowLeft" && ev.key !== "ArrowRight") return;
    ev.preventDefault();
    const tabs = [...document.querySelectorAll(".insp-tab")];
    const i = tabs.indexOf(tab);
    const next = tabs[(i + (ev.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
    next.focus();
    selectInspTab(next);
  });
});

/* ============================ settings ============================= */
/* ⌘, — persisted to the platform data dir, applied live. Personas bind
   allowed working dirs + feature grants; repo access is GitHub's own
   viewerPermission for the watched repos, not an invented role. */

let settingsCache = null;
let settingsVisible = false;
let personaDraft = [];

function activePersona() {
  if (!settingsCache || !settingsCache.active_persona) return null;
  return (settingsCache.personas || []).find((p) => p.name === settingsCache.active_persona) || null;
}

function personaAllows(grant) {
  const p = activePersona();
  if (!p) return true; // no active persona = the owner's machine, full trust
  return !!(p.grants && p.grants[grant]);
}

function personaAllowsDir(cwd) {
  const p = activePersona();
  if (!p || !Array.isArray(p.dirs) || !p.dirs.length) return true;
  return p.dirs.some((d) => {
    const base = String(d).replace(/\/+$/, "");
    return cwd === base || cwd.startsWith(base + "/");
  });
}

async function loadSettings() {
  try {
    settingsCache = await invoke("grove_settings_get");
  } catch (err) {
    noteError("settings", err);
    settingsCache = null;
  }
}

async function saveSettings(next) {
  const saved = await invoke("grove_settings_set", { settings: next });
  settingsCache = saved;
  return saved;
}

function renderPersonas() {
  const box = document.getElementById("set-personas");
  box.textContent = "";
  if (!personaDraft.length) {
    box.appendChild(el2("div", "insp-empty", "no personas — the owner has full trust"));
  }
  for (const p of personaDraft) {
    const row = el2("div", "persona-row");
    const head = el2("div", "persona-head");
    const radio = el2("input");
    radio.type = "radio";
    radio.name = "active-persona";
    radio.checked = settingsCache && settingsCache.active_persona === p.name;
    radio.title = "make active";
    radio.addEventListener("change", () => { personaDraft.forEach((x) => (x._active = false)); p._active = true; });
    head.append(radio, el2("input", "persona-name", ""));
    head.lastChild.value = p.name;
    head.lastChild.addEventListener("input", (e) => { p.name = e.target.value; });
    const del = el2("button", "icon-btn", "×");
    del.title = "remove persona";
    del.addEventListener("click", () => {
      personaDraft = personaDraft.filter((x) => x !== p);
      if (p._active) p._active = false;
      renderPersonas();
    });
    head.append(el2("span", "insp-meta", p.permission || ""), del);
    row.appendChild(head);
    const grants = el2("div", "grants");
    for (const g of ["prompts", "dictation", "lookin", "inspector"]) {
      const label = el2("label");
      const cb = el2("input");
      cb.type = "checkbox";
      cb.checked = !!(p.grants && p.grants[g]);
      cb.addEventListener("change", () => {
        p.grants = p.grants || {};
        p.grants[g] = cb.checked;
      });
      label.append(cb, el2("span", null, g));
      grants.appendChild(label);
    }
    row.appendChild(grants);
    const dirs = el2("input", "persona-dirs", "");
    dirs.placeholder = "allowed working dirs, comma-separated (empty = all)";
    dirs.setAttribute("aria-label", "allowed working directories for " + (p.name || "persona"));
    dirs.value = (p.dirs || []).join(", ");
    dirs.addEventListener("input", () => {
      p.dirs = dirs.value.split(",").map((s) => s.trim()).filter(Boolean);
    });
    row.appendChild(dirs);
    box.appendChild(row);
  }
}

function renderRepoAccess() {
  const box = document.getElementById("set-repo-access");
  box.textContent = "";
  box.appendChild(el2("div", "insp-empty", "checking with gh…"));
  const repos = settingsCache && settingsCache.watched_repos;
  invoke("grove_repo_access", { repos })
    .then((res) => {
      box.textContent = "";
      if (res.gh_ok && res.login) {
        box.appendChild(kvRow("github account", res.login));
      }
      for (const r of res.repos || []) {
        const row = el2("div", "kv");
        row.append(
          el2("span", "k", r.repo),
          el2("span", "v", r.ok ? String(r.permission || "none") : "unavailable"),
        );
        box.appendChild(row);
      }
      if (!res.gh_ok) box.appendChild(el2("div", "insp-empty", res.note || "gh unavailable"));
    })
    .catch((err) => {
      box.textContent = "";
      box.appendChild(el2("div", "insp-empty", String(err).split("\n")[0]));
    });
}

function collectSettings() {
  const next = JSON.parse(JSON.stringify(settingsCache || {}));
  next.version = 1;
  next.default_cwd = document.getElementById("set-default-cwd").value.trim();
  next.inspector_auto_open = document.getElementById("set-insp-auto").checked;
  next.error_report_level = document.getElementById("set-err-level").value;
  next.personas = personaDraft
    .filter((p) => p.name && p.name.trim())
    .map((p) => ({
      name: p.name.trim(),
      dirs: p.dirs || [],
      grants: p.grants || {},
    }));
  const active = personaDraft.find((p) => p._active);
  next.active_persona = active && active.name && active.name.trim() ? active.name.trim() : null;
  return next;
}

let settingsInvoker = null;

function openSettings() {
  // dialog focus discipline: remember the invoker, move focus in, and
  // restore it on close so keyboard/SR users are never stranded behind
  // the overlay
  settingsInvoker = document.activeElement;
  settingsVisible = true;
  document.getElementById("settings-overlay").classList.remove("hidden");
  document.getElementById("set-default-cwd")?.focus();
  loadSettings().then(() => {
    const s = settingsCache || {};
    document.getElementById("set-default-cwd").value = s.default_cwd || "";
    document.getElementById("set-insp-auto").checked = !!s.inspector_auto_open;
    document.getElementById("set-err-level").value = s.error_report_level || "errors";
    personaDraft = (s.personas || []).map((p) => ({ ...p, _active: s.active_persona === p.name }));
    renderPersonas();
    const about = document.getElementById("set-about");
    about.textContent = "";
    about.append(kvRow("app", "Spruce Grove desktop v0.1.0"));
    invoke("grove_diagnostics").then((d) => {
      about.textContent = "";
      about.append(kvRow("os", d.os), kvRow("app", "v" + d.app_version), kvRow("data dir", d.data_dir));
    }).catch(() => {});
    renderRepoAccess();
  });
}

function closeSettings() {
  settingsVisible = false;
  document.getElementById("settings-overlay").classList.add("hidden");
  if (settingsInvoker && document.contains(settingsInvoker)) settingsInvoker.focus();
  settingsInvoker = null;
}

document.getElementById("settings-save").addEventListener("click", () => {
  saveSettings(collectSettings())
    .then(() => {
      document.getElementById("settings-status").textContent = "saved";
      els.status.textContent = "settings saved — applied";
    })
    .catch((err) => {
      document.getElementById("settings-status").textContent = String(err).split("\n")[0];
      noteError("settings", err);
    });
});
document.getElementById("settings-close").addEventListener("click", closeSettings);
document.getElementById("set-persona-add").addEventListener("click", () => {
  personaDraft.push({ name: "persona " + (personaDraft.length + 1), dirs: [], grants: { prompts: true, dictation: true, lookin: true, inspector: true }, _active: false });
  renderPersonas();
});
document.getElementById("set-access-refresh").addEventListener("click", renderRepoAccess);
document.getElementById("settings-overlay").addEventListener("mousedown", (ev) => {
  if (ev.target.id === "settings-overlay") closeSettings();
});
document.addEventListener("keydown", (ev) => {
  if ((ev.metaKey || ev.ctrlKey) && ev.key === ",") {
    ev.preventDefault();
    settingsVisible ? closeSettings() : openSettings();
  }
  if (ev.key === "Escape") {
    if (settingsVisible) { closeSettings(); return; }
    // Escape belongs to the top-most layer only: settings, then picker, then drawer.
    // The picker overlay is lazily mounted by picker.js on first browse — until
    // then getElementById returns null, which is definitionally "not open".
    const pickerEl = document.getElementById("dir-picker");
    const pickerOpen = !!pickerEl && !pickerEl.classList.contains("hidden");
    if (!pickerOpen && inspectorOpen) toggleInspector(false);
  }
});

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
        // receipt: record once the card first appears, not on every update
        const u = data.update;
        const key = u.toolCallId || u.title || "";
        if (key && !acp.ledgerSeen.has(key)) {
          acp.ledgerSeen.add(key);
          recordAction("tool", u.title || u.kind || "tool", u.status || "pending");
        }
      }
      break;
    case "permission":
      if (data && data.params) {
        const title = String(
          (data.params.toolCall && data.params.toolCall.title) || data.params.toolCall || ""
        ).slice(0, 160);
        const line = addMessage("permission", "permission · auto-allowed");
        line.textContent = title;
        recordAction("permission", title || "permission request", "auto-allowed");
      }
      break;
    case "error":
      if (data && data.message) {
        els.status.textContent = data.message;
        noteError("acp", data.message);
        if (/agent process exited/.test(data.message)) selfHeal(acp.sessionId);
      }
      break;
    case "log":
      // CLI diagnostics stream: show the latest line while a turn is live
      if (state.busy && data && data.line) {
        els.status.textContent = "Cedar is working — " + data.line;
      }
      break;
    case "turn-end": {
      finishTurn(data);
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

/* diagnostics: an honest local record — errors ring + counters. Nothing
   here leaves the machine (charter: zero telemetry); the report copies to
   the clipboard only when the human asks. */
const errorRing = [];
const counters = { heals: 0, watchdog: 0, turnsDone: 0, turnsFailed: 0, tokens: 0 };
function noteError(source, message) {
  errorRing.unshift({
    at: new Date().toTimeString().slice(0, 8),
    source,
    message: String(message).slice(0, 220),
  });
  if (errorRing.length > 40) errorRing.length = 40;
  // the oversight charter: the trail survives restarts — persist fire-and-forget
  invoke("grove_note_error", { source, message: String(message).slice(0, 220) }).catch(() => {});
  if (diagVisible) renderDiag();
}

/* Self-heal: the CLI also EXITS outright on provider hiccups (observed:
   ModelAPIError connection error kills it after a turn). The death arrives
   as an error event; revive the session quietly — resume carries the
   history, the next send just works. No human incantation. */
let healing = false;
function selfHeal(deadId) {
  if (healing || !deadId || !acp.ready) return;
  healing = true;
  counters.heals++;
  noteError("self-heal", "cli exited — reviving session " + deadId.slice(0, 12));
  const cwd = els.cwd.value.trim();
  els.status.textContent = "cli exited — reviving the session…";
  setTimeout(async () => {
    invoke("grove_acp_kill").catch(() => {});
    acp.ready = false;
    try { await startAcp(cwd, deadId, true); }
    catch { /* startAcp handles its own fallbacks */ }
    healing = false;
  }, 500);
}

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
  counters.watchdog++;
  noteError("watchdog", "no stream activity for " + Math.round(STALL_KILL_MS / 1000) + "s — hard restart");
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
    setTimeout(() => startAcp(cwd, dead), 400);
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
    addInterruption("steer · queued", prompt);
    els.status.textContent = "steer queued — redirecting Cedar…";
    invoke("grove_acp_cancel", { sessionId: acp.sessionId }).catch(() => {});
    pendingSteerFallbackArm();
    return;
  }

  if (!prompt || !cwd || state.busy) return;

  // persona gates — the choke point for "who may ask what, where"
  if (!personaAllows("prompts")) {
    els.status.textContent = "persona \"" + (activePersona()?.name || "?") + "\" may not send prompts";
    noteError("persona", "send blocked for " + (activePersona()?.name || "?"));
    return;
  }
  if (!personaAllowsDir(cwd)) {
    els.status.textContent = "persona \"" + (activePersona()?.name || "?") + "\" cannot work in " + cwd;
    noteError("persona", "cwd blocked: " + cwd);
    return;
  }

  localStorage.setItem("grove.cwd", cwd);
  pushTurn(prompt);
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
    if (acp.turnActive) {
      addInterruption("canceled", "turn canceled — the CLI was told to stop; history is kept.");
    }
    invoke("grove_acp_cancel", { sessionId: acp.sessionId }).catch(() => {});
    recordAction("cancel", acp.turnActive ? "canceled live turn" : "cancel no-op", "requested");
    return;
  }
  if (state.busy) {
    addInterruption("canceled", "run canceled.");
  }
  recordAction("cancel", "canceled run", "requested");
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
  await startAcp(s.cwd, s.id);
}

async function newChat() {
  const cwd = els.cwd.value.trim();
  acp.firstPrompt = null;
  els.transcript.innerHTML = "";
  currentAgentPre = null; acpAgentPre = null; acpThoughtPre = null;
  const empty = document.createElement("div");
  empty.className = "empty";
  empty.innerHTML = '<p class="empty-title">Fresh ground.</p><p>New session in ' +
    (baseName(cwd) || "the working directory") + ".</p>";
  els.transcript.appendChild(empty);
  // a deliberate new chat is the one flow that may stamp a placeholder row
  await startAcp(cwd, null, { create: true });
}

/* ============================ event wiring ======================== */

/* A rejected listen() means the event bridge itself is dead (capabilities,
   plugin permissions) — the app then looks alive while every stream is
   silently dropped. Name that failure in the cli pill tooltip instead of
   letting the promise rejection vanish. */
function safeListen(name, handler) {
  listen(name, handler)
    .then(() => bootMark("listen-ok"))
    .catch((err) => {
      bootMark("listen-fail-" + name.replace(/[^a-z-]/gi, "_"), String(err));
      els.cliVersion.textContent = "cli: events dead";
      els.cliVersion.title = "event bridge failed for " + name + ": " + err;
    });
}

safeListen("grove://line", (event) => appendLine(event.payload.stream, event.payload.line));
safeListen("grove://exit", (event) => {
  const { code, ok } = event.payload;
  state.resumeFlag = true;
  setBusy(false, ok ? "idle" : "exit code " + (code ?? "unknown"));
});
safeListen("grove://acp", handleAcpEvent);

/* bridge self-check: the shell emits this 2s after boot; acking it proves
   rust -> webview events AND webview -> rust invokes, end to end */
safeListen("grove://bridge-probe", () => invoke("grove_bridge_probe_ack").catch(() => {}));

els.send.addEventListener("click", sendPrompt);
els.engineCheck?.addEventListener("click", () => {
  els.prompt.value = "/onboard-synthetic check";
  sendPrompt();
});
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
  if (!personaAllowsDir(cwd)) {
    els.status.textContent = "persona '" + (activePersona()?.name || "?") + "' does not cover " + cwd;
    noteError("persona", "cwd switch blocked: " + cwd);
    return;
  }
  const last = sessionsForDir(cwd)[0];
  startAcp(cwd, last ? last.id : null, true);
});
els.cwdBrowse?.addEventListener("click", async () => {
  // pick in-webview (ui/picker.js), then ride the existing change flow
  const picked = await window.grovePickDirectory(els.cwd.value.trim());
  if (!picked || picked === els.cwd.value.trim()) return;
  els.cwd.value = picked;
  els.cwd.dispatchEvent(new Event("change"));
});
els.lookin.addEventListener("click", () => {
  if (!personaAllows("lookin")) {
    els.status.textContent = "persona '" + (activePersona()?.name || "?") + "' may not use look-in";
    noteError("persona", "look-in blocked");
    return;
  }
  els.lookinPanel.classList.toggle("hidden");
});
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

/* the mic button carries an icon + label now — only the label ever moves */
function micLabel(text) {
  const lbl = els.mic.querySelector(".lbl");
  if (lbl) lbl.textContent = text; else els.mic.textContent = text;
}

/* recording state lives in a chip: pulsing dot + elapsed seconds */
let recTimer = null;
function recChipShow() {
  const chip = document.getElementById("rec-chip");
  const time = document.getElementById("rec-time");
  const t0 = Date.now();
  const tick = () => {
    if (!time) return;
    const s = Math.floor((Date.now() - t0) / 1000);
    time.textContent = Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0");
  };
  tick();
  recTimer = setInterval(tick, 1000);
  chip?.classList.remove("hidden");
}
function recChipHide() {
  clearInterval(recTimer);
  recTimer = null;
  document.getElementById("rec-chip")?.classList.add("hidden");
}

async function toggleDictation() {
  if (dictation.recording) { dictation.recorder?.stop(); return; }
  if (!personaAllows("dictation")) {
    els.status.textContent = "persona \"" + (activePersona()?.name || "?") + "\" may not record";
    return;
  }
  if (!personaAllows("dictation")) {
    els.status.textContent = "persona '" + (activePersona()?.name || "?") + "' may not dictate";
    noteError("persona", "dictation blocked");
    return;
  }
  try {
    dictation.stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (err) {
    els.status.textContent = "mic unavailable: " + (err.name || err);
    noteError("mic", (err && err.name) || err);
    return;
  }
  dictation.chunks = [];
  dictation.recorder = new MediaRecorder(dictation.stream);
  dictation.recorder.ondataavailable = (e) => { if (e.data && e.data.size) dictation.chunks.push(e.data); };
  dictation.recorder.onstop = finishDictation;
  dictation.recorder.start();
  dictation.recording = true;
  micLabel("stop");
  els.mic.classList.add("rec");
  recChipShow();
  els.status.textContent = state.busy
    ? "recording a steer — stop to redirect Cedar"
    : "recording — click stop when the thought is out";
}

async function finishDictation() {
  dictation.recording = false;
  els.mic.classList.remove("rec");
  micLabel("…");
  els.mic.disabled = true;
  recChipHide();
  dictation.stream?.getTracks().forEach((t) => t.stop());
  dictation.stream = null;
  const blob = new Blob(dictation.chunks, { type: dictation.recorder?.mimeType || "audio/webm" });
  dictation.recorder = null;
  try {
    const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
    const file = await invoke("grove_save_recording", {
      bytes,
      mime: blob.type || undefined,
    });
    els.status.textContent = "transcribing on-device (local whisper)...";
    const text = await invoke("grove_transcribe", { file });
    els.prompt.value = els.prompt.value ? els.prompt.value + "\n" + text : text;
    els.status.textContent = "transcript in the prompt — edit it, then send";
    els.prompt.focus();
  } catch (err) {
    els.status.textContent = "dictation failed: " + String(err).split("\n")[0];
    noteError("dictation", err);
  } finally {
    els.mic.disabled = false;
    micLabel("record");
  }
}

els.mic.addEventListener("click", toggleDictation);

/* ======================= workspace shell profile =================== */
/* A business workspace may carry .spruce_grove/shell.json — the thin layer
   that makes this grove THEIRS: brand name, accent, quick prompts, roster.
   Machine stays generic; the workspace speaks. Failures = stock grove. */

const QUICK_PROMPTS_ID = "quick-prompts";

function applyShellProfile(cwd, profile) {
  if (!profile) return;
  try {
    if (profile.brand) {
      const wm = document.querySelector(".wordmark");
      if (wm) wm.textContent = profile.brand + (profile.brand_mark || "");
      document.title = profile.brand + " — grove";
    }
    if (profile.logo) {
      // the business's own mark, read from their workspace, worn in the sidebar
      const path = cwd.replace(/\/$/, "") + "/" + profile.logo.replace(/^\//, "");
      invoke("grove_read_image_base64", { path })
        .then((dataUrl) => {
          const mark = document.getElementById("brand-mark");
          if (!mark || !dataUrl) return;
          const img = document.createElement("img");
          img.src = dataUrl;
          img.alt = "";
          img.className = "side-mark";
          img.style.width = "44px";
          img.style.height = "auto";
          img.style.display = "block";
          mark.replaceWith(img);
        })
        .catch(() => { /* stock mark stays */ });
    }
    if (profile.accent && /^#[0-9a-fA-F]{6}$/.test(profile.accent)) {
      document.documentElement.style.setProperty("--ember", profile.accent);
    }
    if (profile.tagline) {
      const et = document.querySelector(".empty-title");
      if (et) et.textContent = profile.tagline;
    }
    // quick prompts: preloaded first tasks, one click each — day-one impact
    const composer = document.querySelector(".composer .row");
    if (composer && Array.isArray(profile.quick_prompts) && profile.quick_prompts.length) {
      let bar = document.getElementById(QUICK_PROMPTS_ID);
      if (!bar) {
        bar = document.createElement("div");
        bar.id = QUICK_PROMPTS_ID;
        bar.className = "quick-prompts";
        composer.parentElement.insertBefore(bar, composer);
      }
      bar.textContent = "";
      for (const qp of profile.quick_prompts.slice(0, 4)) {
        if (!qp || !qp.label || !qp.prompt) continue;
        const chip = document.createElement("button");
        chip.className = "quick-prompt";
        chip.title = qp.prompt;
        chip.textContent = qp.label;
        chip.addEventListener("click", () => {
          els.prompt.value = qp.prompt;
          els.prompt.focus();
        });
        bar.appendChild(chip);
      }
    }
    if (profile.note) els.status.textContent = profile.note;
  } catch { /* a broken profile must never take the grove down */ }
}

async function loadShellProfile(cwd) {
  try {
    const raw = await invoke("grove_shell_profile", { cwd });
    applyShellProfile(cwd, raw ? JSON.parse(raw) : null);
  } catch { /* stock grove */ }
}

/* ============================ boot ================================ */

async function initCwd() {
  const saved = localStorage.getItem("grove.cwd");
  if (saved) { els.cwd.value = saved; return; }
  if (settingsCache && settingsCache.default_cwd) {
    els.cwd.value = settingsCache.default_cwd;
    return;
  }
  // Rust owns the repo-location policy (GROVE_REPO, then the canonical
  // clone, then a historical one). Only if the command itself is missing
  // do we fall back -- and never to a hardcoded personal path.
  try { els.cwd.value = await invoke("grove_default_cwd"); }
  catch { els.cwd.value = ""; }
}

loadSessions();
// The running-session list comes from the CLI, so it is independent of the
// settings/ACP boot chain: show it as soon as we can ask, and keep asking.
startLivePolling();
loadSettings()
  .then(() => {
    // the drawer remembers being open (session memory), and settings can
    // make it the default on every launch
    if (settingsCache && settingsCache.inspector_auto_open) toggleInspector(true);
    else if (localStorage.getItem("grove.inspector") === "1") toggleInspector(true);
    return initCwd();
  })
  .then(() => {
    const cwd = els.cwd.value.trim();
    const last = sessionsForDir(cwd)[0];
    return startAcp(cwd, last ? last.id : null);
  })
  .then(() => refreshVersion());
