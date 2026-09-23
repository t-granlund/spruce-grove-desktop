"""Generate handoff/showcase.html - the real ui/ running on the shared mock bridge.

Why this exists: the presentation should show a LIVE, interactive app - not a
screenshot. The app's own test harness already proves ui/ runs against a mocked
`window.__TAURI__` bridge (tests/test_desktop_ui.py). This tool stitches that
exact bridge (single source of truth - imported, never copied) over the real
ui/index.html, rewriting relative asset refs so the page works from its new home
in handoff/. The result is fully interactive offline: no Tauri, no backend, no
server - just open the file (or embed it in an iframe).

Usage: python3 tools/make_showcase.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
from test_desktop_ui import MOCK  # noqa: E402  (single source of truth)

UI = ROOT / "ui"
OUT = ROOT / "handoff" / "showcase.html"

# Prefix every *relative* src/href with ../ui/ so the copy in handoff/ still
# resolves the real app assets. Absolute URLs, data: URIs and anchors are left
# alone.
REF = re.compile(r'(\b(?:src|href)=")(?!https?:|data:|/|#|\.\./)([^"]+)(")')

def rewrite_refs(html: str) -> str:
    return REF.sub(lambda m: f'{m.group(1)}../ui/{m.group(2)}{m.group(3)}', html)


def build() -> str:
    html = (UI / "index.html").read_text()
    first_script = '<script src="boot-probe.js"></script>'
    if first_script not in html:
        raise SystemExit(f"ERROR: anchor not found in ui/index.html: {first_script!r}")
    html = rewrite_refs(html)
    # the bridge must land BEFORE boot-probe.js and main.js read window.__TAURI__
    html = html.replace(
        '<script src="../ui/boot-probe.js"></script>',
        f"<script>{MOCK}</script>\n<script src=\"../ui/boot-probe.js\"></script>",
        1,
    )
    html = html.replace(
        '<title>Spruce Grove</title>',
        '<title>Spruce Grove - live showcase (mock bridge)</title>',
        1,
    )
    return html


def main() -> int:
    check = "--check" in sys.argv[1:]
    html = build()
    if check:
        current = OUT.read_text() if OUT.exists() else ""
        if current != html:
            print("STALE: handoff/showcase.html is out of date - run tools/make_showcase.py", file=sys.stderr)
            return 1
        print("ok: handoff/showcase.html is up to date with ui/")
        return 0
    OUT.write_text(html)
    print(f"wrote {OUT.relative_to(ROOT)} ({len(html)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
