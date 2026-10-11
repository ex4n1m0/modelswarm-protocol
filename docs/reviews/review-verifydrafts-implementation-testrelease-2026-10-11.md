# Review — VerifyDrafts family implementation (sessions 1–5), Test and Release seat

Date: 2026-10-11 (post-implementation pass). Reviewer: Test and Release
Engineer (release seat). Scope: the POST-IMPLEMENTATION release review of the
landed VerifyDrafts family — ADR-032 §7 acceptance-checklist item 8, Test and
Release half (the Security half is a separate review and is NOT discharged by
this document).

Commits reviewed (the family, in order): `fddb972` (session 1: types +
serialization), `4f277cc` (session 2: generator + 10 committed fixtures +
golden test), `f9d0806` (session 3: verifier admission gate + state machine +
admission/divergence property suites), `6730974` (session 4: coordinator
RC-3/RC-2/RC-1 + the authorized deadline-past fixture repair), `f09a50c`
(session 5: mixed-swarm fallback scenarios in `apps/modelswarm-sim`). Also in
range and verified docs-only: `a339e5d` (ADR revision 3), `b71ca99` (§7
implementation-status update). Review base: `main @ b71ca99`.

**Disclosure (independence):** session 5 (`f09a50c`) was THIS seat's own
implementation work. It was re-reviewed here with extra skepticism — every
pinned verdict and both term arithmetics were re-derived by hand from the
landed scheduler 4a test (not from the session-5 handoff's numbers), and the
self-reported gate numbers were discarded and rerun from scratch. Findings
against session 5 are reported in §7 with the same severity ladder as the
others.

## Verdict

**APPROVE.**

Zero blockers, zero required changes. Five advisories (§8), each routed to an
owning seat; none gates the wire-contract tier or any current release
surface (the family carries no traffic — `capi-adapter` stays OFF, no serve
loop exists; the only production-visible change remains the option-A
scheduler term landed earlier at `e85e869`/`bea1749`).

## 1. Gate reruns (independent, this session, machine-checked)

Windows (win32 10.0.26300, Git Bash), Rust per `rust-toolchain.toml`
(1.99.0), default features unless stated, from the repo root at `b71ca99`
with a clean working tree:

| Gate | Command | Result (rerun) | Handoff claim | Match |
|---|---|---|---|---|
| fmt | `cargo fmt --all --check` | exit 0, clean | clean (all sessions) | yes |
| clippy default | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, zero warnings | clean | yes |
| clippy capi | `cargo clippy --workspace --all-targets --features modelswarm-runtime/capi-adapter -- -D warnings` | exit 0, zero warnings | (CI standing step) | yes |
| tests workspace | `cargo test --workspace` | **456 passed / 0 failed / 6 ignored** across 58 result lines | S5: 456/0/6 | **yes, exact** |
| tests bench quic | `cargo test -p modelswarm-bench --features quic-runner` | **79 passed / 0 failed / 6 ignored** | scheduler handoff: 79/0 (6 owner-gated ignored) | **yes, exact** |
| tests session | `cargo test -p modelswarm-session` | **118 passed / 0 failed** (46 lib + 10 fuzz + 1 props + 5 e2e + 7 multi + 5 vec... see below) | per-suite claims below | yes |

Per-binary confirmation inside the workspace run (the venue question in §3):
`modelswarm-session` lib 46; `verifydrafts_vectors` 11;
`verifydrafts_admission_properties` 15; `verifydrafts_divergence_properties`
3; `verifydrafts_coordinator_properties` 15; `modelswarm-sim` lib 7;
`tests/mixed_swarm.rs` 6. All 0 failed.

Count-chain audit vs the session handoffs (method: chain arithmetic against
the recorded pre-family baseline plus the independent final-state rerun
above; intermediate commits were NOT rebuilt):

- Baseline before the family: 374/0/6 (scheduler handoff `2b4fcd3`).
- S1 claimed 399 (+25), S2 409 (+10), S3 428 (+19), S4 447 (+19), S5 456
  (+9). Arithmetically exact: 374+25+10+19+19+9 = 456, and every delta equals
  the handoff's named new tests (S3: 15+3+1; S4: 15+4; S5: 3+6).
- The final state 456/0/6 and the per-binary splits are INDEPENDENTLY
  CONFIRMED by the rerun above. The intermediate-commit claims are accepted
  as chain-consistent, not independently re-executed — recorded honestly as
  the method, not as evidence of those exact historical states.

## 2. §5.3 acceptance checklist — honest discharge (item by item)

| # | Item | Discharged? | Evidence |
|---|---|---|---|
| 1 | fmt/clippy/test green default + capi CI steps | YES | §1 reruns: 456/0/6; clippy default and capi clean. The capi steps exist in `rust.yml` lines 48–51 and run every push. |
| 2 | Golden tests green per-push; generator byte-idempotent | YES | 11 golden tests green in the default workspace run. Generator run TWICE this session: `git diff -- protocol/vectors/` empty after both runs; `git status` clean. |
| 3 | Admission property suite incl. first-failure-wins conjunction sweep | YES | 15 green; the sweep is ALL C(16,2)=120 exhaustive pairs + proptest triples, each asserting the earliest §3.3-order code via an `order()` index per condition. |
| 4 | Divergence/boundary property suite | YES | 3 green, incl. the deterministic sweep len {1,31,32,33} × bwm {1,8,32}. |
| 5 | Coordinator property suite (I2 hostile verifier, SessionAccept honesty, cross-check, per-verifier EWMA isolation) | YES | 15 green; handoff's RC-3 verdicts spot-checked against the code (`admit_result` doc + checks match ADR rev 3's amended (c)). |
| 6 | Scheduler 4a tests green + sign-off recorded | YES (recorded at `b71ca99`; scheduler lib tests green in the rerun; the mirror numbers cross-verified in §7) | |
| 7 | Reject-frame privacy assertion | YES | `reject_frames_never_carry_token_ids` pins the serialized frame keys to exactly {code, prefix_hash, round, session_id} across the whole condition sweep, plus distinctive token-id canaries; the golden replay asserts the same per malformed case. |
| 8 | Handoff docs per step; Security + T/R review sign-offs | HALF: handoffs exist for all five sessions (read in full); THIS document is the T/R sign-off; the **Security review remains open** and must land before item 8 is closed. | |

