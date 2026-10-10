# PASS-1 LAN — 2026-10-10 — first two-machine QUIC pass-1 evidence

> **CORRECTION (2026-10-10, pass-2, commit 4faa6c0):** finding 1's
> mechanism below is wrong. The synthetic executor folds the REQUEST
> seed only — different bridge seeds produce identical continuations
> (pinned by test `non_divergent_peers_agree_per_request_seed`), so
> "per-bridge seeds never match" was never the cause. Pass-1's
> all-fallback was the ENGAGE GATE refusing to speculate (the run
> records' reason strings say so). The numbers and every other
> finding stand; only this causal explanation is superseded. See
> `PASS-2-ENGAGE-*-2026-10-10/ANALYSIS.md`.

Label: `lan-2machine-quic` (every `decision-join.jsonl` row and every
`summary.json`). Never a WAN claim.

## Run identity

- Code: `f4472f3` + same-day harness label fix (`set_lan_bridges` flips
  `env_label`; see below). 30 reps/arm/cell, window 8, corpus 4 prompts,
  cohort cap 3 (only 2 remote bridges ⇒ arms `fastest-single`,
  `fixed-k2`, `planner-k`), acceptance regimes High/Zero/Mixed ⇒
  360 records per cell, 1,080 total, 159.5 s wall.
- Machines: A = 192.168.100.42 (driver, identity seed 0xB0,
  peer `12D3KooWHP2…U5KA`); B = DESKTOP-MBQ7VBM at 192.168.100.43
  (`pass1_serve` v4 exe, 2 synthetic bridges, `--inject-rtt-ms 0`).
  Same /24 subnet, real QUIC + Noise + ADR-026 lease gate end to end.
- Executors are **TEST-ONLY synthetic** (deterministic per-bridge
  seeds): this is protocol/lifecycle/fallback evidence, NOT a model
  throughput claim.
- Calibration honestly skipped for LAN (`method: "skipped-lan-real-rtt"`)
  — the RTT is real; there is no injected term to subtract.

## Results (medians, n=120 per arm per cell)

| cell | fastest-single | fixed-k2 | planner-k | k-arm fallbacks |
|---|---|---|---|---|
| inj0ms-high | 131.11 ms | 130.87 ms (0.998×) | 130.50 ms (0.995×) | 120/120 |
| inj0ms-mixed | 131.35 ms | 120.59 ms (0.918×) | 128.19 ms (0.976×) | 120/120 |
| inj0ms-zero | 194.63 ms | 194.79 ms (1.001×) | 190.54 ms (0.979×) | 120/120 |

Failures: **0 everywhere** (1,080 records).

## Honest findings

1. **Every speculative arm fell back in every run.** The synthetic
   bridges are seeded per-bridge, so proposer drafts never match the
   verifier continuation — by construction. These numbers measure the
   **cost of the fallback path on a real LAN**, not a speculative win.
   (Matching-draft speculative wins need pass-2's C-API `verify_drafts`
   or real engines; the loopback dry run showed the same shape.)
2. **Fallback costs ≈ nothing on this LAN.** Ratios 0.918–1.001 with
   heavily overlapping IQRs (~47–57 ms): no win and no loss claim. The
   release-gate property — a micro-swarm that cannot speculate never
   does meaningfully worse than the fastest eligible single host —
   held over 1,080 real cross-machine records.
3. **The full gated stack survived a real two-machine run**: QUIC
   listener with held bridges, ADR-026 lease verification (a corrupted
   lease was refused `invalid_lease`; replays were refused
   `replayed_request`), session lifecycle, honest RTT labeling.
4. **Regime-dependent single medians** (High/Mixed ≈ 131 ms vs Zero ≈
   194 ms): acceptance state changes candidate ordering and the chosen
   peer path. Noted; not root-caused in this pass.

## Known limitations / follow-ups

- k=2 only. k=3/k=4 sizing needs `--bridges 4` on the serve side —
  free on the same two machines (k=4 *cloud* spend stays deferred per
  gate decision D4).
- Sub-ms–1 ms LAN RTT: the interesting speculative regime (RTT ≥
  proposal window cost) needs injected delay or a WAN pair.
- Window fixed at 8; no window sweep in this pass.

## Record corrections (same day)

- The first sweep of the day (discarded, re-run clean) shipped all
   1,080 rows labeled `loopback+injected-delay`: the LAN test's
   per-cell `set_lan_bridges` re-arm never flipped `env_label`, while
   the console print hardcoded the LAN constant. Fixed + pinned by
   unit test (`set_lan_bridges_labels_cells_lan_not_loopback`); this
   directory is the correctly-labeled re-run from a fresh serve
   process (fresh leases; deterministic request ids make re-runs
   against the same serve process fail `replayed_request` by design).

## Ops record (how B was driven — reusable)

- B is SSH-controlled from A (key auth; OpenSSH Server installed from
  the official GitHub zip after `Add-WindowsCapability` failed
  `0x800f0950`; firewall scoped to 192.168.100.0/24).
- **Processes started over SSH die with the session's job object** —
  `Start-Process` is NOT detached. The serve runs via a registered
  scheduled task (`schtasks /Run /TN MSPPass1`) launching
  `serve-headless.bat` (env + redirect), which survives session close.
  A fresh task run = fresh leases + clean replay dedup.
