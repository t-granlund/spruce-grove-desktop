# Grove Health Review — 2026-09-16

Full-court audit by three independent reviewers (solutions architect, e2e/QA architect, external research) plus a synthesis pass. Calibration: the bones are good — textContent-only DOM discipline (no XSS path), real subprocess timeouts in the inspector, `fail_all_pending` on EOF, the stderr-drain handling, fixture-replay contract testing, and a tiny capability surface are all above-average discipline. The findings below are where that discipline stops.

**The meta-finding:** nearly every high-severity item is a doc-vs-code gap — an intent that is documented (sandboxed screenshot reads, persona permissions, cross-platform targets) but not implemented in the code that was supposed to do it. The best part of this codebase is its honest comments; the code should catch up to them.

Method note: the QA agent could not spawn processes, so it rebuilt the entire Playwright suite in its own headless Chromium (same MOCK, same interactions) — 14 sections, 65/65 assertions green — then ran 4 adversarial probes. Severity = worst realistic outcome; Effort: S <1h, M hours, L a day+.

---

## TOP 20 — biggest impact first

| # | Area | Improvement | Why it matters | Effort |
|---|------|-------------|----------------|--------|
| 1 | **Security/robustness** | **Fix the CSP — it silently breaks shipped features.** `default-src 'self'; style-src 'self' 'unsafe-inline'` blocks `data:` images (look-in panel PNGs, shell.json logos) and the Google Fonts links in the *packaged app only* — the Playwright harness serves without CSP, so tests pass while the real build fails. Set `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; font-src 'self' data:; style-src 'self'` and add a `securitypolicyviolation` listener that boot-marks violations so the harness can see them. | Look-in is dead in production builds; fonts silently fall back; invisible class of breakage | S |
| 2 | **Security** | **Move persona enforcement into Rust and close self-escalation.** Zero persona awareness in src-tauri; `openSettings()` is itself ungated so any persona can ⌘, → reactivate itself → re-grant everything. Also bypassed at 3 `startAcp` call sites (sidebar chips, openSession, boot). Hold the active persona in Tauri state, check grants inside `grove_acp_start`/`grove_send`/`grove_transcribe`/`grove_read_image_base64`, and gate settings mutations. Document that personas are guardrails, not a security boundary. | The permission feature's core claim is currently cosmetic | M |
| 3 | **Robustness** | **Fix the legacy-mode cancel deadlock.** The waiter thread holds the `ActiveRun` mutex across `child.wait()`, so `grove_cancel` blocks forever — the cancel button is a no-op exactly when the CLI is in fallback mode. Copy the ACP path's `Option::take` pattern. | User-facing safety control that cannot work | S |
| 4 | **Cross-platform** | **Fix the Windows spawn path.** `acp::start` spawns `$SHELL -l -c` with a `/bin/zsh` fallback — ACP can never start on Windows 11, and the UI silently degrades to a legacy mode that also can't work. `#[cfg]`-split: direct spawn + enriched PATH on Windows, login shell on Unix. Add a runtime smoke of the spawn seam per OS. | A stated shipping target is structurally broken | M |
| 5 | **UX** | **Fix transcript auto-follow.** `acpAppendText` appends *then* checks near-bottom, so one chunk taller than the viewport permanently kills auto-follow (probe measured 4,341px behind). Capture stickiness *before* mutating; consider `overflow-anchor: auto`. | Every long response degrades the core reading experience | S |
| 6 | **Security** | **Narrow the capabilities.** `core:default` silently expands to nine core plugin permission sets (~27 window getters, menu, tray, image…) when the app only needs `core:event` (+ `core:path`/`core:app` for diagnostics). The bridge probe exists precisely to catch this regression — use it. | Least privilege per Tauri's own docs; shrinks the IPC surface | S |
| 7 | **Security** | **Patch tauri ≥ 2.11.1 (CVE-2026-42184, GHSA-7gmj-67g7-phm9)** — remote pages on subdomains matching registered custom protocols can invoke local-only IPC on Windows/Android. Cargo floats `tauri = "2"`; pin ≥2.11.1 and add `cargo audit`/`cargo-deny` to CI. Not directly exploitable here (no custom protocols, no remote pages) — hygiene, cheaply done. | Known advisory class on a shipping target | S |
| 8 | **UX/a11y** | **Overlay stack + focus management.** Picker and settings can be open simultaneously; one Escape closes *both* (destroying unsaved settings edits). Focus is never moved into `aria-modal` dialogs nor restored on close (probe: `activeElement` stayed BODY). Keep an overlay stack — top-most owns Escape; focus first field on open, restore invoker on close. | Data loss + keyboard/SR users stranded | M |
| 9 | **UX** | **Inspector drawer responsive behavior.** Fixed 320px overlay with no breakpoint/scrim/Escape: at 600px width it covers 96% of the transcript and the composer (probe: FAIL). Below ~720px make it a full-width sheet with scrim, click-outside + Escape to close. | Narrow windows (and Windows snapping) render the app unusable | M |
| 10 | **Robustness** | **Connection epoch on ACP events.** Events carry no session identity, so the death of the *old* connection during newChat/cwd-switch can trigger self-heal against the *new* session (spurious restarts, wrong-id resume). Add an epoch/sessionId field to `AcpEvent`; JS ignores non-current deaths. | Recovery machinery misfiring during routine navigation | S |
| 11 | **Robustness** | **Make the stall watchdog state-aware.** 80s of silence kills healthy long tool runs (a build or test suite legitimately streams nothing between start and completion). If the newest tool chip is running/pending, extend to warn-only; heartbeat if the CLI supports it. | Watchdog eats exactly the work it was built to protect | S |
| 12 | **UX** | **Real error toasts with actions** (VS Code/Raycast pattern): subprocess/ACP failures currently vanish into one dim 11px status line overwritten by the next event. Severity-styled, non-modal toasts with "copy details" / "open settings" actions; never rely on color alone; state cause + next step. | The difference between a stranded user and a self-served one | M |
| 13 | **Performance** | **Batch chunk emission + cap transcript DOM.** One `emit` + one span + one forced reflow per token-level event, nodes never pruned — 50–100 events/sec and unbounded growth in long sessions. Coalesce ~50ms in Rust; trim oldest messages behind an "earlier messages trimmed" sentinel (CLI remains source of truth). | Long sessions degrade; this is the streaming hot path | M |
| 14 | **Performance** | **Dictation audio over IPC as base64/raw, not a JSON number array.** A 5-min recording (~5–25MB) serializes to tens of millions of tokens (~10× inflation) and stalls the main thread at the moment the user wants a transcript. Decode in Rust (the reverse pattern already exists). | Visible stall in a flagship flow | S |
| 15 | **Robustness** | **Timeouts for `grove_version` and `grove_transcribe`.** Both use blocking `.output()` — a hung CLI wedges the version pill forever and bricks the mic button (the `finally` never runs). Promote inspector.rs's `run_with_timeout` to a shared module; 5s / 120s. | One hung child bricks boot chrome or the mic | S |
| 16 | **Security** | **Sandbox `grove_read_image_base64`.** It accepts any absolute image path ≤20MB — an unrestricted file-read primitive whose own doc comment claims a sandbox that doesn't exist. Feeders are agent-influenced (SHOT_PATH scraped from tool output, workspace shell.json). Canonicalize + require `spruce_grove_screenshots_*` or workspace roots. | Closes a documented-but-unbuilt sandbox | S |
| 17 | **Security** | **Fix the Windows `open_target` cmd.exe injection shape (BatBadBut class).** `cmd /C start "" <url>` breaks on `&`/`%`; a hostile git remote can plant `&` in a repo name → malicious checkout + one inspector click = code exec chain. Use `tauri-plugin-opener` or ShellExecuteW; at minimum reject cmd metacharacters. | Real chain on Windows via hostile remote | S |
| 18 | **Robustness** | **Validate settings on `get`, not just `set` — and tighten the schema.** `get` returns raw file contents; a valid-JSON file with `version: 2` or `personas: "x"` drives the permission logic unchecked. Extend validation: grants keys whitelisted, `dirs` shape, `owner/repo` regex for `watched_repos` (they flow into `gh` argv). | Permission control plane trusts an editable file implicitly | S |
| 19 | **Robustness** | **Fix `GROVE_CLI` in the ACP path.** Only the first token is wrapped; the rest become stray `sh -c` args, so multi-token values (`uv run --directory …`) silently lose `--acp`. Proper quoting helper over all tokens + a unit test. | A documented, tested affordance is broken | S |
| 20 | **CI** | **Make CI gate what the docs claim.** Add `clippy -D warnings`, `cargo-deny` (advisories), Windows `.msi` + Linux AppImage bundle builds (the TEST-MAP claim "packaged Win/Linux smoke rides the bundle artifact" currently has no artifact to ride), and bump actions (checkout v5 / setup-python v6 — Node 20 deprecation). | CI is green but doesn't prove the claims | M |

