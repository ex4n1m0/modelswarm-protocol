# Gap Engineering Study — Credits, Execution Presets, Reputation, Scale

**Date:** 2026-10-09 · **Requested by:** owner · **Question:** for each gap
found when cross-checking the quick summary against `zcode-master-prompt.md`,
either engineer it properly or remove it from the design.
**Method:** repo ground truth (file:line evidence below) + verified prior
art (primary sources cited). Two read-only research passes: a repository
evidence audit and a prior-art audit extending `docs/research/prior-art-matrix.md`.

---

## 0. Verdict table

| # | Summary claim | Verdict | One-line reason |
|---|---|---|---|
| G1 | "Compute credits based on useful verified work" | **SPLIT: keep non-transferable granted accounting; REMOVE any credit economy** | Accounting is the receipts-v2 work already proposed; an economy re-imports every BOINC/Gridcoin attack for zero product need |
| G2 | "Fast, Balanced, Deep, Verified, Maximum execution modes" | **ENGINEER as protocol-level policy bundles** | They are the composition layer ADR-013 lacks (HYBRID has no rules today); two major AI vendors independently converged on named tiers in 2025–26 |
| G3 | "Reputation" | **ENGINEER minimal: machine-derived counters + spot-check math, no trust propagation** | Approximate modes are unsafe without it; EigenTrust-style graphs are documented failure modes; ADR-012's suspension machinery is a ready chassis |
| G4a | "Hierarchical micro-swarms" | **KEEP the wording, BUILD nothing new** | Two-level selection (tracker narrows → planner picks k) + deliberation hierarchy is already the design; overlay trees would violate constraint 8 |
| G4b | "Horizontal distribution for many users" | **ENGINEER (small, real gap)** | Serving admission is admit-or-reject only today; bounded wait queue + fair queueing needed, wire vocabulary already exists |
| G4c | Implied 1M-endpoint scale | **REMOVE as a claim, keep as capacity hygiene** | No 1M extrapolation from sim is already a project non-goal; indexes/epochs are cheap hygiene |
| G5 | "Voting" as quality mechanism | **KEEP subordinate** | Sarmenta: voting+spot-checking combine multiplicatively; alone, voting over identical profiles is correlated-error amplification |
| G6 | "Prefix caching" | **KEEP gated as request shaping** | KV-transfer ban (constraint 8) caps it at dedup/shaping until an ADR lifts it |

Removed outright by this study: **transferable credits / any payments
surface**; **EigenTrust-style peer-rating propagation**; **the 1M number
in any public claim**. Two things flagged NEEDS EXPERIMENT: canary-based
spot-checking of non-deterministic LLM serving (G3), and whether
reciprocity alone sustains host supply (G1 rationale, Petals counterexample).

---

## 1. G1 — Compute credits: granted accounting, never an economy

