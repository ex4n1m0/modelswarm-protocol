# ADR-044: Reputation chassis v1 — derived standing grades on the granted ledger

Status: **Accepted** (standing priority item #4 of the owner-approved
2026-10-18 sequence "continue"; grade vocabulary + thresholds mirror
ADR-043's owner-approved classes, env-tunable) · Builds on ADR-030
(granted accounting), ADR-012 (suspension), ADR-043 (fair queueing — the
same decayed-served measure).

## Context

The master plan's trust substrate names three consumers of one evidence
base: granted accounting, fair queueing, and the reputation chassis. The
first two are live (ADR-030 B3 in production; ADR-043 landed 2026-10-18).
The reputation chassis is the third: a peer-visible, honest standing that
answers "what does the swarm know about my contribution" — the
torrent-ratio aesthetic the owner has anchored on throughout, and the
"reputation-as-deliverable" the positioning summary flagged as missing.

Two chassis rules carry over verbatim from ADR-030 §5: **they read, they
never self-report** (every input is hub-derived: intake-validated
receipts, hub-issued suspensions, hub-signed challenges), and **derived,
not stored** — no new ledger table; the grade is computed on read from
the existing counters, so it can never disagree with the evidence or
need a backfill migration.

## Decision

1. **Grades (vocabulary frozen, plain-language):**
   - `member` — hosting, in good standing, base. Everyone valid starts
     here; it is not a penalty.
   - `contributor` — decayed served throughput ≥ T2 (the same bar as
     fair-queue weight 2).
   - `cornerstone` — decayed served throughput ≥ T3 (the weight-4 bar).
   - `recovering` — an ACTIVE suspension holds the peer's slots at zero
     (ADR-012); grade returns automatically when the window ages out.
     A suspension overrides any contribution grade for its duration —
     honesty before achievement.
   - `probation` — reserved: active suspension with `suspensionCount`
     ≥ 3 (ADR-012's escalation band). Ships labeled in v1 but the
     distinction from `recovering` is informational only.

2. **Computation** (`apps/tracker/lib/reputation.ts`, pure): inputs =
   `grantedCounters` decayed tok/s (verified tokens ÷ endpoint seconds,
   7-day half-life — the identical measure ADR-043 bucketed) + the
   suspension row. Thresholds REUSE `MSP_FAIR_WEIGHT_T2`/`T3` (defaults
   1 / 5): one knob set drives both fair-queue priority and standing —
   a peer's grade can never disagree with its queue weight.

3. **Surfaces (v1, both additive):**
   - Heartbeat response gains `standing { grade, served_tok_per_sec,
     served_tokens_decayed, strikes }` — zero extra round-trips (the
     heartbeat already returns per-minute state) and covered by the
     existing signed envelope.
   - `GET /api/v1/accounting?peer_id=` gains the same `standing` object
     (the endpoint ADR-030 §5 already earmarked for this chassis).
   - Desktop: the daemon surfaces `node.standing` in `get_status`; the
     swarm HUD line appends "· standing: contributor" — informational
     display only, no behavior change anywhere.

4. **Explicitly out of scope (future work, own decisions):** roster
   per-peer grades (changes peer-selection semantics), website census
   display, any gating of access (ADR-041/042 own access; reputation is
   display + inputs for later policy, never a v1 gate).

## Honesty note

Grades decay with the same 7-day half-life as the underlying counters —
last month's cornerstone is this month's member until it serves again.
`served_tok_per_sec` is rounded for display. With the current 2–3 peer
swarm everyone is `member`; the first real serving data will move peers
to `contributor` visibly.

## Risks

- Vocabulary freezing: grades are wire strings clients may display;
  renames would need an ADR — hence the small, plain set now.
- The strikes input is the persisted cumulative `suspensionCount`
  (v1 simplification: no trailing-window strike query); ADR-012's
  row-aging already recovers the grade when the window passes.
