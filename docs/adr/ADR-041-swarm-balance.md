# ADR-041: The swarm balance — decayed token reciprocity

Status: Accepted (owner ruling 2026-10-17: "pull at most what you can
push — same capacity to start, adjust later") · Builds on ADR-030
(granted accounting) + ADR-040 (cloud stake).

## Context

The owner directed the fairness mechanism for ADR-040's asymmetry
(cloud-key and local peers consuming others' work): a rate-style
reciprocity — a client may pull from the swarm at most a multiple k of
what its own serving upstreams, in tokens. "Same capacity to start"
(k = 1.0), tunable later. The intent is the cross-profile trade: your
local model serves the swarm's tasks for it while you consume other
profiles' answers.

## Decision (REVISED 2026-10-17, later the same night — the torrent lesson)

The owner rejected history-based gating from experience: "sometimes it is
very hard to gain ratio due to no one pulling from your model… base the
design on the CAPACITY, not the actual operation history." The rule:

1. **Mechanism: capacity, not history.**

   ```text
   tokens consumed in the trailing WINDOW
     <= k × capacity_tokens_per_sec(verified CapacityClass) × WINDOW
   ```

   The verified CapacityClass (cpu / gpu_entry / gpu_mid / gpu_high) is
   assigned from MEASURED challenge timings (D7/ADR-012 — self-reported
   hardware never raises it): "capacity due to my hardware and model
   type," exactly. Nobody pulling from you never matters — the unpopular-
   torrent spiral cannot occur.

2. **The rolling window IS the allowance.** WINDOW defaults to 1 h
   (`MSP_SWARM_BALANCE_WINDOW_SECS`): it is simultaneously the burst
   headroom and the cold start — a fresh host can pull its full window
   budget immediately; no bootstrap, no debt. The window refills
   continuously.

3. **Constants are operator policy, not code releases.** k default 1.0
   (`MSP_SWARM_BALANCE_K`; 0 disables). The class capacity table
   (cpu 8 · gpu_entry 15 · gpu_mid 40 · gpu_high 90 tok/s) is the
   measured-honesty anchor set, revisitable by release with data.

4. **Enforcement point: lease issuance** (unchanged). The balance view
   rides every issuance (additive `balance` field: capacity, consumed,
   budget, headroom); a peer over budget gets `403 balance_exceeded`
   with its actual numbers. **Ships DARK** — `MSP_SWARM_BALANCE_ENFORCE`
   (default off) gates blocking.

5. **The receipts ledger remains the evidence base** (granted +
   consumed, ADR-030/0007) for accounting, the UI display, and the
   future reputation chassis — it no longer GATES consumption. The
   consumption rows still move only on intake-validated receipts.

6. **Network-wide across profiles** (unchanged): any granted capacity
   spends anywhere — serve local tasks, pull smarter models.

## Consequences

+ Fairness matches the owner's torrent intuition while reusing the
  receipts ledger as the only source of truth; constants are config.
+ Cold start works (bootstrap); idle-heavy swarms degrade honestly
  (balances visible before they bind).
− Until enforcement flips on, the asymmetry ADR-040 accepted remains —
  by the owner's explicit "same capacity to start and later we adjust".
− A peer whose profile nobody requests earns nothing and eventually
  rides only the bootstrap (the unpopular-torrent problem); acceptable
  in v0.x swarms where the family profile is shared, revisit with data.
