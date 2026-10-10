# PASS-2 GATED LAN — 2026-10-11 — the enforced never-engage posture on the real two-machine QUIC swarm

Label: `lan-2machine-quic` (every `decision-join.jsonl` row and every
`summary.json`). Never a WAN claim. This is the LAN twin of
`experiments/raw/PASS-2-GATED-LOOPBACK-2026-10-11/` (same machinery,
same pool, both engage gates live, real cross-machine transport).

## Why this run exists

The committed pass-2 LAN run (`experiments/raw/
PASS-2-ENGAGE-LAN-2026-10-10/`) measured the engaged curve by relaxing
the cost model to the 1.5-step batch term: 1,982 engaged completions,
every one losing 1.38–1.94× (cell medians) to the fastest eligible
single. This run re-runs the same 8-cell matrix with BOTH gates live —
the ADR-032 §4 companion correction (wire-true, capability-aware
verification charging: window+1 sequential verifier tokens per round +
per-round re-post of the growing committed prefix; the batch term only
for a `batch_verify`-declared verifier, which none can be today) and
the per-profile engaged-loss EWMA engage gate (α 0.3, warm-up 3, block
at ≥ 1.0). Predicted outcome (ADR-032 §5 option A): **all engagement
blocked everywhere; fallback parity holds; no cell regresses.** That is
what happened — including the gate never letting a single speculative
round run, so the rule-6 guard and the EWMA never had anything to do.

## Run identity

- Code: `fa91ffb` (driver A side, this repo, `--features quic-runner`).
  Harness `9.6-pass2-lan`; executor `synthetic-token-executor`
  (TEST-ONLY) — planner/gate evidence, NOT a model throughput claim.
- Machines: A = 192.168.100.42 (driver, identity seed 0xB0); B =
  DESKTOP-MBQ7VBM at 192.168.100.43 running `pass1_serve.exe`
  **rebuilt from `fa91ffb`** (SHA-256
  `c44b9df2259819f28f2deeafe2b58542ad6bbe8857a82de39aff7c94eb18557c`,
  scp + Get-FileHash verified equal) with `--pool pass2 --bridges 4
  --inject-rtt-ms 0 --bind-ip 192.168.100.43`. The v4 exe B persists
  (`MSPPass1` task) predates the prefix-match family and `--pool
  pass2`, so a fresh exe was required — deployed as a SEPARATE folder
  `ModelSwarm-9.6-LAN-v6` + task `MSPPass1V6` (the documented
  scheduled-task detach pattern; processes started directly over ssh
  die with the session job). B-side wire format unchanged (msp-v1).
- Pool on B (unchanged from pass 2 for comparability): V decode 0.10 /
  prefill 3.0 / queue 0 (gpu_mid); D 4.0 / 6.0 / 300 (gpu_high); M
  0.25 / 1.2 / 400; M2 0.20 / 1.0 / 500. Corpus `synthetic-pass1-v1`,
  output target 16, margin 0.15, τ 0.0, cap 4, **30 reps/arm** (9.6
  design), 4 prompts.
- 8 cells: profiles {high, geo900, geo800, geo700} × windows {8, 16},
  600 records per cell, **4,800 total, zero failures**, 1,071 s wall.
  Rule-6 guard `off` (pass-2 instrument parity; with zero engaged
  rounds it never evaluates). One fresh serve process for the whole
  sweep (per-cell request roots are unique; deterministic request ids
  make re-runs against the same serve fail `replayed_request` — the
  task is restarted per sweep attempt, one attempt sufficed).
- Because no proposer round ever ran, the acceptance/regime dimension
  is moot in this run: the dialed profiles only shape proposer seeds,
  and the gate blocks before any propose request is sent. The cells are
  still run in full for symmetry with the committed pass-2 matrix.

## Results (medians vs the cell's fastest-single median; n=120/arm/cell)

