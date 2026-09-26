# Studio lock/unlock — the security model

**Status:** design, grounded in what this repo actually ships. Written 2026-09-25
after a dry run where *lock worked but unlock silently did nothing* — a real
bug, now fixed (see "The bug that started this"). The bigger ask in that report
was: **make lock/unlock mean something — tie it to a PIN, or to GitHub
permissions per project, so only the project owner can adjust it.**

This document answers that ask with the primitives this machine already has.

---

## 1. The bug that started this (fixed)

`ui/studio.js` `toggleLock()` flushed the summary *before* flipping the lock.
Rust's `update()` refuses any patch on a locked record except `locked: false`,
so on **unlock** the summary flush threw and the unlock patch never ran. Locking
worked; unlocking was structurally impossible. Fixed by skipping the flush when
already locked, plus a lock→unlock regression in `tests/test_desktop_ui.py`.
That was a *correctness* bug. The rest of this doc is the *design* question the
bug exposed: "locked" currently means "the UI greys itself out," and nothing
more.

## 2. What "locked" is today (be honest about it)

`recordings.rs` is explicit in its own header: *"Locking is a statement, not a
permission system."* The flag lives in a **plaintext JSON file** the user can
edit:

```
~/.spruce_grove/desktop/recordings/index.json   → { "locked": true, ... }
```

So today, *anyone who can open the file can unlock the record.* That is fine for
a single-user tool and dishonest as a "committed record." The design question is:
**what is the smallest thing that makes "locked" true, without pretending to a
security boundary the app cannot hold?**

## 3. The threat model — three different people

Do not design one mechanism for three different problems. Name them:

| # | Adversary | What they can do | What actually stops them |
|---|-----------|------------------|--------------------------|
| **T1** | **Future you**, fat-fingering | Click unlock, edit the committed record in a hurry | A confirmation gate (a PIN) — friction, not security |
| **T2** | **A collaborator on the same machine / same account** | Edit `index.json`, or drive the app via ACP | OS file permissions + a secret they don't have |
| **T3** | **A remote collaborator** (the git repo) | `git push` a changed transcript | GitHub-side review: branch protection, CODEOWNERS, signed commits |

The user's instinct ("permissions by user per project, based on GitHub") is
**exactly right for T3** and **wrong for T1/T2** — GitHub has no say over a file
that never leaves your laptop. Conversely a local PIN does nothing about a git
push. **A rock-solid design picks the mechanism per adversary.** Below, one local
mechanism and one repo mechanism, composed.

## 4. Local: the owner PIN (addresses T1 + T2)

### The primitive we already have

macOS ships a Keychain and a CLI for it: `/usr/bin/security`. No new dependency.

### The design

1. **First lock ever** prompts: *"Set an owner PIN for this library."* Store
   **not the PIN** but `argon2(PIN, salt)` in the Keychain item
   `spruce-grove-desktop.studio-owner` (never in `index.json`, never in git).
2. **Locking** is free — locking is the safe direction, anyone may do it.
3. **Unlocking requires the PIN.** `toggleLock()` on an already-locked record
   opens a small modal: *"This is a committed record. Enter the owner PIN to
   edit."* Wrong PIN → the record stays locked. Correct PIN → unlock, and the
   action is written to the audit ledger (`audit.rs`, already present) as
   `{"kind":"unlock","id":...,"at":...}`.
4. **The check is in Rust, not JS.** A new `grove_recording_unlock(id, pin)`
   command verifies against the Keychain before flipping `locked`. The existing
   `update(locked:false)` path is closed off (or routed through the same check),
   so editing `index.json` by hand does not *unlock through the app* — it only
   lies to a file the app would then read. To close that too:

### The honest limit (say it out loud)

A user with a text editor can still flip `"locked": true` in `index.json` and
the app will believe it. **A local PIN is a guardrail for the person using the
app, not a wall against the person owning the disk.** Two cheap mitigations make
it *tamper-evident* without over-promising:

- **HMAC the record.** On lock, store `hmac = sha256(key, record_body)` where the
  key is the same Keychain secret. On load, a locked record whose HMAC does not
  match is shown as **"locked — but the file changed since it was committed"**
  (a visible red state), even if `locked` still reads `true`. This turns a silent
  edit into a loud one. It is not prevention; it is *receipts*, which is this
  project's whole ethos.
- **0600 perms** on `index.json` and the Keychain item's ACL restricts unlock to
  the app. Already necessary for T2 basic hygiene.

This matches the project's own line in `ledger.rs`: *"not a gate — it never
blocks an action, it only refuses to forget one."*

## 5. Repo: GitHub-side enforcement (addresses T3)

The PIN protects the local artifact. When a **transcript/master is committed to
the shared repo**, the enforcement must live where the commit lands: GitHub.

### What actually enforces, today, with no new infra

