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

MOCK = r"""
window.__calls = [];
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
            # close the drawer again
            page.click("#insp-close")
            page.wait_for_function(
                "() => document.getElementById('inspector').classList.contains('hidden')",
                timeout=4000,
            )

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
            # a tablist, two role=tab children, exactly one selected, panes
            if (not a11y["tablist"] or a11y["tabs"] != ["tab", "tab"]
                    or a11y["selectedCount"] != 1 or a11y["panes"] != ["tabpanel", "tabpanel"]):
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
                ".some(t => t.getAttribute('aria-selected') === 'true' && t.dataset.tab === 'diag')",
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
    print("PASS: streaming + dictation + steering + cancel + look-in + sidebar + picker + inspector + settings/diagnostics + a11y + narrow-window, all green")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
