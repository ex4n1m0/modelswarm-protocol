# ACCURACY-DIVERGENCE-LAN (2026-10-11) — ANALYSIS

Cross-peer greedy divergence detection over a REAL two-machine QUIC +
Noise swarm with ADR-026-style experiment leases (environment label
`lan-2machine-quic`; never a WAN claim). Part of the accuracy-benefits
study (`docs/reviews/accuracy-benefits-study-2026-10-11.md`).

## What this run measures

The same protocol-level detection machinery as the loopback cell, but
the four serving bridges run on machine B (real QUIC transport, real
LAN RTT, throwaway-hub-signed leases) and the driver runs on machine A.
Synthetic token executors on the serve side — structural evidence for
the instrument, not an engine-divergence-rate claim.

## Setup

- Machines: A = driver (Intel Raptor Lake-HX laptop, Windows); B = serve
  (Intel i9-9900K, `192.168.100.43`, Windows).
- Commit: `bde7eca`. Serve exe: `target/release/examples/pass1_serve.exe`
  sha256 `c621fbbe0006d2e09d8bc090ecef2248ec938adf1456c349340ed26f1c415e2e`
  (built on A, scp'd to B, `Get-FileHash` verified identical).
- Serve deployment (the documented scheduled-task detach pattern; a
  SEPARATE folder + task so v4/`MSPPass1` stay untouched):
  - Folder `C:\Users\Admin\Desktop\ModelSwarm-ACC-v7\` (exe + bat).
  - `serve-headless.bat`: `pass1_serve.exe --profile msp1:<64x1> --client-peer
    12D3KooWHP2Ve7tpkRQMJACbU4xmq9aDwL6gphLRHLJ3xB6nU5KA --bridges 4
    --inject-rtt-ms 0 --bind-ip 192.168.100.43 --pool accuracy
    [--divergent 2] > serve-out.log 2> serve-err.log` (injected sweep
    adds `--divergent 2`).
  - Firewall rule `ModelSwarm ACC v7` (inbound allow for that exe).
  - Scheduled task `MSPPassAcc` (ONCE, RL HIGHEST) launching the bat;
    started with `schtasks /Run /TN MSPPassAcc`. One FRESH serve per
    sweep (deterministic request ids — a reused serve would reject the
    second cell's ids as `replayed_request`).
- Peer lines captured with
  `Select-String '^MSP_BENCH_PEER' | % {$_.Line}` (5 `|`-fields
  validated), consumed as `MSP_BENCH_PEER_0..3` by the driver.
- Cells: `clean` (no `--divergent`) then `injected` (`--divergent 2`).
  The driver cross-checks every dialed peer id against the pinned pool's
  derived identity (`bridge_peer_id`) and self-checks
  detection == `MSP_BENCH_EXPECT_DIVERGENT` (the serve-side ground truth).

Driver commands (exact, from A):

```text
# clean
MSP_BENCH_PEER_0..3=<B's lines> MSP_BENCH_CELL=clean \
MSP_BENCH_OUT="<repo>/experiments/raw/ACCURACY-DIVERGENCE-LAN-2026-10-11" \
  cargo test -p modelswarm-bench --features quic-runner --test accuracy \
  accuracy_divergence_lan -- --ignored --nocapture
# injected (after B's serve restarted with --divergent 2)
MSP_BENCH_PEER_0..3=<B's lines> MSP_BENCH_CELL=injected \
MSP_BENCH_EXPECT_DIVERGENT=2 MSP_BENCH_OUT=<same> \
  cargo test -p modelswarm-bench --features quic-runner --test accuracy \
  accuracy_divergence_lan -- --ignored --nocapture
```

## Results (script-computed; `experiments/processed/ACCURACY-2026-10-11-summary.json`)

| Cell | Checks | Peer-rows | Diverge | Quorum diverge | Suspended | Check wall (median/p90 ms) |
|---|---|---|---|---|---|---|
| clean | 8 | 24 | **0** (0 false positives) | 0 | 0 | 249.1 / 292.3 |
| injected | 8 | 24 | **8/8 at position 0** (0 false negatives) | 8/8 | exactly index 2 | 251.9 / 350.2 |

- Ledger: offender 8/8 diverge, agreement EWMA 0.0; honest probes 16/16
  agree, EWMA 1.0 (both cells' ledgers in `agreement-ledger.json`).
- De-rank: cohort (V, D, M-offender) -> (V, D, M2) in the injected cell;
  identical before/after in the clean cell.
- Redundant-racing availability (same completions): median tail saved vs
  the slower peer = 69.0 ms (clean) / 88.4 ms (injected); raced min
  median 162.1 / 160.6 ms vs slowest-peer median 215.5 / 218.3 ms.
  Structural numbers on synthetic speeds.
- Peer-id integrity: all four dialed peer ids equaled the pinned pool's
  derived identities (asserted in-driver; the same deterministic seeds
  produce the same peer ids on both machines).

## Findings

1. The detection machinery behaves IDENTICALLY on the two-machine LAN
   and on loopback (0 FP / 8-of-8 detection both sides) — detection is
   transport-independent at these RTTs, as expected for a
   content-comparison instrument (the network only carries the streams).
2. One check costs one extra decode per additional pool member
   (concurrent): median check wall 249-252 ms for 4 concurrent 16-token
   decodes — the diagnostic's price, paid OFF the serving path.

## NOT claimed

- No engine divergence rates (synthetic executors).
- No performance claim of any kind (this is a diagnostic instrument).
- No WAN behavior; two-machine LAN only.

## Ops record (machine B, left as found)

Serve stopped after the injected sweep; verified at cleanup: 0
`pass1_serve`/`llama-server` processes, task `MSPPassAcc` deleted,
folder `ModelSwarm-ACC-v7` removed, firewall rule `ModelSwarm ACC v7`
removed (remaining rules: v2, v4), `MSPPass1` Ready, v4 folder intact.

## Reproduce

At `bde7eca`: build the serve exe
(`cargo build --release -p modelswarm-bench --features quic-runner
--example pass1_serve`), deploy per the ops record, run the two driver
commands above (fresh serve per cell). Aggregation:
`python experiments/processed/aggregate-accuracy.py experiments/raw
experiments/processed/ACCURACY-2026-10-11-summary.json`.
