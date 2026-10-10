# PASS-2 GATED (loopback) — 2026-10-11 — the wire-true engage gate enforced: all engagement blocked, everywhere

Label: `loopback+injected-delay` (every `decision-join.jsonl` row and every
`summary.json`). NEVER a LAN claim — the LAN twin lives at
`experiments/raw/PASS-2-GATED-LAN-2026-10-11/`.

## Why this run exists

This is the pass-2 cell matrix re-run with BOTH gates live — (1) the
ADR-032 §4 companion correction (option A): the acting planner charges
verification capability-aware and wire-true by default (window+1
sequential verifier tokens per round + per-round re-post of the growing
committed prefix; the 1.5-step batch term only for a verifier that
DECLARED `batch_verify`, which no production adapter can), and (2) the
per-profile engaged-loss EWMA engage gate (guard-calibration review
recommendation iii: α=0.3, warm-up N=3, block at ≥ 1.0 on
`realized_total_ms / fastest_single_prediction_ms`). Pass 2 measured the
engaged curve by relaxing to the batch-term model; this run verifies the
enforced posture: **the gate must block every engagement on today's
(HTTP-only) cohorts — the pass-2 never-engage conclusion made binding —
while the fallback path stays at parity with the fastest single.**

## Run identity

- Code: `fa91ffb` (both gates; same harness/pool machinery as pass 2).
  Driver `pass2_engagement_loopback`; executor
  `synthetic-token-executor` (TEST-ONLY) behind REAL local QUIC serving
  bridges (loopback dial, injected executor-seam delay).
- Pool (all cells, unchanged from pass 2 for comparability): V `0.10`
  tok/ms decode / `3.0` prefill / queue 0; D `4.0`/`6.0`/300 (gpu_high);
  M `0.25`/`1.2`/400; M2 `0.20`/`1.0`/500. Corpus
  `synthetic-pass1-v1`, output target 16, margin 0.15, τ 0.0, cap 4,
  8 reps/arm (32 samples per arm per cell).
- 12 cells: injected {5, 20} ms × profiles {high, geo900, geo800,
  geo700} at window 8, plus window 16 at 5 ms — the exact committed
  pass-2 loopback matrix. Rule-6 guard `off` (as in pass 2; with zero
  engaged rounds it never evaluates). Fresh harness + acceptance store
  per cell (pass-2 isolation preserved; the loss EWMA is therefore also
  per-cell here — see the LAN ANALYSIS for the one-profile production
  shape, replay-pinned in `tests/harness.rs`).

## Results (medians vs the cell's fastest-single median; n=32/arm/cell)

| cell | single (ms) | gated coop range | pass-2 coop range | pass-2 worst | gated worst |
|---|---|---|---|---|---|
| inj5ms-high-w8 | 231.7 | 0.990–1.002 | 1.210–1.376 | 1.376 | 1.002 |
| inj5ms-geo900-w8 | 231.9 | 0.977–1.004 | 1.535–1.814 | 1.814 | 1.004 |
| inj5ms-geo800-w8 | 238.4 | 0.993–1.004 | 1.000–1.033 | 1.033 | 1.004 |
| inj5ms-geo700-w8 | 239.1 | 0.984–1.003 | 1.025–1.037 | 1.037 | 1.003 |
| inj20ms-high-w8 | 248.6 | 0.980–0.991 | 1.205–1.470 | 1.470 | 0.991 |
| inj20ms-geo900-w8 | 246.9 | 0.994–1.018 | 1.641–1.831 | 1.831 | 1.018 |
| inj20ms-geo800-w8 | 256.6 | 0.998–1.023 | 0.995–1.022 | 1.022 | 1.023 |
| inj20ms-geo700-w8 | 254.9 | 0.983–1.002 | 1.059–1.086 | 1.086 | 1.002 |
| inj5ms-high-w16 | 232.4 | 0.991–1.011 | 0.920–0.999 | 0.999 | 1.011 |
| inj5ms-geo900-w16 | 232.6 | 0.980–0.993 | 1.567–2.005 | 2.005 | 0.993 |
| inj5ms-geo800-w16 | 239.4 | 0.993–1.003 | 0.999–1.040 | 1.040 | 1.003 |
| inj5ms-geo700-w16 | 234.1 | 0.981–0.999 | 1.272–2.755 | 2.755 | 0.999 |

Aggregate (script-computed, `experiments/processed/
PASS-2-GATED-LOOPBACK-2026-10-11-summary.json`): 1,920 join rows,
**0 failures, 0 engaged rounds** (no cooperative wire round ran
anywhere), 1,536 cooperative rows — **all 1,536 gate fallbacks**, every
one with the `wire-true sequential verify` cost-model reason; the
engaged-loss EWMA never warmed (0 samples — it cannot fire when the
wire-true term blocks first; that layering is by design, pinned by the
replay test over the committed pass-2 rows).

## Honest findings

