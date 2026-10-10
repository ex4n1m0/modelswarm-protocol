# PASS-1 LAN k-sizing — 2026-10-10 — 4 bridges (follow-up to PASS-1-LAN-2026-10-10)

> **CORRECTION (2026-10-10, pass-2, commit 4faa6c0):** the pass-1
> record's causal story "per-bridge synthetic seeds → drafts never
> match" is wrong — the executor folds the request seed only, so
> non-divergent bridges produce identical continuations (pinned by
> test). Pass-1's ~100% fallback was the ENGAGE GATE declining to
> speculate; the two engaged k4 runs matched drafts through exactly
> this shared-continuation property. All k-flatness numbers and the
> prediction-divergence finding stand. See
> `PASS-2-ENGAGE-*-2026-10-10/ANALYSIS.md`.

Label: `lan-2machine-quic`. Same two machines, same method as
`PASS-1-LAN-2026-10-10` (see its ANALYSIS.md for environment + ops
record). Differences: serve side runs `--bridges 4`; the LAN test's
cohort cap is now env-driven (`MSP_BENCH_COHORT_CAP`, default 3 —
this run used 4) ⇒ arms fastest-single / fixed-k2 / k3 / k4 /
planner-k. 3 regimes × 600 records = 1,800 total, 234.8 s wall,
zero failures.

## Results (medians, n=120 per arm per cell)

| cell | fastest-single | fixed-k2 | fixed-k3 | fixed-k4 | planner-k |
|---|---|---|---|---|---|
| inj0ms-high | 155.25 ms | 130.33 (0.840×) | 130.70 (0.842×) | 130.75 (0.842×) | 130.63 (0.841×) |
| inj0ms-mixed | 130.72 ms | 131.50 (1.006×) | 130.50 (0.998×) | 131.14 (1.003×) | 132.18 (1.011×) |
| inj0ms-zero | 130.82 ms | 130.92 (1.001×) | 130.54 (0.998×) | 130.61 (0.998×) | 130.79 (1.000×) |

## Honest findings

1. **All-fallback cost is FLAT in k** on this LAN: k2/k3/k4 medians
   agree within 0.4 ms in every regime. At sub-ms RTT, additional
   cohort members (proposed to in parallel, rejected, fallen back)
   cost nothing measurable — there is no k-sizing penalty up to k=4
   on LAN. The interesting k trade-off only appears when RTT is
   comparable to the proposal window (WAN or injected delay).
2. **First engaged speculative completions on LAN**: mixed/k4 fell
   back 118/120 — two runs had matching drafts and completed
   speculatively. Microscopic but nonzero: with 4 seeds, draft
   alignment occasionally happens by chance.
3. **Prediction-vs-realized divergence, quantified**: in the High
   cell the fastest-single arm's predicted-best peer completed at
   155 ms median while every k-arm's fallback path landed ~130 ms —
   the predicted-fastest peer was ~19% slower than realized-best.
   (In the 2-bridge run the same arm's pick landed at ~131 ms.)
   This is exactly the divergence the shadow planner join measures;
   the ordering signal (advertised queue + acceptance EWMA) misled
   the single-pick under 4 synthetic candidates.
4. Never-worse-than-fastest-single held everywhere (worst arm ratio
   1.011, within noise); zero failures across 1,800 records;
   `invalid_lease`/`replayed_request` gates untouched.

## Sizing answer for the deferred D4 question

On LAN-quality links: **choose k for engagement probability, not
cost** — up to k=4 the cost is indistinguishable from k=2 (and from
single) when drafts don't match. The k=4 *cloud* comparison (real
WAN RTT) remains deferred per D4; when run, expect the flat-in-k
picture to break and the planner's RTT-aware cap to matter.

## Harness delta committed with this run

- `pass1_loopback.rs`: `MSP_BENCH_COHORT_CAP` env (default 3 keeps
  the committed pass-1 evidence reproducible bit-for-bit in
  configuration).
