# Spruce Grove Desktop

<p align="center"><img src="assets/spruce-grove-lockup-stacked.svg" alt="Spruce Grove — three spruces, one ground line; the center tree in gold" width="280"></p>

Thin Tauri 2 shell around the groomed
[`spruce-grove`](https://github.com/t-granlund/SPRUCE-GROVE-OS) CLI
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
- [x] **Self-hosted webfonts** (`ui/fonts/`, vendored by `tools/vendor-fonts.py`):
  Fraunces / Inter / JetBrains Mono / EB Garamond / IM Fell English SC load
  from disk — the packaged CSP and offline launches can no longer degrade the
  transcript's type mid-stream, and the handoff decks ship their own copies.
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
- [x] **Inspector drawer** (⌘I): repository state (branch, commits, dirty
  tree via live git), GitHub (PRs, workflow runs, deep links via `gh`),
  session turns (state, duration, tokens), and a **diagnostics** tab
  (platform, engine seams, analytics, session error ring + copy-report).
- [x] **Settings** (⌘,): persisted to the platform data dir, applied live.
- [x] **Personas with repo-level permissions**: personas bind allowed
  working dirs + feature grants (prompts / dictation / look-in /
  inspector), enforced at the action choke points. "Repo level access"
  is GitHub's own `viewerPermission` for the watched repos
  (`spruce-grove-os`, `spruce-grove-desktop`) via `gh` — no invented roles.
- [x] **Cross-platform chassis**: open/gh/data-dir/login-shell decisions
  live behind a platform module; CI runs `cargo test` on macOS, Windows
  and Ubuntu. Packaged-app smoke on Win11/Linux happens on real machines
  via CI artifacts — this desk is a Mac.

## CLI resolution order

1. `GROVE_CLI` env var — space-separated prefix, e.g.
   `GROVE_CLI="uv run --directory ~/dev/SPRUCE-GROVE-OS spruce-grove" npm run dev`
2. `~/.local/bin/spruce-grove` (the released CLI, installed by `uv tool`)
3. a source checkout's venv, searched in `GROVE_REPO` (if set), then
   `~/dev/SPRUCE-GROVE-OS`, then `~/SPRUCE-GROVE-OS` (historical)
4. bare `spruce-grove` on PATH

Locations are resolved by `grove_repo_dirs()` in `src-tauri/src/main.rs` —
one source of truth for both the CLI path and the default working directory.
Set `GROVE_REPO` to point the shell at a fork.

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
