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
                "() => document.getElementById('transcript').innerText.includes('ACP STREAMING.')",
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
    print("PASS: ACP streaming + dictation + init regression, all green")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
