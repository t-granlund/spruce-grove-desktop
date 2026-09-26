"""Desktop UI validation: dictation flow + ACP streaming, real JS, mocked IPC.

Exercises the actual ui/main.js with a mocked `window.__TAURI__` bridge:
1. cwd seeding + version pill (initCwd fix regression)
2. dictation: record (fake mic) -> stop -> save -> transcribe -> editable prompt
3. ACP: start (ready event) -> send -> chunk/thought/tool stream -> turn-end
   with usage; mode pill flips to ACP; cancel wiring present.

The CLI-side ACP dialect itself is verified by tests/acp_probe.py against the
real binary; this file proves the UI consumes that dialect correctly.
"""
import http.server
import socketserver
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

UI = Path(__file__).resolve().parent.parent / "ui"
PORT = 8123

# The packaged app runs under this CSP (tauri.conf.json). The harness serves
# the same policy so UI tests fail where the real build would fail — that is
# how the data:-image breakage class stays dead. One honest deviation:
# script-src 'unsafe-inline', needed only by the injected MOCK bridge (the
# real app has no inline scripts; Tauri injects its own with nonces).
TEST_CSP = ("default-src 'self'; script-src 'self' 'unsafe-inline'; "
            "style-src 'self' 'unsafe-inline'; img-src 'self' data:; "
            "font-src 'self' data:; connect-src ipc: http://ipc.localhost; "
            "object-src 'none'; base-uri 'none'; form-action 'none'; "
            "frame-ancestors 'none'")

# Hardening directives that must survive in both the packaged policy and the
# harness copy. If someone loosens the CSP, this fails before the app ships.
CSP_HARDENING = ("object-src 'none'", "base-uri 'none'",
                 "form-action 'none'", "frame-ancestors 'none'")


def csp_guard(failures):
    """Assert the packaged CSP keeps its hardening and the harness mirrors it."""
    import json
    conf_path = Path(__file__).resolve().parent.parent / "src-tauri" / "tauri.conf.json"
    packaged = json.loads(conf_path.read_text())["app"]["security"]["csp"]
    for d in CSP_HARDENING:
        if d not in packaged:
            failures.append(f"packaged CSP missing hardening directive: {d}")
        if d not in TEST_CSP:
            failures.append(f"harness TEST_CSP drifted from packaged CSP: {d}")


# The main window is the only surface the app needs. Its permission set is an
# explicit allowlist, not a wildcard: a future PR that widens the capability
# surface (fs:, shell:, http:, any "*") fails here before it can ship.
CAP_ALLOWED = {"core:default"}
CAP_WINDOWS = ["main"]


def cap_guard(failures):
    """Assert the Tauri capability surface stays least-privilege."""
    import json
    cap_path = Path(__file__).resolve().parent.parent / "src-tauri" / "capabilities" / "default.json"
    cap = json.loads(cap_path.read_text())
    if cap.get("identifier") != "default":
        failures.append(f"capability identifier drifted: {cap.get('identifier')!r}")
    if cap.get("windows") != CAP_WINDOWS:
        failures.append(f"capability windows widened: {cap.get('windows')!r} != {CAP_WINDOWS!r}")
    for perm in cap.get("permissions", []):
        if perm not in CAP_ALLOWED:
            failures.append(f"capability permission outside allowlist: {perm!r}")
        if "*" in perm:
            failures.append(f"capability permission uses a wildcard: {perm!r}")

