# Handoff — Test and Release Engineer — 2026-10-10

Task: independent review + guard-calibration study of the Scheduler
Scientist's pass-2 engaged-curve instrument (`4faa6c0` machinery, `3dbc161`
LAN evidence), routed by the Integrator. Full review:
`docs/reviews/review-guard-calibration-2026-10-10.md`.

## 1. Changed files and why

- `docs/reviews/review-guard-calibration-2026-10-10.md` (new) — the review:
  extraction/characterization of the 125 guard-passing engaged LAN runs,
  guard-semantics evaluation, ONE calibration recommendation, evidence-chain
  verdict. Contains an executable extraction script (verified verbatim
  against the committed artifacts).
- `crates/modelswarm-bench/tests/harness.rs` (+1 test) — labeled `REVIEW
  PIN`: `loss_guard_counterfactual_matches_default_arithmetic` pins the
  `LossGuardRecord` counterfactual honesty contract (a relaxed/"off" row
  must still report `would_fire_at_default` == the default-multiplier
  arithmetic `round1_wall > 2.0 × 1.15 × predicted_round`, matching the
  committed artifact rows). No prior test covered it. Bench tests are
  co-owned with the Scheduler Scientist for review purposes; addition is
  minimal and clearly labeled. **No production file touched.**

## 2. Exact commands and outcomes

- `python3 /tmp/extract125.py` (inline in the review doc) over
  `experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/` — 4,800 rows, 1,982
  engaged, 1,857 would-fire, **125 passed the default guard**; full
  characterization in the review.
- `python3 experiments/processed/aggregate-pass2.py experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10 /tmp/reagg-lan.json`
  then `diff` vs committed summary — **bit-identical** (reproducible).
- Loopback twin label/claim check — 1,920 rows all `loopback+injected-delay`,
  946 engaged / 911 would-fire (911/946 published claim re-derived).
- Gates: `cargo fmt --check` clean;
  `cargo clippy -p modelswarm-bench --features quic-runner --all-targets -- -D warnings` clean;
  `cargo test -p modelswarm-bench --features quic-runner` **55 passed / 0
  failed / 4 ignored** (37 lib, 17 harness incl. the new pin, 1 smoke +
  4 owner-gated ignored).

## 3. Test evidence

See §2 gate line. The new pin runs in <0.01 s inside `--test harness`
(verified individually first: 1 passed / 0 failed).

## 4. Outcome summary for the Integrator

- **Recommendation: option (iii)** — keep the reactive guard default at 2.0
  as the backstop (unchanged) and gate engagement on a per-profile
  engaged-loss EWMA keyed on `realized_total_ms /
  fastest_single_prediction_ms` (block at ≥ 1.0, i.e., strict
  beat-or-fall-back). Derived: catching all 125 leak rows needs a
  multiplier < 1.539 (m = 1.5 fires on 1,982/1,982 engaged rows incl. all
  107 engaged row-wins, and makes the median leak-row outcome WORSE,
  1.397× → ≈1.673× abort cost); the guard fires only after round 1 is paid
  (57.7–84.1% of a single run on the leak rows), so within-request
  reaction can never deliver "never meaningfully worse". On this dataset an
  EWMA-gate at ≥ 1.0 blocks 8/8 engaged cells — matching ANALYSIS finding 4.
- **The 125 in three lines**: all window-8, in exactly two cells
  (high-w8 81, geo900-w8 44; planner-k 44 / k2 40 / k3 35 / k4 6), rounds
  2–4, guard ratios 1.770–2.300 (implied multipliers 1.539–2.000). Every
  one lost vs the prompt-best single: 1.179–3.278× (p50 1.397×; +80 ms p50
  at high-w8, +487 ms worst at geo900-w8); zero wins.
- **Evidence chain: PASS, no blockers.** Aggregate re-runs bit-identical;
  labels honest (LAN vs loopback twins verified both directions); negative
  published without overreach. Non-blocking findings recorded (B-side exe
  SHA attested-not-repo-verifiable; "2.8–8.3×" describes the fired
  population; the worst cell's production outcome with the guard ON would
  still be ≈2.07× — worse than single, reinforcing (iii)).

## 5. Assumptions

- The counterfactual abort-cost figures use the code path's actual shape
  (fired → fresh `run_single` on the ordered-best peer) with the committed
  measured components (round-1 wall + prompt-best single); labeled as
  simulation in the review, not a measured arm.
- `DEFAULT_CONFIDENCE_MARGIN = 0.15` is read from
  `modelswarm-scheduler` (the value recorded in every pass-2 ANALYSIS).
- Review scope is the pass-2 LAN set (4,800 rows) plus a label/claim
  spot-check of the loopback twin (1,920 rows); the loopback leak rows were
  not separately characterized (LAN is the flagged risk).

## 6. Unresolved risks

- The recommendation (iii) is ADR-gated Scheduler work: EWMA α, warm-up N,
  cold-start default, and per-(cohort, profile) scope are unspecified until
  they propose it; until then the production posture on msp-v1 remains
  "guard at 2.0 + engage gate", which this evidence says leaks 6.3% of
  engaged runs at 1.18–3.28× loss in high-acceptance/tight-prediction
  cells.
- B-side serve identity for the LAN run rests on the ops-record
  attestation (scp-verified at run time; machine cleaned after).

## 7. Suggested next task

Route §2 of the review to the Scheduler Scientist as a proposal request:
specify the per-profile engaged-loss EWMA engage-gate (α, warm-up, cold
start, scope) and re-run the pass-2 cell matrix with the gate keyed on it,
verifying fallback ratios stay in the 0.95–1.02× band. Owner decision
needed on accepting recommendation (iii) vs. deferring.
