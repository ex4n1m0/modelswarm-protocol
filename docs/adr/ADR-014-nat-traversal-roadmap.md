# ADR-014: NAT traversal roadmap (relay/DCUtR in Phase F)

Status: Accepted (Phase A, 2026-10-04) · Amends ADR-001 (unchanged: Vercel
never relays) and ADR-003 (relay stops being an indefinite non-goal).

## Context

Direct-only QUIC leaves hard-NAT peers stranded. libp2p's standard answer is
a public relay plus DCUtR hole punching, coordinated via AutoNAT/identify.
p2ptokens treats full NAT traversal as baseline engineering, so this is
table stakes for a credible P2P product, not a differentiator.

## Decision

1. Phases B–E remain **direct-first**: QUIC (TCP+Yamux fallback), explicit
   connect-failure states, no relay dependency in correctness tests.
2. Phase F delivers the traversal stack: a **separately hosted public relay
   service** (libp2p relay + DCUtR + AutoNAT), UPnP optional. Hosting choice
   (cheap VPS / Fly.io / similar) is an explicit stop-and-ask decision per
   the paid-resources rule; until then the relay runs only in the local
   simulator.
3. The tracker exchanges signaling metadata only (ADR-001 boundary
   unchanged).
4. `NAT path type` (direct / hole-punched / relayed) becomes a scheduler
   input and a telemetry field (ADR-013); relayed paths carry a cost penalty
   in the scoring formula.
5. Failure honesty: a peer reachable only via relay is labeled as such in
   the UI and never counted as a direct-capable candidate.

## Consequences

+ Hard-NAT peers become usable without lying about topology.
+ The swarm's performance claims stay honest (relay cost visible and priced
  into scheduling).
− New public service to run and secure (Phase F ops runbook).

## Alternatives rejected

- Vercel-hosted relay: violates function-duration/bandwidth bounds
  (ADR-001).
- WebRTC data channels as primary transport: duplicates the libp2p stack;
  revisit only if browser peers ever become a goal.
