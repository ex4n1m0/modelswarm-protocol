# ACCURACY-DIVERGENCE-LOOPBACK (2026-10-11) — ANALYSIS

Cross-peer greedy divergence detection, loopback structural cell
(environment label `loopback-quic`; never a LAN or WAN claim). Part of
the accuracy-benefits study (`docs/reviews/accuracy-benefits-study-2026-10-11.md`).

## What this run measures

The PROTOCOL-LEVEL detection machinery — inject a divergent bridge,
detect, report, de-rank — over four REAL serving bridges on real QUIC
(loopback), synthetic token executors. The synthetic executor is
deterministic BY CONSTRUCTION, so these numbers are structural evidence
for the instrument, NOT an engine-divergence-rate claim (real-engine
rates are measured in
`experiments/raw/ACCURACY-DIVERGENCE-REALENGINE-2026-10-11/`).

## Setup

- Commit: `bde7eca` (`crates/modelswarm-bench` divergence module +
  `tests/accuracy.rs` driver).
- Pool: the pinned accuracy pool (V, D, M, M2 — pass-2 shape; fixed
  seeds 0x51/0x66/0x77/0x88; indexes in `accuracy-manifest.json`).
- Cells: `clean` (no injected fault) and `injected` (pool index 2
  divergent — the determinism-failure/hostile-peer fault model).
- Checks: 4 prompts (frozen `synthetic-pass1-v1` corpus) x 2 seeds; each
  check decodes the SAME prompt+seed on all 4 bridges concurrently,
  16-token budget; every probe compared against the bridge-0
  verifier-reference AND the quorum majority.
- Comparator/ground truth: the serve-side `--divergent` injection IS the
  ground truth; the driver self-checks detection == injection.

Command (exact):

```text
MSP_BENCH_OUT="<repo>/experiments/raw/ACCURACY-DIVERGENCE-LOOPBACK-2026-10-11" \
  cargo test -p modelswarm-bench --features quic-runner --test accuracy \
  accuracy_divergence_loopback -- --ignored --nocapture
```

Per-push CI pin of the same machinery (tempdir, injected only):
`cargo test -p modelswarm-bench --features quic-runner --test accuracy
divergence_detection_over_real_quic`.

## Results (script-computed; `experiments/processed/ACCURACY-2026-10-11-summary.json`)

| Cell | Checks | Peer-rows | Diverge | Quorum diverge | Suspended | Check wall (median/p90 ms) |
|---|---|---|---|---|---|---|
| clean | 8 | 24 | **0** (0 false positives) | 0 | 0 | 236.8 / 300.4 |
| injected | 8 | 24 | **8/8 at position 0** (0 false negatives) | 8/8 | exactly index 2 | 235.5 / 301.6 |

- Ledger (`agreement-ledger.json`): injected cell — offender 8 checks /
  8 divergences / agreement EWMA 0.0; every honest probe 8/8 agree,
  EWMA 1.0. Clean cell: 24/24 agree, EWMA 1.0.
- De-rank (`cohort-derank.json`): without detection the cohort is
  (V, D, M-offender); with detection the offender is suspended and M2 is
  promoted — roster composition changes, not just a log line. Clean
  cell: cohorts identical before/after.
- Redundant-racing availability (same completions, no extra requests):
  median tail saved vs having bet on the slower of the two peers =
  97.0 ms (clean) / 80.5 ms (injected); raced min median 129.7 / 142.2 ms
  vs slowest-peer median 226.4 / 224.7 ms. Structural numbers on
  synthetic speeds — availability shape, not a product claim.

## Findings

1. **Specificity:** 0/24 false positives on the clean control cell (and
   16/24 honest probes unflagged inside the injected cell).
2. **Sensitivity:** 8/8 detection of the injected divergent peer, always
   at divergence position 0, on both the verifier-reference and the
   reference-free quorum read.
3. **De-rank is deterministic and evidence-driven:** suspend after first
   divergence (no recovery policy — honest limitation, same posture as
   the engaged-loss engage gate).

## NOT claimed

- No engine divergence rates (synthetic executors are deterministic by
  construction).
- No latency improvements (nothing here races the serving path; the
  check is a separate diagnostic request set).
- No WAN/natural-network behavior (loopback QUIC, no injected delay).

## Reproduce

At commit `bde7eca` run the exact command above; artifacts are
byte-comparable except wall-clock fields. Aggregation:
`python experiments/processed/aggregate-accuracy.py experiments/raw
experiments/processed/ACCURACY-2026-10-11-summary.json`.
