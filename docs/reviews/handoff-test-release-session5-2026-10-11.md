# Handoff — Test and Release Engineer (session 5, ADR-032 VerifyDrafts)

Date: 2026-10-11. Agent: Test and Release Engineer. Task:
`docs/implementation/adr-032-verifydrafts-plan.md` §5.2 row 5 / ADR-032 §7
execution-venue matrix item 4b — the mixed-swarm fallback scenarios in
`apps/modelswarm-sim`, pinned as per-push sim tests under DEFAULT
features (NOT bench quic-runner-only modules).

## Provenance disclosure (important)

The task briefing stated a previous attempt "left NO changes". That was
stale: the dead session's uncommitted WIP was present on disk in
`apps/modelswarm-sim` (files last written 15:41–15:46 CST, no commit, no
handoff, no test evidence). This session therefore AUDITED the inherited
WIP instead of blindly trusting or discarding it:

- every referenced API was confirmed against HEAD (`a339e5d`) — nothing
  in another crate was modified or widened;
- all eight pinned scenario verdicts and both term arithmetics
  (batch 151 ms / wire ≈303.67 ms over the pass-2 shape) were
  re-derived BY HAND and matched the implementation's output;
- all gates were re-run from this session (below) — no self-reported
  numbers were carried over;
- one corrective edit was made (see changed files): the integration
  test's hardcoded `1.5` batch-steps literal was replaced with the
  module's exported `VERIFY_BATCH_STEPS` (the module documents that
  coefficient as a hand-mirrored value that must not drift silently).

## 1. Changed files (paths) and why

| Path | Change | Why |
|---|---|---|
| `apps/modelswarm-sim/src/mixed_swarm.rs` | new module (from inherited WIP, audited) | The item-4b decision model: `SimPeerSpec`/`MixedSwarmSpec` fixtures, `negotiate`-style ledger via the landed `peer_reject_reason`/`validate_session_accept`/`eligible_batch_verifier`, class-blind single ranking, cohort selection, `VerifyTerm::of_cohort` term charge, `predicted_swarm_mirror_ms` (the labeled mirror of the landed scheduler-test arithmetic), the frozen engage rule, the eight-case pinned matrix, the `run_mixed()` self-verifying CLI scenario, 3 unit tests |
| `apps/modelswarm-sim/tests/mixed_swarm.rs` | new integration test file (audited + 1 fix) | The four scenario families pinned as per-push tests (default features): 6 tests asserting verdicts, negotiation-ledger completeness, determinism, JSON round-trip, and that the single ranking equals the scheduler's own `predicted_single_ms` |
| `apps/modelswarm-sim/src/lib.rs` | `pub mod mixed_swarm;` + module docs | Module registration + scenario-list documentation (decision-level: no sockets, no engine) |
| `apps/modelswarm-sim/src/main.rs` | `mixed` CLI scenario | Self-verifying scenario prints one JSON summary; `ok` is true only when every pinned verdict holds |
| `apps/modelswarm-sim/Cargo.toml` | + `modelswarm-scheduler`, `modelswarm-types`, `serde` | The decision surface is IMPORTED from the landed scheduler crate (cannot drift from the option-A term guarding production); `modelswarm-types` for `ModelProfileId`; `serde` for the record derives |
| `Cargo.lock` | dependency-edge updates only | Reflects the above (no new external crates) |
| `docs/reviews/handoff-test-release-session5-2026-10-11.md` | this file | AGENTS.md handoff obligation |

No other crate was touched. `zcode-master-prompt.md` (pre-existing
untracked, not mine) is deliberately excluded from the commit.

## 2. Exact commands run and outcomes

All from repo root, default features, Windows (win32 10.0.26300, Rust
per `rust-toolchain.toml` = 1.99.0):

- `cargo fmt --all --check` → clean (after `cargo fmt --all` normalized
  the one edited line).
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0,
  zero warnings.
- `cargo test --workspace` → exit 0. Summed across all 58 test binaries:
  **456 passed / 0 failed / 6 ignored** (baseline before this change:
  447 passed / 6 ignored at `6730974`/`a339e5d`; delta = +9 new tests =
  3 unit + 6 integration, exactly the added tests).
- `cargo test -p modelswarm-sim` → 7 lib + 6 `mixed_swarm` + 3
  `scenarios` integration + 0 bin tests, all green by name.
- `cargo run -q -p modelswarm-sim -- mixed` → one JSON summary,
  `"ok": true`, all eight `verdict_ok` rows true (output transcribed in
  §3).

## 3. Test evidence

`cargo test -p modelswarm-sim` (named results, all `ok`):

- `mixed_swarm::tests::mirror_matches_the_landed_scheduler_test_numbers`
  — the sim's prediction mirror reproduces the landed scheduler 4a test
  totals exactly: batch 151 ms, wire 303.666…7 ms; wire verification
  alone (180 ms) exceeds the single's whole decode budget (160 ms);
  partial acceptance strictly raises the prediction.
- `mixed_swarm::tests::margin_below_floor_is_an_error` — margin 0.04 is
  rejected ("below the frozen floor"), never clamped.
- `mixed_swarm::tests::dishonest_disclosure_fails_validation` —
  `batch_window_max = WINDOW_CAP+1` accept fails
  `validate_session_accept` ("engine disclosure invalid").
