# Post-implementation security review — VerifyDrafts family (ADR-032 checklist item 8)

Date: 2026-10-11. Reviewer: Security Engineer (independent blocking seat).
Scope: implementation sessions 1–5 of
`docs/implementation/adr-032-verifydrafts-plan.md` — commits `fddb972`
(types/serialization), `4f277cc` (fixtures + golden test), `f9d0806`
(verifier gate + admission/divergence property suites), `6730974`
(coordinator admission/EWMA/audit + deadline-past fixture repair),
`f09a50c` (mixed-swarm sim scenarios, session 5).
Binding contract: `docs/adr/ADR-032-batched-speculative-verification.md`
revision 3 (the ADR TEXT was already reviewed and accepted; this review
passes judgment on the IMPLEMENTATION against it).

## Verdict

**APPROVE WITH REQUIRED CHANGES** for the implementation (distinct from
the ADR-text verdict, which stands accepted).

Blocker: **0**. Required changes: **2** (both pre-light-up serve-loop /
gate-API contract items — the landed sessions' code is green and the
wire-contract tier's byte rules, admission order, replay defenses,
privacy shape, fixture integrity, DoS caps, and audit/telemetry surfaces
all verify as implemented). Advisories: **7** (behavior-determining
nuances; two are now frozen by pinned tests in
`crates/modelswarm-session/tests/verifydrafts_security_review_pins.rs`
so any amendment is deliberate).

## Verification against the contract (the 8 minimum-set points)

### 1. Signature surface — VERIFIED, with one missing hook (RC-1)

- Signature-omitted canonical preimage: `signing_preimage`
  (`crates/modelswarm-session/src/verifydrafts.rs:420-429`) serializes
  the message, removes the `signature` key ENTIRELY (not null, not empty
  string), then canonicalizes via `modelswarm_types::canonical_json`.
  Unit test `signing_preimage_omits_signature_entirely`
  (`verifydrafts.rs:2779-2810`) asserts key absence and
  present-as-null survival; the golden test recomputes it from `fields`.
- Sender binding FIRST: `verify_family_signature`
  (`verifydrafts.rs:470-493`) checks
  `installation_id_for(expected_key) == sender` before any Ed25519
  verdict; pinned by `sender_binding_is_checked_first`
  (`verifydrafts.rs:2754-2777`).
- base64 STANDARD: `verifydrafts.rs:59`, decoded with the strict
  64-byte `Signature::from_slice` path (`verifydrafts.rs:486-489`).
- **RC-1 (required change): the QUIC-PeerId equality hook exists on the
  coordinator side but is MISSING on the verifier side.**
  `VerifyDraftsCoordinator::admit_result(frame, remote_peer_id)`
  (`verifydrafts.rs:1826`) checks the serve-loop-supplied PeerId against
  `peer_id_for(verifier_key)` (`verifydrafts.rs:1841-1843`,
  `ResultViolation::RemotePeer`) — correct. The verifier-side gate
  `VerifyDraftsGate::admit_verify_drafts(frame, arrival_ms, now_ms)`
  (`verifydrafts.rs:1021-1026`) takes NO peer-id parameter; stage 3
  (`verifydrafts.rs:1064-1068`) verifies the signature under the pinned
  coordinator key only. ADR-032 §1 rule 5 requires `sender` to equal the
  installation id AND the QUIC-authenticated remote PeerId. Because the
  gate API cannot accept the PeerId, no serve loop can supply it — the
  contract is unenforceable at the crate's own admission point, and the
  plan's §1.5/§3.5 staging claimed this discipline for these sessions
  (session-1 handoff risk list; sessions 3–4 did not deliver it). Fix:
  add a `remote_peer_id: &str` parameter (or an equivalent trait/hook)
  checked against `peer_id_for(coordinator_key)`, rejecting with
  `bad_frame` at stage 3 — the mirror of `ResultViolation::RemotePeer`.
  Routed to the owning session/architect, not edited here (reviewer does
  not change implementation behavior).

### 2. Admission order — VERIFIED (first-failure-wins, exact partition)

