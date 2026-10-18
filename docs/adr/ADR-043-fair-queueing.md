# ADR-043: Fair queueing — bounded wait queue + DRR over granted contribution

Status: **Accepted** (owner go 2026-10-18 "sounds good" on the design and
the default thresholds 1 / 5 decayed tok/s, both env-tunable without a
code release) · Closes the ADR-028 deferral ("§6.3
`accepted` emission DEFERRED to M10 fair queueing — bounded wait queue +
DRR") · Builds on ADR-030 (granted accounting), ADR-041 (capacity
balance), ADR-042 (capacity stake).

## Context

Serving admission today is admit-or-reject with hardcoded limits
(`Admission::defaults()` = 8 concurrent sessions, 2 per peer,
`crates/modelswarm-node/src/serving.rs:384`): a request beyond the limit
is refused immediately with `overloaded` (`session_limit` /
`per_peer_session_limit`). There is no wait queue, and the §6.3
`accepted` event is never emitted. Meanwhile msp-v1 §6.4 already
promises a **10 s queue admission deadline**, and §6.3 already defines
`accepted` — ADR-028 deferred exactly this work.

The fairness intent follows the owner's torrent framing and ADR-030's
stated purpose ("accounting buys a fair-queueing weight class"): when a
serving host is contended, peers who have **served** the swarm should be
admitted first. ADR-041 already caps how much anyone may pull
(capacity-based, not history); this ADR is the complementary
**contributor-priority lane** — it reorders waiting requesters, it never
denies access.

## Decision

1. **Bounded wait queue at serving admission.** A session that finds all
   concurrent-session slots taken waits in a bounded queue (capacity 16,
   matching the `ServingInbox` buffer) up to the §6.4 queue admission
   deadline of **10 s**. On deadline expiry the refusal is `overloaded`
   with reason `queue_deadline` (closed-vocabulary addition, R0.5-style
   client+server together). When the queue itself is full, today's
   immediate `overloaded` / `session_limit` refusal stands.

2. **DRR (deficit round robin) over waiting requesters, weighted by
   hub-signed contribution class.** The weight is carried IN the
   `EligibilityLease` as `queue_weight: u8` (values 1 / 2 / 4,
   `#[serde(default)]` → 1):

   - the tracker computes it at lease issuance from **decayed granted
     counters** (`grantedCounters`, 7-day half-life — served
     tokens/second measured only from intake-validated receipts;
     nothing self-reported), bucketed by two env-tunable thresholds
     `MSP_FAIR_WEIGHT_T2` / `MSP_FAIR_WEIGHT_T3` (proposed defaults
     1 / 5 decayed tok/s);
   - serving peers never query the hub on the admission hot path, and
     the value is hub-signed so a peer cannot self-report a better
     class;
   - forward/backward compatible: `lease.rs` has no
     `deny_unknown_fields` (verified) — old serving binaries ignore the
     field, new binaries reading old leases get class 1.

   DRR semantics: each waiting peer accumulates credit at `quantum ×
   queue_weight` per scheduler round; the peer with the largest deficit
   is admitted when a slot frees. Class 1 is everyone with a valid
   lease — a free-rider still gets served, just last under contention.

3. **`accepted` emission un-deferred.** Serving emits the §6.3
   `accepted` event exactly once per stream at admission, with
   `queue_position` (0 for immediate admission, otherwise the position
   held when scheduled) and `eta_ms` (queue_length × mean admission
   slice, 0 when immediate). Requester-side safety is already in the
   binaries: explicit no-op arms exist (`remote.rs:553`, `remote.rs:1167`
   — "Accepted never appears on this wire path today"). Cost: one small
   frame (< 100 bytes) per request.

4. **Scope fences.** The concurrency limits themselves stay host-side
   constants (8 / 2); wiring registered `maxSlots` / heartbeat
   `freeSlots` into `Admission` is explicitly OUT of scope (future
   work). The queue engages only under real contention — a 2–3 peer
   swarm sees no behavior change; this ships as structure, not as a
   claimed performance win, and the first real contention data (either
   direction) gets published.

## Implementation map (three seats)

| Seat | Change |
|---|---|
| Tracker | `queue_weight` bucketing at lease issuance (`lib/eligibility.ts` + `grantedCounters`); msp-v1 §3.3/§5 lease-field row + §9 changelog row closing the ADR-028 deferral |
| Node serving (`serving.rs`) | bounded wait queue + DRR scheduler + 10 s deadline + `queue_deadline` reason + `accepted` emission |
| Test & Release | golden vector for the `accepted` frame; lease-with-`queue_weight` vector; guards vocabulary row for `queue_deadline`; contended-admission-order test (class 4 beats class 1 under saturation) |

## Honesty note

With `maxSessions = 8` and current swarm sizes the queue never engages.
Negative results (e.g., "queue added latency, never reordered anything"
in the first months) will be recorded as visibly as any speedup.

## Risks

- `accepted` on every stream changes the observable frame sequence for
  older requesters — tolerated (no-op arms), verified by a wire-compat
  golden test before release.
- The hub-signed class lags real contribution between lease refreshes;
  leases are short-lived, accepted.
- Wrong thresholds would make classes unreachable — mitigated by
  env-tunability without a code release, revisited with census data.
