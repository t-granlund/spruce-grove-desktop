# Spruce Grove Desktop

<p align="center"><img src="assets/spruce-grove-lockup-stacked.svg" alt="Spruce Grove — three spruces, one ground line; the center tree in gold" width="280"></p>

Thin Tauri 2 shell around the groomed [`spruce-grove`](../SPRUCE-GROVE-OS) CLI
(bead SPRUCE-GROVE-OS-5al.6). The shell owns **no agent logic** — every prompt
spawns the CLI in headless mode and streams its output back. Mockingbird is
the reference shape (Tauri 2, local, zero telemetry); this repo consumes the
CLI rather than re-implementing the harness, so the fork's diff does not grow.

## Status: v0 scaffold — GRAN-flushed, hand-off ready (2026-09-14)

> **The hand-off:** `handoff/presentation.html` (the deck, arrow keys) and
> `handoff/training-guide.html` (the deep guide) — prepared for Dustin Boyd,
> founding member, author of Mockingbird, owner of the desktop experience.

- [x] Thin shell launches (Tauri 2 + static webview UI, no build step)
- [x] Talks to the CLI: `grove_send` spawns `spruce-grove --prompt ...`
  in the chosen working dir, streams stdout/stderr as window events
- [x] Conversation continuity via `--quick-resume` on turns 2+
- [x] ANSI stripping, cancel button, single-active-run guard
- [x] GRAN flush: official mark in the sidebar, grain veil, never-flat-black canvas, rune empty state, ember favicon (chrome only — the four contracts untouched)
- [x] Dictation-to-agent loop on-device: mic → MediaRecorder →
  `grove_save_recording` → `spruce-grove --transcribe` (local whisper rig) →
  editable prompt. Remaining acceptance: one human mic click in the real
  .app (the WKWebView grant cannot be automated — TEST-MAP).
- [x] ACP live sessions (`spruce-grove --acp`): structured chunks, thinking,
  tool cards with status chips, per-turn usage, mid-run steering, and
  session resume across process death (`session/load`).
- [x] Directory picker for the working-dir field: an in-webview folder
  browser over `grove_list_dirs` — deliberately not a native dialog, so
  the Playwright harness can drive it like every other flow.

## CLI resolution order

1. `GROVE_CLI` env var — space-separated prefix, e.g.
   `GROVE_CLI="uv run --directory ~/src/fork spruce-grove" npm run dev`
2. `~/SPRUCE-GROVE-OS/.venv/bin/spruce-grove` (in-house fork venv)
3. bare `spruce-grove` on PATH

## Dev

```sh
npm install
npm run icon   # once, after editing assets/source-icon.png
npm run dev    # launches the shell
npm run build  # produces a .app bundle
```

## Provenance

Brand assets copied from the SPRUCE-GROVE-OS fork (which is itself a rebrand
of upstream mpfaffenberger/code_puppy). Zero telemetry, zero network calls
added by this shell; the CLI's own provider traffic is the only wire.
