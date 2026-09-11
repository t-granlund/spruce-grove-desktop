"""Dictation UI validation: real JS, mocked Tauri IPC, fake mic stream.

Exercises the actual ui/main.js flow: click record -> (fake mic) -> stop ->
grove_save_recording -> grove_transcribe -> transcript lands in the editable
prompt box. Chromium fake-media flags supply the mic; __TAURI__ is mocked
with a transcript payload so no CLI/whisper is needed for THIS layer (the
CLI verb itself is already verified end-to-end against real audio).
"""
import http.server
import socketserver
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

UI = Path("/Users/tygranlund/spruce-grove-desktop/ui")
PORT = 8123

MOCK = """
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls = window.__calls || [];
      window.__calls.push({ cmd, args });
      if (cmd === "grove_version") return "spruce-grove 0.1.0";
      if (cmd === "grove_default_cwd") return "/tmp/sg-dogfood";
      if (cmd === "grove_save_recording") return "/tmp/mock-dictation.webm";
      if (cmd === "grove_transcribe") {
        if (!args.file) throw new Error("no file given");
        return "Launch the desktop dictation loop: record, review, send.";
      }
      if (cmd === "grove_send") return undefined;
      return undefined;
    },
  },
  event: { listen: async () => () => {} },
};
"""


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(UI), **kw)

    def log_message(self, *a):  # quiet
        pass


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
                    "--autoplay-policy=no-user-gesture-required",
                ]
            )
            page = browser.new_page()
            page.add_init_script(MOCK)
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(f"http://127.0.0.1:{PORT}/index.html")

            # 1. the dictation bug-fix: cwd seeded from the mocked backend
            cwd = page.input_value("#cwd")
            if cwd != "/tmp/sg-dogfood":
                failures.append(f"cwd not seeded from backend: {cwd!r}")
            if "cli: spruce-grove 0.1.0" not in (
                page.text_content("#cli-version") or ""
            ):
                failures.append("version pill did not populate (initCwd fix check)")

            # 2. record -> stop -> transcript lands in the prompt
            page.click("#mic")
            page.wait_for_timeout(400)
            label = page.text_content("#mic").strip()
            if label != "stop":
                failures.append(f"mic button did not enter stop state: {label!r}")
            page.wait_for_timeout(600)  # fake mic emits a tone; keep it short
            page.click("#mic")
            page.wait_for_function(
                "() => document.getElementById('prompt').value.length > 0",
                timeout=8000,
            )
            prompt = page.input_value("#prompt")
            if "Launch the desktop dictation loop" not in prompt:
                failures.append(f"unexpected transcript in prompt: {prompt!r}")
            if page.text_content("#mic").strip() != "record":
                failures.append("mic button did not return to record state")
            if "transcript in the prompt" not in page.text_content("#status"):
                failures.append("status did not confirm readiness")

            # 3. the recorded bytes actually reached the save command
            calls = page.evaluate("window.__calls")
            save = next(c for c in calls if c["cmd"] == "grove_save_recording")
            if len(save["args"]["bytes"]) < 100:
                failures.append("recording bytes suspiciously small")
            tr = next(c for c in calls if c["cmd"] == "grove_transcribe")
            if tr["args"]["file"] != "/tmp/mock-dictation.webm":
                failures.append("transcribe did not receive saved file path")

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
    print("PASS: dictation UI flow validated (record -> save -> transcribe -> prompt)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