---

## The remaining 30 — grouped, no less real

### UX & a11y (12)
21. **Morph send → "steer ▸" while busy** — today the send button silently means cancel+steer; an accidental click kills a long turn.
22. **Preserve reading position; add a "new activity ↓" jump pill** — force-scroll only when near bottom; make look-in auto-open opt-in.
23. **Permission/failure errors get an inline banner with an action** ("open settings"), not a status-line blip.
24. **Dictation failure mapping + REC chip controls** — actionable copy ("no mic permission" vs "whisper rig offline"),  discard on the chip, `.recording` state on the mic button.
25. **Engine-failure onboarding checklist** — when ACP start fails, swap the empty state for "CLI present? cwd valid? provider key?" + a run-engine-check button.
26. **⌘K command palette** (sessions, settings, inspector actions) — the cross-tool convention (GitHub, VS Code); reuse the picker's list/keyboard-nav pattern.
27. **Keyboard cheat sheet ("?" overlay)** — ⌘I, ⌘,, Enter/Shift+Enter live only in `title` attributes today.
28. **Real button semantics for clickable rows** — `.sess-item`/`.dir-item`/`.insp-row` are divs; role/tabindex/Enter+Space + `aria-current`.
29. **Split status channel + aria-live** — one 11px line is the choke point for every event class; give it `aria-live="polite"` and announce turn completions.
30. **SR semantics pass** — `aria-label` on the prompt (placeholder vanishes on input), transcript as `role="log"` with `aria-live` off (announce summaries only, not token streams).
31. **Z-index budget documented** — the grain veil at 9999 will silently bury future toasts/banners (modal 60 / drawer 45 / toast 70 / veil 1).
32. **Contrast floor guardrail** — `--text-dim` ≈6.9:1 and `--text-faint` ≈6.2:1 pass WCAG 4.5:1 today (no rounding allowed); codify the floor in token comments so palette tweaks can't regress it.

