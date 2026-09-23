# Research Brief — Triton Ventures breakfast/demo

**Purpose:** one honest, sourced page of intel so we can *meet them where they are*.
Every claim below is marked **[VERIFIED]** (pulled from a real source this session)
or **[UNVERIFIED]** (no working source — do not present as fact).

**Sources:** the Tuesday Sizzle newsletter bundle (Issues #0–#15, extracted from the
emtech-breakfast.club SPA), r4digitalsystems.com, hermes-agent.nousresearch.com,
Wikipedia REST (The Fourth Turning). Raw caches in `/tmp/*.txt`.

---

## 1. Who we're meeting

### Michael "Mike" Routen — **[VERIFIED]** (his own site)
- Founder, **R4 Digital Systems** (r4digitalsystems.com).
- **30+ years** in software development and CRM.
- Pitch: *"Get Control of the Busywork That Keeps Your Business Stuck"* — practical
  systems for small businesses, built around tools they already use.
- **The R4 Framework** (his vocabulary — use it):
  - **Reputation** — trust before the first conversation (reviews, referrals, social proof)
  - **Readiness** — converting and delivering when opportunity shows up (site, intake, CRM, follow-up)
  - **Re-engagement** — reworking people who already know you (past customers, dormant leads)
  - **Reach** — enough of the right people finding you (SEO, content, community)
- Process: 20-min Alignment Call → R4 Business Systems Alignment → **R4 Systems
  Opportunity Map** (now/later/not-yet) → optional **Starter Build** (CRM cleanup,
  automation, AI assistance, follow-up workflows).
- Proof he shows: bookkeeper 2 hrs → 15 min with an AI research workflow; a 12-room
  hotel whose **Voice AI agent** handles 200+ calls.
- **How to meet him:** he sells *simplification, not more software*. Frame our chassis as
  "the process is simplified before it's automated." He'll recognize that line — it's his.

### Zak Morris — **[UNVERIFIED]**
- User context: runs **emerging-AI-tech community events**; first **Fayetteville**
  instance is **Thursday**. Weekly updates live at emtech-breakfast.club.
- A "**Watney**" AI-companion story is attributed to him in our notes — *not confirmed*.
- **[UNVERIFIED]** role/background. Confirm spelling + bio with Tyler before the deck.

### Bill Akin — **[UNVERIFIED]**
- No working source found. Treat as an investor/board-side view until confirmed.
- **Ask Tyler** for role, thesis, and what he cares about.

---

## 2. Triton Ventures / The Triton Foundation — **[VERIFIED]** (Sizzle masthead)
- **The Triton Foundation** = the **non-profit arm of Triton Ventures** — a **501(c)(3)**.
- The **Tuesday Sizzle** newsletter is *produced by The Triton Foundation* and is billed as
  *"A Triton Foundation Community Resource."*
- So the newsletter we've been reading is literally their house organ — speaking its
  vocabulary is speaking their language.
-  `tritonventures.com` served an **expired TLS certificate** when checked this session.
  Confirm the correct current URL with Tyler before putting a link in the deck.

---

## 3. Hermes Agent (Nous Research) — **[VERIFIED]**
Mike raised it; it's the obvious comparable. Nous Research, "The Internet's Own AI."
- **MIT license**, free and open source. Desktop (macOS/Windows), Linux via terminal,
  or optional cloud. Plans: Free / Plus $20 / Super $100 / Ultra $200 (monthly credits).
- Six headline features, verbatim from the site:
  1. **Connect** — Telegram, Discord, Slack, WhatsApp, Signal, Email, CLI; one agent, one memory.
  2. **Remember** — persistent memory; *auto-generates skills*; never forgets a solved problem.
  3. **Schedule** — natural-language scheduling, unattended via the gateway.
  4. **Delegate** — **isolated subagents** with their own conversations/terminals/Python RPC.
  5. **Search** — web search, browser automation, vision, image gen, TTS, multi-model reasoning.
  6. **Experiment** — **five sandbox backends** (local, Docker, SSH, Singularity, Modal) with
     **container hardening and namespace isolation**.
- **Why it matters to us:** #4 and #6 are *containment*. Hermes markets the same
  "agents you can trust to run loose" story our chassis story answers. Where we differ:
  we go a layer **under** the sandbox — packaged-app CSP, least-privilege Tauri capability
  surface, and *regression guards that fail the build if the floor drops*. That's the wedge.

