# Triton breakfast - how to run the showcase

## Open the deck

Double-click `handoff/triton-ventures.html` (any browser). That is the whole
setup - no server, no build, no network. It works offline from `file://`.

## Walk it

- **Arrow keys** (or Space / click) advance. **Home / End** jump to the ends.
- **12 slides.** Two of them embed the *real app*, live.

## Play with the live demo

Slides 5 ("Watch it work") and 7 ("The receipts, on demand") show the actual
`ui/` front-end running against a canned bridge - the same bridge the test
suite uses. It is genuinely interactive: type a prompt, press send, watch
thinking / tool calls / tokens stream in. Open the inspector with Ctrl-Cmd-I.

- **Click into the app to play.** While you are playing, arrow keys drive the
  app, not the deck (that is intended).
- **Click "return to slides"** (under the app) to hand the keyboard back to the
  deck and keep walking.
- The app keeps its state across slides - wander off and come back and your
  session is still there.

## If the embedded app looks blank

It loads from `handoff/showcase.html`, a generated file next to the deck. Make
sure you copied the whole `handoff/` folder. To regenerate it:

    python3 tools/make_showcase.py

To verify it has not drifted from the real UI:

    python3 tools/make_showcase.py --check

## What the embed actually is

`showcase.html` is `ui/index.html` with the mock `window.__TAURI__` bridge
(imported from `tests/test_desktop_ui.py` - single source of truth, never
copied) injected before boot. That means the demo is the *real* front-end:
if the deck's embed works, the app's UI works.

## Prove the app still works

    python3 tests/test_desktop_ui.py        # UI: streaming, dictation, steering, cancel, look-in, inspector, a11y
    cd src-tauri && cargo test              # 16 Rust seams

## The companion brief

`handoff/research-brief.md` - sourced intel on the room (R4, Hermes, the
Sizzle vocabulary, the Fourth Turning), with every unverified claim flagged.
