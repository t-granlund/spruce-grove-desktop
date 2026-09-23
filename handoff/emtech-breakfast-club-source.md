# The Emerging Technology Breakfast Club — source & canon

> Companion to `research-brief.md`. This is the **audience canon**: the body of
> work the room already reads and reacts to every week. Captured directly from
> the club's own broadcast, 2026-09-23.

## What it is

- **Publication:** *Signal // Tuesday* (early issues ran under *The Tuesday Sizzle*
  and *Emerging Tech Breakfast Club Edition*).
- **Publisher:** **Triton Station** — masthead: *"A Triton Station Broadcast · The
  Crow's Nest."*
- **Hosts:** **Zak Morris** (Founder, Triton Ventures / The Triton Foundation) and
  **Bill Akins**. Bentonville, AR.
- **Cadence:** weekly, Tuesdays. Issues #0 (2026-06-16) → **#15 (2026-09-21 feed)**,
  ~16 editions and counting.
- **Location:** <https://www.emtech-breakfast.club> — passphrase **`sizzle`** →
  *Enter Room*. (Passphrase confirmed by Tyler; the gate is client-side on the
  "Gate" page.)
## The two dates (aligned 2026-09-23)

**1. The breakfast — Thursday, September 24, 2026, 8:30–9:45 AM CT** 🥇
- **Emerging Tech Breakfast Club – Fayetteville**, at the **Fayetteville Public
  Library** — **Ziegler room**. Park in the **South Parking Lot**; enter through the
  **McIlroy Entrance** off School Ave (closest to Rock St).
- **Hosted by Zak Morris** (Triton Foundation) with **Bill Akins of Triton Foundation**.
  ~18 going. Registration/updates: <https://luma.com/8k2n4jp0>.
- This is the in-person room the deck (`triton-ventures.html`) is written for.

**2. The discovery call — Friday, September 25, 2026, 9:00–9:45 AM CT** 🥈
- **Triton Ventures – Discovery Conversation** with **Zak Morris** (booked via Google
  appointment scheduling; confirmation sent to `hello@tylergranlund.com`).
- 45 minutes of *dedicated* time — the follow-up the deck's closing slide invites
  ("the follow-up is a conversation, not a demo"). Prep:
  `triton-discovery-call-prep.md`.
- **Note:** this one was booked off-calendar; it is **not in the synced Apple/Google
  calendar** as of 2026-09-23 — worth adding so it doesn't slip.

### Editorial desks (the taxonomy the room thinks in)

`NWA Local` · `Frontier Pulse` · `Security` · `Hardware` · `Mechanics` ·
`Policy` · `Sovereign AI` · `Research` · `Workforce` · `Creative` · `Culture` ·
`Guest`. Earlier issues used `Deep Impact / Technical / Releases / Horizon`.

## Issue index (mastheads, verbatim)

| # | Date | Masthead |
|---|---|---|
| 0 | 2026-06-16 | Signal // Tuesday — Emergent Tech Breakfast Club Edition |
| 1 | 2026-06-23 | Signal // Tuesday — Emerging Technology Breakfast Club Edition |
| 3 | 2026-06-30 | Signal // Tuesday — Issue #3 |
| 4 | 2026-07-07 | The Tuesday Sizzle — Issue #4 |
| 5 | 2026-07-14 | The Tuesday Sizzle — Issue #5 |
| 6 | 2026-07-21 | The Tuesday Sizzle — Issue #6 |
| 7 | 2026-07-28 | Signal // Tuesday — Issue #7 |
| 8 | 2026-08-04 | Signal // Tuesday — Issue #8 |
| 9 | 2026-08-11 | Signal // Tuesday — Issue #9 |
| 10 | 2026-08-18 | Signal // Tuesday — Issue #10 |
| 11 | 2026-08-25 | Signal // Tuesday — Issue #11 |
| 12 | 2026-09-01 | Signal // Tuesday — Issue #12 |
| 13 | 2026-09-08 | Signal // Tuesday — Issue #13 |
| 14 | 2026-09-15 | Signal // Tuesday — Issue #14 |
| 15 | 2026-09-21 | Signal // Tuesday — Issue #15 |