| cell | single (ms) | gated coop range | pass-2 coop range | pass-2 worst arm | gated worst arm |
|---|---|---|---|---|---|
| inj0ms-high-w8 | 226.1 | 0.998–1.005 | 1.378–1.588 | 1.588 | 1.005 |
| inj0ms-geo900-w8 | 215.1 | 0.981–0.993 | 1.476–1.591 | 1.591 | 0.993 |
| inj0ms-geo800-w8 | 213.6 | 0.943–0.997 | 0.983–1.006 | 1.006 | 0.997 |
| inj0ms-geo700-w8 | 226.8 | 0.946–1.000 | 0.990–1.015 | 1.015 | 1.000 |
| inj0ms-high-w16 | 225.0 | 0.958–1.005 | 0.983–1.058 | 1.058 | 1.005 |
| inj0ms-geo900-w16 | 226.4 | 0.982–1.004 | 1.448–1.938 | 1.938 | 1.004 |
| inj0ms-geo800-w16 | 214.2 | 0.988–0.991 | 0.951–1.020 | 1.020 | 0.991 |
| inj0ms-geo700-w16 | 215.1 | 0.998–1.058 | 1.040–1.303 | 1.303 | 1.058 |

Aggregate (script-computed, `experiments/processed/
PASS-2-GATED-LAN-2026-10-11-summary.json`): 4,800 join rows;
**0 engaged completions** (no cooperative round ran on the wire), 3,840
cooperative rows — **all 3,840 gate fallbacks**, every reason string
naming `wire-true sequential verify` (the ADR-032 §4 term); the
engaged-loss EWMA recorded 0 samples (cold — see finding 3).

## Honest findings

1. **The gate blocked engagement on the real wire exactly as the
   wire-true term demands: 3,840/3,840 cooperative attempts, 8/8
   cells**, including the two pass-2 leak cells (high-w8, geo900-w8 —
   where pass-2's tight predictions let 125 engaged runs through the
   old default guard to lose 1.18–3.28×) and the w16 parity corner.
   Zero propose/verify wire requests were sent; acceptance stores
   stayed empty. The pass-2 never-engage conclusion is now enforced
   behavior, not advice: the same pool, same seeds, same machines that
   produced 1,982 engaged losses produce none.
2. **Fallback parity held; no cell regresses vs the committed pass-2
   numbers.** Per-cell worst cooperative medians: every cell improved
   or held (largest improvements geo900-w16 1.938 → 1.004, high-w8
   1.588 → 1.005; the mostly-fallback cells stayed at their pass-2
   levels: geo800-w8 1.006 → 0.997, geo700-w8 1.015 → 1.000). The
   gated cooperative band is **0.943–1.058** — wider than pass-2's
   mostly-fallback band (0.95–1.02) at the edges, stated plainly:
   (a) the sub-1.0 tail (0.943–0.946 at geo700-w8/geo800-w8) is
   fallback singles beating the single ARM's median — both are
   independent executions against the same fastest peer, so medians
   scatter around 1.0 by run-to-run LAN noise (n=120); (b) the 1.049–
   1.058 highs sit in geo700-w16, the LAST cell of the sweep, in the
   arms that run latest per prompt (k3/k4/planner) — consistent with
   late-sweep drift on the serve side, visible here because it is not
   hidden, and still far inside every pass-2 engaged loss.
3. **The engaged-loss EWMA never warmed — the correct layering, pinned
   separately.** The wire-true term blocks structurally BEFORE anything
   is paid, so the EWMA (the second layer, for cohorts whose
   predictions pass the margin rule) has nothing to feed on this wire.
   Its blocking behavior is pinned against THIS dataset's committed
   pass-2 twin in `crates/modelswarm-bench/tests/harness.rs`
   (`engage_loss_gate_replay_of_committed_pass2_lan_rows`): per-cell
   replay blocks 7/7 warmed cells at engaged attempt 4–5; one
   per-profile gate over the whole 1,982-attempt sweep lets exactly the
   3 warm-up attempts through and refuses 1,979 pre-round.
