"""Standalone mock-backed UI server for interactive QA swarms.

Serves the repo root; `/` and `/index.html` return ui/index.html with the
shared MOCK `window.__TAURI__` bridge (imported from test_desktop_ui.py —
single source of truth for canned IPC) injected before boot-probe.js and
relative asset paths rewritten to /ui/... so the page works from the root.
Everything else is served as-is, so /handoff/presentation.html and
/ui/main.js resolve naturally.

Usage: python3 tests/mock_server.py [port]   (default 8199)
"""
import http.server
import socketserver
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_desktop_ui import MOCK, UI  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

# relative refs inside ui/index.html -> repo-root URLs
REWRITES = {
    '<script src="boot-probe.js"></script>': '<script src="ui/boot-probe.js"></script>',
    '<script src="picker.js"></script>': '<script src="ui/picker.js"></script>',
    '<script src="main.js"></script>': '<script src="ui/main.js"></script>',
    '<link rel="stylesheet" href="fonts.css" />': '<link rel="stylesheet" href="ui/fonts.css" />',
    '<link rel="stylesheet" href="styles.css" />': '<link rel="stylesheet" href="ui/styles.css" />',
    '<link rel="stylesheet" href="gran.css" />': '<link rel="stylesheet" href="ui/gran.css" />',
}


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(ROOT), **kw)

    def log_message(self, *a):  # quiet
        pass

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path in ("/", "/index.html"):
            html = (UI / "index.html").read_text()
            first_script = '<script src="ui/boot-probe.js"></script>'
            html = html.replace(
                '<script src="boot-probe.js"></script>', first_script, 1
            )
            for old, new in REWRITES.items():
                html = html.replace(old, new)
            html = html.replace(first_script, f"<script>{MOCK}</script>\n{first_script}", 1)
            body = html.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            super().do_GET()


def main() -> int:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8199
    socketserver.TCPServer.allow_reuse_address = True
    server = socketserver.ThreadingTCPServer(("127.0.0.1", port), Handler)
    print(f"mock UI server on http://127.0.0.1:{port}/index.html", flush=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        threading.Event().wait()  # serve until killed
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
