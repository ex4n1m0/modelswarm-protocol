# Review: rule-6 observed-loss guard calibration (pass-2 LAN evidence) — 2026-10-10

Reviewer: Test and Release Engineer (independent seat). Routed by the
Integrator. Reviewed object: the Scheduler Scientist's pass-2 engaged-curve
instrument at `4faa6c0` + `3dbc161` evidence, specifically the
`MSP_BENCH_LOSS_MULTIPLIER` parameterization and the per-row
`LossGuardRecord` / `would_fire_at_default` counterfactual, and the open
risk "threshold calibration study suggested". This is a recommendation to
the owner/Integrator. **No production guard code was changed by this
review.**

Inputs: `experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/` (8 cells, 4,800
committed decision-join rows), `experiments/processed/aggregate-pass2.py`,
`experiments/processed/PASS-2-ENGAGE-LAN-2026-10-10-summary.json`,
`crates/modelswarm-bench/src/{harness,params}.rs` at `3dbc161`/`bbde8f6`.

Guard semantics under review (code, `crates/modelswarm-bench/src/harness.rs`
~lines 1026–1088): on the FIRST cooperative round only, after the round's
wire work completes, the guard compares the realized round-1 wall to
`multiplier × (1 + margin) × predicted_round` with `margin = 0.15`
(`DEFAULT_CONFIDENCE_MARGIN`) and `predicted_round = prediction /
rounds_expected`. The default production multiplier is 2.0, so the firing
edge is `round1_wall > 2.3 × predicted_round`. Firing aborts to a fresh
single run (`rule 6`: pre-first-committed-token — but the round-1 wire work
and wall-clock are already paid).

## 1. The 125 guard-passing engaged runs (extraction from committed artifacts)

Discriminator exactly per `experiments/processed/aggregate-pass2.py`:
engaged = join row with `loss_guard` present and `status == "Completed"`
(a `rounds > 1` test would miss single-round engaged completions). Verifier
totals re-derived independently: 4,800 rows, 8 cells, 0 failures (2,942
Completed = 1,982 engaged + 960 fastest-single; 1,858 FellBackToSingle);
1,982 engaged, of which **1,857 (93.7%) carry `would_fire_at_default=true`**
and **125 (6.3%) passed the default 2.0× guard**. All 1,982 engaged rows
record `multiplier: "off"` (the recorded relaxation, never silent). The
recorded counterfactual flag re-derives arithmetically from each row's own
numbers (`round1_wall > 2.0 × 1.15 × predicted_round`) with **0 mismatches
in 1,982 rows**.

Extraction script (executable; run from the repo root, output below is from
commit `bbde8f6` artifacts):

```python
# extract_guard_pass_rows.py — guard-calibration review 2026-10-10
import json, statistics
from pathlib import Path
from collections import Counter, defaultdict
ROOT = Path("experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10")
MARGIN, DEFAULT_M = 0.15, 2.0
rows = []
for p in sorted(ROOT.glob("*/decision-join.jsonl")):
    for line in p.read_text(encoding="utf-8").splitlines():
        r = json.loads(line); r["_cell"] = p.parent.name; rows.append(r)
single_best = {}
for r in rows:  # the inviolable comparator: best independently-executed
    if r["arm"] == "fastest-single" and r["status"] == "Completed":  # single of the SAME prompt
        k = (r["_cell"], r["prompt_id"])
        single_best[k] = min(single_best.get(k, 1e18), r["realized_total_ms"])
engaged = [r for r in rows if r.get("loss_guard") and r["status"] == "Completed"]
passed = [r for r in engaged if not r["loss_guard"]["would_fire_at_default"]]
for r in passed:
    g = r["loss_guard"]
    r["_gr"] = g["round1_wall_ms"] / g["predicted_round_ms"]       # what the guard sees
    r["_lr"] = r["realized_total_ms"] / single_best[(r["_cell"], r["prompt_id"])]  # the promise metric
print(len(rows), len(engaged), len(passed))
print(Counter(r["_cell"] for r in passed), Counter(r["arm"] for r in passed))
print(Counter(r["rounds"] for r in passed))
print(stats := "guard ratio", min(r["_gr"] for r in passed),
      statistics.median(r["_gr"] for r in passed), max(r["_gr"] for r in passed))
print("loss ratio", min(r["_lr"] for r in passed),
      statistics.median(r["_lr"] for r in passed), max(r["_lr"] for r in passed))
```

Characterization of the 125:

