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

## Decision

1. **Mechanism: the decayed two-sided balance.** Every installation
   accumulates `granted_tokens` (ADR-030 granted accounting, all
   profiles) and `consumed_tokens` (recorded at receipt intake, where
   the posting installation IS the requester). Both use the same
   7-day-half-life read-time decay. Consumption is allowed while:

   ```text
   decayed_consumed ≤ k × decayed_granted + BOOTSTRAP_TOKENS
   ```

   In steady state this is exactly "pull at most k× your push rate";
   the half-life is the averaging window and the anti-farm horizon
   (old credit dies; standing is maintained by serving now).

2. **Constants are operator policy, not code releases.**
   `MSP_SWARM_BALANCE_K` (default **1.0**; `0` disables the gate) and
   `MSP_SWARM_BOOTSTRAP_TOKENS` (default 100_000, one-time headroom per
   installation so a fresh peer's first hours work; it is NOT granted
   credit and never counts as served work).

3. **Enforcement point: lease issuance.** `POST /peers/lease` is the
   single choke point every consume credential already passes through.
   A peer over balance receives `403 balance_exceeded` with its actual
   numbers (`granted`, `consumed`, `k`) — the app renders the honest
   "swarm balance" line from them. No other surface changes.

4. **Shadow first.** `MSP_SWARM_BALANCE_ENFORCE` (default **off**):
   the balance is ALWAYS computed and logged at issuance
   (`balance.shadow` telemetry row: would-pass / would-block + numbers);
   blocking starts only after real traffic shapes the constants. The
   owner's own machines cannot be stranded by a brand-new rule.

5. **Network-wide across profiles.** Granted tokens from ANY profile
   (local or cloud) count toward consumption of ANY profile — this is
   what makes "serve local tasks, consume smarter models" work (the
   owner's stated trade). Per-profile balances are a future policy
   refinement, not v1.

6. **Evidence discipline unchanged.** Both sides move ONLY on
   intake-validated receipts (signed, both signatures, enrolled keys).
   Nothing self-reported; a sybil ring still grants nothing (ADR-030
   §4), so rings cannot manufacture balance.

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