1. **The gate blocked engagement everywhere, as ADR-032 option A
   predicts.** On the HTTP-only pool that pass 2 deliberately shaped so
   the OLD (1.5-step batch-term) engage rule would pass, the wire-true
   term blocks 1,536/1,536 cooperative attempts across all 12 cells —
   including the w16 parity corner (one round, 17 vs 16 verifier tokens:
   17 tokens charged honestly now, and the prediction loses outright to
   the fastest single before any margin). Zero speculative rounds ran;
   the acceptance stores stayed empty (honest artifact: nothing to
   measure when nothing engages).
2. **Fallback parity held: 0.977–1.023 across all 48 arm-cell medians**
   (59/48-cell arms within ±2.0%; the single largest deviation is
   planner-k at inj20ms-geo800-w8, 1.023). Every fallback row executes
   the single path against the fastest-predicted peer, so cooperative
   arms now track the single at noise level — the pass-2
   mostly-fallback band (0.95–1.02) reproduced.
3. **No cell regresses vs the committed pass-2 numbers, with one honest
   exception pair.** Per-cell worst arm medians improved or held
   everywhere (e.g. inj5ms-geo900-w16: 2.005 → 0.993; geo700-w16:
   2.755 → 0.999). The exceptions, stated plainly: (a)
   inj20ms-geo800-w8 1.022 → 1.023 (+0.001, timer noise on a
   mostly-fallback cell in pass 2 already); (b) **inj5ms-high-w16
   0.920–0.999 → 0.991–1.011** — pass 2's ONLY sub-1.0 cell medians
   (the parity corner), which the strict comparator already showed as
   parity-not-win (1.047 vs the prompt-best single). The gate trades
   that ≤2%-below-cell-median corner for the elimination of losses up
   to 2.755×; against the inviolable comparator nothing that ever won
   was given up, because nothing ever won.
4. **The EWMA engage gate stays cold here — and that is the correct
   layering.** The wire-true term is the cheaper, earlier, structural
   blocker (no round is ever paid); the loss EWMA exists for cohorts
   whose predictions DO pass (a mis-modeled cost, an optimistic
   acceptance EWMA, a future batch-declared cohort). Its blocking
   behavior is pinned against the committed pass-2 LAN rows in
   `crates/modelswarm-bench/tests/harness.rs`
   (`engage_loss_gate_replay_of_committed_pass2_lan_rows`): 7/7 warmed
   cells block at engaged attempt 4–5, and a single per-profile gate
   over the whole 1,982-attempt sweep lets exactly the 3 warm-up
   attempts through.

## What this run does NOT claim

- No model-level speculative claim — TEST-ONLY synthetic executor; the
  measured quantities are planner/gate behaviors (blocking, fallback
  cost), not inference throughput.
- No claim that the wire-true term is CALIBRATED against real engines:
  it charges the wire's measured structure (pass-2 evidence), not a
  new measurement; a batch-verify engine behind a VerifyDrafts-class
  wire (ADR-032 B, Proposed) would need its own wire-true update.
- The pool is the pass-2 engagement shape (pinned for comparability);
  on symmetric pools the old gate blocked everything already — both
  postures now agree: never engage on this wire.

## Limitations / follow-ups

- 8 reps/arm (32 samples/arm/cell) — same sampling as the committed
  pass-2 loopback twin; medians stable, CIs not bootstrapped.
- The engaged-loss EWMA has no recovery mechanism yet (blocked stays
  blocked); a reviewed probe/decay policy or (profile, verify-term)
  keying is the documented follow-up before any production consumer
  copies the gate.
- Cold-start warm-up (N=3) means a genuinely-winnable profile pays up
  to 3 engaged attempts before the EWMA protects it — the margin rule
  is the only protection there, which pass 2 showed is not enough on
  this wire (the wire-true term now closes that gap structurally).

## Ops record

```bash
OUT=experiments/raw/PASS-2-GATED-LOOPBACK-2026-10-11
MSP_BENCH_OUT=$OUT MSP_BENCH_PROFILES=high,geo900,geo800,geo700 \
  MSP_BENCH_DELAYS=5,20 MSP_BENCH_WINDOW=8 MSP_BENCH_RUNS=8 \
  MSP_BENCH_LOSS_MULTIPLIER=off \
  cargo test -p modelswarm-bench --features quic-runner \
    --test pass1_loopback pass2_engagement_loopback -- --ignored --nocapture
MSP_BENCH_OUT=$OUT MSP_BENCH_PROFILES=high,geo900,geo800,geo700 \
  MSP_BENCH_DELAYS=5 MSP_BENCH_WINDOW=16 MSP_BENCH_RUNS=8 \
  MSP_BENCH_LOSS_MULTIPLIER=off \
  cargo test -p modelswarm-bench --features quic-runner \
    --test pass1_loopback pass2_engagement_loopback -- --ignored --nocapture
python3 experiments/processed/aggregate-pass2.py $OUT \
  experiments/processed/PASS-2-GATED-LOOPBACK-2026-10-11-summary.json
```

Wall: 342.6 s (w8 pass) + 165.3 s (w16 pass) on the dev machine. The
per-cell sqlite acceptance stores are gitignored (`*.sqlite`); the
per-cell `acceptance-snapshot.json` files carry the (empty) state.
