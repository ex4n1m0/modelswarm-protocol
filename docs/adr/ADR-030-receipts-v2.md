# ADR-030: Receipts v2 — counter-signed work receipts and granted verified-work accounting

Status: Accepted (2026-10-09) · Owner-approved gate D3/§C.1. Implements the
trust substrate of M10 (master prompt §10 "Trust and fairness substrate";
scheduler handoff build items 7–9). Amends msp-v1 §7 additively: the v1
receipt shape is not edited in place — receipts v2 is a NEW signed object;
old verifiers are unaffected. Sources:
`docs/reviews/gap-engineering-study-2026-10-09.md` §1 ·
`docs/reviews/expanded-mission-roadmap-review-2026-10-08.md` §4.3 ·
`docs/reviews/handoff-protocol-architect-2026-10-09.md` §2b/§6 ·
`docs/reviews/handoff-scheduler-scientist-2026-10-09.md` §5 (items 7–9).

## Context

Today's receipts are specified (msp-v1 §7, `messages.proto` `JobReceipt`)
but unimplemented on the single-mode wire — the Rust wire deliberately omits
them ("receipts land with the gateway/session layer",
`modelswarm-transport/src/message.rs:140-151`); only the cooperative
`Receipt{session_id, rounds, committed_tokens}` exists
(`modelswarm-session/src/spec.rs:306-315`); the tracker stores
`{receiptDigest, outcome}` rows whose digest preimage is **undefined
anywhere** (architect finding); production writes no accounting rows at all
(`Store::record_job` has zero callers, scheduler handoff S7). Fairness
enforcement and the reputation chassis cannot start from claims — they need
granted, counter-signed evidence.

## Decision

### 1. Receipt v2 object (content-blind; canonical JSON per msp-v1 §2.2)

```text
ReceiptV2 {
  request_id, profile_id,
  preset, mode_id,                    // ADR-029: negotiated tier + resolved primitive mode
  cohort_digest,                      // "sha256:" + hex of SHA-256 over the canonical JSON array of the sorted "peer_id:role" strings
  peers: [ { peer_id, installation_id, role } ],
  started_at, ended_at, outcome,      // RFC 3339; outcome per msp-v1 §7 vocabulary
  accepted_tokens, verified_tokens,   // u64 counters, machine-derived
  endpoint_seconds,                   // u64: Σ over peers of wall-seconds the session engaged them
  usage_digest,
  budgets,                            // effective SessionOffer budget actually applied
  server_signature,                   // ed25519 by the serving/coordinating peer
  requester_signature                 // ed25519 by the requesting peer (phase 2)
}
```

Two-phase exchange (unchanged pattern from msp-v1 §7): the server's
`completed` event carries the receipt with `server_signature` = ed25519 over
the canonical JSON of the object **with both signature fields absent**; the
requester verifies, then adds `requester_signature` = ed25519 over the
canonical JSON **with `server_signature` present and `requester_signature`
absent** (skip-serializing form, same discipline as the lease wire,
ADR-012/026). A receipt without both signatures is never accounting
evidence. Timings, counts, and digests only — never prompt text (content
blindness holds; the hub sees only the digest + outcome).

### 2. Canonical receipt digest preimage (the undefined `receiptDigest`)

The digest posted to `POST /events/job-result` SHALL be:

```text
receiptDigest = "sha256:" + lowercase_hex(SHA-256(
    canonical JSON of the FINAL counter-signed ReceiptV2 (both signatures present)))
```

Frozen here. This closes the architect finding that `/events/job-result`'s
`receiptDigest` hashed an undefined preimage. The duplicate `/api/v1/receipt`
intake consolidates into `/events/job-result` under ADR-028 §1 when this
lands.

### 3. Granted verified-work accounting (never claimed)

- Counters are per `(peer_id, profile_id)` and non-transferable:
  `endpoint_seconds_granted`, `verified_accepted_tokens_granted`.
- Counters increment **only** from ReceiptV2 objects that (a) carry both
  signatures, and (b) pass intake validation — plus audit pass/fail events.
  Nothing self-reported increments anything (the BOINC lesson: the worker
  never prices its own work; same discipline as ADR-012's
  "measured, not self-reported" `verified_capacity`).
- **No payment, transfer, or pricing surface exists or may be added in
  v0.x** (AGENTS.md constraint 8; the Gridcoin lesson — monetizing the
  ledger re-imports every accounting attack). What accounting buys:
  fair-queueing weight class, a client display line, reputation inputs. It
  is not a medium of exchange; if host retention fails with accounting
  visible (the Petals question, gap-study experiment E-B), the answer is a
  product decision via a new ADR, never a silent currency.

### 4. Grant exclusions

- **Self-work contributes zero**: receipts where a serving installation
  equals the requesting installation.
- **Sybil-ring work contributes zero**: receipts whose peer set violates
  the installation + subnet/ASN diversity constraint (expanded-mission §7)
  contribute nothing for those peers.
- **New identities start at zero accounting** — no newcomer grace, no
  imported standing; a fresh identity must host the exact pinned artifact
  with an advertised slot to earn anything (costly identity; whitewashing
  is priced in because identity reset loses all accounting).

### 5. Time-windowed decay

Counters and the availability EWMA use **time-decay weighting (half-life
form)**, not the per-observation alpha of the existing `Ewma`
(scheduler handoff S9: add a second constructor; do not overload the frozen
one). Half-life initial value: 7 days — **initial value, tunable, R0.5+
measurement-informed**; the decay shape (not the constant) is frozen here.

### 6. Implementation and evidence requirements

- Rust wire: the receipt lands in the production serving/chat path (reuse
  the spec-layer sign/verify machinery); `Store::record_job` gains real
  callers; scheduler handoff build item 7's grant rules apply verbatim.
- **Golden vectors required**: `protocol/vectors/` gains a receipts-v2
  canonical-form vector (phase-1 signed bytes, phase-2 counter-signed
  bytes, `receiptDigest` value), consumed by both Rust and TS CI, before
  any second implementation reads the object (standing rule after four
  shipped wire bugs).
- Receipts v2 lands **before any approximate mode is default-on** and
  before fair queueing consumes the counters (M10 ordering).
- The minimal reputation chassis (gap study §3) consumes these counters via
  ADR-012's suspension machinery — which first requires migration-0004
  (audit F8/P3: slots-to-zero is currently impossible; policy fn has no
  feeder). Ordering edge recorded in
  `docs/implementation/dependency-graph.md`.

## Consequences

+ The tracker's stored digests become verifiable evidence with a defined
  preimage; accounting, fair queueing, and reputation share one granted
  evidence base.
+ Self-reported inflation paths are structurally closed (counters move only
  on counter-signed work and audits).
− Production accounting starts empty (S7) — the first real entries land
  with the M10 receipts-v2 implementation, not with this ADR.
− The 7-day half-life and the diversity thresholds will need one measured
  retune when real traffic exists (by design).