4. **The honest headline is a negative: on the msp-v1 whole-request
   wire, cooperative speculative mode is now formally dead for
   HTTP-only cohorts.** No engaged round may run unless a verifier
   declares `batch_verify` (no production adapter can, ADR-032 §4) AND
   the margin rule beats the fastest eligible single AND the profile's
   engaged-loss EWMA stays under 1.0. Speculative wins require the
   batch-verify engine path (ADR-032 B, Proposed; P16's measured 1.45×
   loopback with a real model is the engine-side complement).

## What this run does NOT claim

- No model-level speculative claim — TEST-ONLY synthetic executor; the
  measured quantities are planner/gate behaviors on a real transport.
- No WAN or cloud claim; sub-millisecond LAN RTT only.
- No claim that the 0.943–1.058 fallback band is a PERFORMANCE result:
  fallback rows execute the same single path as the comparator arm; the
  band measures noise, and the two edge groups are explained above.
- The pool remains the deliberately asymmetric pass-2 engagement shape
  (pinned for comparability); on symmetric pools the old gate already
  blocked everything — both postures now agree.

## Limitations / follow-ups

- The engaged-loss EWMA's no-recovery property (blocked stays blocked)
  and cold-start warm-up cost (≤ 3 engaged attempts before protection)
  are documented in `crates/modelswarm-bench/src/engage_gate.rs`; both
  need a reviewed policy before any production consumer copies the
  gate.
- The gated run cannot exercise the EWMA on the wire (by construction);
  its evidence is the committed-row replay pin, not a live firing.
- Single-transport-RTT LAN; the sync term is invisible (loopback twin
  carries the injected-delay dimension).

## Ops record (B driven over SSH; B left as found)

```bash
# A: build the serve exe fresh from fa91ffb (the persisted v4 exe
# predates --pool pass2 + the prefix-match family).
cargo build --release -p modelswarm-bench --example pass1_serve \
  --features quic-runner          # sha256 c44b9df2…eb18557c
scp target/release/examples/pass1_serve.exe \
  lanb:C:/Users/Admin/Desktop/pass1_serve_v6.exe
# B: separate folder + task (v4 + MSPPass1 untouched), then per sweep:
ssh lanb 'Stop-Process -Name pass1_serve -Force -ErrorAction SilentlyContinue; \
           schtasks /Run /TN MSPPass1V6'
ssh lanb 'Select-String -Path "$env:USERPROFILE\Desktop\ModelSwarm-9.6-LAN-v6\serve-out.log" \
           -Pattern "^MSP_BENCH_PEER" | ForEach-Object { $_.Line }' \
  | tr -d '\r' | cut -d= -f2-    # 4 lines, 5 '|'-fields each, validated

# A: the sweep (OUT is absolute).
MSP_BENCH_OUT=<abs>/experiments/raw/PASS-2-GATED-LAN-2026-10-11 \
MSP_BENCH_PEER_0..3=<B's lines> MSP_BENCH_PROFILES=high,geo900,geo800,geo700 \
MSP_BENCH_WINDOW=8 MSP_BENCH_WINDOW2=16 MSP_BENCH_RUNS=30 \
MSP_BENCH_COHORT_CAP=4 MSP_BENCH_LOSS_MULTIPLIER=off \
cargo test -p modelswarm-bench --features quic-runner \
  --test pass1_loopback pass2_lan_run -- --ignored --nocapture   # 1,070.9 s

python3 experiments/processed/aggregate-pass2.py \
  experiments/raw/PASS-2-GATED-LAN-2026-10-11 \
  experiments/processed/PASS-2-GATED-LAN-2026-10-11-summary.json

# B afterward (verified): serve stopped (0 processes), MSPPass1V6 task
# deleted, ModelSwarm-9.6-LAN-v6 folder removed; MSPPass1 + v4 intact.
```

Notes: one serve process served the whole 8-cell sweep (unique
per-cell request roots; no `replayed_request` hits — serve-err.log
shows only the expected per-cell `serving.session_ended`
connection-lost warnings at cell boundaries).
