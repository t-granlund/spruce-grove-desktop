#!/usr/bin/env python3
"""Inline the showcase deck's assets into one self-contained HTML file.

Why: `showcase.html` is a demo of the real `ui/` front-end running against a
mock bridge, so it pulls in six sibling files (`../ui/*.css`, `../ui/*.js`).
That makes it a *packet* only in name -- send it alone and the recipient opens
a page with no styling and no interactivity. Verified 2026-09-23: six external
references, all via `../ui/`.

This script produces `showcase-standalone.html`: the same deck with every
referenced asset inlined, so it works from a single file over `file://` with no
server, no build step and no network. Fonts stay as they are (the deck already
inlines its icon; `fonts.css` declares families that fall back gracefully).

Reproducible on purpose: re-run it after any `ui/` change rather than editing
the output by hand. `--check` reports drift without writing.

Usage:
    python3 handoff/build_self_contained.py           # write the standalone deck
    python3 handoff/build_self_contained.py --check    # exit 1 if out of date
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE / "showcase.html"
OUT = HERE / "showcase-standalone.html"

#: `<link rel="stylesheet" href="../ui/x.css" />`
CSS_RE = re.compile(
    r'[ \t]*<link[^>]+rel=["\']stylesheet["\'][^>]*href=["\']\.\./ui/([^"\']+)["\'][^>]*>'
)
#: `<script src="../ui/x.js"></script>`
JS_RE = re.compile(
    r'[ \t]*<script[^>]+src=["\']\.\./ui/([^"\']+)["\'][^>]*>\s*</script>'
)


def _read_asset(name: str) -> str:
    path = HERE.parent / "ui" / name
    if not path.is_file():
        raise FileNotFoundError(f"deck references a missing asset: {path}")
    return path.read_text(encoding="utf-8")


def _guard_inline(text: str, name: str) -> str:
    """Refuse to inline content that would break the surrounding <style>/<script>.

    An asset containing the closing tag would terminate the block early and
    produce a silently broken page -- exactly the class of failure this script
    exists to prevent. Fail loudly instead.
    """
    if "</style" in text.lower() or "</script" in text.lower():
        raise ValueError(f"{name} contains a closing tag and cannot be inlined safely")
    return text


def build() -> str:
    html = SRC.read_text(encoding="utf-8")

    def css_sub(match: re.Match) -> str:
        name = match.group(1)
        body = _guard_inline(_read_asset(name), name)
        return f"<style>\n/* inlined from ui/{name} */\n{body}\n</style>"

    def js_sub(match: re.Match) -> str:
        name = match.group(1)
        body = _guard_inline(_read_asset(name), name)
        return f"<script>\n/* inlined from ui/{name} */\n{body}\n</script>"

    html, css_count = CSS_RE.subn(css_sub, html)
    html, js_count = JS_RE.subn(js_sub, html)

    if css_count != 3 or js_count != 3:
        raise SystemExit(
            f"expected to inline 3 stylesheets and 3 scripts, "
            f"got {css_count} and {js_count} -- the deck's markup changed"
        )
    return html


def main() -> int:
    built = build()

    if "--check" in sys.argv:
        current = OUT.read_text(encoding="utf-8") if OUT.is_file() else ""
        if current != built:
            print(f"OUT OF DATE: {OUT.name} differs from a fresh build")
            return 1
        print(f"up to date: {OUT.name}")
        return 0

    OUT.write_text(built, encoding="utf-8")
    print(f"wrote {OUT.relative_to(HERE.parent)}  ({len(built):,} chars)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
