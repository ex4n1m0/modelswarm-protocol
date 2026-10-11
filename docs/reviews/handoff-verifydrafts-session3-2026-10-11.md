# Handoff — VerifyDrafts session 3: verifier-side admission gate + state machine + the two property suites

Agent: implementation session under the Protocol Architect contract
(ADR-032 revision 2; plan `docs/implementation/adr-032-verifydrafts-plan.md`
§3, step 3 — EXACTLY §3, nothing from §4).
Base: `4f277cc` (session 2: fixtures + golden test, landed; session 1
`fddb972`: types/serialization).
Date: 2026-10-11.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-session/src/verifydrafts.rs` (modified) | Plan §3.1–§3.5, all in this module per the plan: the `BatchVerifyEngine` trait (the plan §3.1 sketch EXACTLY — `verify_window -> Result<(u32, Option<u32>), RuntimeError>` + `capabilities()`; mockable, no capi dependency); `GateState {Prefilled, Validating, Verify, WaitCommit, Committed}`; `VerifyDraftsGate` + `VerifyDraftsGateConfig` carrying the full §3.2 state (session binding, committed hash + `last_handled_round` — they diverge under gaps, the ONE outstanding request incl. window/nonce/clamped-deadline/result, session-scoped consumed-nonce set, per-sender `last_seq`, budget ledger, per-peer violation counters, lease expiry, the verifier's own result-`seq` counter); `admit_verify_drafts` executing plan §3.3 stages 1–12 EXACTLY in order (first-failure-wins; seq stage before all other semantic state; deadline CLAMP not reject; one-outstanding never queued; window cap before engine work; budget per I8); `dispatch_outstanding` (stage-13 deadline re-check after queue wait, then ONE engine pass; unsigned result draft for the serve loop to sign); `apply_prefix_commit` (RC-4 WAIT_COMMIT admission + the gate's OWN chain application — NOT `SessionStateMachine::apply`; round > last committed with gaps allowed, parent-hash equality, `compute_prefix_hash` reused, bounded `DUPLICATE_MEMORY_ROUNDS` duplicate memory); `expire_wait_commit_if_due` (RC-5(b)); `advance_session_seconds` (§3.5 serve-loop deltas); serialized `RejectionFrame`/`CancelRoundFrame` wire shapes carrying codes/counts/hashes ONLY; `GatePreStateSeed` test/fixture-replay seeding. Also: the session-1 `sample_result` test-helper fix (see §4). |
| `crates/modelswarm-session/tests/verifydrafts_admission_properties.rs` (new) | Plan §3.6 file 1 — the 13 numbered admission properties, incl. the first-failure-wins conjunction sweep: ALL 120 exhaustive pairs of 16 injectable §2-table conditions (deterministic) plus random triples (proptest), each asserting the EARLIEST code in §3.3 order. |
| `crates/modelswarm-session/tests/verifydrafts_divergence_properties.rs` (new) | Plan §3.6 file 2 — divergence/boundary properties: random windows × mock outcomes (divergence == accepted < len on rejection; null + full on acceptance; correction present except EOG; committed = `window[..accepted] ++ [correction]`; independent `new_prefix_hash` recomputation; RC-4 advance + duplicate no-op), random cap boundary, and the pinned deterministic sweep len ∈ {1,31,32,33} × `batch_window_max` ∈ {1,8,32} incl. gap-tolerant chaining (I3) and the EOG prefix-only commit (F-2). |
| `crates/modelswarm-session/tests/verifydrafts_vectors.rs` (modified) | Session-3 ACTIVATION of every session-2 `TODO(session 3)` hook: each family fixture (1–4) replays through the real gate (admission with zero engine work, exactly ONE pass at dispatch, the emitted draft equal to the recorded result on every signed field, re-signing the draft with the recorded verifier fixture key reproducing the recorded signature byte-for-byte, RC-4 advance + duplicate no-op); the len-33 `over_cap_case` replays to `payload_too_large` with ZERO `verify_window` calls; all 12 malformed cases replay through a gate seeded from their recorded `context` and assert the EXACT `expected_reject` code; `wrong-profile-id` additionally asserts the teardown. Added an independent `unix_ms` (days-from-civil) anchor test pinned to the three session-2-verified values. |
| `docs/reviews/handoff-verifydrafts-session3-2026-10-11.md` | This handoff. |

Scope check: only `crates/modelswarm-session/**` and `docs/reviews/`
touched. `protocol/` untouched (see the fixture-defect record in §5).
No dependency changes.

## 2. Exact commands and outcomes

Run from the repo root, Windows/Git Bash, workspace default features:

```
$ cargo fmt --all --check          # clean (after one `cargo fmt --all` pass)
$ cargo clippy --workspace --all-targets -- -D warnings
                                    # clean, exit 0
$ cargo test --workspace            # 428 passed, 0 failed, 6 ignored
                                    # (56 result lines, all "0 failed")
$ cargo test -p modelswarm-session --test verifydrafts_vectors                 # 11 passed
$ cargo test -p modelswarm-session --test verifydrafts_admission_properties    # 15 passed
$ cargo test -p modelswarm-session --test verifydrafts_divergence_properties   # 3 passed
```

Baseline after session 2 was 409 passed / 6 ignored → **+19** = 15
admission + 3 divergence + 1 anchor-helper test. No pre-existing test
was modified except the session-1 unit test that asserted the
(now-fixed) `sample_result` construction (§4).

## 3. Test evidence (gates)

| Gate | Command | Result |
|---|---|---|
| fmt | `cargo fmt --all --check` | clean |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| tests | `cargo test --workspace` | **428 passed, 0 failed**, 6 ignored (pre-existing owner-gated suites) |
| scoped golden | `cargo test -p modelswarm-session --test verifydrafts_vectors` | 11 passed, 0 failed |
| scoped admission | `cargo test -p modelswarm-session --test verifydrafts_admission_properties` | 15 passed, 0 failed |
| scoped divergence | `cargo test -p modelswarm-session --test verifydrafts_divergence_properties` | 3 passed, 0 failed |

Golden-hook activation result: every hook green — fixtures 1–4 replay
end-to-end (admit → ONE pass → byte-equal result incl. signature
reproduction → RC-4 advance → duplicate no-op); the len-33 case rejects
at stage 10 with zero engine passes; all 12 malformed cases emit exactly
their recorded `expected_reject` codes through the real gate.

Conjunction-sweep verdict: **first-failure-wins holds** — all 120
exhaustive condition pairs and 256 random triples emit the earliest
§3.3-order code. Notably `BudgetTokens + DeadlinePast → overloaded`
(budget stage 11 precedes deadline stage 12), the conjunction the
committed `deadline-past` fixture context actually encodes (§5).

## 4. The session-1 `sample_result` fix (task-required note)

`verifydrafts.rs`'s session-1 unit-test helper `sample_result` computed
`new_prefix_hash = compute_prefix_hash(GENESIS, &[999])` while claiming
`accepted_prefix_len: 8` over the 8-token sample window — it hashed the
correction only, violating ADR-032 §1 rule 2
(`(parent ‖ accepted prefix ‖ [correction])`). Fixed: the helper now
hashes `[101..108, 999]` (the full accepted prefix ++ bonus), and the one
session-1 assertion that pinned the old wrong construction
(`prefix_chain_reuses_the_shipped_hash_function`) now asserts the ADR
construction. Test-only change; no production code path computed the
hash before this session. The committed fixtures already byte-pinned the
correct construction (session 2), and the session-3 gate reproduces them
exactly.

## 5. Contract conflicts, unpinned mappings, and one fixture defect (recorded, never improvised)

1. **FIXTURE DEFECT — `protocol/vectors/verifydrafts-malformed.json`
   `deadline-past` context is over-constrained (NOT edited; protocol/ is
   not this session's path).** Every malformed case's `context` carries
   the shared template `committed_output_tokens: 16 ==
   budget.max_output_tokens: 16` — the injected condition of the
   `budget-exhausted` case. Under the ADR's own first-failure-wins order
   (§2 table: budget row precedes deadline row) a gate seeded LITERALLY
   from the `deadline-past` context classifies that wire as `overloaded`,
   not the recorded `deadline_exceeded`. Resolution: the gate implements
   the ADR order with NO special case; the golden replay seeds the
   at-ceiling budget only where it is the case's injected condition
   (`budget-exhausted`), replays `deadline-past` with a fresh ledger to
   reproduce its expected `deadline_exceeded`, AND additionally asserts
   the literal seeding emits `overloaded` — pinning the defect in code.
   The property suite proves the correct precedence. **Action for the
   fixture owner (Protocol Architect / future session 4): regenerate the
   malformed fixture with per-case budget context** (zero the ledger for
   `deadline-past`), then the extra literal-seeding assertion can be
   dropped. Everything else in the file replays literally.
2. **`generation_params_hash` equality has no dedicated §2-table row.**
   The binding list (§2) includes the params hash, but the admission
   partition has no params-mismatch row and R1 calls the table complete.
   Chosen mapping (escalation note): params-hash inequality rides the
   SESSION-BINDING stage (4) → `bad_frame`, consistent with the table's
   "`session_id` unknown or ≠ the stream's admitted session" row being
   the binding stage. No fixture exercises it; the sweep does not inject
   it.
3. **`profile_id` wire FORM checked at the schema stage (2).**
   `"msp1:<64 hex>"` is pinned by the ADR §1 example; a malformed form
   is `bad_frame` at stage 2 rather than reaching the profile-equality
   stage (7). Semantic mismatches (well-formed, wrong profile) still
   fire `profile_mismatch` + teardown at stage 7 (fixture `wrong-profile-id`
   replays exactly).
4. **Expiry extended to the `Verify` state.** RC-5(b) pins the
   deadline+250 ms abandonment for WAIT_COMMIT; an admitted-but-never-
   dispatched round past the same bound is equally abandoned (its
   dispatch is already impossible — the stage-13 re-check would refuse),
   otherwise it would block all future rounds at stage 9 forever.
   Documented on `expire_wait_commit_if_due`; the WAIT_COMMIT behavior
   asserted by property 8 is unchanged.
5. **`verify_pass_ms` source.** The plan §3.1 trait sketch returns only
   `(accepted_len, correction)` (kept EXACTLY as sketched), so the gate
   cannot measure the pass from the engine. `dispatch_outstanding`
   takes a `pass_end_ms` injected-clock parameter;
   `verify_pass_ms = pass_end_ms − now_ms` (§3.5's "mocked as fixed in
   tests" discipline; production serve loop supplies real clock reads).
   This keeps the golden replay byte-exact (`pass_end = anchor + 35`).
6. **Two-clock admission signature.** `admit_verify_drafts(frame,
   arrival_ms, now_ms)`: `now_ms` is the injected admission clock (plan:
   clock injected on every admission call); `arrival_ms` anchors the
   deadline math so the queued-frame `deadline-past` shape is
   expressible (equal to `now_ms` in the no-queue case). Deadline
   expiry uses `arrival + clamp(deadline_ms) <= now` (fail-closed at
   equality); the same `<=` holds at the stage-13 re-check.
7. **Verifier-side violation-counter policy (unpinned by the ADR).**
   §2's threshold-two rule is stated for coordinator-side result
   admission; the plan lists "per-peer violation counters" as gate state
   without an increment policy. Implemented: `profile_mismatch`
   increments (tears down immediately by its own row) and each
   `commit_mismatch` increments; reaching
   `TEARDOWN_VIOLATION_THRESHOLD` (2) on commit mismatches tears the
   session down (asserted in property 9). Admission rejects do NOT
   increment.
8. **`verifier_next_seq` config field.** RC-7 makes the result's `seq`
   the verifier's own counter; fixtures record `4` as its first emitted
   value (the §1 illustration made normative bytes), so the counter is
   constructor-seeded (default callers pass 1; the golden replay passes
   the recorded value). Not a wire change.
9. **Frame-size stage (1)** measures the serialized JSON length of the
   frame `Value` against `modelswarm_transport::frame::MAX_FRAME_BYTES`
   (msp-v1 §6.4's 256 KiB — the existing constant, no new one).
10. No plan-vs-ADR conflict requiring the ADR to override the plan was
    encountered in §3 itself; the plan's §3 text and ADR §2/§8 agree on
    stage order, RC-3/4/5 semantics, and the WAIT_COMMIT admission. Item
    1 above is a fixture-internal inconsistency resolved per the ADR.

## 6. Unresolved risks

1. The `deadline-past` fixture defect (§5.1) needs a fixture
   regeneration by the owner of `protocol/` — this session's golden test
   carries the compensating seeding plus the literal-seeding pin; a
   future consumer that seeds contexts literally will classify that one
   case `overloaded`.
2. The consumed-nonce set is unbounded within a session (the ADR pins no
   bound). A hostile coordinator admitted-then-abandoned in a loop grows
   it; the lease and session lifetime bound exposure in practice, but a
   future amendment may want a bounded set with eviction semantics.
3. `GateState::Validating` is transient inside the synchronous admit
   call and never observable between calls; RC-5(a)'s one-outstanding
   rule covers it structurally (property 7 exercises `Verify` and
   `WaitCommit`).
4. The serve-loop integration (signing result drafts, the
   remote-PeerId equality of §1 rule 5, WAIT_COMMIT timeout polling in a
   real event loop) is intentionally absent — this session is the pure
   gate per plan §3; wiring belongs to the Windows Product / Runtime
   seats after session 4.
5. `apply_prefix_commit` accepts the shipped `CommitPrefix` shape; the
   old vocabulary's binding gaps (ADR-032 §2) apply to it until the
   freeze ADR — noted, inherited deliberately.

## 7. Suggested next task for the integrator

Session 4 (plan §4): coordinator-side result admission (`admit_result`
(a)–(e), consumed-nonce drops, threshold-two teardown), the per-verifier
EWMA + `verify_pass_ms` cross-check (RC-2), the telemetry/audit trait
surfaces (RC-1), and `tests/verifydrafts_coordinator_properties.rs`.
Before that, the architect seat should rule on §5.1 (fixture
regeneration) and bless the §5.2 mapping if a dedicated row is preferred.
