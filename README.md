# Spruce Grove Desktop

Thin Tauri 2 shell around the groomed [`spruce-grove`](../SPRUCE-GROVE-OS) CLI
(bead SPRUCE-GROVE-OS-5al.6). The shell owns **no agent logic** — every prompt
spawns the CLI in headless mode and streams its output back. Mockingbird is
the reference shape (Tauri 2, local, zero telemetry); this repo consumes the
CLI rather than re-implementing the harness, so the fork's diff does not grow.

## Status: v0 scaffold

- [x] Thin shell launches (Tauri 2 + static webview UI, no build step)
- [x] Talks to the CLI: `grove_send` spawns `spruce-grove --prompt ...`
  in the chosen working dir, streams stdout/stderr as window events
- [x] Conversation continuity via `--quick-resume` on turns 2+
- [x] ANSI stripping, cancel button, single-active-run guard
- [ ] Dictation-to-agent loop on-device (acceptance gap — planned)
- [ ] Upgrade path: speak ACP (`spruce-grove --acp`) for structured
  streaming/tool-call UI instead of line scraping
- [ ] Directory picker for the working-dir field

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
