# PASS-2 ENGAGEMENT LAN — 2026-10-10 — engaged speculative runs on a real two-machine QUIC swarm

Label: `lan-2machine-quic` (every `decision-join.jsonl` row and every
`summary.json`). Never a WAN claim. This is the LAN twin of
`experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10/` (same machinery,
same pool, real cross-machine transport instead of injected delay).

## Run identity

- Code: `4faa6c0` + same-commit doc-comment corrections (no functional
  delta vs `4faa6c0`). Driver = `pass2_lan_run` (A side, this repo's
  test harness, `--features quic-runner`). Harness `9.6-pass2-lan`;
  executor `synthetic-token-executor` (TEST-ONLY) — protocol/planner
  evidence, NOT a model throughput claim.
- Machines: A = 192.168.100.42 (driver, identity seed 0xB0); B =
  DESKTOP-MBQ7VBM at 192.168.100.43 running `pass1_serve.exe`
  **rebuilt from this commit** (SHA-256
  `d26d487f4142f2b42416e542929a41250220e286c0bb04a7afbc7021fa1164e7`,
  scp-verified equal) with `--pool pass2 --bridges 4 --inject-rtt-ms 0
  --bind-ip 192.168.100.43`. The executor change is serve-side
  behavior (prefix-match family); a v4-era serve would emit
  non-matching drafts for family seeds — hence the fresh exe. B-side
  wire format is unchanged (msp-v1 requests).
- Pool on B (engagement shape): V decode 0.10 / prefill 3.0 tok/ms /
  queue 0 (gpu_mid); D 4.0 / 6.0 / 300 (gpu_high); M 0.25 / 1.2 / 400;
  M2 0.20 / 1.0 / 500. Corpus `synthetic-pass1-v1`, output target 16,
  margin 0.15, τ 0.0, cap 4, **30 reps/arm** (9.6 design), 4 prompts.
- 8 cells: profiles {high, geo900, geo800, geo700} × windows {8, 16},
  600 records per cell, **4,800 total, zero failures**, 1,435 s wall.
- Acceptance is DIALED by the driver through the prefix-match seed
  family and MEASURED on the wire (real propose request → real verifier
  request → client-side leading match; acceptance-EWMA recorded per
  proposer). Rule-6 guard `off` for the engaged-curve instrument; every
  engaged row records `would_fire_at_default` (production multiplier
  2.0). Per-cell acceptance isolation (fresh store per cell).

## Results (medians vs the cell's fastest-single median; n=120/arm/cell)

| cell | single (ms) | fixed-k2 | fixed-k3 | fixed-k4 | planner-k | k-arm fallbacks |
|---|---|---|---|---|---|---|
| inj0ms-high-w8 | 213.9 | 1.378 | 1.384 | 1.588 | 1.397 | 0 |
| inj0ms-geo900-w8 | 224.3 | 1.591 | 1.476 | 1.532 | 1.544 | 26/120 |
| inj0ms-geo800-w8 | 213.5 | 1.005 | 1.006 | 1.006 | 0.983 | 115/120 |
| inj0ms-geo700-w8 | 208.6 | 0.998 | 0.995 | 1.015 | 0.990 | 119–120/120 |
| inj0ms-high-w16 | 228.6 | 1.022 | 0.983 | 1.058 | 0.994 | 0 |
| inj0ms-geo900-w16 | 225.4 | 1.780 | 1.754 | 1.938 | 1.448 | 4–30/120 |
| inj0ms-geo800-w16 | 229.5 | 1.013 | 1.020 | 1.018 | 0.951 | 115–120/120 |
| inj0ms-geo700-w16 | 235.2 | 1.303 | 1.210 | 1.040 | 1.043 | 60–90/120 |

Aggregate (script-computed, `experiments/processed/
PASS-2-ENGAGE-LAN-2026-10-10-summary.json`): 4,800 join rows;
**1,982 engaged completions** (a cooperative round ran on the real
wire), 1,858 gate fallbacks, 0 failures. Against the strictest
comparator (best independently-executed single completion of the same
prompt in the same cell): **no engaged arm won** — best engaged median
ratio **1.071** (inj0ms-high-w16, parity by construction), worst
**4.257** (inj0ms-geo700-w16).

## Honest findings