MOCK = r"""
window.__calls = [];
window.__library = {};
window.__acpHandler = null;
window.__turnCount = 0;
window.__turnTimers = [];
window.__canceledTurn = null;
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push({ cmd, args });
      if (cmd === "grove_version") return "spruce-grove 1.0.0";
      if (cmd === "grove_default_cwd") return "/tmp/sg-dogfood";
      if (cmd === "grove_console_sessions") {
        // Two live terminals plus one orphan, in the CLI's --console --json
        // shape. Set window.__liveSessions = null to simulate a CLI that
        // cannot answer at all.
        if (window.__liveSessions === null) throw new Error("console unavailable");
        if (window.__liveSessions) return window.__liveSessions;
        return [
          { pid: 100, ppid: 99, tty: "ttys001", terminal_open: true,
            kind: "terminal", session_name: "auto_session_20260922_180404_125317_100",
            title: "Desktop App QA Round Two", subtitle: null, agent_name: "code-puppy",
            cwd: "/tmp/sg-dogfood", message_count: 362, total_tokens: 259523,
            last_autosave: "2026-09-23T13:30:23", uptime_seconds: 3600, is_live: true },
          { pid: 200, ppid: 199, tty: "ttys003", terminal_open: true,
            kind: "terminal", session_name: "auto_session_20260922_201559_223288_200",
            title: "Arvest Statement Import", subtitle: null, agent_name: null,
            cwd: "/tmp/sg-dogfood/proj-a", message_count: 156, total_tokens: 100,
            last_autosave: "2026-09-23T13:00:00", uptime_seconds: 7200, is_live: true },
          { pid: 300, ppid: 299, tty: "ttys009", terminal_open: false,
            kind: "terminal", session_name: null, title: null, subtitle: null,
            agent_name: null, cwd: null, message_count: null, total_tokens: null,
            last_autosave: null, uptime_seconds: 90000, is_live: false },
        ];
      }
      if (cmd === "grove_list_dirs") {
        const tree = {
          "/tmp/sg-dogfood": ["proj-a", "proj-b", "zeta-lab"],
          "/tmp/sg-dogfood/proj-a": ["nested"],
          "/tmp/sg-dogfood/proj-a/nested": [],
        };
        const dirs = tree[args.path];
        if (!dirs) throw new Error("read " + args.path + ": no such directory");
        const parent = args.path.replace(/\/+$/, "").split("/").slice(0, -1).join("/") || null;
        return { path: args.path, parent, dirs, total: dirs.length };
      }
      if (cmd === "grove_save_recording") return "/tmp/mock-dictation.webm";

      // ---- the recording library (studio) ------------------------------
      // A faithful in-memory stand-in: takes append, order is explicit,
      // lock is enforced, and splice renumbers. Mirrors recordings.rs.
      // Owner-PIN state for this session: hasPin + the PIN itself. Initialize
      // ONCE (this handler runs on every invoke, so a bare assignment here
      // would reset the PIN on each call).
      if (window.__studioHasPin === undefined) {
        window.__studioHasPin = false;
        window.__studioPin = null;
      }
      if (cmd === "grove_studio_has_pin") {
        return window.__studioHasPin;
      }
      if (cmd === "grove_studio_set_pin") {
        if (window.__studioHasPin) throw new Error("an owner PIN is already set");
        if (!args.pin || args.pin.length < 4) throw new Error("PIN must be at least 4 characters");
        window.__studioHasPin = true;
        window.__studioPin = args.pin;
        return null;
      }
      if (cmd === "grove_library_takes") {
        const id = args.recordingId || "rec-mock";
        let rec = window.__library[id];
        if (!rec) {
          rec = { id, name: "mock recording", created_ms: 1, updated_ms: 1,
                  summary: "", locked: false, takes: [] };
          window.__library[id] = rec;
        }
        rec.takes.push({ file: "take-" + String(rec.takes.length).padStart(3, "0") + ".webm",
                         mime: args.mime || "audio/webm", created_ms: 1,
                         duration_s: args.durationS || 0,
                         transcript: args.transcript || "" });
        rec.updated_ms++;
        return JSON.parse(JSON.stringify(rec));
      }
      if (cmd === "grove_recordings_list") {
        return Object.values(window.__library).map(r => JSON.parse(JSON.stringify(r)));
      }
      if (cmd === "grove_recording_get") {
        const rec = window.__library[args.id];
        if (!rec) throw new Error("no recording " + args.id);
        return JSON.parse(JSON.stringify(rec));
      }
      if (cmd === "grove_recording_audio") {
        // A REAL mono PCM WAV (16kHz, 0.4s, 440Hz), so the waveform path is
        // genuinely exercised: decodeAudioData must succeed and the canvas must
        // be drawn. The old 4-byte "RIFF" stub never decoded, so nothing
        // asserted the waveform and the CSP-blocked fetch went unnoticed.
        if (args.file !== "final.wav") throw new Error("no master yet");
        const rec = window.__library[args.id];
        if (!rec || !rec.master) throw new Error("no master yet");
        if (!window.__testWav) window.__testWav = (function () {
          const sr = 16000, n = Math.floor(sr * 0.4), amp = 0.3;
          const bytes = [];
          const push = (arr) => { for (const b of arr) bytes.push(b & 255); };
          const u32 = (v) => [v & 255, (v >> 8) & 255, (v >> 16) & 255, (v >> 24) & 255];
          const u16 = (v) => [v & 255, (v >> 8) & 255];
          const dataLen = n * 2;
          push([82, 73, 70, 70]); push(u32(36 + dataLen)); push([87, 65, 86, 69]);
          push([102, 109, 116, 32]); push(u32(16)); push(u16(1)); push(u16(1));
          push(u32(sr)); push(u32(sr * 2)); push(u16(2)); push(u16(16));
          push([100, 97, 116, 97]); push(u32(dataLen));
          for (let i = 0; i < n; i++) {
            const v = Math.round(amp * 32767 * Math.sin(2 * Math.PI * 440 * i / sr));
            push(u16(v < 0 ? v + 65536 : v));
          }
          return bytes;
        })();
        return window.__testWav;
      }
      if (cmd === "grove_recording_patch") {
        const rec = window.__library[args.id];
        if (!rec) throw new Error("no recording " + args.id);
        if (rec.locked && args.locked !== false) throw new Error("locked");
        if (rec.locked && args.locked === false) {
          // mirrors recordings.rs: unlock through the plain patch path is only
          // allowed when NO owner PIN is set
          if (window.__studioHasPin) throw new Error("unlock requires the owner PIN");
          rec.locked = false; delete rec.commit_tag;
        }
        if (args.name) rec.name = args.name;
        if (args.summary !== undefined) rec.summary = args.summary;
        if (args.locked === true) { rec.locked = true; if (window.__studioHasPin) rec.commit_tag = "mock"; }
        if (args.takeIndex !== undefined && args.transcript !== undefined) {
          rec.takes[args.takeIndex].transcript = args.transcript;
        }
        rec.updated_ms++;
        return JSON.parse(JSON.stringify(rec));
      }
      if (cmd === "grove_recording_unlock") {
        const rec = window.__library[args.id];
        if (!rec) throw new Error("no recording " + args.id);
        if (!window.__studioHasPin) throw new Error("no owner PIN is set");
        if (args.pin !== window.__studioPin) throw new Error("wrong PIN");
        rec.locked = false; delete rec.commit_tag; rec.tampered = false;
        rec.updated_ms++;
        return JSON.parse(JSON.stringify(rec));
      }
      if (cmd === "grove_recording_drop_take") {
        const rec = window.__library[args.id];
        if (!rec) throw new Error("no recording " + args.id);
        if (rec.locked) throw new Error("locked");
        rec.takes.splice(args.takeIndex, 1);
        return JSON.parse(JSON.stringify(rec));
      }
      if (cmd === "grove_recording_splice") {
        const rec = window.__library[args.id];
        if (!rec) throw new Error("no recording " + args.id);
        if (rec.locked) throw new Error("locked");
        if (args.order.length !== rec.takes.length) throw new Error("order length mismatch");
        const byOld = rec.takes.slice();
        rec.takes = args.order.map((i, slot) => {
          const t = Object.assign({}, byOld[i]);
          t.file = "take-" + String(slot).padStart(3, "0") + ".webm";
          return t;
        });
        rec.master = true;
        rec.masterBytes = [82, 73, 70, 70];
        rec.updated_ms++;
        return JSON.parse(JSON.stringify(rec));
      }
      if (cmd === "grove_recording_export") {
        return "/tmp/mock-export.md";
      }
      if (cmd === "grove_transcribe") {
        if (!args.file) throw new Error("no file given");
        return "Launch the desktop dictation loop: record, review, send.";
      }
      if (cmd === "grove_acp_start") {
        if (!args.cwd) throw new Error("no cwd");
        if (args.resume) {
          // mirror the real backend: session/load replays stored history as
          // ordinary session/update chunks before any new prompt arrives
          const emit = window.__acpHandler || (() => {});
          setTimeout(() => emit({ payload: { kind: "chunk", data: {
            update: { sessionUpdate: "agent_message_chunk",
                      content: { type: "text", text: "replay: earlier you asked about the grove." } } } } }), 40);
          return { sessionId: "sess_mock", model: "syn:large:text", resumed: true };
        }
        return { sessionId: "sess_mock", model: "syn:large:text" };
      }
      if (cmd === "grove_acp_prompt") {
        const emit = window.__acpHandler || (() => {});
        const mk = (kind, data) => ({ payload: { kind, data } });
        const upd = (sessionUpdate, extra) => ({
          update: { sessionUpdate, ...extra },
        });
        window.__turnCount += 1;
        const turn = window.__turnCount;
        const tok = turn === 1 ? 4321 : 777;
        // every stream event is a tracked timer: session/cancel must stop
        // the fake stream dead (the real backend resolves the pending
        // session/prompt with stopReason "cancelled" — an orderly turn-end)
        const at = (ms, kind, data) => {
          const t = setTimeout(() => {
            if (window.__canceledTurn === turn) return;
            emit(mk(kind, data));
          }, ms);
          window.__turnTimers.push(t);
        };
        at(30, "chunk", upd("agent_message_chunk", { content: { type: "text", text: "ACP " } }));
        at(60, "thought", upd("agent_thought_chunk", { content: { type: "text", text: "pondering the grove" } }));
        at(90, "chunk", upd("agent_message_chunk", { content: { type: "text", text: "STREAMING." } }));
        at(120, "tool", upd("tool_call", { toolCallId: "t1", title: "read_file", kind: "read", status: "completed", rawInput: { path: "x" } }));
        if (turn >= 2) {
          // every turn from the second on carries a screenshot tool call —
          // the steer/cancel turns must not depend on timing for the
          // look-in panel to open
          at(60, "tool", upd("tool_call", { toolCallId: "shot1", title: "take_screenshot", kind: "other", status: "completed",
            rawOutput: JSON.stringify({ success: true, screenshot_path: "/tmp/shots/scr_1.png" }) }));
        }
        if (turn >= 3) {
          // long-tail turns give the cancel test a real window: cancel must
          // stop this stream dead, not merely outpace its 200ms end_turn
          at(400, "chunk", upd("agent_message_chunk", { content: { type: "text", text: "TAIL-1 " } }));
          at(800, "chunk", upd("agent_message_chunk", { content: { type: "text", text: "TAIL-2 " } }));
          at(1500, "turn-end", { ok: true, result: { stopReason: "end_turn", usage: { totalTokens: tok } } });
        } else {
          at(200, "turn-end", { ok: true, result: { stopReason: "end_turn", usage: { totalTokens: tok } } });
        }
        return undefined;
      }
      if (cmd === "grove_acp_cancel") {
        window.__canceledTurn = window.__turnCount;
        for (const t of window.__turnTimers) clearTimeout(t);
        window.__turnTimers = [];
        const emit = window.__acpHandler || (() => {});
        setTimeout(() => emit({ payload: { kind: "turn-end",
          data: { ok: true, result: { stopReason: "cancelled" } } } }), 20);
        return undefined;
      }
      if (cmd === "grove_read_image_base64") {
        if (!args.path) throw new Error("no path");
        return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
      }
      if (cmd === "grove_send") return undefined;
      if (cmd === "grove_git_state") {
        if (!args.cwd) throw new Error("no cwd");
        return {
          branch: "main",
          dirty: [{ status: "M", path: "ui/main.js" }],
          dirty_total: 1,
          commits: [{ hash: "abc1234", subject: "fix the thing", when: "2h ago" }],
          repo: "t-granlund/spruce-grove-desktop",
          urls: {
            repo: "https://github.com/t-granlund/spruce-grove-desktop",
            commits: "https://github.com/t-granlund/spruce-grove-desktop/commits",
            actions: "https://github.com/t-granlund/spruce-grove-desktop/actions",
            pulls: "https://github.com/t-granlund/spruce-grove-desktop/pulls",
            issues: "https://github.com/t-granlund/spruce-grove-desktop/issues",
          },
          github: {
            gh_ok: true,
            repo: "t-granlund/spruce-grove-desktop",
            prs: [{ number: 7, title: "Add inspector drawer", url: "https://github.com/t-granlund/spruce-grove-desktop/pull/7", state: "OPEN", isDraft: false }],
            runs: [{ displayTitle: "ci", url: "https://github.com/t-granlund/spruce-grove-desktop/actions/runs/99", status: "completed", conclusion: "success", headBranch: "main", createdAt: "2026-09-16T09:00:00Z", updatedAt: "2026-09-16T09:01:30Z" }],
          },
        };
      }
      if (cmd === "grove_ledger_record") { window.__ledger = (window.__ledger || []); window.__ledger.push(args); return undefined; }
      if (cmd === "grove_ledger_tail") {
        return { entries: [
          { at: "2026-09-23T13:00:00Z", kind: "tool", cwd: "/tmp/sg-dogfood", summary: "read_file", outcome: "completed" },
          { at: "2026-09-23T13:00:05Z", kind: "permission", cwd: "/tmp/sg-dogfood", summary: "write_file", outcome: "auto-allowed" },
        ], path: "/data/action-ledger.jsonl" };
      }
      if (cmd === "grove_self_audit") {
        return { csp: "default-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
          csp_directives: ["default-src 'self'", "object-src 'none'", "base-uri 'none'", "form-action 'none'", "frame-ancestors 'none'"],
          required_hardening: ["object-src 'none'", "base-uri 'none'", "form-action 'none'", "frame-ancestors 'none'"],
          missing_hardening: [],
          capability: { windows: ["main"], permissions: ["core:default"], wildcards: [] },
          floor_intact: true,
          verdict: "floor intact — CSP hardening present, capability surface least-privilege (one window, no wildcards)" };
      }
      if (cmd === "grove_project_state") {
        if (!args.cwd) throw new Error("no cwd");
        return { is_beads: true, bd_found: true, bd_program: "bd", ok: true, cwd: args.cwd,
          counts: { open: 2, in_progress: 1, blocked: 1, closed: 17, deferred: 0, total: 21 },
          issues: [
            { id: "spruce-grove-desktop-qp4", title: "Containment + receipts", status: "in_progress", priority: 2, issue_type: "feature", updated_at: "2026-09-23T00:00:00Z" },
            { id: "spruce-grove-desktop-o5l", title: "Jev vs Laya", status: "open", priority: 2, issue_type: "task", updated_at: "2026-09-23T00:00:00Z" },
          ],
          docs: [
            { path: "GOVERNANCE.md", label: "governance", present: true },
            { path: "PLAN.md", label: "plan", present: true },
            { path: "BRAND.md", label: "brand", present: false },
          ] };
      }
      if (cmd === "grove_open_url" || cmd === "grove_open_path") return undefined;
      if (cmd === "grove_diagnostics") {
        return { app_version: "0.1.0", os: "macOS 27.0 (26A428)", arch: "arm64", pid: 4242,
          tmpdir: "/tmp", data_dir: "/data", bridge_probe_acked: true,
          boot_mainjs: true, boot_listen_ok: true,
          persisted_errors: [
            { at: "2026-09-16T09:00:00Z", source: "self-heal", message: "cli exited — revived (past session)" }
          ] };
      }
      if (cmd === "grove_settings_get") {
        return window.__settings || { version: 1, default_cwd: "", inspector_auto_open: false,
          error_report_level: "errors",
          watched_repos: ["t-granlund/spruce-grove-os", "t-granlund/spruce-grove-desktop"],
          personas: [{ name: "builder", dirs: [], grants: { prompts: true, dictation: true, lookin: true, inspector: true } }],
          active_persona: "builder" };
      }
      if (cmd === "grove_settings_set") { window.__settings = args.settings; return args.settings; }
      if (cmd === "grove_repo_access") {
        return { gh_ok: true, login: "t-granlund", repos: [
          { repo: "t-granlund/spruce-grove-os", ok: true, permission: "ADMIN" },
          { repo: "t-granlund/spruce-grove-desktop", ok: true, permission: "MAINTAIN" } ] };
      }
      return undefined;
    },
  },
  event: {
    listen: async (event, handler) => {
      if (event === "grove://acp") window.__acpHandler = handler;
      return () => {};
    },
  },
};
"""


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(UI), **kw)

    def log_message(self, *a):  # quiet
        pass

    def end_headers(self):
        # every response rides the packaged app's CSP (TEST_CSP above), so
        # the suite proves the UI under the real policy — data:-image media
        # included — instead of passing against a policy the build forbids
        self.send_header("Content-Security-Policy", TEST_CSP)
        super().end_headers()


