#!/usr/bin/env python3
"""The showcase packet must be genuinely sendable as one file.

Two failure modes this pins, both silent:

1. **Drift.** `showcase-standalone.html` is generated from `showcase.html` plus
   six `ui/` assets. Edit any of those and forget to regenerate, and the packet
   quietly ships an old deck -- the recipient sees a demo of a version that no
   longer exists.
2. **Regression to a broken packet.** If someone adds a seventh external
   reference to the deck, a naive inline build would leave it dangling, and the
   packet would arrive unstyled -- exactly the original problem.

Plain script, no test framework: this repo has no pytest, matching
`tests/test_desktop_ui.py`. The browser proof runs only if playwright is
present, and says so rather than passing silently when it is not.

Usage:  python3 tests/test_packet.py
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
HANDOFF = REPO / "handoff"
BUILDER = HANDOFF / "build_self_contained.py"
PACKET = HANDOFF / "showcase-standalone.html"

REMOTE_PREFIXES = ("http://", "https://", "data:", "#", "mailto:")


def _external_refs(html: str) -> list[str]:
    return [
        ref
        for ref in re.findall(r'(?:src|href)="([^"]+)"', html)
        if not ref.startswith(REMOTE_PREFIXES)
    ]


def check_structure(failures: list[str]) -> None:
    if not PACKET.is_file():
        failures.append(
            "showcase-standalone.html is missing -- build it with "
            "python3 handoff/build_self_contained.py"
        )
        return

    html = PACKET.read_text(encoding="utf-8", errors="replace")

    refs = _external_refs(html)
    if refs:
        failures.append(f"packet still references sibling files: {refs}")

    if html.count("<style>") < 3 or html.count("<script>") < 3:
        failures.append("assets were not inlined (expected 3 styles + 3 scripts)")
    if "inlined from ui/" not in html:
        failures.append("packet lacks inlining provenance comments")

    # Guard the premise: if the source deck stopped needing inlining, this
    # builder has become pointless and should be removed, not kept forever.
    source = (HANDOFF / "showcase.html").read_text(encoding="utf-8")
    if not _external_refs(source):
        failures.append(
            "showcase.html no longer references ui/ assets -- the standalone "
            "build is pointless now; delete the builder and this test"
        )


def check_no_drift(failures: list[str]) -> None:
    """Re-running the builder must reproduce the committed packet exactly."""
    result = subprocess.run(
        [sys.executable, str(BUILDER), "--check"],
        capture_output=True,
        text=True,
        cwd=REPO,
    )
    if result.returncode != 0:
        failures.append(
            "showcase-standalone.html is stale (ui/ changed, packet did not). "
            "Regenerate: python3 handoff/build_self_contained.py\n"
            f"    {result.stdout.strip()}{result.stderr.strip()}"
        )


def check_renders_offline(failures: list[str]) -> None:
    """The honest test: load it in a browser with the network switched off."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("  (skipped: playwright not installed -- cannot prove offline render)")
        return

    errors: list[str] = []
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        page.on("pageerror", lambda e: errors.append(str(e)))
        # A self-contained packet must survive with the network gone.
        page.route(
            "**/*",
            lambda route: route.abort()
            if route.request.url.startswith(("http://", "https://"))
            else route.continue_(),
        )
        page.goto(PACKET.as_uri(), wait_until="load")
        page.wait_for_timeout(800)
        applied = page.evaluate("document.styleSheets.length")
        composer = page.query_selector("#prompt") is not None
        browser.close()

    if applied < 3:
        failures.append(f"stylesheets did not apply offline ({applied})")
    if not composer:
        failures.append("the deck's live demo did not wire up (main.js inert)")
    if errors:
        failures.append(f"page errors: {errors}")


def main() -> int:
    failures: list[str] = []
    check_structure(failures)
    check_no_drift(failures)
    check_renders_offline(failures)

    if failures:
        print("FAIL:")
        for failure in failures:
            print(f" - {failure}")
        return 1
    print("PASS: showcase packet is self-contained, current, and renders offline")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
