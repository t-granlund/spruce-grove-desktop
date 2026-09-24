# Triton breakfast - how to run the showcase

## Open the deck

Double-click `handoff/triton-ventures.html` (any browser). That is the whole
setup - no server, no build, no network. It works offline from `file://`.

## Sending the demo as a packet

`handoff/showcase.html` is a demo of the real `ui/` front-end, so it pulls in
six sibling files (`../ui/*.css`, `../ui/*.js`). **Sending it alone delivers a
broken page** - no styling, no interactivity.

Use `handoff/showcase-standalone.html` instead: the same deck with every asset
inlined, so one file is genuinely enough. Regenerate it after any `ui/` change:

```
python3 handoff/build_self_contained.py          # write it
python3 handoff/build_self_contained.py --check  # fail if out of date
```

Verified by loading it in a browser with the network fully blocked: three
stylesheets applied, composer wired, zero page errors.

**The passcode is a gate, not encryption** - it ships in the same folder as the
packet, so rotate it before any send (`data/showcase-passcode.txt` in the
OUTREACH repo).

## Walk it

- **Arrow keys** (or Space / click) advance. **Home / End** jump to the ends.
- **14 slides.** Three of them run live embeds.

## Play with the live demo

- **Slide 6 "Watch it work"** and **slide 9 "The receipts, on demand"** show the actual
  `ui/` front-end running against a canned bridge - the same bridge the test suite uses.
  It is genuinely interactive: type a prompt, press send, watch thinking / tool calls /
  tokens stream in. Open the inspector with Ctrl-Cmd-I.
- **Slide 8 "Same engine, no shell"** shows the engine *headless* - a terminal replay of a
  real session's shape (labelled `REPLAY` on purpose; the live CLI is the real demo).
- **Slide 11 "Where 'local vs cloud' stops being philosophy"** is the Jev/Laya story from this
  week's signal - the decision layer, closed API vs open weights. Sourced in
  `emtech-breakfast-club-source.md`; read it before presenting.

## The emerging-tech context (read before the Thursday breakfast)

`emtech-breakfast-club-source.md` is the club's canon - the weekly *Signal // Tuesday*
broadcast from Triton Station (Zak Morris + Bill Akins). It maps the room's recurring desks
(sovereignty, agent governance, receipts, NWA-local) so the deck speaks their language.

## The two-date runway (confirmed 2026-09-23)

- **Thu Sep 24, 8:30-9:45 AM CT** - the breakfast, **Fayetteville Public Library**
  (Ziegler room; South Parking Lot / McIlroy Entrance). Hosted by Zak Morris + Bill Akins
  (Triton Foundation). The room this deck is written for.
- **Fri Sep 25, 9:00-9:45 AM CT** - **Triton Ventures Discovery Conversation** with Zak.
  Dedicated follow-up time. Prep notes: `triton-discovery-call-prep.md`.

- **Click into a demo to play.** While you are playing, arrow keys drive the embed, not the
  deck (that is intended).
- **Click "return to slides"** (under the embed) to hand the keyboard back to the deck.
- The app keeps its state across slides - wander off and come back and your session is
  still there.

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
Sizzle vocabulary, the Fourth Turning), the **Apple Retail / Ron Johnson mission
spine**, and every unverified claim flagged. Section 10 logs **Jev/Laya** as a
parked thread - do not present it until the source resurfaces.

## Honesty notes (say these if asked)

- The desktop embed is the **real UI** on a **canned bridge** - real front-end,
  fake backend. Say so; it is more impressive, not less.
- The CLI slide is a **replay of a session's shape**, not a recording. The live
  CLI is the demo.
- The Apple Retail frameworks (`feel/felt/found`, `acknowledge/align/assure`,
  `fearless feedback`, One-to-One) are **Tyler's firsthand curriculum**, not
  quoted Apple doctrine. Attribute them to him.
