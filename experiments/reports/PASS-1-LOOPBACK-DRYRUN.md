# PASS-1-LOOPBACK-DRYRUN — 9.6 pass-1 harness dry run (2026-10-09)

**ENVIRONMENT LABEL: loopback + injected delay (executor-seam). NOT LAN. NOT a
product performance claim.** Testbed-ladder position: in-process loopback with
deterministic delay injection at the serving-executor seam. The 2-machine LAN
run (wire authenticity) is a separate, owner-gated pass (see
`docs/reviews/handoff-scheduler-scientist-2026-10-09.md`, 9.6-harness section).

## What ran

- Harness: `modelswarm-bench` `quic-runner` feature — REAL serving bridges
  (`modelswarm-node::serving::serve_sessions` over real QUIC listeners on
  `127.0.0.1:0`), driven by the pooled `RemoteExecutor` with F15 `PeerMetrics`
  attached. The engine behind each bridge is the TEST-ONLY
  `synthetic-token-executor` (deterministic, token-aligned, speed-configured);
  every manifest's `runtime.name` says so and `harness_version` carries
  `env=loopback+injected-delay`.
- Arms from one candidate set per cell (roster facts + F15 observations warmed
  by 2 scripted completions + idle RTT probes per bridge, mapped through the
  shadow planner): `fastest-single` (independently executed comparator),
  `fixed-k2`, `fixed-k3`, `planner-k` (deterministic §5 heuristic; τ = 0.0
  placeholder; measured acceptance EWMAs from the persistent store).
- Sweep: injected request delay {5, 20} ms × acceptance regime {high, zero,
  mixed} × frozen corpus `synthetic-pass1-v1` (4 prompts) × 2 reps/arm;
  window 8, output target 16, cohort cap 3 (k=4 is spend-gated, D4).
- Bridge pool per cell: free verifier-class (decode 0.10 tok/ms), free second
  (0.08), fast-but-busy drafter (2.0, advertised queue 300 ms — the untrusted
  advertisement is charged, never trusted).

## Exact commands (Git Bash, Windows, repo root)

```bash
cargo test -p modelswarm-bench --features quic-runner --test pass1_loopback -- --ignored --nocapture
# with artifacts directed at a fresh directory:
MSP_BENCH_OUT="$PWD/target/pass1-dryrun" cargo test -p modelswarm-bench \
  --features quic-runner --test pass1_loopback -- --ignored --nocapture
python3 experiments/processed/aggregate-pass1.py \
  experiments/raw/PASS-1-LOOPBACK-DRYRUN \
  experiments/processed/PASS-1-LOOPBACK-DRYRUN-summary.json
```

Observed (2026-10-09, this machine, dev profile): 6 cells, 192 mode-result
records, run wall ~70 s. Calibration: 5/6 cells within the manifest's strict
10% tolerance; `inj5ms-zero` measured 5.94 ms vs 5 ms nominal and is recorded
`calibration_valid: false` (visible, not hidden; Windows timer noise on a
5 ms injection).

## Headline numbers (all loopback + injected delay — NOT LAN)

| Cell | fastest-single median | best cooperative ratio | worst cooperative ratio | engage outcome |
|---|---|---|---|---|
| inj5ms-high | 267.3 ms | 0.998 (fell back) | 1.002 | 24/24 fell back |
| inj5ms-zero | 321.1 ms | 0.996 (fell back) | 1.005 | 24/24 fell back |
| inj5ms-mixed | 266.6 ms | 1.002 (fell back) | 1.009 | 24/24 fell back |
| inj20ms-high | 82.4 ms | 3.357 (engaged) | 3.361 | 12/24 engaged, 12 fell back |
| inj20ms-zero | 283.6 ms | 0.996 (fell back) | 0.999 | 24/24 fell back |
| inj20ms-mixed | 93.4 ms | 1.007 (fell back) | 1.070 | 23/24 fell back |

Aggregate (script-computed, `experiments/processed/`): 192 records;
131/144 cooperative runs fell back at the engage gate; the 13 engaged runs
(all in `inj20ms-high`/`inj20ms-mixed`) realized 1.07×–3.36× the
independently executed fastest single.

**Negative result (first-class):** in this loopback configuration no
cooperative arm beat the fastest eligible single host. Where the cost model
engaged (`inj20ms-high`: injected delay inflates the single's measured-queue
charge), the realized wire rounds ran 3.36× slower than the single — the
structural per-round cost of re-posting the whole prefix over the request/
reply wire (audit S3), exactly what pass 1 exists to measure. Where the model
refused to engage, the planner recorded visible fallbacks (ratio ≈ 1.0 means
"ran the single path") with machine-readable reasons in
`decision-join.jsonl`.

Measured acceptance tracked the regime ground truth (real wire rounds,
client-side delta comparison): high ≈ 1.00, mixed ≈ 0.667 (ground truth 2/3),
zero cells never engaged so no acceptance was recorded there.

## Honest limitations

- Windows timer granularity: injected delays use a sleep+bounded-spin shim
  (`precise_delay`) for sub-tick accuracy; 5 ms cells still show ±0.7 ms
  noise (one cell outside the strict 10% band, recorded as such). The 20 ms
  cells calibrated cleanly.
- The synthetic executor is token-aligned, so delta comparison is exact
  here; the production executor batches up to 16 tokens per delta, where a
  partial in-delta mismatch conservatively rejects the whole delta
  (documented pass-1 limitation; pass 2 / engine adapter removes it).
- Warm-up contention on a 4-worker runtime inflates measured-queue estimates
  under concurrency (spinning serving tasks); visible in
  `candidate-set.json`. The LAN pass has real machines and real RTT.
- `acceptance.sqlite` (persistent store) is gitignored by pattern; the
  per-cell `acceptance-snapshot.json` files carry the state.

## Raw evidence

- Per cell under `experiments/raw/PASS-1-LOOPBACK-DRYRUN/<cell>/`:
  `manifest-<arm>.json` (run-manifest/v1), `mode-results.jsonl`
  (mode-result/v1), `decision-join.jsonl` (request-id-level decision↔
  completion joins incl. planner sweeps and fallback reasons),
  `calibration.json`, `candidate-set.json` (measured decision inputs +
  ranking), `acceptance-snapshot.json`, `summary.json`.
- Aggregate: `experiments/processed/PASS-1-LOOPBACK-DRYRUN-summary.json`
  (script: `experiments/processed/aggregate-pass1.py`).
- Frozen corpus: `experiments/manifests/prompt-corpus-synthetic-pass1-v1.json`
  (digest-pinned to the compiled-in set by unit test).