---

## 4. The Fourth Turning (Mike name-dropped it) — **[VERIFIED]**
- *The Fourth Turning: An American Prophecy* — **Neil Howe & William Strauss, 1997**
  (Broadway Books). Based on the **Strauss–Howe generational theory**: American history
  moves in ~80-year **saecula**, each with four turnings — a crisis-era **Fourth Turning**
  follows an unraveling and precedes a new high.
- Sequel: ***The Fourth Turning Is Here*, 2023**.
- Reception is mixed and worth knowing: Kirkus positive; historian **David Kaiser** calls it
  "provocative," gamely betting on it; **Michael Lind** calls it vague/"pseudoscience" and
  "non-falsifiable"; NYT's Jeremy Peters notes many academics dismiss it as "astrology."
  It has influenced policymakers (Steve Bannon).
- **How to meet him:** don't litigate the theory. Use it the way he does — as a *narrative
  frame* for "crisis eras reward builders who lay down durable infrastructure." Agree with
  the mood, not the astrology. One slide, no lecture.

---

## 5. The Sizzle worldview — the vocabulary to mirror — **[VERIFIED, quoted]**
From the newsletter bundle (Issues #0–#15; #13 Sep 8, #14 Sep 15 healthcare special with
Paige Francis, #15 is the vocabulary issue):

- **Harness engineering:** *"designing the execution environment an agent runs inside —
  context, permissions, retries, state."* Later sharpened: *"context management, tool
  permissions, retries, logging, state persistence across calls."* It is *"a
  platform/infrastructure problem — it belongs to the team building the agent runtime."*
- **Loop engineering:** *"the layer above the harness"* — the loop runs steps *the model*
  determines at runtime (a cron job runs steps fixed at write time). It's the *product/workflow*
  problem, a different owner from the harness.
- These two names were coined/discussed by **Peter Steinberger** and **Boris Cherny**
  (who leads Claude Code at Anthropic). Sizzle headline: *"Harness Engineering vs. Loop
  Engineering: The Field Finally Has Vocabulary."*
- **Hot Toast (their take):** *"The organizations that will build durable AI systems in 2026
  are the ones that treat harness and loop as separate engineering disciplines with separate
  owners... if you're doing both without distinguishing them, you're building technical debt
  at the architecture layer."*
- **Sovereign AI:** *"AI capability you control yourself, local or open-weight models,
  portable architecture, so no outside party can switch it off."* Called *"the quiet backbone
  of sovereign AI"* — Vincent's thesis from Issue #0, cross-referenced to GLM-5.
- Confirmed topic coverage across issues: AI benchmarks, security, hardware economics,
  robotics, sovereign AI, policy, research, health.

**The alignment:** our harness hardening commit this session is *literally* "harness
engineering" in their sense — tool permissions, state, a floor that fails the build on drift.
We can say that sentence in their own words and it will land.

---

## 6. The Leather Apron Club spine (our through-line) — **[VERIFIED, internal]**
- Franklin's **1727 Junto**: a 12-member mutual-improvement circle that traded questions,
  tested ideas, and did useful civic work. Source doc: `1.MASTER-ORCHESTRATION/Junto-Leather
  Apron Club.md`; already a section in `handoff/triton-ventures.html`.
- **Why it fits:** Triton's own framing is *"community resource"* (a non-profit arm). The
  Junto is the 300-year-old version of that: practical people, gathered, building durable
  things together. It gives the deck a spine that isn't a pitch.

---

## 7. Our proof-of-work (say this, don't just claim it)
- Packaged-app **CSP** hardened: `object-src 'none'; base-uri 'none'; form-action 'none';
  frame-ancestors 'none'` — plus `csp_guard()` that fails the suite if either the packaged
  policy or the harness copy drifts from that floor.
- Tauri **capability surface** locked: main window only, explicit allowlist (`core:default`),
  no wildcards — `cap_guard()` fails the build on widening.
- Both guards **negative-tested** (injected `fs:*` → suite FAILED for both reasons → reverted).
- Green: `cargo test` 16/16; `python3 tests/test_desktop_ui.py` PASS.
- Shipped: `07ed7ab` (CSP), `9e81ce9` (capability), `8231abf` (.gitignore) — all on `origin/main`.

---

## 8. Open questions to close with Tyler before presenting
1. **Bill Akin** role/thesis — nothing verified.
2. **Zak Morris** bio + the "Watney" story — nothing verified; confirm Thursday details.
3. **tritonventures.com** TLS is expired — is that the right URL for the deck?
4. **"Jev"** (TypeSafe System One) — **RESOLVED 2026-09-23**: source is the Emerging
   Tech Breakfast Club broadcast (Issue #15, Frontier Pulse). See §10 and
   `emtech-breakfast-club-source.md`. **"Opus 5.5 launching today"** — still unconfirmed
   by search; confirm spelling/source before it appears on a slide.
5. Broken `web-retriever` subagent (CLI v1.0.65 `ToolContext` ImportError) — fix or just
   work around? (blocks re-running live people research)

---

## 9. The Apple Retail lineage (Tyler's own background) — the mission spine

This is the bridge between who built this and what this is. It is also the honest answer to
"why should we believe you can do this?"

### Ron Johnson — **[VERIFIED]** (Wikipedia, MacRumors 2026-09-18, AppleInsider 2026-09-22)
- Ron Johnson: **VP of merchandising at Target**, then **SVP of retail operations at Apple**
  where he **developed the concept of the Apple Retail Stores *and the Genius Bar***; later
  CEO of JCPenney and founder of Enjoy Technology.
- He started at Apple **Feb 1, 2000**; the first two stores opened **May 2001**. A full-scale
  store was mocked up in a Cupertino warehouse before a single store opened.
- **His new book:** ***Shop Different: How Retail Revealed Apple's Genius***, with **Zander
  Nethercutt** — released **Tuesday, September 22, 2026**. That is *this week*. It is a live,
  current reference the room may well have seen.
- **Why it matters here:** Johnson's core insight was that a store is not a shelf — it is a
  *place where a person is understood*. The Genius Bar existed to **diagnose the person, not
  just the product**. That is the same instinct as "meet people where they are."

### The training frameworks — **[FIRSTHAND: Tyler]**
Tyler trained thousands of Apple Retail employees (internal promotions and external hires)
across iOS + macOS hardware/software troubleshooting theory, customer service, and best
practices — in a small-footprint store (Oakbrook, pre-relocation) and made it work.
He names the lead-learning principles he taught and still runs professional interactions by:
- **"Feel, felt, found"** — empathic acknowledgment before any advice.
- **"Acknowledge, align, assure"** — the service/recovery arc.
- **"Fearless feedback"** — direct, kind, no-politics critique.
- **One-to-One membership training** — the paid, human, hands-on teaching model Apple
  pioneered; teaching as a product, not a cost center.
*These specific wordings are Tyler's lived curriculum. Search engines could not corroborate
the exact phrasing this session, so the deck attributes them to him — not to Apple as
published doctrine.* That is both honest and more credible.

**The through-line:** Apple Retail under Johnson proved that **the human in front of you is
the product**. Tyler ran that playbook at scale. Spruce Grove is that same playbook, applied
to a machine: it *listens first, shows its work, and never pretends to know something it
doesn't.*

## 10. Jev / Laya — RESOLVED (source found)
- **The source surfaced on 2026-09-23.** It is **Issue #15, Frontier Pulse, lead story**
  of the **Emerging Technology Breakfast Club** broadcast *Signal // Tuesday* (Triton
  Station; Zak Morris + Bill Akins): *"Jev Turns Decision Inference Into a Product"*
  (dated 09·15).
- **Jev** = TypeSafe's **System One** model, early access **Sept 15, 2026** — typed
  state→decisions, **not generated text**; closed/paid API. **Laya** = Apache-2.0,
  421M-param open-weight alternative (Sept 18). **Cloud vs. local, made concrete.**
- Full verbatim capture, quotes, sources, and the room's recurring desks are in
  **`emtech-breakfast-club-source.md`** (this folder).
- **Where it goes in the deck:** the **"why local / sovereign AI"** slide. Present it as
  *the cloud/sovereign fork made concrete in one decision layer* — **not** as a
  benchmark contest (the issue itself flags Laya's comparison claims as unverified).
- **The club is the audience canon.** Sovereign/local-first is *their* recurring desk
  (`Sovereign AI`), and containment/receipts is their agent-governance thread — both
  validate Spruce Grove's thesis directly. See the companion file before the Thursday
  breakfast.
