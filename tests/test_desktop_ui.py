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

MOCK = r"""
window.__calls = [];
window.__acpHandler = null;
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
        return { sessionId: "sess_mock", model: "syn:large:text" };
      }
      if (cmd === "grove_acp_prompt") {
        const emit = window.__acpHandler || (() => {});
        const mk = (kind, data) => ({ payload: { kind, data } });
        const upd = (sessionUpdate, extra) => ({
          update: { sessionUpdate, ...extra },
        });
        setTimeout(() => emit(mk("chunk", upd("agent_message_chunk", { content: { type: "text", text: "ACP " } }))), 30);
        setTimeout(() => emit(mk("thought", upd("agent_thought_chunk", { content: { type: "text", text: "pondering the grove" } }))), 60);
        setTimeout(() => emit(mk("chunk", upd("agent_message_chunk", { content: { type: "text", text: "STREAMING." } }))), 90);
        setTimeout(() => emit(mk("tool", upd("tool_call", { toolCallId: "t1", title: "read_file", kind: "read", status: "completed", rawInput: { path: "x" } }))), 120);
        window.__turnCount = (window.__turnCount || 0) + 1;
        const tok = window.__turnCount === 1 ? 4321 : 777;
        if (window.__turnCount === 2) {
          setTimeout(() => emit(mk("tool", upd("tool_call", { toolCallId: "shot1", title: "take_screenshot", kind: "other", status: "completed",
            rawOutput: JSON.stringify({ success: true, screenshot_path: "/tmp/shots/scr_1.png" }) }))), 60);
        }
        setTimeout(() => emit(mk("turn-end", { ok: true, result: { stopReason: "end_turn", usage: { totalTokens: tok } } })), 200);
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


def window_calls(page):
    return page.evaluate("window.__calls")


def main() -> int:
    socketserver.TCPServer.allow_reuse_address = True
    server = socketserver.TCPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    failures = []
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
    print("PASS: streaming + dictation + steering + look-in + sidebar + picker + inspector + settings/diagnostics, all green")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