### Evidence
- Today's receipts: signed `{requestId, profileId, peerIds, startedAt,
  endedAt, outcome, usageDigest}` two-phase (server signs, requester
  counter-signs) — `protocol/msp-v1.md:322-331`, `protocol/messages.proto:79-96`.
  Rust wire deliberately omits the receipt ("receipts land with the
  gateway/session layer", `crates/modelswarm-transport/src/message.rs:140-151`);
  spec sessions already sign `{session_id, rounds, committed_tokens}`
  (`crates/modelswarm-session/src/spec.rs:306-315, 495-547`); the store
  keeps unsigned accounting rows (`crates/modelswarm-store/src/lib.rs:180-205`).
- Receipts-v2 (proposed, expanded-mission §4.3) already adds the needed
  scalars: modeId, cohort digest, per-peer roles, accepted/verified token
  counts, **endpoint-seconds**.
- Fairness enforcement today: none — serving admission is admit-or-reject
  (8 sessions, 2 per peer, `crates/modelswarm-node/src/serving.rs:148-207`);
  the wire already carries `queue_position`/`eta_ms` (always 0,
  `message.rs:80-81`) and msp-v1 §6 defines a 10 s queue deadline.

### Prior art (what it says)
- BOINC credits are **claimed** from self-reported benchmarks but
  **granted** only after redundant validation; credits have "no monetary
  value" by design (Anderson, *An Incentive System for Volunteer
  Computing*, boinc.berkeley.edu). Benchmark-inflation gaming is
  community-documented; CreditNew made exaggeration self-defeating by
  normalizing against the claimer's own history. Lesson: **granted-not-
  claimed, and never let the worker price its own work**.
- **Gridcoin cautionary tale:** the moment an external token monetized
  BOINC's non-transferable credits, every credit-gaming attack regained
  a financial motive. Monetizing the ledger destroys the ledger.
- Comparable systems sit at extremes with nothing in between: Petals
  (no economy — and the volunteer swarm went dormant, repo last push
  2024-09), exo (personal clusters, no economy), Gensyn/Bittensor/
  Hyperspace (payments, staking/slashing, points→USDC). p2ptokens'
  BitTorrent-style ratio is the closest shipped analog to what we want.

### Design (keep half)
1. **Unit:** non-transferable verified-work accounting, measured in
   `endpoint-seconds` and `verified accepted tokens` — machine-derived
   from counter-signed two-phase receipts. No price, no transfer, no
   balance between users. This is what the summary's "compute credits
   based on useful verified work" can honestly mean.
2. **Grant rule (BOINC lesson):** a peer's counters increment only from
   receipts *it served and the requester counter-signed*, plus audit
   pass/fail events. Nothing self-reported increments anything — same
   discipline as ADR-012's `verified_capacity` ("measured, never
   self-reported", `ADR-012-eligibility-lease.md:31-32`).
3. **Self-work exclusion:** receipts where the serving installation
   equals the requesting installation (or its sybil ring, per the
   subnet/ASN diversity constraint already proposed in expanded-mission
   §7) contribute zero. Counters are per `(peerId, profileId)`.
4. **Newcomer discrimination + decay:** zero initial accounting; time-
   windowed weighting (EWMA) so stale contribution fades — verified
   mitigations from the reputation-attack survey (Hoffman et al., ACM
  CSUR 42(1)); whitewashing is priced in because a fresh identity must
   still host the exact artifact with an advertised slot (costly
   identity) to earn anything.
5. **What accounting buys:** (a) gateway fair-queueing weight class
   (quota classes ride the lease, expanded-mission §6); (b) a
   contribution line in the client ("your swarm standing") — display
   only; (c) reputation inputs (G3). It is *not* a medium of exchange.

### Removed half
Any transferable credit, token, or pricing surface: banned by AGENTS.md
constraint 8, re-imports the entire BOINC/Gridcoin attack catalogue,
adds regulatory surface, and answers a question the project rule already
answers — free-riders can't consume at all because consuming requires
actively hosting the same profile (ADR-012/026). The Petals dormancy
counterexample says volunteer-only fades; our bet is that reciprocity +
utility beats a currency at sustaining supply — **and that bet is
testable**: track host retention with accounting visible vs hidden in
the k=4 harness and early swarms. If retention fails, the answer is a
product decision (owner, new ADR), not a silent currency insertion.

### Roadmap home
Receipts-v2 ADR (R0) carries the fields; gateway fair queueing (DRR +
per-installation token bucket, already specified expanded-mission §5/§6)
consumes them; wire work is small because `queue_position`/`eta_ms`
already exist (stop hard-rejecting at admission; bound the wait by the
existing 10 s queue deadline).

---

## 2. G2 — Five execution presets as protocol design (not a UI choice)

The owner's clarification is decisive: Fast/Balanced/Deep/Verified/
Maximum are **protocol semantics** — the named contract surface above
the primitive-mode registry — not a user preference to be A/B-tested.

### Evidence
- ADR-013 registers five primitive modes (`single`, `hedged`,
  `speculative_exact`, `search_verified`, `map_reduce`); only `single`
  is default-on; **no composition rules exist anywhere** — HYBRID is
  named in the mission but ungoverned (`ADR-013:16-26,49-51`).
- No mode field exists on the wire at all — no `mode` in
  `messages.proto`, `InferenceRequest`, or `Handshake`; the only mode
  labels are off-wire bench/test enums
  (`modelswarm-bench/src/records.rs:23-37`, `modelswarm-session/src/spec.rs:668-683`).
  `SessionOffer` does not exist yet (it is proposed in expanded-mission §4.2).
- Privacy contract requires "the exact recipient set per mode"
  disclosed before any cooperative request is served
  (`docs/privacy.md:35-40`).

### Prior art
- OpenAI ships `reasoning_effort: low|medium|high` as one enum over
  reasoning-token internals; Anthropic is **migrating extended-thinking
  from a raw `budget_tokens` knob to a named `effort` parameter** — two
  vendors independently converging on named tiers is the strongest
  external signal this is the right contract shape.
- The failure modes are equally documented: OpenAI's hidden cost
  (billed reasoning tokens with no visible response), tier churn
  (`minimal`/`xhigh` drift across generations), silent coupling
  (Anthropic budget change invalidating prompt cache). The successes
  (Redis `appendfsync`, PostgreSQL durability recipes, Unity/Unreal
  quality levels) share: tiny tier set, one blessed default, and a
  **quantified cost/loss cliff stated per tier**.

### Design
A preset is a named, ADR-governed execution policy — a *bundle* of
registry modes plus budgets plus disclosure, negotiated at session open
and recorded in the receipt:

| Preset | Primitive modes allowed | Cohort cap | Correctness label | Extra disclosure |
|---|---|---|---|---|
| FAST | `single` | 1 | lossless (the baseline) | none beyond today's |
| BALANCED | `single`, `hedged` (honesty-patched) | 3 | lossless | recipient set = raced peers |
| DEEP | BALANCED + `best_of_n` + deliberation roles | per budget | **approximate — never lossless** | recipient set + role assignment + compute budget |
| VERIFIED | BALANCED + `audited` chain | per budget | detection-oriented; carries audit proofs | auditors see canary-class tasks |
| MAXIMUM | HYBRID = composition of individually-gated modes only | per budget | label = union of component labels | full composition + budgets |

Key properties:
1. **Presets give HYBRID its missing composition rules.** HYBRID stops
   being "anything composed" and becomes "one of the sanctioned bundles,
   each component individually gated" — this closes the open item the
   expanded-mission review left for last.
2. **Preset is the negotiated field; primitive-mode resolution stays
   planner-internal.** The requester offers `{preset, budget}` (rides
   the proposed `SessionOffer`); the planner still picks the smallest
   useful cohort *within* the preset envelope (master prompt §11 is
   unchanged — presets constrain the search space, they don't override
   the optimizer). The receipt records preset + resolved modes + budgets,
   which is exactly the explainability the cost model already demands.
3. **Consent granularity solved:** five human-readable recipient-set
   disclosures (privacy.md's requirement) instead of thirteen protocol
   modes. FAST ships immediately with today's recipient set.
4. **Anti-failure-mode by construction:** five tiers, one blessed
   default (FAST until cooperative gates pass, then BALANCED), and the
   cost/loss cliff stated per tier on the wire and in the UI (each
   preset declares its compute budget and correctness label — no OpenAI
   cost opacity). Tier semantics pin in the ADR; the wire enum is
   additive and frozen like the rest of msp-v1.
5. **UI consequence is minimal, not extra:** the preset *is* part of the
   state the master prompt's §1 minimal screen already shows — one label
   plus a selector; no per-mode chrome.

### Roadmap home
One ADR extending the mode registry (preset layer + budgets + recipient
sets), landed with the R0 governance batch; wire addition rides the
proposed `SessionOffer`/msp-cooperative-v1 namespace; FAST is live from
day one; BALANCED/DEEP/VERIFIED/MAXIMUM light up exactly when their
component modes pass their existing gates (R2→R5). Nothing about preset
engineering is a new physics risk — it is naming, budgets, and
discipline over modes that were already planned.

---

## 3. G3 — Reputation: minimal, machine-derived, detection-first

### Evidence
- Nothing exists today: telemetry is generic counters
  (`modelswarm-telemetry/src/lib.rs:99-192`, sole production counter
  `node.starts`); the tracker keeps a 10k observation ring and
  digest-only receipts with **no rollup queries**
  (`apps/tracker/migrations/0001_init.sql:87-95,132-137`); "reputation"
  appears only in docs.
- The chassis is already built: ADR-012 leases carry `audit_epoch`
  (O(1) revocation) and a suspension mechanism that zeroes slots;
  msp-v1 §7 states "receipts are the evidence base for reputation."

### Prior art
- **Sarmenta spot-checking** gives the sizing math: with spot-check
  probability *q*, a saboteur's surviving error contribution shrinks
  geometrically with blacklisting opportunities (~(1−q)^n); without
  blacklisting, damage is bounded at roughly 1/q bad results *per
  identity* — and ModelSwarm identities are expensive (a fresh one must
  host the pinned artifact). Combining redundancy (hedging/BoN) with
  spot-checking shrinks error exponentially at ~2.5× cost even against
  20% saboteurs.
- **EigenTrust is the counter-example to avoid:** trust propagation
  fails to a 40% colluding collective, is Sybil-vulnerable absent costly
  identity, and its newcomer pool is gameable by ghost identities.
- The ACM CSUR reputation-attack survey names our enemies precisely:
  self-promoting (sybil rings), whitewashing, slandering (vendetta),
  orchestrated combos; verified mitigations are costly identity,
  newcomer discrimination, temporal weighting, pairwise/direct
  observation — **not** graph propagation.

### Design
1. **Inputs (all machine-derived, hub stays content-blind):** granted
   receipt outcomes (G1 accounting), audit pass/fail events from the
   AUDITED mode, availability EWMAs from the existing heartbeat
   observations table. Per `(peerId, profileId)`, time-windowed.
2. **No peer ratings, no propagation, no global trust vector.** Direct
   observations only. This structurally eliminates slandering/vendetta
   (nobody rates anybody) and most self-promotion paths (counters move
   only on counter-signed work and audits).
3. **Enforcement ladder, soft → hard:** scheduler EWMA weight → cohort
   exclusion in the planner → suspension by zeroing slots through the
   existing ADR-012 machinery bound to `audit_epoch`. Hard enforcement
   only after the hostile-peer gauntlet (seeded-wrongness detection
   curves, ring detection via installation/subnet-ASN diversity,
   whitewash test = identity reset loses accounting and must re-host).
4. **Audit-rate sizing is Sarmenta's q, measured not invented** — same
   discipline as the planner's τ. Start q conservative, publish
   detection-rate curves as first-class results (expanded-mission T3
   already demands ≥0.9 detection at seeded wrongness with <1% false
   accusation).
5. **NEEDS EXPERIMENT (flagged, not assumed):** spot-checking assumes
   re-executable work. Greedy deterministic serving is re-executable
   (E0 proved cross-vendor byte-identity); **sampled** serving is not,
   and canary-prompt auditing of non-deterministic LLM serving is an
   unverified research hypothesis — it goes to the experiment track
   (P-series / 9.6-style), not into a shipping gate.
6. **No public per-peer reputation surface:** requester-private,
   quantized, on-demand; avoids enumeration, shaming, and brigading.

### Roadmap home
Chassis (counters + rollup query + suspension wiring) lands with R4
(before any approximate mode is default-on); detection curves are a T3
release gate; the canary hypothesis is an explicit experiment.

---

## 4. G4 — Scale language: keep the words where they're true, cut the claim

- **"Hierarchical micro-swarms"** — true of the design already: two-level
  selection (tracker narrows randomized bounded candidates → planner
  picks the smallest useful k) and hierarchical deliberation groups. No
  overlay trees, no DHT hierarchy, no super-peers — those violate
  constraint 8 and the recorded non-goals. **Verdict: keep the phrase,
  build nothing new for it.**
- **"Horizontal distribution for many users"** — a real but small
  engineering gap: today serving admission is admit-or-reject with no
  wait structure (`serving.rs:148-207`) while the wire already has
  `queue_position`/`eta_ms` and msp-v1 §6 defines a 10 s queue deadline.
  **Verdict: engineer a bounded wait queue + DRR fair queueing at the
  gateway** (already specified expanded-mission §5/§6); this is the
  concrete work item behind the marketing phrase.
- **1M endpoints** — keep the capacity hygiene already proposed (GIN
  index, randomized lookup, notice epochs — cheap, correct at any
  scale); **remove the number from any public claim** — "no 1M
  extrapolation from sim" is already a project non-goal, and the
  summary should not smuggle it back in as an implied capability.

## 5. G5/G6 — minor confirmations

- **Voting:** keep, strictly subordinate. Sarmenta's result is that
  redundancy × spot-checking shrinks error exponentially — voting alone
  over identical profiles is correlated-error amplification (identical
  weights, identical failure modes). The CONSENSUS contract already
  reports agreement as evidence; that stays.
- **Prefix caching:** keep PREFIX_AWARE as request shaping (profile
  residency, prompt-prefix dedup) — KV transfer stays banned (constraint
  8); the privacy-oracle constraint (cache-hit timing must never be
  hub-visible) is already recorded.

---

## 6. Consolidated changes this study implies

To `zcode-master-prompt.md` (on owner approval):
1. §1/§3: add the five-preset policy layer table (G2) as protocol
   design; FAST default until component gates pass.
2. New M-item or R-item: gateway bounded wait queue + DRR fair queueing
   on receipts-v2 accounting (G1 keep-half, G4b).
3. M10 additions: granted-accounting grant rules (self-work exclusion,
   newcomer discrimination, EWMA decay) and the minimal reputation
   chassis with the enforcement ladder + gauntlet gates (G3).
4. §12 threat model: add the four named reputation attacks with their
   structural mitigations; note Sarmenta q-sizing for AUDITED.
5. DoD: swap "compute credits" language to "verified-work accounting";
   never a payment surface (G1 remove-half).

Experiments to schedule (not blockers):
- E-A: canary spot-check detection on sampled serving (G3 hypothesis).
- E-B: host retention with visible vs hidden accounting (G1 sustainability
  bet — the Petals question).

Nothing in this study changes a frozen surface: every addition rides
ADRs (mode-registry preset extension, receipts-v2, threat-model v2)
exactly as the expanded-mission proposal already prescribes.