`VerifyDraftsGate::run_admission_stages` (`verifydrafts.rs:1041-1159`)
implements the §2 partition exactly, in order: (1) frame size →
`payload_too_large` (1048-1051), (2) strict schema + wire forms →
`bad_frame` (1052-1063; empty window fails at parse,
`verifydrafts.rs:306-315`), (3) signature → `bad_frame` (1066-1068),
(4) session binding → `bad_frame` (1069-1075), (5) torn-down →
`cancelled_by_peer`, then lease → `expired_token` (1076-1082), (6)
seq/nonce → `replayed_request` (1083-1092), (7) profile →
`profile_mismatch` + teardown (1093-1098), (8) round →
`CancelRound(round_mismatch)`, parent → `CancelRound(parent_mismatch)`
(1099-1105), (9) one-outstanding → `replayed_request` (repeat nonce) /
`bad_frame` (fresh) (1106-1116), (10) window cap →
`payload_too_large` before any engine work (1117-1122), (11) budget →
`overloaded` (1123-1130), (12) deadline clamp-then-check →
`deadline_exceeded` (1132-1140). No stage can be skipped by a crafted
payload: every stage is reachable only through the previous ones, and
the conjunction sweep
(`tests/verifydrafts_admission_properties.rs:511-541` — all 120
exhaustive pairs + random triples) asserts the earliest code; the
budget-before-deadline precedence (`overloaded` wins) is pinned. All 12
committed malformed fixtures replay through the real gate to their exact
recorded codes (`tests/verifydrafts_vectors.rs:682-988`). Every stage's
failure string matches the §2 table; `FrameRejectReason`/`CancelRoundReason`
are asserted 1:1 onto the effective §6.5 registry with no new codes
(`crates/modelswarm-session/src/negotiation.rs:308-348, 271-301,
365-382`).

### 3. Replay/reorder protections — VERIFIED

- First-valid-consumes on both sides: verifier consumes at admission
  (`verifydrafts.rs:1144-1145`); coordinator consumes on the first valid
  result, and replaces/abandons also consume (`verifydrafts.rs:1897-1899,
  1784-1786, 1801-1809`) so late genuine results drop harmlessly
  (`ResultDrop::{ConsumedNonce, NoOutstandingRequest}`).
