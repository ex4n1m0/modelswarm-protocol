# PASS-2 ENGAGEMENT (loopback) — 2026-10-10 — speculative arms finally ENGAGE, honestly

Label: `loopback+injected-delay` (every `decision-join.jsonl` row and every
`summary.json`). NEVER a LAN claim — the LAN twin of this run lives at
`experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/`.

## Why this run exists (pass-1's honest gap)

Pass 1 (loopback dry run + two LAN runs) measured the all-fallback COST:
its speculative arms almost never ran a wire round, so acceptance curves,
window trade-offs, and engaged win/loss numbers did not exist. Pass 2
makes arms ENGAGE — through the frozen ADR-013 engage gate, never around
it — by (a) an engagement-shaped pool (V = fastest single: fast prefill,
slow decode; D = 40×-decode drafter, queue-penalized so it is drafted
from and never the comparator; M/M2 = busy mids) and (b) a DIALABLE
acceptance knob implemented at the protocol level: the driver encodes a
match length in the proposer's request seed (prefix-match family), the
serve-side synthetic executor emits the reference continuation for that
prefix and its own stream after, and acceptance stays MEASURED from real
propose → transport → verify → leading-match rounds. Nothing is fed from
a store; the EWMA only learns what the wire produced.

## Run identity

- Code: `4faa6c0` (+ same-commit doc-rate correction). Harness
  `9.6-pass2`; executor `synthetic-token-executor` (TEST-ONLY) behind
  REAL local QUIC serving bridges (loopback dial, injected executor-seam
  delay).
- Pool (all cells): V `0.10` tok/ms decode / `3.0` prefill / queue 0;
  D `4.0`/`6.0`/queue 300 (gpu_high); M `0.25`/`1.2`/400; M2
  `0.20`/`1.0`/500. Corpus `synthetic-pass1-v1` (4 prompts), output
  target 16 tokens, margin 0.15, τ 0.0, cohort cap 4, 8 reps/arm
  (32 samples per arm per cell).
- 12 cells: injected {5, 20} ms × profiles {high, geo900, geo800,
  geo700} at window 8, plus window 16 at 5 ms. Profiles are geometric
  per-token match probabilities; implied acceptance RATES at w8 are
  ≈1.0 / 0.64 / 0.42 / 0.27 (classic model `p(1-p^w)/((1-p)w)`).
- Rule-6 observed-loss guard: `MSP_BENCH_LOSS_MULTIPLIER=off` for this
  instrument — every engaged row records the multiplier AND the
  counterfactual `would_fire_at_default` (production multiplier 2.0).
- Per-cell acceptance isolation (fresh harness + store per cell): a
  cross-profile EWMA carryover would flip gate-blocked profiles into
  engaged ones — a confound, not a feature. Pass 1 deliberately resumed
  one store; pass 2 records the isolation instead.

## Headline numbers (medians vs the cell's fastest-single; n=32/arm/cell)

| cell | single (ms) | k2 | k3 | k4 | planner-k | k-arm fallbacks |
|---|---|---|---|---|---|---|
| inj5ms-high-w8 | 232.4 | 1.210 | 1.236 | 1.376 | 1.223 | 0 |
| inj5ms-geo900-w8 | 231.8 | 1.535 | 1.635 | 1.814 | 1.694 | 0 |
| inj5ms-geo800-w8 | 239.1 | 1.033 | 1.030 | 1.033 | 1.000 | 27/32 |
| inj5ms-geo700-w8 | 239.2 | 1.028 | 1.025 | 1.028 | 1.037 | 31/32 |
| inj20ms-high-w8 | 248.7 | 1.263 | 1.206 | 1.470 | 1.205 | 0 |
| inj20ms-geo900-w8 | 247.1 | 1.641 | 1.649 | 1.831 | 1.702 | 0 |
| inj20ms-geo800-w8 | 256.1 | 1.022 | 1.022 | 1.021 | 0.995 | 27/32 |
| inj20ms-geo700-w8 | 254.4 | 1.086 | 1.082 | 1.083 | 1.059 | 19/32 |
| inj5ms-high-w16 | 231.9 | 0.952 | 0.921 | 0.999 | 0.920 | 0 |
| inj5ms-geo900-w16 | 232.4 | 1.686 | 1.825 | 2.005 | 1.567 | 0 |
| inj5ms-geo800-w16 | 238.9 | 1.034 | 1.032 | 1.040 | 0.999 | 27/32 |
| inj5ms-geo700-w16 | 232.6 | 2.661 | 2.755 | 1.273 | 1.272 | 9–16/32 |

Aggregate (script-computed, `experiments/processed/
PASS-2-ENGAGE-LOOPBACK-2026-10-10-summary.json`): 1,920 join rows;
**946 engaged completions** (a cooperative round ran on the wire), 590
gate fallbacks, 0 failures. Against the STRICTEST comparator — the best
independently-executed single completion of the same prompt in the same
cell — **no engaged arm ever won**: best engaged median ratio **1.047**
(inj5ms-high-w16 planner-k ≈ parity by construction, see finding 2),
worst **3.579** (inj5ms-geo700-w16).

## Honest findings

