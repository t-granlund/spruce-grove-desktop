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
| Rust unit (16) | `src-tauri/src/acp.rs` + `inspector.rs` + `main.rs` `#[cfg(test)]` | ANSI stripping; ACP router: response/agent-request/notification classification, chunk text extraction, permission option selection (allow-kind preferred), session/new parsing (sessionId + model), ndjson noise tolerance; dir listing (sorted, dotfiles hidden, symlinks resolved, parent reported, bad path errors honestly); agent-death fails every pending request (no hung turns); inspector parsers (remote→owner/repo across https/ssh shapes, git-log unit-separator rows, porcelain status caps+counts); settings validation (version, persona shape, garbage rejection); persisted error trail (ring caps, timestamps) | `cd src-tauri && cargo test` |
| Cross-platform gate | CI matrix | `cargo test` on macOS + Windows 11 + Ubuntu runners — the platform seams (open/gh paths, data dir, login-shell spawn) compile and test everywhere; packaged Win/Linux smoke rides the bundle artifact | push to `main` |
| **ACP contract fixture** | `src-tauri/acp.rs::replay_probe_fixture` + `fixtures/acp/session.jsonl` | The recorded `--acp` session replays through the real router: initialize (protocolVersion 1), ≥3 responses, message chunks present, available_commands present, zero agent-requests in no-tools mode. **Protocol drift fails loudly here first.** | `cargo test` (fixture is committed) |
| UI integration | `tests/test_desktop_ui.py` | Real `ui/main.js` in Chromium: cwd seeding + version pill (initCwd regression), ACP ready pill with model, streaming chunks + thinking + tool cards with status chips, per-turn usage in status, **mid-run steer** (busy send → cancel → redirected prompt in same session, with a persistent `.msg.steer` transcript marker), **cancel mid-turn** (mock stops the fake stream on session/cancel, orderly `cancelled` turn-end, persistent marker, shell stays healthy), **live look-in** (screenshot path in tool payload → panel + data-URL image — under the packaged app's real CSP), dictation flow (fake mic → save → transcribe → editable prompt), **working-dir picker** (browse → descend → choose → ACP restart in the chosen dir; escape cancels untouched), **inspector drawer** (⌘I/toggle → branch + commits + dirty tree + PRs + runs render; run rows carry branch/duration meta and open via grove_open_url; PR rows carry state chips; commit rows deep-link to GitHub; Escape closes top-most; diagnostics tab shows platform/engine/analytics + persisted error trail; data-dir reveal via grove_open_path), **settings modal** (⌘, → personas + live gh viewerPermission + save; focus moves into the dialog on open and back to the invoker on close), **accessibility pack** (role=status live region on #status, transcript as role=log with live off, aria-labels on every icon-only control, tablist/tab/tabpanel + arrow-key tab walking, keyboard-operable session rows), **narrow window** (at 500x700: no horizontal overflow, cwd field on its own wrapped row, browse/send/sidebar-toggle all hit-testable), **session sidebar honesty** (no phantom "new session" row on boot; no stale active highlight on failed starts) | `~/SPRUCE-GROVE-OS/.venv/bin/python tests/test_desktop_ui.py` |
| ACP dialect probe | `tests/acp_probe.py` | Ground-truth explorer against the real binary; `--lifecycle` proves **session/load survives process death** (marker recalled by a fresh agent) | `python3 tests/acp_probe.py [--lifecycle]` |
| CI gate | `.github/workflows/ci.yml` | Every push/PR: `cargo test` on the macOS + Windows + Ubuntu matrix, the UI suite (Linux, mocked IPC), and a `tauri build` whose plist lint re-proves the mic grant; the .app comes back as a CI artifact. Proves compile + unit tests cross-platform; packaged Win/Linux runtime smoke is still a human download away | push to `main` |

## Layer 3 — the seams (where engines and chassis meet)

| Seam | Status | Guard |
|---|---|---|
| ACP dialect (CLI speaks ↔ Rust parses) | **pinned** by the fixture replay + probe | regenerate fixture after intentional CLI changes; the replay test then diff-reviews the drift |
| **Webview event bridge** | **was silently dead in the packaged app** — no `src-tauri/capabilities/` existed, so every `listen()` was permission-denied (invoke kept working, which masked it: handshake + send + version all ride invoke, only streaming rides events). The app looked alive and streamed nothing. | **bridge probe**: Rust emits `grove://bridge-probe` every 2s until acked; the webview acks via `grove_bridge_probe_ack`, which drops a per-PID marker in the app's TMPDIR. Missing marker = bridge dead. Boot breadcrumbs (`sg-boot-mainjs-loaded`, `sg-boot-listen-ok`) pinpoint how far boot got. NOTE: GUI-app `std::env::temp_dir()` is the launchd per-user dir (`/var/folders/...`), not `/tmp`. |
| Dictation → `--transcribe` verb | CLI side verified against real audio (Station 4 wav/m4a); UI side verified with mocked IPC | `spruce-grove --transcribe file` |
| Packaged-app mic (WKWebView grant) | plist declared (`NSMicrophoneUsageDescription` in built bundle, plutil-verified + CI-grepped); **the tap is human**: one mic click in the real .app | `npm run build` → open → record |
| Packaged-app ACP smoke (pill flips to `ACP · syn:large:text`, streamed prompt) | **human click** — but the bridge half is now automated by the probe; what remains human is eyes-on streaming | open the .app → send a prompt |
| Agent death mid-turn | stdout EOF now emits an `error` event and fails every pending request (unit-tested) — a dead agent ends the turn honestly instead of hanging until the 80s watchdog; the UI then **self-heals**: quiet kill + resume restart (observed live cause: provider `ModelAPIError: Connection error` kills the CLI after a turn), carrying the old session's sidebar title and dropping its stale row | `kill -9` the CLI child → revived child + ready pill within seconds, no human click |
| Relaunch durability | **proven at protocol level** (`--lifecycle`); UI wires resume via stored session id per cwd | relaunch the app → pill shows `· resumed` |
| Packaged CSP | **guarded in the harness**: `test_desktop_ui.py` serves every response under the exact CSP from `tauri.conf.json` (one honest deviation: `script-src 'unsafe-inline'` for the injected mock bridge only). The CSP now also pins `object-src`/`base-uri`/`form-action`/`frame-ancestors` to `'none'`; `csp_guard()` fails the suite if the packaged policy or the harness copy drifts from that floor, so data:-image media (look-in screenshots, workspace logos) and local fonts fail in CI if the policy regresses — the class of bug that used to ship invisible | the look-in image assertion runs under CSP |

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