| dimension | concentration |
|---|---|
| cell | `inj0ms-high-w8` 81, `inj0ms-geo900-w8` 44 — **only these two; zero w16 rows passed** |
| arm | planner-k 44, fixed-k2 40, fixed-k3 35, fixed-k4 6 (k3/k4 live almost entirely in geo900-w8) |
| rounds | 2 (106), 3 (13), 4 (6) — the ≥2.0× loss subset (15 rows) is all geo900-w8, mostly k3/k4 at rounds 3–4 |
| guard ratio `round1_wall/predicted_round` | 1.770 – 2.300 (p50 2.123); implied multiplier at the firing edge (`ratio/1.15`) 1.539 – 2.000 |
| loss ratio vs prompt-best single | **1.179 – 3.278 (p50 1.397); zero wins; every one of the 125 lost ≥ 1.179×** |
| absolute excess vs prompt-best single | high-w8: p50 +80 ms, max +149 ms (~174 ms singles); geo900-w8: p50 +154 ms, **max +487 ms** (fixed-k3, 4 rounds, 701.4 ms vs 214.0 ms) |
| arm-median loss among the 125 | k2 1.376, planner 1.377, k3 1.608, k4 1.699 |
| fired population for contrast | guard ratio 2.300 – 81.03 (p50 3.962) |

Stated plainly: **a 2.0× threshold lets 6.3% of engaged runs through, and
everything it lets through still loses** — median +39.7%, worst +227.8%
(3.278×, i.e. +487 ms on a ~214 ms single) versus the fastest eligible
single. The leak is not a tail of jitter: it is two whole cells (high-w8,
geo900-w8) whose *predictions were tight* (realized/predicted round ≤ 2.3)
while the *engaged mode itself loses* on this wire.

## 2. Guard reaction semantics vs the release promise — recommendation

The release promise: a micro-swarm never does meaningfully worse than the
fastest eligible single host. Three facts from the extracted rows decide
the calibration question:

1. **The guard's key is uncorrelated with the promise metric.** The guard
   compares realized round-1 wall to the *cooperative prediction*; the
   promise is about the *single host*. The prediction model undercharges
   msp-v1's whole-request verification by ~(window+1)/1.5 (6× at w8, 11× at
   w16), so the guard fires on wins as easily as on losses: the dataset's
   107 row-level engaged wins (all high-w16, the parity corner) have guard
   ratios 2.866–4.147 — **the default guard would have aborted every one of
   them**. No multiplier fixes a wrong key.
2. **Tightening catches everything and helps almost nothing.** To catch all
   125, the multiplier must drop below 1.539 (the smallest implied
   multiplier among them); m = 1.5 (edge 1.725× predicted) fires on
   **1,982/1,982 engaged rows** — identical set to "never engage", minus
   the 125 whose outcome changes from engaged-through to abort. That change
   is a wash-to-negative at the median: the pass-through rows' round-1 wall
   alone is 57.7–84.1% of a whole single run, so the code path
   `fired → fresh run_single` costs ≈ 1.577–1.841× (p50 1.673×) versus
   their actual engaged outcome p50 1.397× — **aborting costs ~0.26× MORE
   at the median**; it only caps the tail (3.278× → ~1.84×). Reaction
   after round 1 bounds damage at ≈ 1 + round1/single (1.6–2.1× on this
   pool — geo700-w16 would cap at ≈ 2.07× vs the realized 4.13×) but can
   never reach "never meaningfully worse".
3. **The only mechanism that held the promise in this dataset is the engage
   gate** (mostly-fallback cells realized 0.95–1.02×), and its blind spot
   is exactly the two leak cells: acceptance there was high (1.000 and
   0.741 measured), so the acceptance-EWMA margin rule engaged 480/480 and
   372/372 — while engaged losses ran 1.467×/2.059× (cell p50 vs
   prompt-best). Acceptance cannot see the structural verification-wire
   loss; a loss signal can.

**Recommendation — option (iii), structural: keep the reactive default at
2.0 (unchanged backstop) and gate engagement on a per-profile engaged-loss
EWMA.** Derivation from the extracted rows:

- Key on production-feasible quantities already in every join row —
  `realized_total_ms / fastest_single_prediction_ms` of engaged attempts
  (no extra single execution needed). Per-cell engaged medians of that key
  in this run: high-w16 1.012, high-w8 1.310, geo900-w8 1.715, geo900-w16
  1.903, geo800-w8 1.994, geo800-w16 2.424, geo700-w8 2.925, geo700-w16
  3.379 — a clean monotone band well separated from 1.0 after a handful of
  attempts.
