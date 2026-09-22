#!/usr/bin/env python3
"""Vendor the app's webfonts locally (issue: self-host webfonts).

Why: the shell loaded Fraunces / Inter / JetBrains Mono / EB Garamond /
IM Fell English SC from fonts.googleapis.com at runtime. The packaged
app's CSP blocks that fetch, and offline launches silently fall back
mid-stream (the transcript's display face changes the moment streaming
starts — observed by QA). Zero-network is also the charter.

What this does:
  1. Fetches each family's css2 stylesheet with a modern Chrome UA
     (that UA is what yields woff2 + variable-weight responses).
  2. Keeps the latin and latin-ext subsets (the app's copy is English).
  3. Downloads each woff2 into ui/fonts/ (and copies to handoff/fonts/
     for the decks, which must survive offline sharing too).
  4. Emits ui/fonts.css + handoff/fonts.css with @font-face rules that
     reference the local files, preserving font-weight/style ranges and
     unicode-range so variable fonts keep behaving.

Re-run after intentionally changing the family/weight list below.
"""
import re
import shutil
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UI_FONTS = ROOT / "ui" / "fonts"
HANDOFF_FONTS = ROOT / "handoff" / "fonts"

# modern Chrome UA: Google serves woff2 + variable axes only to it
UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")

# family css2 URLs — variable ranges where the family has them, so one
# file covers every weight the UI uses (400..600 covers 400/500/600).
FAMILIES = {
    "fraunces": "https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,400..600&display=swap",
    "inter": "https://fonts.googleapis.com/css2?family=Inter:wght@400..600&display=swap",
    "jetbrains-mono": "https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400..600&display=swap",
    "eb-garamond": "https://fonts.googleapis.com/css2?family=EB+Garamond:ital,wght@0,400..500;1,400..500&display=swap",
    "im-fell-english-sc": "https://fonts.googleapis.com/css2?family=IM+Fell+English+SC&display=swap",
}
KEEP_SUBSETS = ("latin", "latin-ext")


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read()


def parse_blocks(css: str):
    """Yield (subset, block_text) pairs; Google labels each block with a
    /* subset */ comment directly above it."""
    return re.findall(r"/\*\s*([\w-]+)\s*\*/\s*(@font-face\s*\{[^}]*\})", css)


def slug_font_file(family_slug: str, style: str, weight: str, subset: str, url: str) -> str:
    ext = url.rsplit(".", 1)[-1]
    w = weight.strip().replace(" ", "-")
    return f"{family_slug}-{style}-{w}-{subset}.{ext}"


def rewrite_block(block: str, url_map: dict) -> str:
    out = block
    for remote, local in url_map.items():
        out = out.replace(remote, local)
    return out


def process_family(slug: str, css_url: str, font_dir: Path, css_out: Path, url_prefix: str) -> None:
    css = fetch(css_url).decode()
    blocks = parse_blocks(css)
    if not blocks:
        raise SystemExit(f"no @font-face blocks parsed for {slug} — parser rotted")
    rules = []
    for subset, block in blocks:
        if subset not in KEEP_SUBSETS:
            continue
        url_m = re.search(r"url\((https://[^)]+)\)", block)
        if not url_m:
            continue
        remote = url_m.group(1)
        style = re.search(r"font-style:\s*([^;]+);", block).group(1).strip()
        weight = re.search(r"font-weight:\s*([^;]+);", block).group(1).strip()
        fname = slug_font_file(slug, style, weight, subset, remote)
        target = font_dir / fname
        if not target.exists() or target.stat().st_size == 0:
            target.write_bytes(fetch(remote))
        rules.append(rewrite_block(block, {remote: f"{url_prefix}{fname}"}))
        print(f"  {slug}: {subset} {style} {weight} -> {fname} ({target.stat().st_size} bytes)")
    if not rules:
        raise SystemExit(f"no latin subsets found for {slug}")
    with css_out.open("a") as f:
        f.write(f"\n/* {slug} — vendored {css_url.split('family=')[1].split('&')[0]} */\n")
        f.write("\n".join(rules) + "\n")


def main() -> int:
    UI_FONTS.mkdir(parents=True, exist_ok=True)
    HANDOFF_FONTS.mkdir(parents=True, exist_ok=True)
    for css_path in (ROOT / "ui" / "fonts.css", ROOT / "handoff" / "fonts.css"):
        if css_path.exists():
            css_path.unlink()
    header = ("/* Self-hosted webfonts — vendored by tools/vendor-fonts.py.\n"
              "   Zero network at runtime: the packaged app's CSP forbids\n"
              "   fonts.googleapis.com and offline must not degrade type. */\n")
    (ROOT / "ui" / "fonts.css").write_text(header)
    (ROOT / "handoff" / "fonts.css").write_text(header)
    for slug, url in FAMILIES.items():
        print(f"vendoring {slug}…")
        process_family(slug, url, UI_FONTS, ROOT / "ui" / "fonts.css", "fonts/")
        # decks share the same files, referenced relative to handoff/
        process_family(slug, url, HANDOFF_FONTS, ROOT / "handoff" / "fonts.css", "fonts/")
    print("done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
