# Benchmark Harness Specification (frozen in Phase A; executable from Phase C)

Owner: Scheduler Scientist · Crates: `modelswarm-bench`, `modelswarm-scheduler` ·
Schemas: `experiments/schemas/` · Rule: ADR-013 (fastest-single comparator,
honest negative results). **Phase A freezes definitions; nothing runs until
Phases B–C deliver transport + runtime** — recorded deliberately.

## Comparator (the scientific rule)

Every cooperative measurement is compared with the **fastest eligible single
host available to that requester at that moment**, measured in the same run
cell: same profile, prompt corpus, generation parameters, network condition.
`single` mode must be executed in every cell; its best-of selection is
recorded (`fastest_single_prediction` and `fastest_single_actual`).

## Frozen network matrix

| Dimension | Values |
|---|---|
| RTT | loopback/LAN, 5, 10, 20, 40, 80, 150 ms |
| Jitter | none, low (±10% RTT), high (±30% RTT) |
| Packet loss | 0%, 0.1%, 1%, 3% |
| Path | direct, relayed (Phase F+) |
| Hardware | symmetric, asymmetric (fast verifier + slow proposers and inverse) |

Impairment is applied by the harness (userspace shaper), calibrated with a
round-trip measurement recorded in every run manifest; a cell whose measured
RTT deviates >10% from nominal is marked invalid, not silently accepted.

## Modes under test

`single` (mandatory), `hedged` (2, 3 peers), `speculative_exact` (2 and 4
peers; window sweep 1/2/4/8/16), multi-proposer tree (Phase E, when live).

## Statistical protocol (frozen before any comparison)

- Warm-up: 5 unrecorded runs per cell.
- 30 recorded runs per cell; fixed seed list committed with the harness.
- Report median, p10/p90, IQR; comparisons use medians with bootstrap 95% CIs
  (10 000 resamples); a mode "wins" only when its CI excludes the
  comparator's median.
- Outliers are kept and reported; removal requires a written rule match
  (e.g., harness crash), never judgment.
- Every report includes failures, fallbacks, and regressions with the same
  visibility as wins (ADR-013; risk R21).

## Records

Machine-readable JSON conforming to `experiments/schemas/`:
`run-manifest` (environment pinning: profile id, runtime build, hardware
class, OS, impairment calibration, seeds, harness version) and
`mode-result` (per-run metrics per ADR-13's telemetry vocabulary). Raw files
land in `experiments/raw/` (gitignored when large; CI retains artifacts),
aggregates in `experiments/processed/`, human reports in
`experiments/reports/` citing raw paths. No number is ever copied by hand.