- Per-sender strict-monotone `seq` on both sides (`verifydrafts.rs:1085-1089`
  verifier; `verifydrafts.rs:1865-1867` the RC-7 mirror on results —
  strictly greater, first result establishes the baseline; the
  ADR-internal equality-vs-monotone reconciliation is correctly
  resolved in the ADR's own worked-example direction, per revision 3).
- One-outstanding-verify, never queued (stage 9; property
  `one_outstanding_never_queued`, admission suite 764-840).
- Threshold-two teardown is PER PEER on both sides (`violations`
  keyed by sender/remote peer id, `verifydrafts.rs:837-838, 1729`;
  coordinator test `rc3_second_violation_by_same_peer_tears_down_exactly_at_two`,
  coordinator suite 653-721, incl. one-violation-per-distinct-peer does
  NOT tear down).

### 4. Privacy — VERIFIED (codes/counts/hashes only)

`RejectionFrame {session_id, code, round, prefix_hash}` and
`CancelRoundFrame {session_id, round, reason}`
(`verifydrafts.rs:686-713`) carry codes, counts, and hashes only — no
token ids by construction. `reject_frames_never_carry_token_ids`
(admission suite 1256-1304) pins the exact key sets and the absence of
token ids; the golden malformed replay adds a serialized-leak assertion
(`tests/verifydrafts_vectors.rs:907-911`). Fixtures leak nothing beyond
the F-7 fixture-only keys: token ids in fixtures are synthetic constants
(101…108, 201…208, 300…331, 500…532, 601–603, 700–708, 777/999/4242),
no prompt text exists in any fixture, and the seeds `[0x11;32]` /
`[0x22;32]` appear ONLY in the generator and `#[cfg(test)]` helpers
(verified by grep over `crates/` and `apps/`).

### 5. Fixture integrity — VERIFIED

Seeds are fixture-only and cannot collide with production identities
(F-7; no production path references them). The golden test RECOMPUTES,
not string-compares: canonical bytes from the parsed struct, sha256,
Ed25519 verification under the recorded keys, sender derivation, chain
hashes from recorded construction inputs, and byte-for-byte signature
reproduction by re-signing the gate's emitted draft
(`tests/verifydrafts_vectors.rs:161-224, 309-428`). Generator
idempotency re-verified this review: double run leaves
`git diff protocol/vectors/` empty. The session-4 deadline-past repair
is confined to that case (fresh budget ledger, `committed_output_tokens: 0`
+ `budget_ledger` note; confirmed by direct inspection of
`protocol/vectors/verifydrafts-malformed.json`).

### 6. DoS surfaces — VERIFIED

- Window cap + `batch_window_max` enforced at stage 10 BEFORE any engine
  work, with zero-`verify_window`-call assertions (admission suite
  714-758; golden len-33 replay, `tests/verifydrafts_vectors.rs:604-633`;
  boundary proptest `cap_boundary_admits_iff_len_fits_the_effective_cap`).
- WAIT_COMMIT bounded by `deadline_ms + 250 ms` grace (`verifydrafts.rs:97,
  1341-1353`; property 876-908); the extension to the undispatched
  `Verify` state is documented and consistent with the stage-13
  re-check.
- Budget ledger (I8) actually refuses: token and seconds ceilings →
  `overloaded` (`verifydrafts.rs:1123-1130`; property
  `budget_exhaustion_refuses_with_overloaded`, admission suite 1043-1107).
- Deadline RE-CHECKED immediately before engine work at dispatch
  (`verifydrafts.rs:1185-1189`; property
  `deadline_rechecked_at_dispatch_after_queue_wait`, admission suite
  1192-1250).

### 7. Audit/telemetry traits — VERIFIED

Cache-cold is documented as a REQUIRED property on `WindowAuditor`
(`verifydrafts.rs:2241-2254`) citing the measured llama.cpp greedy-flip
finding; the sampling policy (first window + one after every
acceptance/divergence transition) is implemented in `AuditSampler`
(`verifydrafts.rs:2294-2325`). No engine access leaks into the session
crate: engine work is the `BatchVerifyEngine` trait only
(`verifydrafts.rs:543-555`), mockable per-push; `modelswarm-telemetry`
is a DEV-dependency of `modelswarm-session` (Cargo.toml) and the
production surface is the `VerifyTelemetry` trait
(`verifydrafts.rs:2206-2218`). No capi/llama dependency exists in the
crate.

### 8. Session-flagged risks — assessed

See advisories A1–A7 below. The security-relevant ones are: the missing
verifier-side PeerId hook (RC-1), the unpinned verifier-side
commit-mismatch violation policy (A1), the unbounded consumed-nonce
set (A3), and the serve-loop wiring obligations (RC-2).

## Findings

### Blocker

None.

### Required changes

**RC-1 — verifier-side QUIC PeerId equality hook missing.**
`crates/modelswarm-session/src/verifydrafts.rs:1021-1026`
(`admit_verify_drafts` has no peer-id parameter; stage 3 at
`verifydrafts.rs:1064-1068` checks signature only). ADR-032 §1 rule 5
cannot be enforced at the gate; the coordinator side
(`verifydrafts.rs:1826, 1841-1843`) is the correct pattern to mirror.
Proposed: add `remote_peer_id: &str` to `admit_verify_drafts`, reject
`bad_frame` when it differs from `peer_id_for(coordinator_key)`. Owner:
VerifyDrafts session / Protocol Architect. (No serve loop exists yet,
so nothing is exploitable today — but the API must exist before any
serve loop can be correct.)

**RC-2 — serve-loop enforcement obligations must be pinned as a
light-up gate.** Three I8/§8 mechanisms exist in the crate but are
enforced only by the serve loop, which does not exist yet:
(a) the seconds ledger advances only via `advance_session_seconds`
(`verifydrafts.rs:925-927`) — without real served-time deltas, a
hostile coordinator can burn engine passes on admit-and-abandon rounds
bounded only by the lease, since the token ledger counts committed
tokens only (`verifydrafts.rs:1125-1128`); (b) WAIT_COMMIT expiry is
polled (`expire_wait_commit_if_due`, `verifydrafts.rs:1341-1353`; any
new frame also triggers it via `settle`) — a silent stream holds a
round forever if never polled; (c) emitted drafts are unsigned
(`verifydrafts.rs:1227-1229`) and must be signed with the verifier key
before sending. Route: the light-up serve-loop ticket must name (a)–
(c) plus RC-1 as its admission checklist, mirroring the ADR's own
light-up gating.

### Advisories

**A1 — verifier-side commit-mismatch violation counting is unpinned ADR
policy and can tear down an honest-but-slow coordinator.**
`apply_prefix_commit` (`verifydrafts.rs:1290-1316`) counts ANY
non-reproducing commit — including a LATELY-arriving commit that
exactly reproduces the result after WAIT_COMMIT expiry discarded it
(`expire_wait_commit_if_due`, `verifydrafts.rs:1341-1353`) — as a
violation; two of them tear the session down. The ADR pins
threshold-two for coordinator-side RESULTS only; the verifier-side
counting policy is the implementation's own (session-3 handoff §5.7).
Suggest treating post-expiry reproductions as no-op drops (the mirror
of the coordinator's consumed-nonce drop) or excluding them from the
counter. Pinned by the new test
`pin_verifier_late_commit_after_expiry_is_commit_mismatch`.

**A2 — coordinator drop-vs-violation partition is signature-first
(behavior-determining; consistent with session-4 handoff assumption 3,
worth a ruling).** In `admit_result` (`verifydrafts.rs:1826-1867`) the
consumed-nonce and no-outstanding drops run AFTER the signature and
remote-peer checks: a genuine replay drops with no state change
(RC-3 tail satisfied), while a frame failing signature/remote-peer
counts a violation even when it carries a consumed nonce or no round is
outstanding. This matches the ADR's "replayed GENUINE signed result"
parenthetical; a strictly literal reading of "any later result carrying
a consumed nonce … dropped" would order the drop first. Pinned by
`pin_coordinator_replay_partition_genuine_drop_garbage_violation`.

**A3 — unbounded per-session sets.** `consumed_nonces`, `last_seq`,
`violations` (`verifydrafts.rs:835-838`; coordinator `1725-1729`) grow
without bound within a session; a hostile coordinator can grow the
nonce set by admit-and-abandon loops. Bounded in practice by
lease/session lifetime; a bounded set with eviction is a sensible
future amendment (both sessions flagged it).

**A4 — stage-1 frame-size check measures the re-serialized Value, not
the raw frame.** `verifydrafts.rs:1048-1051` measures compact
`serde_json::to_string(frame)`; the raw 256 KiB limit is enforced by
`modelswarm-transport::frame::MAX_FRAME_BYTES` (`crates/modelswarm-transport/src/frame.rs:24`)
before the gate. The serve loop must map raw-oversize rejections to
`payload_too_large` for the §2 row to hold end-to-end.

**A5 — unguarded addition in WAIT_COMMIT expiry.**
`verifydrafts.rs:1348` computes
`out.deadline_expires_at_ms + WAIT_COMMIT_GRACE_MS` with plain `+`.
Unreachable with real serve-loop clocks (arrival uses
`saturating_add`, `verifydrafts.rs:1137`), but a fuzz harness injecting
extreme clocks could panic debug builds; `saturating_add` is a one-word
hardening for the owner.

**A6 — de-rank policy (plan flag F-4) remains unpinned.**
`lie_count` is exposed (`verifydrafts.rs:2124-2126`) but the
scheduler-facing de-rank semantics need a pinned value before light-up
(session-4 handoff risk 4).

**A7 — `GatePreStateSeed` is a public production-surface type for
test/fixture replay** (`verifydrafts.rs:780-797`). A production caller
could in principle seed admission defenses; it is used only by tests
today. Suggest doc(hidden) + a strong misuse warning (integration
tests require it public unless a test-only feature is added).

## Tests added by this review (pins only — no behavior change)

`crates/modelswarm-session/tests/verifydrafts_security_review_pins.rs`
(2 tests, clearly labeled):
- `pin_coordinator_replay_partition_genuine_drop_garbage_violation`
  (freezes A2's partition, incl. the exactly-two teardown).
- `pin_verifier_late_commit_after_expiry_is_commit_mismatch`
  (freezes A1's current classification and teardown, for the
  architect's ruling).

## Gates (run from a clean state, workspace default features)

| Gate | Command | Result |
|---|---|---|
| fmt | `cargo fmt --all --check` | clean, exit 0 |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| tests | `cargo test --workspace` | **458 passed / 0 failed / 6 ignored** (456 session-5 baseline + 2 review pins) |
| scoped | `cargo test -p modelswarm-session --test verifydrafts_security_review_pins` | 2 passed |
| fixture idempotency | generator run + `git diff protocol/vectors/` | empty diff (10 files byte-stable) |

## Positive conclusions (implementation-vs-contract)

The byte rules (§1 rules 1–5), the strict two-pass present-as-null
schema, the integral-only params-hash rendering (F-1), the EOG
correction discipline (F-2), the extended negotiation shapes (F-6),
the RC-3 (a)–(e) coordinator admission with transitive profile/parent
binding, the RC-7 result-seq mirror, RC-4 commit reproduction, RC-5
state-machine pinning (one-outstanding, WAIT_COMMIT bound, gap-tolerant
rounds), I8 budget enforcement, the RC-2 per-verifier EWMA with the
+25 ms inclusive cross-check and structural peer isolation, the RC-1
audit path with the REQUIRED cache-cold property, the verifier-claimed
receipt-honesty label, and the class-blind fastest-single sim scenarios
(8 pinned cases, decision-level, default features) are all implemented
exactly as contracted and covered by per-push tests. The two required
changes are integration-contract items that are already gated behind
light-up; nothing in the landed wire-contract tier is bypassable by a
crafted payload today.

## Changed files (this review)

- `crates/modelswarm-session/tests/verifydrafts_security_review_pins.rs`
  (NEW — 2 pin tests, co-owned tests/ per review mandate).
- `docs/reviews/review-verifydrafts-implementation-security-2026-10-11.md`
  (this document).

## Unresolved risks

RC-1 and RC-2 remain open and route to the owning session/architect and
the serve-loop light-up ticket. A1 needs the architect's ruling on the
verifier-side violation policy. A3/A5/A6/A7 are candidate follow-up
amendments.

## Suggested next task for the integrator

Merge this review; route RC-1 (gate API) to the VerifyDrafts owner,
record RC-2 in the light-up checklist, and schedule the A1 ruling; the
ADR-032 §7 checklist item 8 then closes with the Security verdict
APPROVE WITH REQUIRED CHANGES (required changes pre-light-up, non-
blocking for the landed wire-contract tier).
