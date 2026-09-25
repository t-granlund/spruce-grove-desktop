"""Generate presentation backups so the demo survives a hostile room.

The live deck is the show. This tool makes the *fallbacks*:

  1. handoff/triton-ventures.pdf   - every slide as a printable/PDF page, so a
     dead laptop or a projector with no browser still shows the whole story.
  2. backups/slides/*.png          - one PNG per slide, in order, for a phone
     or a slides viewer that only takes images.
  3. backups/receipts-walk.webm    - a short screen recording of the slide-11
     live receipts demo (inspector auto-opening on the project pane), so the
     "live" moment has a non-live twin if the embed misbehaves.

Everything is derived from handoff/triton-ventures.html - the same single
source of truth - so backups cannot drift from the real deck.

Usage: python3 tools/make_backups.py
Deps: playwright (already used by the UI suite), ffmpeg (for webm -> mp4).
"""
from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
DECK = ROOT / "handoff" / "triton-ventures.html"
PDF = ROOT / "handoff" / "triton-ventures.pdf"
SLIDES_DIR = ROOT / "backups" / "slides"
VIDEO_DIR = ROOT / "backups"
RECEIPTS_WEBM = VIDEO_DIR / "receipts-walk.webm"
RECEIPTS_MP4 = VIDEO_DIR / "receipts-walk.mp4"

DECK_URL = "file://" + str(DECK)

#: The second deck — the technical read. Same tool, same guarantees: its PDF
#: and slide PNGs are derived, never hand-made, so they cannot drift from the
#: HTML the way a separately-exported file would.
TECH_DECK = ROOT / "handoff" / "triton-tech-deepdive.html"
TECH_PDF = ROOT / "handoff" / "triton-tech-deepdive.pdf"
TECH_SLIDES_DIR = VIDEO_DIR / "tech-slides"
TECH_URL = "file://" + str(TECH_DECK)


def slide_count(page) -> int:
    return page.evaluate("() => document.querySelectorAll('.slide').length")


def export_pdf(page) -> None:
    PDF.parent.mkdir(parents=True, exist_ok=True)
    page.pdf(path=str(PDF), width="1280px", height="720px",
             print_background=True, page_ranges="1-40")
    print(f"wrote {PDF.relative_to(ROOT)}")


def export_slide_pngs(page) -> int:
    SLIDES_DIR.mkdir(parents=True, exist_ok=True)
    for old in SLIDES_DIR.glob("slide-*.png"):
        old.unlink()
    n = slide_count(page)
    # force every slide visible via the print stylesheet, then shoot each
    page.emulate_media(media="print")
    page.add_style_tag(content=".slide{display:flex!important;position:relative;height:720px}")
    for i in range(n):
        handle = page.query_selector(f".slide:nth-of-type({i + 1})")
        if handle is None:
            # nth-of-type counts sections; fall back to the indexed node
            handle = page.evaluate_handle(
                "(i) => document.querySelectorAll('.slide')[i]", i).as_element()
        if handle:
            handle.screenshot(path=str(SLIDES_DIR / f"slide-{i + 1:02d}.png"))
    page.emulate_media(media="screen")
    print(f"wrote {n} slide PNGs to {SLIDES_DIR.relative_to(ROOT)}")
    return n


def record_receipts_walk() -> None:
    """Record the walk onto the receipts slide: the inspector self-opens."""
    VIDEO_DIR.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context(
            viewport={"width": 1280, "height": 720},
            record_video_dir=str(VIDEO_DIR),
            record_video_size={"width": 1280, "height": 720},
        )
        page = ctx.new_page()
        page.goto(DECK_URL)
        page.wait_for_timeout(900)
        idx = page.evaluate(
            "() => [...document.querySelectorAll('.slide')]"
            ".findIndex(s => s.hasAttribute('data-receipts'))")
        for _ in range(idx):
            page.keyboard.press("ArrowRight")
            page.wait_for_timeout(260)
        # let the embed load and the inspector auto-open
        page.wait_for_timeout(3200)
        # nudge: show it opening on project, then flip to the self-audit pane
        try:
            frames = [f for f in page.frames if "showcase" in (f.url or "")]
            if frames:
                frames[-1].evaluate(
                    "() => window.postMessage({type:'grove-show-receipts',tab:'diag'},'*')")
                page.wait_for_timeout(2600)
        except Exception as exc:  # never fail the backup on a nudge
            print(f"  (nudge skipped: {exc})")
        video = page.video
        ctx.close()
        browser.close()
        if video:
            produced = Path(video.path())
            shutil.move(str(produced), str(RECEIPTS_WEBM))
            print(f"wrote {RECEIPTS_WEBM.relative_to(ROOT)}")


def convert_video() -> None:
    if not shutil.which("ffmpeg"):
        print("  (ffmpeg absent - leaving webm only)")
        return
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-i", str(RECEIPTS_WEBM),
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
         str(RECEIPTS_MP4)],
        check=True,
    )
    print(f"wrote {RECEIPTS_MP4.relative_to(ROOT)}")


def backup_tech_deck() -> int:
    """PDF + slide PNGs for the technical deck, derived from its HTML."""
    if not TECH_DECK.exists():
        print(f"ERROR: tech deck missing: {TECH_DECK}", file=sys.stderr)
        return 1
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1280, "height": 720})
        errs: list[str] = []
        page.on("pageerror", lambda e: errs.append(str(e)))
        page.goto(TECH_URL)
        page.wait_for_timeout(700)
        n = slide_count(page)
        print(f"tech deck: {n} slides, {len(errs)} page errors")
        page.pdf(path=str(TECH_PDF), width="1280px", height="720px",
                 print_background=True, page_ranges="1-40")
        print(f"wrote {TECH_PDF.relative_to(ROOT)}")
        TECH_SLIDES_DIR.mkdir(parents=True, exist_ok=True)
        for old in TECH_SLIDES_DIR.glob("slide-*.png"):
            old.unlink()
        page.emulate_media(media="print")
        page.add_style_tag(content=".slide{display:flex!important;position:relative;height:720px}")
        for i in range(n):
            handle = page.evaluate_handle(
                "(i) => document.querySelectorAll('.slide')[i]", i).as_element()
            if handle:
                handle.screenshot(path=str(TECH_SLIDES_DIR / f"slide-{i + 1:02d}.png"))
        page.emulate_media(media="screen")
        print(f"wrote {n} slide PNGs to {TECH_SLIDES_DIR.relative_to(ROOT)}")
        browser.close()
    return 0


def main() -> int:
    if not DECK.exists():
        print(f"ERROR: deck missing: {DECK}", file=sys.stderr)
        return 1
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1280, "height": 720})
        errs = []
        page.on("pageerror", lambda e: errs.append(str(e)))
        page.goto(DECK_URL)
        page.wait_for_timeout(700)
        n = slide_count(page)
        print(f"deck: {n} slides, {len(errs)} page errors")
        export_pdf(page)
        export_slide_pngs(page)
        browser.close()
    record_receipts_walk()
    convert_video()
    backup_tech_deck()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