def window_calls(page):
    return page.evaluate("window.__calls")


def main() -> int:
    socketserver.TCPServer.allow_reuse_address = True
    server = socketserver.TCPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    failures = []
    csp_guard(failures)
    cap_guard(failures)
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(
                args=[
                    "--use-fake-ui-for-media-stream",
                    "--use-fake-device-for-media-stream",
                ]
            )
            page = browser.new_page()
            page.add_init_script(MOCK)
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(f"http://127.0.0.1:{PORT}/index.html")

            # -- 1. initCwd + version pill ---------------------------------
            if page.input_value("#cwd") != "/tmp/sg-dogfood":
                failures.append("cwd not seeded from backend")
            if "cli: spruce-grove 1.0.0" not in (page.text_content("#cli-version") or ""):
                failures.append("version pill did not populate")

            # -- 1b. Escape closes the inspector before the picker is ever
            # mounted (picker.js lazily builds #dir-picker on first browse;
            # the Escape handler must null-guard it — TypeError regression)
            page.click("#inspector-toggle")
            page.wait_for_selector("#inspector:not(.hidden)", timeout=4000)
            page.keyboard.press("Escape")
            page.wait_for_function(
                "() => document.getElementById('inspector').classList.contains('hidden')",
                timeout=4000,
            )

            # -- 2. ACP start + ready pill ---------------------------------
            page.wait_for_function(
                "() => document.getElementById('mode').dataset.state === 'acp'",
                timeout=8000,
            )
            if "ACP · syn:large:text" not in (page.text_content("#mode") or ""):
                failures.append("mode pill missing model")

            # -- 3. ACP streaming send -------------------------------------
            page.fill("#prompt", "Say the thing.")
            page.click("#send")
            page.wait_for_function(
                "() => { const t = document.getElementById('transcript').innerText; return t.includes('ACP ') && t.includes('STREAMING.'); }",
                timeout=8000,
            )
            page.wait_for_function(
                "() => document.getElementById('status').textContent.includes('4,321 tok')",
                timeout=8000,
            )
            body = page.text_content("#transcript") or ""
            for needle in ("pondering the grove", "read_file", "you"):
                if needle not in body:
                    failures.append(f"transcript missing {needle!r}")
            chip = page.text_content(".msg.tool .chip") or ""
            if chip.strip() != "completed":
                failures.append(f"tool chip not completed: {chip!r}")

            # -- 4. dictation flow (regression) ----------------------------
            page.click("#mic")
            page.wait_for_timeout(300)
            if (page.text_content("#mic") or "").strip() != "stop":
                failures.append("mic did not enter stop state")
            page.wait_for_timeout(400)
            page.click("#mic")
            page.wait_for_function(
                "() => document.getElementById('prompt').value.length > 0", timeout=8000
            )
            if "Launch the desktop dictation loop" not in page.input_value("#prompt"):
                failures.append("dictation transcript missing from prompt")

            # -- 5. mid-run steer: send while busy -> cancel -> redirect ----
            page.fill("#prompt", "First task.")
            page.click("#send")
            page.wait_for_function(
                "() => document.getElementById('status').textContent.includes('streaming')",
                timeout=8000,
            )
            page.fill("#prompt", "STEER: change course.")
            page.click("#send")
            page.wait_for_function(
                "() => (window.__calls.filter(c => c.cmd === 'grove_acp_cancel')).length === 1",
                timeout=8000,
            )
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_acp_prompt' && (c.args.text||'').includes('STEER:'))",
                timeout=12000,
            )
            steer_prompt = [c for c in window_calls(page) if c["cmd"] == "grove_acp_prompt"][-1]
            if "STEER: change course." not in steer_prompt["args"]["text"]:
                texts = [c["args"].get("text", "")[:60] for c in window_calls(page) if c["cmd"] == "grove_acp_prompt"]
                failures.append(f"steer text missing; prompt texts seen: {texts}; last call: {steer_prompt}")
            # the redirect must leave a persistent transcript marker, not a
            # 200ms status blip: .msg.steer carrying the queued steer text
            steer_marker = page.evaluate(
                "() => [...document.querySelectorAll('.msg.steer')]"
                ".map(m => m.textContent).join('\\n')"
            )
            if "steer · queued" not in steer_marker or "STEER: change course." not in steer_marker:
                failures.append(f"persistent steer marker missing from transcript: {steer_marker!r}")

            # -- 5b. cancel mid-turn: stops the stream + leaves a marker ---
            page.fill("#prompt", "Cancel me mid-flight.")
            page.click("#send")
            page.wait_for_function(
                "() => document.getElementById('status').textContent.includes('streaming')",
                timeout=8000,
            )
            page.click("#cancel")
            page.wait_for_function(
                "() => document.getElementById('status').textContent.includes('cancelled')",
                timeout=8000,
            )
            cancel_marker = page.evaluate(
                "() => [...document.querySelectorAll('.msg.steer')]"
                ".map(m => m.textContent).join('\\n')"
            )
            if "turn canceled" not in cancel_marker:
                failures.append(f"persistent cancel marker missing: {cancel_marker!r}")
            # the canceled turn's stream must actually stop: record the
            # chunk count at cancel time, then prove it stops growing
            chunks_at_cancel = page.evaluate("() => (document.getElementById('transcript').innerText.match(/STREAMING\\./g) || []).length")
            page.wait_for_timeout(400)
            chunks_after_cancel = page.evaluate("() => (document.getElementById('transcript').innerText.match(/STREAMING\\./g) || []).length")
            if chunks_after_cancel > chunks_at_cancel:
                failures.append(
                    f"canceled stream kept streaming: {chunks_at_cancel} -> {chunks_after_cancel}"
                )
            # the shell must still be healthy: a fresh prompt works
            page.fill("#prompt", "Post-cancel sanity.")
            page.click("#send")
            page.wait_for_function(
                "() => document.getElementById('status').textContent.includes('done')",
                timeout=8000,
            )

            # -- 6. look-in: screenshot in tool payload -> panel + image ----
            page.wait_for_function(
                "() => !document.getElementById('lookin-panel').classList.contains('hidden')",
                timeout=8000,
            )
            src_img = page.get_attribute("#lookin-img", "src") or ""
            if not src_img.startswith("data:image/png"):
                failures.append("look-in image not rendered from data URL")

            # -- 7. sidebar: session + directory memory --------------------
            page.wait_for_function(
                "() => document.querySelectorAll('#session-list .sess-item').length >= 1",
                timeout=8000,
            )
            s_titles = page.text_content("#session-list") or ""
            if "Say the thing." not in s_titles:
                failures.append(f"session title missing from sidebar: {s_titles!r}")
            if "sg-dogfood" not in (page.text_content("#dir-list") or ""):
                failures.append("directory chip missing from sidebar")

            # -- 7b. running-now panel (sourced from the CLI) --------------
            # It must reflect what the CLI reports, including the orphan,
            # rather than any local guess about what is running.
            page.wait_for_function(
                "() => document.querySelectorAll('#live-list .live-item').length === 3",
                timeout=8000,
            )
            live_text = page.text_content("#live-list") or ""
            if "Desktop App QA Round Two" not in live_text:
                failures.append(f"live session title missing: {live_text!r}")
            if "pid 100" not in live_text:
                failures.append(f"live session pid missing: {live_text!r}")
            if "1h" not in live_text:
                failures.append(f"live session age missing: {live_text!r}")
            if (page.text_content("#live-count") or "").strip() != "3":
                failures.append(
                    "live count wrong: " + repr(page.text_content("#live-count"))
                )
            orphans = page.evaluate(
                "() => document.querySelectorAll('#live-list .live-item.orphan').length"
            )
            if orphans != 1:
                failures.append(f"orphan not marked: {orphans} flagged, expected 1")
            if "orphan" not in (page.text_content("#live-list") or ""):
                failures.append("orphan session not labelled")

            # clicking a named live session attaches to *that* session
            page.evaluate(
                "() => [...document.querySelectorAll('#live-list .live-item')]"
                ".find(n => n.textContent.includes('Arvest Statement Import')).click()"
            )
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_acp_start'"
                " && c.args.resume === 'auto_session_20260922_201559_223288_200')",
                timeout=8000,
            )
            attached = [
                c for c in window_calls(page)
                if c["cmd"] == "grove_acp_start"
                and c["args"].get("resume") == "auto_session_20260922_201559_223288_200"
            ]
            if attached[0]["args"]["cwd"] != "/tmp/sg-dogfood/proj-a":
                failures.append(
                    f"attach used wrong cwd: {attached[0]['args']['cwd']!r}"
                )
            # Attaching moved the working directory; put it back so later
            # steps (the dir picker seeds from the cwd) see what they expect.
            page.evaluate("() => { document.getElementById('cwd').value = '/tmp/sg-dogfood'; }")

            # -- 7c. an unreachable CLI must not claim "none running" ------
            page.evaluate("() => { window.__liveSessions = null; }")
            page.evaluate("() => refreshLiveSessions()")
            page.wait_for_function(
                "() => (document.getElementById('live-list').textContent || '')"
                ".includes('view unavailable')",
                timeout=8000,
            )
            unavailable = page.text_content("#live-list") or ""
            if "none" in unavailable:
                failures.append(
                    "unreachable CLI reported as 'none' instead of unavailable"
                )
            page.evaluate("() => { window.__liveSessions = undefined; }")

            # -- 8. sidebar collapse / restore -----------------------------
            page.click("#sidebar-toggle")
            if "collapsed" not in (page.get_attribute("#sidebar", "class") or ""):
                failures.append("sidebar did not collapse")
            page.click("#sidebar-open")
            if "collapsed" in (page.get_attribute("#sidebar", "class") or ""):
                failures.append("sidebar did not restore")

            # -- 9. new chat: fresh session, no resume ---------------------
            starts_before = len([c for c in window_calls(page) if c["cmd"] == "grove_acp_start"])
            page.click("#new-chat")
            page.wait_for_function(
                "() => document.getElementById('transcript').innerText.includes('Fresh ground.')",
                timeout=8000,
            )
            starts = [c for c in window_calls(page) if c["cmd"] == "grove_acp_start"]
            if len(starts) <= starts_before:
                failures.append("new chat did not start a fresh ACP session")
            if starts[-1]["args"].get("resume") is not None:
                failures.append("new chat should not resume a previous session")

            # -- 10. resume: clicking a session loads it via session/load --
            page.click("#session-list .sess-item")
            page.wait_for_function(
                "() => (window.__calls.filter(c => c.cmd === 'grove_acp_start' && (c.args.resume||null) === 'sess_mock')).length >= 1",
                timeout=8000,
            )
            if "Resuming" not in (page.text_content("#transcript") or ""):
                failures.append("resume notice missing from transcript")
            # session/load replays stored history: the mock mirrors the real
            # backend and emits the prior conversation as ordinary chunks
            page.wait_for_function(
                "() => document.getElementById('transcript').innerText.includes('replay: earlier you asked about the grove.')",
                timeout=8000,
            )
            # The "Resuming session…" placeholder must be GONE once real
            # content lands — it used to stay stamped over the pane forever,
            # because removal keyed on #empty-state while openSession created a
            # plain .empty card (found live in the packaged app dry run).
            leftovers = page.evaluate(
                "() => document.querySelectorAll('#transcript .empty').length"
            )
            if leftovers:
                failures.append(
                    f"resume placeholder not cleared after replay ({leftovers} .empty left)"
                )

            # -- 11. working-dir picker: browse -> descend -> choose -------
            page.click("#cwd-browse")
            page.wait_for_selector("#dir-picker:not(.hidden)", timeout=4000)
            page.wait_for_function(
                "() => document.querySelectorAll('#dir-picker .pk-item').length >= 3",
                timeout=4000,
            )
            if "/tmp/sg-dogfood" not in (page.text_content(".pk-path") or ""):
                failures.append("picker did not seed from the current cwd")
            page.click('.pk-item:has-text("proj-a")')
            page.wait_for_function(
                "() => (document.querySelector('.pk-path')||{}).textContent?.includes('proj-a')",
                timeout=4000,
            )
            page.click(".pk-use")
            page.wait_for_function(
                "() => document.getElementById('cwd').value.endsWith('proj-a')",
                timeout=4000,
            )
            starts = [c for c in window_calls(page) if c["cmd"] == "grove_acp_start"]
            if not starts or starts[-1]["args"].get("cwd") != "/tmp/sg-dogfood/proj-a":
                failures.append("picker did not restart ACP in the chosen directory")

            # -- 12. picker cancel: escape closes, cwd untouched -----------
            page.click("#cwd-browse")
            page.wait_for_selector("#dir-picker:not(.hidden)", timeout=4000)
            page.keyboard.press("Escape")
            page.wait_for_function(
                "() => document.getElementById('dir-picker').classList.contains('hidden')",
                timeout=4000,
            )
            if not page.input_value("#cwd").endswith("proj-a"):
                failures.append("picker cancel changed the cwd")

            # -- 13. inspector: drawer opens, renders repo/gh/turns --------
            page.click("#inspector-toggle")
            page.wait_for_selector("#inspector:not(.hidden)", timeout=4000)
            page.wait_for_function(
                "() => (document.getElementById('insp-branch').textContent || '').includes('main')",
                timeout=4000,
            )
            body = page.text_content("#inspector") or ""
            for needle in ("abc1234", "fix the thing", "ui/main.js", "#7",
                           "Add inspector drawer", "uncommitted"):
                if needle not in body:
                    failures.append(f"inspector missing {needle!r}")
            # a turn row from the section-3 send, marked done with tokens
            page.wait_for_function(
                "() => (document.getElementById('insp-turns').textContent || '').includes('Say the thing.')",
                timeout=4000,
            )
            # clicking a workflow run opens its URL through the shell
            page.click("#insp-runs .insp-row")
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_open_url' && (c.args.url||'').includes('/actions/runs/99'))",
                timeout=4000,
            )
            # run row shows branch + duration; PR row shows its state chip
            run_body = page.text_content("#insp-runs") or ""
            if "main" not in run_body or "90s" not in run_body:
                failures.append("run row missing branch/duration meta")
            if "open" not in (page.text_content("#insp-prs") or ""):
                failures.append("PR row missing state chip")
            # a commit row deep-links to the commit on GitHub
            page.click("#insp-commits .insp-row")
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_open_url' && (c.args.url||'').includes('/commit/abc1234'))",
                timeout=4000,
            )
            # Escape closes the drawer (top-most overlay owns Escape)
            page.keyboard.press("Escape")
            page.wait_for_function(
                "() => document.getElementById('inspector').classList.contains('hidden')",
                timeout=4000,
            )
            # close via the X (reopen first)
            page.click("#inspector-toggle")
            page.wait_for_selector("#inspector:not(.hidden)", timeout=4000)
            page.click("#insp-close")
            page.wait_for_function(
                "() => document.getElementById('inspector').classList.contains('hidden')",
                timeout=4000,
            )

            # -- 14. settings (Cmd+,) + diagnostics tab ---------------------
            page.keyboard.press("Control+,")
            page.wait_for_selector("#settings-overlay:not(.hidden)", timeout=4000)
            page.wait_for_function(
                "() => (document.getElementById('set-repo-access').textContent || '').includes('ADMIN')",
                timeout=4000,
            )
            try:
                if page.input_value(".persona-row .persona-name", timeout=4000) != "builder":
                    failures.append("persona editor missing 'builder'")
            except Exception:
                failures.append("persona editor missing 'builder'")
            page.click("#settings-save")
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_settings_set')",
                timeout=4000,
            )
            page.keyboard.press("Escape")
            page.wait_for_function(
                "() => document.getElementById('settings-overlay').classList.contains('hidden')",
                timeout=4000,
            )

            page.click("#inspector-toggle")
            page.wait_for_selector("#inspector:not(.hidden)", timeout=4000)
            page.click(".insp-tab[data-tab='diag']")
            page.wait_for_function(
                "() => (document.getElementById('diag-platform').textContent || '').includes('macOS 27.0')",
                timeout=4000,
            )
            if "live (probe acked)" not in (page.text_content("#diag-engine") or ""):
                failures.append("diagnostics missing bridge probe state")
            # the persisted error trail renders + data dir opens
            if "revived (past session)" not in (page.text_content("#diag-persisted") or ""):
                failures.append("persisted error trail missing")
            page.click("#diag-data-dir")
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_open_path' && (c.args.path||'') === '/data')",
                timeout=4000,
            )

            # -- 14b. self-audit + action ledger (containment + receipts) ----
            page.wait_for_function(
                "() => (document.getElementById('diag-floor').textContent || '').includes('INTACT')",
                timeout=4000,
            )
            if "4/4 directives" not in (page.text_content("#diag-floor") or ""):
                failures.append("self-audit floor missing hardening count")
            if "read_file" not in (page.text_content("#diag-ledger") or ""):
                failures.append("action ledger tail missing recorded entry")
            # a tool call during the turn was recorded to the ledger
            if not page.evaluate("() => (window.__ledger || []).some(e => e.kind === 'tool')"):
                failures.append("tool call was not written to the action ledger")

            # -- 14c. core project view: beads tracker + governance docs -----
            page.click(".insp-tab[data-tab='project']")
            page.wait_for_function(
                "() => (document.getElementById('proj-counts').textContent || '').includes('bd list')",
                timeout=4000,
            )
            proj_text = page.text_content("#proj-issues") or ""
            if "Containment + receipts" not in proj_text:
                failures.append(f"project tracker missing issue: {proj_text!r}")
            if "in_progress" not in proj_text:
                failures.append("project tracker missing status label")
            docs_text = page.text_content("#proj-docs") or ""
            if "governance" not in docs_text or "GOVERNANCE.md" not in docs_text:
                failures.append(f"project docs missing governance entry: {docs_text!r}")
            # a present doc is clickable and opens the path
            page.click("#proj-docs .proj-doc")
            page.wait_for_function(
                "() => window.__calls.some(c => c.cmd === 'grove_open_path' && (c.args.path||'').includes('GOVERNANCE.md'))",
                timeout=4000,
            )

            # close the drawer again
            page.click("#insp-close")
            page.wait_for_function(
                "() => document.getElementById('inspector').classList.contains('hidden')",
                timeout=4000,
            )

            # -- 14d. deck hook: postMessage opens the receipts pane ---------
            # the presentation embeds this page and posts a message to open the
            # inspector on a pane (slide 11's live receipts demo). Prove the hook
            # works and is idempotent/closeable, so the walkthrough cannot break.
            page.evaluate("""() => window.postMessage(
                {type: 'grove-show-receipts', tab: 'project'}, '*')""")
            page.wait_for_function(
                "() => !document.getElementById('inspector').classList.contains('hidden')"
                " && document.querySelector('.insp-tab.active').dataset.tab === 'project'",
                timeout=4000,
            )
            page.evaluate("""() => window.postMessage(
                {type: 'grove-show-receipts', tab: 'diag'}, '*')""")
            page.wait_for_function(
                "() => document.querySelector('.insp-tab.active').dataset.tab === 'diag'",
                timeout=4000,
            )
            page.evaluate("""() => window.postMessage({type: 'grove-hide-receipts'}, '*')""")
            page.wait_for_function(
                "() => document.getElementById('inspector').classList.contains('hidden')",
                timeout=4000,
            )

            # -- 14e. studio: record -> takes -> edit -> reorder -> lock ---
            # Record two takes through the real mic path FIRST (the composer is
            # behind the overlay, exactly as a user would meet it), then open
            # the studio to edit what was captured.
            # Start from an empty library so the counts below are deterministic;
            # the earlier dictation section deliberately leaves a take behind.
            page.evaluate("() => { window.__library = {}; }")
            page.evaluate("() => { document.getElementById('prompt').value = ''; }")
            for _ in range(2):
                page.click("#mic"); page.wait_for_timeout(250)
                page.click("#mic"); page.wait_for_timeout(900)

            lib = page.evaluate("() => Object.values(window.__library)[0]")
            if not lib:
                failures.append("studio: no recording was created by dictation")
            else:
                if len(lib["takes"]) != 2:
                    failures.append(f"studio: expected 2 takes, got {len(lib['takes'])}")
                # open the studio: the list shows the session we just recorded
                page.click("#studio-toggle")
                page.wait_for_selector("#studio-overlay:not(.hidden)", timeout=4000)
                page.wait_for_timeout(300)
                rows = page.eval_on_selector_all("#studio-recordings .studio-row", "els => els.length")
                if rows < 1:
                    failures.append("studio: recording list is empty after recording")
                page.click("#studio-recordings .studio-row")
                page.wait_for_selector("#studio-open:not(.hidden)", timeout=4000)
                page.wait_for_timeout(300)

            takes_shown = page.eval_on_selector_all("#studio-takes .take", "els => els.length")
            if takes_shown != 2:
                failures.append(f"studio: {takes_shown} take cards shown, expected 2")

            # edit a transcript in place (blur commits it)
            page.eval_on_selector("#studio-takes .take textarea",
                                  "el => { el.value = 'edited take one'; el.dispatchEvent(new Event('blur')); }")
            page.wait_for_timeout(400)
            edited = page.evaluate("() => Object.values(window.__library)[0].takes[0].transcript")
            if edited != "edited take one":
                failures.append(f"studio: transcript edit did not persist ({edited!r})")

            # reorder, then splice
            page.eval_on_selector_all("#studio-takes .take .take-btn",
                                      "els => els[1].click()")  # move take 1 later
            page.wait_for_timeout(200)
            page.click("#studio-splice")
            page.wait_for_timeout(600)
            spliced = page.evaluate("() => Object.values(window.__library)[0]")
            if not spliced.get("master"):
                failures.append("studio: splice did not write a master")
            files = [t["file"] for t in spliced["takes"]]
            if files != ["take-000.webm", "take-001.webm"]:
                failures.append(f"studio: splice did not renumber takes ({files})")

            # the waveform must actually render (regression: the CSP blocked the
            # fetch(blob:) re-read, so every waveform said "could not be decoded")
            page.wait_for_timeout(500)
            wave = page.evaluate("""() => ({
              canvas: !!document.querySelector('#studio-wave canvas.studio-canvas'),
              note: (document.querySelector('#studio-wave .studio-note') || {}).textContent || ''
            })""")
            if not wave["canvas"]:
                failures.append(f"studio: waveform did not render ({wave['note']!r})")

            # --- lock: the first lock offers to set an owner PIN ----------
            page.click("#studio-lock-btn")
            page.wait_for_selector("#studio-pin:not(.hidden)", timeout=4000)
            # wrong-length PIN is refused by the bridge, modal stays up
            page.eval_on_selector("#studio-pin-input",
                                  "el => { el.value = '12'; }")
            page.click("#studio-pin-ok")
            page.wait_for_timeout(200)
            if not page.evaluate("() => !document.getElementById('studio-pin').classList.contains('hidden')"):
                failures.append("studio: PIN modal closed on an invalid PIN")
            # set a real PIN and confirm
            page.eval_on_selector("#studio-pin-input", "el => { el.value = '246810'; }")
            page.click("#studio-pin-ok")
            page.wait_for_timeout(500)
            has_pin = page.evaluate("() => window.__studioHasPin")
            if not has_pin:
                failures.append("studio: owner PIN was not set on first lock")
            locked = page.evaluate("() => Object.values(window.__library)[0].locked")
            if not locked:
                failures.append("studio: lock did not stick")
            disabled = page.evaluate(
                "() => document.getElementById('studio-summary').disabled")
            if not disabled:
                failures.append("studio: summary should be read-only once locked")
            # an edit attempt through the bridge must raise
            refused = page.evaluate("""async () => {
              try { await window.__TAURI__.core.invoke('grove_recording_patch',
                    { id: Object.keys(window.__library)[0], name: 'nope' }); return false; }
              catch (e) { return true; }
            }""")
            if not refused:
                failures.append("studio: locked recording accepted an edit")

            # --- unlock: PIN is required (regression: unlock used to be
            # structurally impossible because the summary flush threw) --------
            page.click("#studio-lock-btn")
            page.wait_for_selector("#studio-pin:not(.hidden)", timeout=4000)
            # wrong PIN leaves it locked
            page.eval_on_selector("#studio-pin-input", "el => { el.value = '000000'; }")
            page.click("#studio-pin-ok")
            page.wait_for_timeout(400)
            still_locked = page.evaluate("() => Object.values(window.__library)[0].locked")
            pin_err = page.evaluate("() => document.getElementById('studio-pin-err').textContent")
            if not still_locked:
                failures.append("studio: wrong PIN unlocked the record")
            if not pin_err:
                failures.append("studio: wrong PIN showed no error")
            # right PIN unlocks
            page.eval_on_selector("#studio-pin-input", "el => { el.value = '246810'; }")
            page.click("#studio-pin-ok")
            page.wait_for_timeout(500)
            unlocked = page.evaluate("() => Object.values(window.__library)[0].locked")
            if unlocked:
                failures.append("studio: correct PIN did not unlock (record still locked)")
            editable = page.evaluate(
                "() => !document.getElementById('studio-summary').disabled")
            if not editable:
                failures.append("studio: summary still read-only after unlock")
            # and an edit must now be accepted again
            accepted = page.evaluate("""async () => {
              try { await window.__TAURI__.core.invoke('grove_recording_patch',
                    { id: Object.keys(window.__library)[0], name: 'renamed after unlock' });
                    return true; }
              catch (e) { return false; }
            }""")
            if not accepted:
                failures.append("studio: unlocked recording refused a valid edit")

            # --- tamper banner: a locked record whose signature went stale ----
            page.evaluate("""() => {
              const r = Object.values(window.__library)[0];
              r.locked = true; r.commit_tag = 'mock'; r.tampered = true;
            }""")
            page.evaluate("() => window.groveStudio.refresh()")
            page.wait_for_timeout(300)
            page.click("#studio-recordings .studio-row")
            page.wait_for_timeout(300)
            banner = page.evaluate(
                "() => !document.getElementById('studio-tamper').classList.contains('hidden')")
            if not banner:
                failures.append("studio: tampered record did not show the warning banner")

            page.click("#studio-close")
            page.wait_for_timeout(200)
            if not page.evaluate("() => document.getElementById('studio-overlay').classList.contains('hidden')"):
                failures.append("studio: close did not hide the panel")

            # -- 15. accessibility pack: labels, live regions, tabs, focus --
            a11y = page.evaluate("""() => {
              const q = (s) => document.querySelector(s);
              const label = (s) => (q(s) ? q(s).getAttribute('aria-label') : null);
              return {
                statusRole: q('#status').getAttribute('role'),
                statusLive: q('#status').getAttribute('aria-live'),
                transcriptRole: q('#transcript').getAttribute('role'),
                transcriptLive: q('#transcript').getAttribute('aria-live'),
                sidebarToggle: label('#sidebar-toggle'),
                sidebarOpen: label('#sidebar-open'),
                inspRefresh: label('#insp-refresh'),
                inspClose: label('#insp-close'),
                cancel: label('#cancel'),
                prompt: label('#prompt'),
                cwd: label('#cwd'),
                tablist: !!q('.insp-tabs[role="tablist"]'),
                tabs: [...document.querySelectorAll('.insp-tab')].map(t => t.getAttribute('role')),
                selectedCount: [...document.querySelectorAll('.insp-tab')].filter(t => t.getAttribute('aria-selected') === 'true').length,
                panes: [...document.querySelectorAll('.insp-pane')].map(p => p.getAttribute('role')),
              };
            }""")
            for key, want in {
                "statusRole": "status", "statusLive": "polite",
                "transcriptRole": "log", "transcriptLive": "off",
                "sidebarToggle": "collapse sidebar", "sidebarOpen": "show sidebar",
                "inspRefresh": "refresh inspector", "inspClose": "hide inspector",
                "cancel": "cancel the running turn", "prompt": "prompt for the grove",
                "cwd": "working directory",
            }.items():
                if a11y.get(key) != want:
                    failures.append(f"a11y {key}: expected {want!r}, got {a11y.get(key)!r}")
            # the drawer remembers the last tab, so assert the structure:
            # a tablist, three role=tab children (repo/project/diagnostics),
            # exactly one selected, three role=tabpanel panes
            if (not a11y["tablist"] or a11y["tabs"] != ["tab", "tab", "tab"]
                    or a11y["selectedCount"] != 1
                    or a11y["panes"] != ["tabpanel", "tabpanel", "tabpanel"]):
                failures.append(f"a11y inspector tablist broken: {a11y}")
            # dialog focus management: focus moves into the dialog on open
            # and returns to the invoker on close
            page.focus("#new-chat")
            page.keyboard.press("Control+,")
            page.wait_for_selector("#settings-overlay:not(.hidden)", timeout=4000)
            if page.evaluate("() => document.activeElement && document.activeElement.id") != "set-default-cwd":
                failures.append("settings did not move focus into the dialog")
            page.keyboard.press("Escape")
            page.wait_for_function(
                "() => document.getElementById('settings-overlay').classList.contains('hidden')",
                timeout=4000,
            )
            if page.evaluate("() => document.activeElement && document.activeElement.id") != "new-chat":
                failures.append("settings close did not restore focus to the invoker")
            # arrow keys walk the inspector tabs; the session row is
            # keyboard-operable without a pointer
            page.click("#inspector-toggle")
            page.wait_for_selector("#inspector:not(.hidden)", timeout=4000)
            page.focus(".insp-tab[data-tab='repo']")
            page.keyboard.press("ArrowRight")
            page.wait_for_function(
                "() => [...document.querySelectorAll('.insp-tab')]"
                ".some(t => t.getAttribute('aria-selected') === 'true' && t.dataset.tab === 'project')",
                timeout=4000,
            )
            page.click("#insp-close")
            starts_before_kb = len([c for c in window_calls(page) if c["cmd"] == "grove_acp_start"])
            page.focus("#session-list .sess-item")
            page.keyboard.press("Enter")
            page.wait_for_function(
                f"() => window.__calls.filter(c => c.cmd === 'grove_acp_start').length > {starts_before_kb}",
                timeout=4000,
            )

            # -- 16. narrow window: nothing bleeds over the sidebar --------
            page.set_viewport_size({"width": 500, "height": 700})
            page.wait_for_timeout(200)
            nav = page.evaluate("""() => {
              const doc = document.documentElement;
              const rect = (id) => document.getElementById(id).getBoundingClientRect();
              const toggle = rect('sidebar-toggle');
              const browse = rect('cwd-browse');
              const send = rect('send');
              const hits = (r, id) => {
                const el = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
                return !!el && (el.id === id || !!el.closest('#' + id));
              };
              return {
                xScroll: doc.scrollWidth > doc.clientWidth,
                fieldLeft: document.querySelector('.field').getBoundingClientRect().left,
                toggleHit: hits(toggle, 'sidebar-toggle'),
                browseHit: hits(browse, 'cwd-browse'),
                sendVisible: send.right <= doc.clientWidth + 1 && send.width > 0,
              };
            }""")
            if nav["xScroll"]:
                failures.append("narrow window: page overflows horizontally")
            if nav["fieldLeft"] < 0:
                failures.append(f"narrow window: cwd field bleeds left of the viewport ({nav['fieldLeft']})")
            if not nav["toggleHit"]:
                failures.append("narrow window: sidebar-toggle click-intercepted")
            if not nav["browseHit"]:
                failures.append("narrow window: browse button unreachable")
            if not nav["sendVisible"]:
                failures.append("narrow window: send button clipped")
            page.set_viewport_size({"width": 1100, "height": 760})

            if errors:
                failures.append(f"page errors: {errors}")
            browser.close()
    finally:
        server.shutdown()
    if failures:
        print("FAIL:")
        for f in failures:
            print(" -", f)
        return 1
    print("PASS: streaming + dictation + studio (takes/edit/reorder/splice/lock) + steering + "
          "cancel + look-in + sidebar + picker + inspector + settings/diagnostics + a11y + "
          "narrow-window, all green")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