1. **Speculative arms engage end-to-end on a real two-machine swarm**
   — 1,982 wire-round completions over real QUIC + Noise + ADR-026
   leases, with measured acceptance exactly tracking the dialed
   profiles: high 1.000, geo900-w8 0.741 (372 engaged runs),
   geo800-w8 0.528 (15), geo700-w8 0.275 (2 — the gate blocked
   119–120/120 there, so the number is consistent with the classic
   model's 0.275 for p=0.7, w=8 but thinly sampled), and window
   effects as predicted (geo900 drops to 0.567 at w16; geo700-w16
   0.248).
2. **Engaged means slower here — visibly.** Every engaged cell loses to
   the fastest single: high-w8 1.38–1.59×, geo900-w8 1.48–1.59×,
   geo900-w16 1.45–1.94×, geo700-w16 up to 4.26× engaged-median. The
   structural reason is the wire itself: each round re-posts the whole
   prefix and the verifier executes window+1 sequential tokens —
   per-round cost ≥ the single's per-token decode on the same peer
   class. Window 16 + acceptance 1.0 is the parity corner (one round,
   17 vs 16 verifier tokens: measured 0.98–1.06× cell-median, 1.071×
   vs prompt-best — parity, not a win).
3. **The engage gate protects exactly where it should.** geo800/geo700
   at w8 fell back 115–120/120 — the frozen margin rule over MEASURED
   acceptance refuses to speculate at ~0.3–0.5 acceptance on this pool;
   the never-worse property shows in the mostly-fallback cells' ratios
   (0.95–1.02×, noise-level). The deterministic planner was again more
   conservative than fixed-k (geo900-w16: planner 30/120 fallback vs
   fixed-k 4/120; geo800 cells: planner 120/120) — measured acceptance
   EWMA makes the adaptive arm refuse low-value engagement.
4. **The production rule-6 guard would have aborted 1,857/1,982
   engaged completions at round 1** (`would_fire_at_default=true`).
   Realized/predicted per engaged cell: **2.8×–8.3×** — the frozen
   `VERIFY_BATCH_STEPS=1.5` verification term undercharges the
   request/reply wire's window+1-token sequential verification by
   ~(window+1)/1.5 (6× at w8, 11× at w16). With a wire-true
   verification term, every engagement in this run would have been
   gate-blocked. Consequence for the planner: on msp-v1's
   whole-request wire, adaptive cohort sizing should never engage
   speculation; wins require a batch-verify engine (P16 C-API measured
   1.45× on loopback with a real model — the engine-side story this
   wire-side run complements).
5. **k-size when engaged: more proposers cost visibly.** fixed-k4 is
   the worst engaged arm in 6/8 cells (up to 1.94×) — with linear
   client-side verification and identical seeds per proposer, extra
   cohort members add traffic, not acceptance. (Pass-1's flat-in-k
   finding covered the all-fallback world; the engaged world is NOT
   flat in k.)

## What this run does NOT claim

- No model-level speculative claim — TEST-ONLY synthetic executor; the
  measured quantities are protocol/planner/scheduling behaviors
  (engagement, gating, acceptance measurement, per-round wire costs).
- No WAN or cloud claim; sub-millisecond LAN RTT only.
- The pool is deliberately asymmetric (V fastest-single shape) so the
  frozen gate can legitimately pass; on pass-1's symmetric pools the
  gate blocks everything, which per finding 4 is the correct production
  posture until a batch-verify engine exists.

## Limitations / follow-ups

- 125 engaged runs passed even the default guard (tight-prediction
  cells high-w8/geo900-w8 where realized/predicted stayed under
  2.3×) — the guard threshold's calibration against the wire is itself
  measurable from these rows.
- Single-transport-RTT (~sub-ms) LAN: the sync term is invisible; WAN
  or injected-delay cells (loopback twin) carry that dimension.
- The proposer-side cost is unrealistically cheap (drafter at 40× the
  verifier's decode); a real-draft-model proposer (P16-class engine on
  B) is the pass-3 step that can turn the parity corner into a win —
  with batched verify, which this wire cannot express.

## Ops record (B driven over SSH; reusable)

```bash
# B: fresh serve per sweep attempt (deterministic request ids +
# replayed_request dedup; leases minted fresh with the process).
ssh lanb 'Stop-Process -Name pass1_serve -Force -ErrorAction SilentlyContinue; \
           schtasks /Run /TN MSPPass1V5'   # launches v5\serve-headless.bat
ssh lanb 'Select-String -Path "$env:USERPROFILE\Desktop\ModelSwarm-9.6-LAN-v5\serve-out.log" \
           -Pattern "^MSP_BENCH_PEER" | ForEach-Object { $_.Line }' \
  | tr -d '\r' | cut -d= -f2-   # 4 lines, 5 '|'-fields each, validated

# A: the sweep (this run; OUT is absolute).
MSP_BENCH_OUT=<abs>/experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10 \
MSP_BENCH_PEER_0..3=<B's lines> MSP_BENCH_PROFILES=high,geo900,geo800,geo700 \
MSP_BENCH_WINDOW=8 MSP_BENCH_WINDOW2=16 MSP_BENCH_RUNS=30 \
MSP_BENCH_COHORT_CAP=4 MSP_BENCH_LOSS_MULTIPLIER=off \
cargo test -p modelswarm-bench --features quic-runner \
  --test pass1_loopback pass2_lan_run -- --ignored --nocapture

python3 experiments/processed/aggregate-pass2.py \
  experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10 \
  experiments/processed/PASS-2-ENGAGE-LAN-2026-10-10-summary.json

# B afterward: serve stopped, MSPPass1V5 task + v5 folder removed
# (B left as found: only the untouched v4 setup remains).
```

Notes: one earlier sweep attempt against a just-started serve failed in
warm-up (transient; discarded; fresh serve re-run per the ops rule —
the failed attempt's artifacts were deleted before this run). Serve
telemetry (`serve-err.log`) shows only expected per-cell
`serving.session_ended` connection-lost warnings at cell boundaries.

## Record correction (pass-1 mechanism)

See the loopback twin's record correction: pass-1's "per-bridge seeds
never match" explanation for the LAN all-fallback was wrong (the
executor folds the REQUEST seed only); the pass-1 arms fell back at the
engage gate. This run proves drafts match under seed parity on the real
wire — 1,982 engaged completions with acceptance tracking the dialed
profiles to three digits.