- Block threshold ≥ 1.0 (strict beat-or-fall-back per the competitive
  revision plan). On this dataset that blocks engagement in **8/8 engaged
  cells** after EWMA warm-up — exactly matching the pass-2 ANALYSIS finding
  4 ("with a wire-true verification term, every engagement in this run
  would have been gate-blocked"). The gate says "don't engage" for free;
  the guard says it only after paying round 1.
- Why not (i) alone: 2.0 unmodified lets the two leak cells lose 1.38–1.74×
  (p50) with a 3.28× tail while the promise says never-meaningly-worse.
- Why not (ii): the derived number would be m ≤ 1.5 (anything above 2.000
  catches none of the 125; catching all requires < 1.539), and §2.2 shows
  that multiplier converts the guard into an expensive never-engage switch
  that fires on 100% of engaged runs — including every engaged win the
  dataset managed — while making the median leak-row outcome worse. The
  multiplier is not the promise lever; the engage decision is.

This is a recommendation to the owner/Integrator (production gate changes
are ADR-gated Scheduler Scientist work). If accepted, the natural next step
is a Scheduler proposal specifying: EWMA α and warm-up N, the per-(cohort,
profile) scope, cold-start behavior (default-block or margin-gated), and a
re-run of the pass-2 cells with the gate keyed on the EWMA to verify the
fallback ratios stay in the 0.95–1.02× band.

## 3. Review pin (test tree only)

`crates/modelswarm-bench/tests/harness.rs` gained one labeled
`REVIEW PIN` test, `loss_guard_counterfactual_matches_default_arithmetic`:
two `LossGuardRecord` JSON rows lifted verbatim from the committed LAN
artifacts (one the default guard would have aborted, one boundary
pass-through row) must (a) deserialize, (b) record `multiplier: "off"`
(never-silent relaxation), (c) keep `fired == false` under the off guard
while (d) `would_fire_at_default` exactly equals
`round1_wall > DEFAULT_LOSS_MULTIPLIER × (1 + DEFAULT_CONFIDENCE_MARGIN) ×
predicted_round`, matching the committed artifact value — the
counterfactual honesty contract. No existing test covered this (the src
`params` tests cover env parsing only; `join_rows_drive_the_summary_ratios`
uses `loss_guard: None`). No production file was touched.

## 4. Evidence-chain review of the pass-2 LAN artifacts

- **Reproducible from recorded commands: YES.** Re-ran
  `python3 experiments/processed/aggregate-pass2.py experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10 /tmp/reagg-lan.json`
  → **bit-identical** to the committed
  `experiments/processed/PASS-2-ENGAGE-LAN-2026-10-10-summary.json`
  (byte-for-byte diff clean). Headline numbers independently re-derived
  from raw joins: 4,800 rows / 1,982 engaged / 1,857 would-fire / 0
  failures — all match the ANALYSIS and the commit message. The ANALYSIS
  ops record embeds the exact driver invocation (env including
  `MSP_BENCH_LOSS_MULTIPLIER=off`, matching every row's recorded
  `"off"`) and the B-side rebuild/cleanup sequence.
- **Labels honest: YES.** All 4,800 LAN rows carry `lan-2machine-quic`;
  `calibration.json` honestly records `skipped-lan-real-rtt` with the
  transport note; the loopback twin's 1,920 rows all carry
  `loopback+injected-delay` (no cross-contamination), and its published
  946-engaged / 911-would-fire re-derives exactly.
- **Negative stated without overreach: YES.** "No engaged arm won" holds
  at the arm-median level (best 1.071 = the parity corner, called "parity,
  not a win"); the 107 row-level wins inside the parity corner are visible
  in the committed rows, not hidden, and the ANALYSIS's "What this run
  does NOT claim" section covers model-level and WAN claims. The
  limitation noting the 125 leak rows is accurate — this review quantifies
  it (§1).
- **Findings (non-blocking):**
  1. The B-side serve exe SHA-256 (`d26d487f…`) is attested in the ops
     record (scp-verified at run time, B cleaned afterward) but is not
     re-verifiable from the repo alone; acceptance tracking the dial to
     three digits (geo700-w8 measured 0.275 vs classic-model 0.275) is
     strong corroborating evidence the rebuilt serve ran. Accepted as an
     ops-record attestation.
  2. The ANALYSIS's per-cell "realized/predicted 2.8×–8.3×" describes the
     fired population (re-derived: p50 3.96, max 81.0); the pass-through
     population (1.77–2.30) is covered by the limitation note. No
     misstatement.
  3. The worst-cell engaged median 4.257× (geo700-w16, fixed-k2) is
     counterfactually guard-caught (guard ratio ≥ 2.300 there), so the
     published negative is, if anything, understated as a *production*
     risk: with the guard ON, that cell aborts at round 1 into ≈2.07× —
     still meaningfully worse than the single, which is §2's point.
- **Blockers: none.** The evidence chain, labels, and published negative
  all check out; the calibration risk is real but correctly flagged by the
  author, and this review resolves it into a recommendation (§2) rather
  than a gate failure. No release is pending on this evidence (protocol/
  planner evidence from a TEST-ONLY synthetic executor; no product
  performance claim is made, per the artifacts' own notes).

## Gates (this review)

- `cargo fmt --check` — clean.
- `cargo clippy -p modelswarm-bench --features quic-runner --all-targets -- -D warnings` — clean.
- `cargo test -p modelswarm-bench --features quic-runner` — **55 passed,
  0 failed, 4 ignored** (owner-gated LAN/pass-2 drivers): 37 lib, 17
  harness (16 prior + this review's pin), 1 loopback smoke + 4 ignored.