| Layer | Setting | Stops |
|-------|---------|-------|
| **CODEOWNERS** | `recordings/** @t-granlund` (+ the steward org when it exists) | Merging a change to a committed record without owner review |
| **Branch protection** | Require review from Code Owners; dismiss stale approvals; require signed commits | Self-merging a transcript edit |
| **Signed commits** | `git commit -S` + "Require signed commits" | Attributing a forged commit to the owner |
| **Rulesets** | Restrict who can push to the `recordings/` path | Direct pushes bypassing review |

These are **the permissions the user described** ("based on the permissions
established in GitHub") — and they are *already the industry-correct answer*.
The desktop app does not need to re-implement them; it needs to **respect** them.

### What the desktop should do with GitHub

Read-only, via the `gh` CLI **already authenticated on this machine**
(`gh auth status` → account `t-granlund`, keyring-backed, `repo` scope). No new
OAuth app, no browser flow, no stored token.

- **Show provenance.** In the locked-overview panel, each committed record that
  has a git remote can display: last commit, author, signature status
  (`gh api` / `git log --format=%G?`). A record whose HEAD commit is unsigned,
  or touched by someone outside CODEOWNERS, is flagged.
- **Gate nothing on the network.** GitHub is consulted to *annotate*, never to
  *authorize a local edit*. The app must work on a plane. (If T3 is the threat,
  the gate is the merge, which happens on GitHub regardless of the app.)

### On "GitHub OIDC federated credentials"

The user floated OIDC-fed creds. **Honest answer: it is the wrong tool here, and
I'd push back.**

- OIDC federation is for **workloads running in CI** (GitHub Actions exchanging a
  short-lived token for cloud creds). It authenticates a *job*, not a *human at a
  laptop*.
- A local desktop app authorizing "who may unlock" has no OIDC issuer to trust:
  the identity it wants is "the human owner," which is a **local** question
  (§4), and the committed-artifact question is a **GitHub permissions** question
  (§5). Neither needs OIDC.
- Where OIDC *would* earn its keep: **CI-side verification** — a workflow that
  checks a committed record's HMAC/provenance and fails the build on a mismatch.
  That is a nice future step; it is not "lock the studio."

Recommendation: **PIN locally, CODEOWNERS+branch-protection remotely.** Skip
OIDC for this feature; note it as the CI-verification tool for later.

## 6. Composition — how the two halves meet

```
LOCAL (this machine)                    REMOTE (the repo)
─────────────────────                   ─────────────────────
owner PIN  ──verifies──► unlock         CODEOWNERS ──► review required
  │                                       │
  └─ HMAC(record) ──tamper-evident        └─ signed commits ──► attribution
        │
        └────────► commit the record ────────► merge (gated on GitHub)
```

The app's job: **make the local half real (PIN + HMAC + ledger), make the
remote half visible (provenance in the overview), and never claim to do the
remote half's job locally.**

## 7. Build order (smallest honest step first)

1. **DONE:** unlock actually works; transcribe is ANSI-clean; the master WAV
   decodes under the packaged CSP (all three were real bugs from the dry run).
2. **DONE:** owner-PIN unlock — `studio_auth.rs` (PBKDF2-HMAC-SHA256 verifier +
   random HMAC key in the macOS Keychain), `grove_recording_unlock` /
   `grove_studio_has_pin` / `grove_studio_set_pin`, the PIN modal (set on first
   lock, required to unlock), and a ledger receipt on unlock.
3. **DONE:** HMAC-on-lock + the "changed since committed" banner — a locked
   record edited on disk now reads back `tampered` and shows a warning instead
   of silently trusting the file.
4. **Next:** locked-overview dashboard (bead `-0bu`) shows provenance
   (last commit, author, signature) read-only via `gh`.
5. **Later:** a CI check that verifies committed-record HMACs (where OIDC, if
   ever, fits).

Implementation notes worth keeping:
- The PIN is stored **only** as a PBKDF2 verifier; the HMAC key is separate and
  random, so knowing the verifier never reveals the tagging key.
- The Keychain item is `spruce-grove-desktop.studio-owner` (verified writable
  via `security add-generic-password -U` on this box). Non-macOS / no-Keychain
  falls back to a **0600 file** and says so — weaker, and documented as weaker.
- `GROVE_PBKDF2_ITERS` lowers the iteration count **only** on the file backend
  (tests/scripts); the Keychain backend is pinned at 600k.
- A whole library restamps with **one** Keychain read (`OwnerGuard`), not one
  subprocess per record.

## 8. What I could NOT verify (do not present as fact)

- That `--transcribe` on *this* build emits the OSC preamble — verified by the
  *dry-run screenshot* and by `--version` sharing the boot palette, not by a
  fresh capture this session. The fix is belt-and-braces either way.
- The exact Keychain ACL semantics for a renamed/signed app bundle — needs a
  build-and-test on the packaged `.app`, not the dev server.
- Whether the steward org (transfer pending in `SPRUCE-GROVE-OS-r8c`) changes the
  CODEOWNERS target — it will; wire it when the org exists.