1. **Engagement works and is measurable.** high/geo900 engaged every
   run (0 fallbacks) at window 8; geo800/geo700 mostly fell back AT THE
   GATE after the measured acceptance EWMA warmed (27–31/32) — the
   frozen margin rule refusing to speculate against measured low
   acceptance is the never-worse property doing its job. The
   deterministic planner arm REFUSED geo800 entirely (32/32 fallback at
   w8) while fixed-k engaged the cold-prior early runs — measured
   acceptance making the planner more conservative than fixed-k is
   exactly the adaptive behavior 9.6 set out to test.
2. **Parity is the structural best case on this wire; wins do not
   exist.** At window 16 with acceptance 1.0 the engaged run is ONE
   round in which the verifier executes window+1 = 17 tokens vs the
   single's 16 — parity by construction (measured 0.92–1.05× vs the
   cell median; 1.047× vs the strict prompt-best comparator). Every
   other engaged point is a visible loss that grows with rounds:
   1.2× (high, w8, 2 rounds) → 1.5–1.8× (geo900, ~3 rounds) → 2.7–3.6×
   (geo700-w16, ~5 rounds). The verifier executes window+1 real tokens
   per round on the same peer class that defines the fastest single —
   an engaged run pays ≥ the single's decode cost per round.
3. **The production rule-6 guard would have aborted nearly every
   engaged run at round 1**: 911/946 guard-carrying rows have
   `would_fire_at_default=true`. The frozen cost model charges
   verification as `VERIFY_BATCH_STEPS=1.5` verifier decode steps per
   round (the batch-verify ENGINE model, ADR-013); the request/reply
   wire charges window+1 sequential tokens. Realized/predicted for
   engaged runs: **2.9×–7.4×** (median per cell). This divergence is
   the pass-2 quantification of the S3 structural cost — on msp-v1's
   whole-request wire, the planner's verification term is wrong by
   (window+1)/1.5 ≈ 6× (w8) to 11× (w16) unless the engine can batch
   (the P16 C-API adapter's measured 1.45× loopback win is that
   engine-side story; this run is the wire-side story).
4. **Acceptance is measured and matches the model**: high 1.000,
   geo900-w8 0.720, geo800-w8 0.528, geo700-w8 0.33–0.41, geo900-w16
   0.502–0.591 (implied rates shift with window exactly as the classic
   model predicts — larger windows lower the rate at fixed p). The
   acceptance-EWMA store recorded every proposer's rounds.
5. **k costs are visible when engaged**: k4 is consistently the worst
   engaged arm (1.38–2.0× at high/geo900) — extra proposers add wire
   traffic without improving the committed rate in this linear-verify
   world (all proposers receive the same seed, so additional proposers
   cannot raise acceptance here; multi-proposer diversity needs the
   tree-verify engine, which the public C API cannot express).

## What this run does NOT claim

- No model-level speculative claim: the executor is TEST-ONLY synthetic
  (deterministic token streams at configured speeds). The numbers
  measure the PROTOCOL/PLANNER behavior — engagement, gate decisions,
  acceptance measurement, fallback cost — not inference throughput.
- No LAN/WAN claim: loopback + executor-seam injected delay only.
- The pool is deliberately asymmetric to let the gate pass; on
  symmetric pools (pass 1) the gate blocks everything, which remains
  the correct production posture per finding 3.

## Limitations / follow-ups

- 8 reps/arm (32 samples per arm-cell) — medians are stable across the
  12 cells but CIs are not bootstrapped here; the LAN twin runs 30
  reps/arm.
- The rule-6 guard was disabled to measure the engaged curve; with it
  enabled (production default), every engaged run here would abort at
  round 1 and complete as single — i.e., today's production stack is
  protected from exactly the losses this run exhibits.
- Window is capped at 16 by the harness bound; the parity point at
  w=target suggests testing w ≥ target only matters as the
  acceptance-1.0 corner.

## Ops record

```bash
OUT=experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10
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
  experiments/processed/PASS-2-ENGAGE-LOOPBACK-2026-10-10-summary.json
```

Wall: ~7 min (w8 pass) + ~4 min (w16 pass) on the dev machine. The
per-cell sqlite acceptance stores are gitignored (`*.sqlite`); the
per-cell `acceptance-snapshot.json` files carry the state.

## Record correction (pass-1 mechanism)

Pass-1's ANALYSIS files state the LAN all-fallback happened because
"the serve-side synthetic bridges are seeded per-bridge, so proposer
drafts never match the driver-side verifier continuation". That
mechanism is wrong and this pass corrects it: the synthetic executor's
fold keys on the REQUEST seed only (the bridge seed keys identity and
speeds, not the token stream), so drafts match whenever request seeds
match — the loopback dry run's measured high-acceptance ~1.0 and the
two engaged 4-bridge LAN runs (matching drafts) both contradict the
seed-mismatch story. The pass-1 LAN arms fell back at the ENGAGE GATE
(every fallback row's reason string says so: cooperative prediction ×
1.15 vs fastest single), because the pass-1 pools were
symmetric/slow-drafter shaped. Pinned by
`runner::tests::non_divergent_peers_agree_per_request_seed`.