## 3. Venue honesty (per-push vs manual dispatch)

- `rust.yml` paths (`crates/**`, `apps/**`, `protocol/**`, `Cargo.toml`,
  `Cargo.lock`, …) cover every file the family touched — pushes containing
  these commits cannot skip the `check` job. No workflow file was modified
  in the range (verified: `git log fddb972~1..f09a50c -- .github/` is empty);
  none needed to be — **no release-owned fix was required**.
- Every new test runs in the per-push default-feature job: the job's
  `cargo test --workspace` step is byte-identical to the rerun in §1, whose
  log shows `tests/verifydrafts_{vectors,admission_properties,divergence_properties,coordinator_properties}.rs`
  and `tests/mixed_swarm.rs` all executing and passing. `modelswarm-sim` has
  no `[features]` section — the mixed-swarm tests cannot be feature-gated
  off.
- Nothing claims the manual-dispatch venue that shouldn't: `real-model-e2e.yml`
  remains `workflow_dispatch:`-only and untouched; no family test, doc, or
  commit message asserts a real-engine result. The sim's adapter-cohort
  cases are explicitly decision-level (no engine) — labeled in the module
  docs, the crate docs, the CLI JSON `note` field ("predictions are labeled
  mirrors … not measurements"), and the handoff.
- The two standing statements HOLD: (a) item 2 (exactness pins incl. I4
  one-pass decode count) and the real-engine adapter half of 4b are NOT
  green per-push — no per-push test performs engine work (mock engine
  trait only; sim is pure arithmetic); (b) the capi-adapter clippy/test job
  exists on every push (`rust.yml` "Clippy/Tests (capi-adapter feature)").

## 4. Golden-vector discipline

- **Idempotency:** generator executed twice this session; `git diff` on
  `protocol/vectors/` empty after each run (the repo's `text=auto`
  normalization applies equally to both runs; the check standard matches the
  handoffs').
- **Coverage vs the §7 table — every fixture row is consumed by a test:**
  `verifydrafts-1-accept` → `golden_verifydrafts_1_accept_full_acceptance`
  (canonical bytes + sha256 + signatures + installation-id derivation +
  chain-hash reconstruction + full gate replay incl. one-pass count and
  signature reproduction); `-2-reject` → `..._2_reject_mid_window`
  (divergence == accepted == 3, replacement token, recomputed chain);
  `-3-boundary` → `..._3_boundary_cap_edge` (len-32 accepted at bwm 32 AND
  the len-33 `over_cap_case` replayed through the real gate to
  `payload_too_large` with `engine.calls() == 0`); `-4-eos` →
  `..._4_eos_correction_null` (present-as-null + prefix-only commit);
  `verifydrafts-malformed` → all 12 cases replayed through gates seeded
  LITERALLY from each case's recorded context, each asserting the EXACT
  `expected_reject` code, with the frozen-registry membership check applied
  to every code first; `session-offer-2`/`accept-2`/`reject-1` → three
  dedicated golden tests (all four reject reasons canonicalized); the
  base-form `session-offer-1`/`accept-1` →
  `golden_session_offer_and_accept_1_base_forms_pin_f6` (pins base bytes AND
  asserts they fail deserialization into the extended structs); plus
  `golden_fixture_set_shares_one_anchor_and_keypair`.
- **No fixture-key leakage:** `coordinator_seed_hex`/`verifier_seed_hex` and
  the `[0x11;32]`/`[0x22;32]` seeds appear only in the fixtures, the golden
  consumer test, and `#[cfg(test)]` helpers (module test section starts at
  `verifydrafts.rs:2427`); no production path reads fixture key material.
  The `0x11`/`"1111…"` occurrences elsewhere in the workspace
  (bench runner test deltas; `modelswarm-node/src/artifact.rs`) pre-date the
  family (verified via `git log -S`) and are unrelated.

## 5. Cross-commit integrity (the 6730974 fixture repair)

- The golden test asserts the DIRECT replay: `replay_malformed_case` seeds
  `committed_output_tokens` literally from every case's recorded context;
  the session-3 compensating-seeding workaround block is GONE (the only
  remaining `deadline-past`-specific code asserts the FIXTURE's content —
  `committed_output_tokens == 0` and the `budget_ledger` note — which is a
  data pin, not a behavioral workaround). The committed fixture itself
  carries the fresh ledger (`committed_output_tokens: 0`, note recorded;
  `budget-exhausted` still carries 16), so any future consumer seeding
  literally gets `deadline_exceeded` — consumer-neutrality preserved.
- The budget-vs-deadline ordering stays pinned where it belongs: the
  session-3 conjunction sweep attributes stage indices (BudgetTokens (11,0),
  BudgetSeconds (11,1), DeadlinePast (12,0)) and asserts the earliest code
  over all 120 pairs and random triples; its proptest comment explicitly
  cross-references the fixture repair. ADR rev 3's correction note matches
  the code and the fixture.

## 6. Release-blocker scan (fddb972..f09a50c diff range)

- Per-commit file lists: sessions 1–4 touch only
  `crates/modelswarm-session/**`, `protocol/vectors/**`, `Cargo.lock`,
  `Cargo.toml`, and handoffs; session 5 touches only
  `apps/modelswarm-sim/**`, `Cargo.lock`, and its handoff; `a339e5d`/`b71ca99`
  touch only the ADR. No other crate, no `spec.rs` behavior, no workflows.
- `Cargo.lock`: exactly two hunks (+4 lines), both mechanical dependency-list
  edits inside existing `[[package]]` blocks — `modelswarm-session` +
  `modelswarm-types` (session 1's Cargo.toml addition) and `modelswarm-sim`
  + `modelswarm-scheduler`/`modelswarm-types`/`serde` (session 5's). No new
  external package, no version churn.
- `crates/modelswarm-session/src/lib.rs`: exactly the two `pub mod`
  registrations; the sim's `lib.rs`/`main.rs`: module registration, honest
  decision-level documentation, and the CLI scenario wiring.
- No behavior changed outside the family's scope. No secrets, keys, prompts,
  or model weights in the diff (fixtures carry recorded fixture-only seeds
  per the lease-hubkey-1 precedent).

## 7. Session-5 self-review (adversarial; own work, extra skepticism)

Re-derived BY HAND from the landed scheduler 4a test shape (verifier decode
0.10 / prefill 3.0 / rtt 5, prompt 153, target 16, window 8, acceptance 1.0,
drafter 4.0, risk 5, margin 0.15): fastest single 216 ms (= 51 + 160 + 5);
wire verify alone 180 > the single's whole decode budget 160; wire prediction
2×(157/3 + 2 + 5) + 180 + 5 = 303.6667 ms (never engages); batch prediction
2×(51 + 2 + 5) + 30 + 5 = 151 ms (engages at 151×1.15 = 173.65 < 216);
window-cap case 4 rounds × 57 + 60 + 5 = 293 ms (flips to fallback);
`http-best` single at decode 0.25 = 120 ms. All eight pinned matrix verdicts
follow from these numbers, and the sim's mirror unit test pins 151 and
303.6666666666667 to 1e-9 — the numbers are honest and match the scheduler
test's own arithmetic.

- **Can the mirror drift silently?** Partially, and the module says so. The
  TERM (`VerifyTerm::decode_steps_per_round`), the engage rule
  (`should_engage_cooperative`), the selector (`select_microswarm`), and the
  single comparator (`predicted_single_ms`) are IMPORTED — those cannot
  drift. What is hand-mirrored is (i) the `VERIFY_BATCH_STEPS = 1.5`
  constant and (ii) the engagement-pool arithmetic
  (`predicted_swarm_mirror_ms`: rounds × (repost + window/drafter + rtt) +
  verify + rollback + risk). Both sides pin the same NUMBERS independently,
  so a scheduler-side arithmetic change that updates the scheduler test
  would NOT fail the sim's mirror test (only a term change would).
  Consequence of drift is a wrong sim VERDICT (test-only), never production
  behavior; the module/handoff label it a mirror everywhere. Classified
  advisory A-1 with the structural fix routed to the Scheduler seat.
- **Is the decision-level labeling honest?** Yes — verified in the module
  docs, crate docs, the CLI JSON `note`, the handoff, and ADR §7's status
  text. No performance claim is made anywhere; the pinned numbers are
  labeled predictions from the pass-2 shape.
- One coverage gap found in my own matrix (A-5 below): no scenario exercises
  the eligibility SKIP path (an ineligible fastest-ranked single, e.g.
  `slots: 0`), so `ranked_singles.iter().find(|e| e.eligible)` never
  skips in the pinned cases.

## 8. Findings

**Blockers: none. Required changes: none.**

Advisories (routed; none release-gating — the family carries no traffic
until the separately-gated light-up tier):

- **A-1 (Scheduler seat):** the sim's engagement-pool arithmetic is a
  hand-mirrored copy of the scheduler-test shape; only the shared imported
  surfaces are drift-proof. Structural fix: expose a cohort-level
  `predicted_swarm_ms` on `modelswarm-scheduler` (the session-5 handoff
  already records this as API friction) and delete the mirror.
- **A-2 (Protocol Architect):** §8e's budget check is would-exceed for
  TOKENS but already-at-cap (`served_ms >= cap`) for SECONDS — one
  deadline+grace-bounded round can complete past the seconds cap. Intent
  ("never burn unbounded engine seconds") is honored; the letter is
  approximated. Pin or amend before light-up.
- **A-3 (implementation sessions 3/4, on record already):** the
  session-scoped consumed-nonce sets are unbounded within a session
  (verifier and coordinator sides); lease/session lifetime bounds exposure.
  A bounded set with eviction semantics is the future amendment.
- **A-4 (implementation sessions 2/3):** the privacy canaries are decimal
  substrings ("701", "90000") that are also valid hex substrings — a
  prefix-hash hex string could theoretically contain one and fail the
  assertion spuriously (fails LOUD, never silently; deterministic today).
  The admission suite's structural keys-pin ({code, prefix_hash, round,
  session_id} only) is the robust form; mirror it in the golden replay.
- **A-5 (T/R seat, own work):** add one pinned mixed-swarm case whose
  fastest-ranked single is INELIGIBLE (slots 0) so the class-blind
  comparator's eligibility skip is exercised in the sim matrix.

## 9. Conclusion

The §5.3 wire-contract checklist is honestly discharged for items 1–7 with
exact-number evidence this seat independently reproduced; item 8 is now
half-closed (T/R sign-off = this document; the Security review remains the
open half). The implementation matches ADR-032 revision 3 including its
amended contract points (F-1/F-2/F-6, RC-3(c) seq mirror, transitive
profile/parent, cache-cold audit rule). Release posture: the wire tier is
landed, green, and inert (no serving path); the only production-visible
family change remains the option-A scheduler term, reviewed separately.