(No #2 in the bundle; numbering skips it.)

## The Jev / Laya item — source located

**Why it matters:** `research-brief.md` §10 parked "Jev (cloud) vs Laya (local)" as
*unverifiable — do not invent it into the deck.* **The source is now found.** It is
**Issue #15, Frontier Pulse, the lead story**, dated `09·15`:

> **"Jev Turns Decision Inference Into a Product"**
> *TypeSafe's early-access Jev turns typed decisions and calibrated probabilities into
> an API product, while an Apache-2.0 alternative followed days later.*

Verbatim facts from the issue:

- **TypeSafe released Jev** in **early access on September 15, 2026.** It is a
  **"System One" model**: software sends a *state + typed questions*, and receives
  **structured choices, scores, or yes/no probabilities — not generated text.**
  Positioned for **routing, classification, scoring, and guardrail decisions**.
- **Vendor figures:** 70–500 ms response, **$0.042 / million input tokens** for
  System-One-shaped tasks. (Issue flags these as vendor numbers; savings/accuracy
  "require workload-level evidence.")
- **LangChain integration (Sept 17):** wires Jev into **model routing and
  pre-action guardrail middleware.**
- **Laya (Sept 18):** a public repository + model card, **Apache-2.0 weights, a
  421M-parameter System One alternative**; author traces the line to 2025
  non-autoregressive decision research.
- **Issue's own framing:** *"A distinct application layer is taking shape around a
  familiar job: return a bounded decision with a usable confidence measure. A vendor
  API, a framework integration, open weights, and community recreations now compete
  around that job."*
- **The hard part, per the issue:** *"arrives after release, when outputs meet real
  failures, escalation paths, and someone accountable for the threshold."*
- **Editorial pull-quote:** *"Jev is the shiny object. The rest of the stack is where
  I keep getting stuck."*

**Cloud vs. local, stated honestly:** Jev is a **closed, paid API** (cloud); Laya is
**open weights** (local/sovereign-capable). The issue presents them as **separate
projects addressing the same technical problem**, and explicitly warns that Laya's
comparison tables use different prompts and publish Jev's own figures —
**"performance claims remain unverified."** Do not present the two as a benchmark
contest; present them as **the cloud/sovereign fork made concrete in one decision
layer.**

Sources the issue itself cites:
- TypeSafe Jev release — `https://typesafe.ai/blog/introducing-system-one-models-and-jev`
- LangChain integration — `https://www.langchain.com/blog/building-a-harness-with-jev`
- Laya model card — `https://huggingface.co/convaiinnovations/laya`
- Laya repository — `https://github.com/NandhaKishorM/laya`
- Architecture probe (speculative) — `https://archerhume.com/posts/jevs-architecture-unmasked/`
- Open alternatives roundup — `https://apidog.com/blog/openjev-open-source-jev-alternatives/`
- SemIf open implementation — `https://github.com/TheoLeeCJ/SemIf`
- `jevlike` research starter — `https://github.com/vinnylarouge/jevlike`

## Themes the room returns to (the "ways of working" signal)

Pulled from the recurring desks/newsletter across #0–#15:

1. **Sovereignty / local-first is a standing rubric** — its own `Sovereign AI` desk.
   Recurring: *"The Dependency Trap — Why You Need a Sovereign AI Plan, Even If You
   Don't Need It Yet"* (#7); self-hostable frontier models (Kimi K3, Bonsai 27B on a
   phone); open weights overtaking closed. **This is exactly Spruce Grove's thesis,
   and where Jev/Laya live.**
2. **Agent governance & containment** — *"AI Agent Sprawl Is a Governance Problem"*
   (#11); *"You Can't Secure What You Can't See"* (#5/#6); *"Agent Sprawl —
   Organizations Are Drowning in Redundant, Conflicting AI Bots"* (#7); *ACAAI v1.0:
   Accountability by Design for Agentic AI* (#9).
3. **Receipts / audit / accountability** — *"AI Governance: Who Is Accountable When
   AI Makes the Decision?"* (#5); the clockwork *"someone accountable for the
   threshold"* line in the Jev piece; Illinois frontier-AI safety audits (#9/#11).
4. **NWA-local as a first-class desk** — MIT PATH/RAISE NWA, NWACC training paths,
   NWA Tech Summit (Sept 1), FUEL Accelerator, Triton Ventures invited to the MIT
   pilot summit. **The club is regional and proud of it.**
5. **Physical AI / hardware** — humanoids, robotics, solid-state, ARPA-H/health.
6. **Security reality** — agentic ransomware, escaped red-team agents, the
   open/closed cyber-capability gap, post-quantum deadlines.
7. **Human-centered design / "meet people where they are"** — cognitive debt,
   delegation over prompting (#13: *"The Work Is Shifting From Prompting Toward
   Delegation"*), One-to-One-style teaching.

## How this ties to Spruce Grove

- **The deck is the room.** `handoff/triton-ventures.html` addresses this exact
  audience (Zak, Bill, the Triton room). The passphrase `sizzle` matches the deck's
  own *Sizzle harness/loop* vocabulary — same root.
- **Sovereign/local-first is not our opinion — it is their desk.** Present Spruce
  Grove as *the local-first answer the club already asks for*.
- **Containment + receipts = our beads `qp4`.** The club's agent-governance thread
  is the external validation for the action-ledger / self-audit surface.
- **Jev/Laya is the freshest concrete example** of the fork the sovereign desk
  describes: a decision layer where a closed API and open weights now compete.

---

*Provenance: extracted from the `emtech-breakfast.club` client bundle on 2026-09-23.
Quotes are verbatim from the broadcast. Vendor performance numbers are flagged as
vendor-supplied. Verify live at the source before any claim appears on a slide.*
