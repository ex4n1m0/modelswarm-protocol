# Handoff — VerifyDrafts session 4: coordinator side (RC-3 admission, RC-2 timing trust, RC-1 audit path) + the authorized deadline-past fixture repair

Agent: implementation session under the Protocol Architect contract
(ADR-032 revision 2; plan `docs/implementation/adr-032-verifydrafts-plan.md`
§4 EXACTLY, plus the one fixture repair the task authorized — see §5).
Base: `f9d0806` (session 3: verifier gate + property suites).
Date: 2026-10-11.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-session/src/verifydrafts.rs` (modified) | Plan §4, all in this module per the plan. **§4.1**: `VerifyDraftsCoordinator` + `VerifyDraftsCoordinatorConfig` + `OutstandingVerifyRequest` — RC-3 (a)–(e) `admit_result(frame, remote_peer_id)` EXACTLY: (a) signature under the session's pinned verifier key + sender derivation FIRST (`verify_verify_drafts_result_signature`, the `verify_commit_signature` discipline) + the QUIC-authenticated remote-PeerId equality via the serve-loop-supplied `remote_peer_id` parameter (checked as `peer_id_for(pinned_key)` — ADR-020: the PeerId is the identity-multihash derivation, a different string from the installation id; both derive from the SAME pinned key); (b) `nonce_echo` equality; (c) session/protocol/round/seq binding (profile/parent transitively, §4 item 3 below); (d) `accepted_prefix_len ≤ len(window_tokens)` + divergence shape consistency in BOTH directions (null iff full acceptance, `== accepted_prefix_len` iff partial); (e) `new_prefix_hash` recomputed with the shipped `compute_prefix_hash` over `(parent ‖ accepted ‖ [correction])`. First valid result consumes the nonce; later/consumed/no-outstanding results DROPPED with no state change (`ResultDrop::{ConsumedNonce, NoOutstandingRequest, SessionTornDown}`); every check failure is a `ResultViolation` counted per remote peer, and the SECOND by the same peer tears the session down (`TEARDOWN_REASON_RESULT_VIOLATION`, threshold `TEARDOWN_VIOLATION_THRESHOLD == 2`). **§4.2**: `VerifierTiming` (per-verifier EWMA, alpha default `EWMA_ALPHA_DEFAULT = 0.25`, configurable via `with_alpha((0.0, 1.0])`; fed ONLY by requester-measured `record_round`; `cross_check(verify_pass_ms, observed_wall) -> CrossCheckOutcome{Ok, Lie}` with the inclusive `VERIFY_PASS_CLOCK_NOISE_MS = 25` bound; lie counter exposed for the de-rank policy) + `VerifierTimings` (the peer-keyed table; structural isolation — no method blends across peers). **§4.3**: `VerifyTelemetry` (`verify_ms_lie`/`verifier_divergence`/`budget_refused`), `WindowAuditor` + `AuditVerdict` (with the REQUIRED cache-cold implementation property documented on the trait, citing the accuracy-benefits study's live llama.cpp greedy-flip-on-repeat finding), `AuditSampler` (first window + one after every acceptance/divergence transition), `VerifyDraftsCoordinator::audit_committed_window` (on Divergence: telemetry event + switch to SingleDecode for the remainder + `ReceiptOutcome::AuditDivergence`; idempotent in fallback; auditor errors propagate unmarked), and `RECEIPT_COUNTER_HONESTY_LABEL = "verifier-claimed"` (the RC-1 receipt-intake honest-label surface; NO new wire fields). **SessionAccept honesty**: `validate_session_accept` (echoes, `batch_window_max` in `1..=WINDOW_CAP` when `batch_verify: true`, budget clamped DOWN only), `eligible_batch_verifier` (backend pairing + batch_verify + valid disclosure), `peer_reject_reason` (backend mismatch → `SessionRejectReason::BackendMismatch`). 4 new module unit tests. |
| `crates/modelswarm-session/tests/verifydrafts_coordinator_properties.rs` (new) | Plan §4.4 — the coordinator property suite, 15 tests: I2 hostile fake verifier (false-but-hash-consistent result ADMITS through RC-3, caught by the `WindowAuditor` mock → `verifier_divergence` via mock `VerifyTelemetry`, SingleDecode fallback, receipt marked, idempotent second audit, Match control case); RC-3 (a)–(e) negatives each with no-state-change assertions; consumed-nonce replays + no-outstanding + abandoned-round late results dropped; second-violation teardown at exactly two (incl. one-violation-per-peer does NOT tear down); `SessionAccept` honesty (out-of-range `batch_window_max` schema-invalid, budget inflation, echo mismatches; roster selection picks exactly the capable CPU batch verifier; `batch_verify: false` and Vulkan peers never selected); backend mismatch → serialized `SessionReject` reason `backend_mismatch`; `verify_pass_ms` cross-check at the exact boundary (125 Ok / 126 Lie at observed 100) firing `verify_ms_lie` through the mock; per-verifier EWMA isolation (liar flagged 5×, honest peer 0 lies, EWMA 225 from its own rounds only); plus a proptest: random windows × accepted × correction — honest shape always admits with the recomputable committed ids, tampered-and-resigned hashes always violate (e). |
| `crates/modelswarm-session/examples/gen-verifydrafts-vectors.rs` (modified) | The AUTHORIZED fixture repair at the SOURCE: the `deadline-past` malformed case's `context_extra` now overrides the shared template with a FRESH budget ledger (`committed_output_tokens: 0` + a recorded `budget_ledger` note + extended description). Rationale in §5. Nothing else in the generator changed. |
| `protocol/vectors/verifydrafts-malformed.json` (regenerated) | Output of `cargo run -p modelswarm-session --example gen-verifydrafts-vectors` (full regeneration; idempotency double-run verified). `git diff` shows ONLY the deadline-past case: `committed_output_tokens: 16 → 0`, the new `budget_ledger` note, and its extended `description` — no wire bytes, signatures, canonical pins, or other cases/files touched. |
| `crates/modelswarm-session/tests/verifydrafts_vectors.rs` (modified) | Workaround removal (session-3 handoff §5.1 follow-up): `replay_malformed_case` now seeds `committed_output_tokens` LITERALLY from each case's recorded context for ALL cases, and the entire `if name == "deadline-past"` literal-seeding/`overloaded`-assertion workaround block is deleted. The deadline-past classification assertions additionally pin the fresh ledger (`committed_output_tokens == 0` + `budget_ledger` present). The fixture now replays DIRECTLY to `deadline_exceeded` (green). |
| `docs/reviews/handoff-verifydrafts-session4-2026-10-11.md` | This handoff. |

Scope check: only `crates/modelswarm-session/**`, `protocol/vectors/**`
(regeneration via the generator only), and `docs/reviews/`. Nothing else.
No dependency changes.

## 2. Exact commands and outcomes

Run from the repo root, Windows/Git Bash, workspace default features:

```
$ cargo run -p modelswarm-session --example gen-verifydrafts-vectors
                                    # wrote 10 fixture files
$ cargo run -q -p modelswarm-session --example gen-verifydrafts-vectors   # double run
                                    # idempotent: git diff unchanged after rerun
$ git diff protocol/vectors/        # ONLY verifydrafts-malformed.json,
                                    # 3 insertions / 2 deletions, deadline-past
                                    # case only (context + description)
$ cargo fmt --all --check           # clean (after one cargo fmt --all pass)
$ cargo clippy --workspace --all-targets -- -D warnings
                                    # clean, exit 0, 0 errors/warnings
$ cargo test --workspace            # 447 passed, 0 failed, 6 ignored
                                    # (57 result lines, all "0 failed")
$ cargo test -p modelswarm-session --test verifydrafts_coordinator_properties   # 15 passed
$ cargo test -p modelswarm-session --test verifydrafts_vectors                 # 11 passed
$ cargo test -p modelswarm-session --lib verifydrafts                          # 21 passed
```

Baseline after session 3 was 428 passed / 6 ignored → **+19** = 15
coordinator property tests + 4 module unit tests (EWMA math/alpha bounds,
the inclusive +25 ms cross-check boundary, the audit sampler
first-and-transitions policy, the receipt-honesty label pin). No
pre-existing test was weakened; the only pre-existing-test edit is the
golden-test workaround REMOVAL (§5) — the 11 golden tests stayed green
with the literal context seeding.

## 3. Test evidence (gates)

| Gate | Command | Result |
|---|---|---|
| fmt | `cargo fmt --all --check` | clean |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| tests | `cargo test --workspace` | **447 passed, 0 failed**, 6 ignored (pre-existing owner-gated suites) |
| scoped coordinator | `cargo test -p modelswarm-session --test verifydrafts_coordinator_properties` | 15 passed, 0 failed |
| scoped golden | `cargo test -p modelswarm-session --test verifydrafts_vectors` | 11 passed, 0 failed |
| scoped unit | `cargo test -p modelswarm-session --lib verifydrafts` | 21 passed, 0 failed |
| fixture idempotency | generator double-run + `git diff protocol/vectors/` | byte-stable; only the intended deadline-past change |

**RC-3 property verdicts** (suite evidence): (a) wrong-key signature,
foreign sender label, and wrong QUIC remote peer each violate with no
state change; an honest result on the right stream admits. (b) wrong and
fabricated nonce echoes violate; consumed nonces drop harmlessly. (c)
wrong session/protocol/round and non-monotone verifier seq violate;
profile rides the pinned key + request tracking; parent rides (e) — a
hash computed under any other parent cannot recompute. (d) over-long
accepted length and all three divergence-shape inconsistencies violate.
(e) tampered-and-resigned hashes, wrong-parent hashes, and
length-swapped hashes violate; random windows × shapes admit exactly
when the hash recomputes (proptest). First valid result consumes the
nonce; replays of genuine results, late results for abandoned rounds,
and results for replaced rounds all DROP with no state change and zero
violations; the second violation by the SAME peer tears down at exactly
two, with one-violation-per-distinct-peer correctly NOT tearing down.

**Audit-path (RC-1/I2) verdicts**: the hostile fake verifier's
false-but-hash-consistent result ADMITS through RC-3 (pinned in the test
with a comment — that is why the audit path exists); the first-window
sampling policy fires; `AuditVerdict::Divergence` produces the
`verifier_divergence` telemetry event against the verifier's
installation id, switches the session to SingleDecode, marks the receipt
`AuditDivergence`, and the already-fallback session short-circuits
(zero further re-executions); `Match` leaves the session live with a
clean receipt and no telemetry. `verify_pass_ms` cross-check: the +25 ms
bound is inclusive (125 Ok at observed 100, 126 Lie), the lie fires
`verify_ms_lie` through the mock telemetry with correct session/peer,
under-reporting stays Ok (ADR: undetectable from timing alone), and the
per-verifier table never lets the liar's five lies touch the honest
peer's lie count or EWMA.

## 4. Assumptions made (unpinned mappings, recorded)

1. **RC-3 (c) `seq` — ADR-internal reconciliation.** §2 RC-3(c) names
   `seq` EQUALITY with the outstanding request, but §1/RC-7 pin the
   result's `seq` as the VERIFIER's own per-(session, sender) counter —
   the ADR's own worked example (request `seq: 12`, result `seq: 4`,
   valid) makes literal equality unsatisfiable. Implemented: the RC-7
   mirror of the verifier's stage-6 rule — the result's `seq` must be
   strictly greater than the last `seq` this coordinator saw from the
   verifier (first result establishes the baseline; its counter start is
   not disclosed on the wire); `seq <= last` is a `Binding("seq")`
   violation. Documented on `admit_result`.
2. **RC-3 (c) `profile_id`/`parent_prefix_hash` are NOT on the result
   wire** (ADR §1 result shape carries neither). Their equality is
   enforced TRANSITIVELY: parent through (e)'s recomputation under the
   outstanding request's parent; profile through the session-pinned
   verifier key of (a) plus `track_request`'s config binding. Test
   coverage: wrong-parent hash violates (e); wrong-profile request
   tracking is refused; wrong-key results violate (a).
3. **Coordinator-side violation policy.** The ADR's drop rule explicitly
   covers consumed-nonce and no-outstanding results; every OTHER RC-3
   failure (including schema-unparseable frames, attributed to the
   authenticated remote peer) counts toward the threshold-two teardown.
   Schema failures are violations, not drops: unsolicited garbage on the
   stream is a protocol violation even without an outstanding request.
4. **Nonce consumption on replace/abandon.** The ADR pins "the first
   valid result consumes the request's nonce"; `track_request` (replace)
   and `abandon_outstanding_request` also consume the replaced/abandoned
   nonce so a LATE GENUINE result for a dead round is a harmless drop
   rather than a violation. Consistent with the
   "undetectable-to-harmless" rationale.
5. **Teardown reason string** `verify_result_violation` (an honest
   label; the ADR pins the threshold, not the string).
6. **`peer_reject_reason` covers only `backend_mismatch`** — the one
   firing condition the ADR pins for the peer side. A
   batch-verify-incapable peer does NOT reject: per ADR §4 it accepts
   declaring `batch_verify: false` (tested); the other three frozen
   reasons fire on peer-policy judgments left to the serve loop.
7. **Auditor errors propagate unmarked** (a failed audit is not evidence
   of divergence; the ADR pins no behavior — the serve loop decides the
   honest stream outcome).
8. **De-rank policy**: `cross_check` exposes a per-verifier `lie_count`;
   the actual scheduler-facing discount is plan flag F-4 (pin before
   light-up).

## 5. The authorized fixture repair (recorded ruling)

Session-3 handoff §5.1 found `verifydrafts-malformed.json`'s
`deadline-past` case over-constrained: the shared context template
records an at-ceiling budget (`committed_output_tokens: 16 ==
budget.max_output_tokens: 16` — the `budget-exhausted` case's injected
condition), so the ADR's OWN first-failure-wins stage order (§2 table:
budget row precedes deadline row; plan §3.3 stages 11 < 12) classifies a
literally-seeded `deadline-past` as `overloaded`, contradicting the
recorded `deadline_exceeded`.

Ruling (escalated to the protocol owner and authorized by the Integrator
per the plan's ADR-wins rule): **the ADR's stage order wins; the fixture
was over-constrained.** Repaired at the SOURCE (generator, not the
JSON): the `deadline-past` case's context now carries a FRESH budget
ledger (`committed_output_tokens: 0` — coherent with the shared
pre-state under RC-5(c) gap semantics: round 1 was admitted but
abandoned before any commit, so the nonce/`last_handled_round` seeds
stand while zero tokens are committed), plus a recorded `budget_ledger`
note and an extended description. All 10 fixtures were regenerated, the
idempotency double-run left `git diff` empty, and the diff shows ONLY
the intended change (3 insertions / 2 deletions in
`protocol/vectors/verifydrafts-malformed.json`, deadline-past case
only). The session-3 golden-test workaround (fresh-ledger compensating
seeding + the literal-seeding `overloaded` assertion) was REMOVED;
`replay_malformed_case` now seeds every context LITERALLY and the case
replays directly to `deadline_exceeded`. Nothing else in
`protocol/vectors/` was touched.

## 6. Contract conflicts

No plan-vs-ADR conflict required the ADR to override the plan in §4's
text itself; the two ADR-INTERNAL tensions found are recorded as §4
items 1–2 (the `seq` equality reconciliation and the absent
profile/parent result fields) — both resolved in the direction that
keeps the ADR's own §1 result shape and RC-7 semantics normative.

## 7. Unresolved risks

1. The serve-loop integration (real PeerId supply, timing measurement,
   audit scheduling against the real engine, receipt intake labeling)
   remains intentionally absent — this session is the pure coordinator
   per plan §4; wiring belongs to the Windows Product / Runtime seats
   (and the real-engine path to the ADR-031 productionization gate).
2. The consumed-nonce set is unbounded within a session (same
   observation as session 3's verifier-side set; lease/session lifetime
   bound exposure).
3. `verifier_last_seq` accepts any first value; a hostile verifier could
   start at `u64::MAX` and then be unable to send a second valid result
   (fail-closed against the verifier, not exploitable).
4. De-rank semantics (F-4) need a pinned scheduler-facing value before
   light-up; the session crate only exposes `lie_count`.
5. Cross-machine CPU audit soundness remains E0's open sub-question
   (F-8); within-backend-family audit + the cache-cold rule are what the
   contract requires today.

## 8. Suggested next task for the integrator

Plan §5.2 rows 5–6 are the remaining family work: the T/R-owned
mixed-swarm fallback scenarios (sim, gated on 4a + the §1 negotiation
fields — both now landed) and the Runtime-owned engine-side capi wiring
behind the ADR-031 gate items 1–5. The wire-contract tier's acceptance
checklist (plan §5.3 items 1–5, 7) is now fully green through sessions
1–4; item 6 (scheduler 4a sign-off recording) remains with the
Scheduler/Protocol seats. Security + Test and Release review of this
handoff is the immediate gate per the ownership map.