### Robustness & correctness (7)
33. **Single steer state machine** — the 1200ms fallback fakes `turnActive=false` and can double-fire `session/prompt` on a still-busy session; re-send only on the canceled turn's end.
34. **Delete or gate the legacy line-mode transport** — it doubles the state machine and masks "CLI missing" with a mode that cannot work either; honest failure taxonomy instead (CLI missing / dialect mismatch / other).
35. **Typed event contract** — one serde tagged enum for `grove://acp` kinds; generate the Playwright mock from `fixtures/acp/session.jsonl`; log unknown kinds to the error ring instead of silently dropping.
36. **Temp hygiene** — voice recordings are world-readable (0644) and never deleted; markers use predictable symlink-following names; create 0600 with O_EXCL semantics, delete after transcription.
37. **Truncate session titles at capture** — full first prompts (pasted tokens!) persist in localStorage and render in the sidebar.
38. **Fix/replace `grove_open_path`'s `contains("..")`** — security theater (passes `~/.ssh`, rejects legit `..` names); canonicalize + workspace-subtree check or remove and label honestly.
39. **Persistent error ring** — survive restarts by persisting the ring to the data dir; the diagnostics "copy report" then has real history.

### Performance & architecture (6)
40. **TTL-cache `repo_state` (~15s) + parallelize the two gh calls + skip turn-end refresh when fresh** — worst case today is ~20s of serial git+gh on every drawer open and every turn end.
41. **Split main.rs** — `cli.rs` (resolution/spawn policy), `dictation.rs`, `shell_profile.rs`, `markers.rs`, one shared `run_with_timeout`; 15 commands + two subprocess idioms in one file is the grab-bag smell.
42. **Self-host the fonts** — removes the remote-content dependency (CSP-clean), kills the boot network fetch, guarantees brand typography offline.
43. **CLI version floor check** — warn when `spruce-grove --version` < a tested minimum; catches ACP dialect drift before it manifests as mystery behavior.
44. **WebView2 CDP fixture for real-shell smoke** — Playwright's official `connectOverCDP` pattern (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port`) gives a true Windows-shell e2e lane; keep `tauri-driver` for the protocol-sanctioned path.
45. **Modernize Playwright usage** — web-first `expect(...).toBeVisible()` style assertions instead of `wait_for_function` polling (Playwright's own best-practices docs discourage polling patterns).

### Testing & docs (5)
46. **No-iframe regression check** — one assert in the UI suite; locks the advisory-class invariant (CVE-2024-35222) permanently.
47. **Dirty-file row truncation fix** — the known cosmetic (`i/main.js` for `ui/main.js`); min-width + ellipsis.
48. **Fixture-driven mock payloads** — hand-maintained mock shapes let Rust↔JS drift pass CI by construction; derive from the committed fixture (pairs with #35).
49. **Update TEST-MAP honesty** — "cross-platform gate" should say what cargo test proves (compiles + unit tests) vs what it doesn't (runtime spawn seams); the spawn-smoke row (#4) belongs there.
50. **Document the persona threat model** — one paragraph in README: personas are guardrails for humans clicking buttons, the agent process runs with user authority, the settings file is user-writable plaintext. Honest docs are the charter.

---

## Where the reviewers converged (highest confidence)

- **CSP/capabilities**: solutions architect (finding 4/18) and web-retriever (findings 1–5) independently flagged the same breakage from different directions — one from behavior, one from docs.
- **Persona enforcement gap**: architect verified it by grep (zero Rust references, three bypassed call sites).
- **Escape/overlay + focus**: QA probe reproduced it live with DOM measurements.
- **core:default breadth**: both architect and researcher, same recommendation.

## What's already right (don't touch)

textContent-only DOM writes (no XSS path found), inspector subprocess timeouts + 256KB caps, `fail_all_pending` on EOF, stderr drain, the bridge probe's self-diagnosing instrumentation, the fixture-replay contract test, reduced-motion honoring, clean tab order, WCAG-passing text contrast, and a genuinely tiny dependency surface.