- `tests/mixed_swarm.rs::http_only_cohorts_never_engage_speculation`
  (family 1), `::adapter_cohorts_engage_only_when_prediction_beats_the_fastest_single`
  (family 2), `::mixed_swarms_fall_back_to_the_fastest_single_regardless_of_class`
  (family 3), `::honesty_edge_batch_false_single_and_backend_mismatch_continues`
  (family 4), `::pinned_matrix_verdicts_hold_and_run_mixed_is_ok`
  (end-to-end + determinism + JSON round-trip + CLI green),
  `::single_ranking_is_the_frozen_scheduler_formula` (comparator equals
  the scheduler's own formula).

CLI `mixed` scenario verdicts (independently re-derived by hand in this
session — numbers follow the landed pass-2 shape: verifier decode 0.10 /
prefill 3.0 tok/ms, rtt 5 ms, prompt 153, target 16, window 8,
acceptance 1.0, drafter 4.0 tok/ms, risk 5 ms, margin 0.15; fastest
single 216 ms; batch prediction 151 ms engages at 151×1.15 = 173.65 <
216; wire prediction ≈303.67 ms never engages):

| Case | Verdict |
|---|---|
| `http-only-never-engages` | fallback `http-1`, term SequentialWire |
| `adapter-cohort-engages` | ENGAGED, term Batch |
| `adapter-faster-single-wins` | fallback `ada-fast`, term Batch |
| `adapter-honest-window-cap-falls-back` | fallback `ada-cap`, term Batch (window 8→4, rounds 2→4, prediction 151→293) |
| `mixed-http-fastest-single-wins` | fallback `http-fast`, term SequentialWire |
| `mixed-adapter-cohort-engages-with-http-proposer` | ENGAGED, term Batch, HTTP peer rides as proposer |
| `mixed-adapter-fastest-single-wins` | fallback `ada-fast`, term Batch |
| `honesty-backend-mismatch-continues` | fallback `http-best` (batch_verify:false plain single), `ada-vulkan` rejected `backend_mismatch`, requester continued |

The four requested scenario verdicts, mapped:

1. **HTTP-only cohorts never engage** — GREEN (family-1 test + the
   structural pins: term SequentialWire, wire verification alone exceeds
   the comparator's decode budget, full prediction loses outright
   before any margin).
2. **Adapter cohorts engage only when predicted×(1+margin) beats the
   fastest eligible single** — GREEN (engage at 173.65 < 216; fallback
   twins on a faster single AND on an honest `batch_window_max` that
   shrinks the window; the disclosure degrades the prediction, never
   inflates it).
3. **Mixed swarms class-blind** — GREEN (fastest single of EITHER class
   wins the fallback; adapter-only twin of the blocked mixed roster
   engages, proving the comparator — not mixedness — decided; batch
   capability only changes the term).
4. **Honesty edge** — GREEN (`batch_verify:false` peers are plain
   singles/proposers; backend mismatch → recorded
   `SessionReject(backend_mismatch)` with the frozen wire string; the
   negotiation ledger accounts for every roster peer — no silent drops).

## 4. Assumptions

- The pinned scenarios are DECISION-level (no sockets, no engine): the
  ADR's venue row pins the decision outcome in per-push CI; the
  adapter-cohort half against a real engine remains the manual-dispatch
  `real-model-e2e.yml` tier (the ADR's standing statement — it can
  never go green in per-push CI).
- The prediction arithmetic is the landed scheduler-4a-test shape
  (bench coefficients, pass-2 measured wire costs), labeled a MIRROR
  everywhere; no claim-bearing performance number is produced (honesty
  rule: predictions, never measurements, in per-push artifacts).
- `VERIFY_BATCH_STEPS = 1.5` is mirrored in the sim rather than
  importing the bench crate (which would pull `modelswarm-store` into
  the sim graph); the value is triple-pinned (bench const, scheduler 4a
  test literals, sim const + exact-number tests) so a divergence fails
  loudly.
- `docs/adr/ADR-032…md` §7 implementation status ("session 5 sim (in
  flight)") is Protocol-Architect-owned and NOT edited here; this
  handoff is the record for that update.

## 5. Unresolved risks / API-friction findings (requested)

- **Friction (none blocking):** no other crate's API needed widening —
  the scheduler's `VerifyTerm`/`select_microswarm`/`predicted_single_ms`/
  `should_engage_cooperative` and the session crate's negotiation types
  plus `validate_session_accept`/`eligible_batch_verifier`/
  `peer_reject_reason` were already public and sufficient. Two notes for
  the architect: (a) the swarm-side prediction had to be mirrored in the
  sim because the scheduler exposes the frozen `SwarmInputs` SUM but no
  engagement-pool composition over a cohort (a cohort-level
  `predicted_swarm_ms` helper on the scheduler would remove the mirror);
  (b) negotiation is still an in-process exchange — when a serve loop
  lands, the sim's decision core should be re-pointed at real sessions
  (the `run_mixed_swarm_decision` step list is the seam).
- **Mirror-drift risk:** the batch-steps coefficient and the
  engagement-pool arithmetic are pinned by exact-number tests on both
  sides (scheduler 4a test and the sim's mirror test); a bench
  coefficient change that flips a decision fails CI loudly, but a small
  non-flipping change would not — acceptable for a TEST-ONLY constant.
- **Scope risk (pre-existing):** the family still has zero
  serve-loop/light-up coverage (per the ADR, correctly gated behind the
  light-up tier and the `MSP_LIVE=1` harness obligation).

## 6. Suggested next task for the integrator

Record session 5 as landed in ADR-032 §7's implementation status (the
architect's edit set; this handoff is the evidence), then run the
remaining wire-tier items: the scheduler 4a sign-off recording and the
post-implementation Security + Test/Release review pass over sessions
1–5 (per §7 "REMAINING" list — this session is the T/R half of item 4b,
not that review).
