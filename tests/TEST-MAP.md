# TEST-MAP — what is tested, why, and how

> The one-page answer to "do we know what we have, why we have it, and how it
> is implemented?" Read top to bottom: the CLI is the engine, this repo is the
> chassis, and the contract between them is the part most projects forget to
> test. We do not.

## Layer 1 — the CLI (spruce-grove, the engine)

| Suite | Where | Scale | What it proves | How to run |
|---|---|---|---|---|
| Unit + plugin + tool tests | `SPRUCE-GROVE-OS/tests/` | **363 test files** | Every tool, hook, plugin tier, i18n catalog (CI-audited), session storage, model factory (incl. the `custom_openai` branch the Synthetic rig rides) | `uv run pytest tests/` in the fork |
| Live-model integration | `tests/integration/*_live.py` | a handful | Real completions through configured providers (needs keys) | same, keys required |
| Publish gate | `.github/workflows/publish.yml` | CI | The whole suite + coverage on macOS before any release | push to `main` |
| Voice loop | `~/.spruce_grove/plugins/mockingbird/` + build-log 27/29 | manual + pexpect | mic → whisper → review → send, validated end-to-end with real audio | `/rec` in a session |

Why it matters: the desktop shell owns **no agent logic** — every behavior it
displays is produced here. A CLI regression is a desktop regression, which is
why the publish gate runs the CLI suite on every push.

## Layer 2 — this shell (spruce-grove-desktop, the chassis)

| Test | File | Proves | Run |
|---|---|---|---|
| Rust unit (10) | `src-tauri/src/acp.rs` + `main.rs` `#[cfg(test)]` | ANSI stripping; ACP router: response/agent-request/notification classification, chunk text extraction, permission option selection (allow-kind preferred), session/new parsing (sessionId + model), ndjson noise tolerance; dir listing (sorted, dotfiles hidden, symlinks resolved, parent reported, bad path errors honestly) | `cd src-tauri && cargo test` |
| **ACP contract fixture** | `src-tauri/acp.rs::replay_probe_fixture` + `fixtures/acp/session.jsonl` | The recorded `--acp` session replays through the real router: initialize (protocolVersion 1), ≥3 responses, message chunks present, available_commands present, zero agent-requests in no-tools mode. **Protocol drift fails loudly here first.** | `cargo test` (fixture is committed) |
| UI integration | `tests/test_desktop_ui.py` | Real `ui/main.js` in Chromium: cwd seeding + version pill (initCwd regression), ACP ready pill with model, streaming chunks + thinking + tool cards with status chips, per-turn usage in status, **mid-run steer** (busy send → cancel → redirected prompt in same session), **live look-in** (screenshot path in tool payload → panel + data-URL image), dictation flow (fake mic → save → transcribe → editable prompt), **working-dir picker** (browse → descend → choose → ACP restart in the chosen dir; escape cancels untouched) | `~/SPRUCE-GROVE-OS/.venv/bin/python tests/test_desktop_ui.py` |
| ACP dialect probe | `tests/acp_probe.py` | Ground-truth explorer against the real binary; `--lifecycle` proves **session/load survives process death** (marker recalled by a fresh agent) | `python3 tests/acp_probe.py [--lifecycle]` |
| CI gate | `.github/workflows/ci.yml` | Every push/PR: `cargo test` (macOS), the UI suite (Linux, mocked IPC), and a `tauri build` whose plist lint re-proves the mic grant; the .app comes back as a CI artifact | push to `main` |

## Layer 3 — the seams (where engines and chassis meet)

| Seam | Status | Guard |
|---|---|---|
| ACP dialect (CLI speaks ↔ Rust parses) | **pinned** by the fixture replay + probe | regenerate fixture after intentional CLI changes; the replay test then diff-reviews the drift |
| Dictation → `--transcribe` verb | CLI side verified against real audio (Station 4 wav/m4a); UI side verified with mocked IPC | `spruce-grove --transcribe file` |
| Packaged-app mic (WKWebView grant) | plist declared (`NSMicrophoneUsageDescription` in built bundle, plutil-verified); **the tap is human**: one mic click in the real .app | `npm run build` → open → record |
| Packaged-app ACP smoke (pill flips to `ACP · syn:large:text`, streamed prompt) | **human click**, pending | open the .app → send a prompt |
| Relaunch durability | **proven at protocol level** (`--lifecycle`); UI wires resume via stored session id per cwd | relaunch the app → pill shows `· resumed` |

## Known gaps (tracked, not hidden)

- `tauri-driver` WebDriver E2E: **not possible on macOS** — `tauri-driver`
  prints "not supported on this platform" (WKWebView exposes no public
  automation bridge; tauri-driver's WebDriver support is Linux/Windows only).
  Attempted 2026-09-11 with tauri-driver 2.0.6; see tests/e2e_tauri_driver.py
  (kept as the script for Linux/Windows CI, where the same assertions run
  against the real webview). macOS coverage therefore = Playwright
  (UI logic) + acp_probe --lifecycle (protocol) + one human smoke click
  (real WKWebView), by design of the platform, not by omission.
- CI: **closed** — `.github/workflows/ci.yml` runs `cargo test` (macOS),
  the Playwright UI suite (Linux runner, mocked IPC — no CLI needed), and a
  `tauri build` bundle job that lints the built plist and uploads the .app
  as an artifact. Still human: the two in-app smoke clicks (mic grant,
  ACP pill) — WKWebView grants cannot be automated, by platform design.
- VU meter / waveform in the recording window (YAGNI'd in build-log 27).

## The philosophy, in one line

Unit tests prove the parts; the fixture pins the contract; the UI test proves
the user sees what the engine does; the probe proves the engine still speaks
the same language. Anything less and "it works on my machine" is the only
remaining test.
