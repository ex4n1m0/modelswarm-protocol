# Transport Plan (Phase C implementation; relay in Phase F per ADR-014)

Owner: Network Engineer · Crate: `modelswarm-transport`.

## Stack

- rust-libp2p: QUIC (primary), TCP + Yamux (fallback), Noise (identity-bound
  to ADR-004 keys), Identify, Ping.
- Phase F additions: AutoNAT, Circuit Relay v2 (separate public service),
  DCUtR hole punching, optional UPnP. Until then: direct-only, explicit
  failure states, no relay dependency in tests.

## Behavior requirements

1. **Signed envelopes everywhere**: every state-changing message carries the
   msp-v1 §2.3-style binding (version, ids, round/sequence, profile id,
   prefix hash, generation-parameter hash, sender peer id, deadline, nonce,
   signature) per the revision's cooperative requirements.
2. **Bounded everything**: max frame 256 KiB (msp-v1 §6.4), bounded queues,
   per-peer concurrent-request caps = advertised slots, per-stream deadlines.
3. **Idempotent handlers**: duplicate commits never advance session state;
   stale-prefix proposals rejected (Phase B property gates).
4. **Backpressure**: bounded libp2p buffers; slow consumers cannot grow
   memory unboundedly (acceptance row from the original plan carries over).
5. **Diagnostics**: measured RTT/jitter/loss per peer feeding the scheduler;
  `NAT path type` (direct / hole-punched / relayed) surfaced from Phase F.

## Test topology (simulator + CI)

- Loopback multi-peer via `apps/modelswarm-sim`.
- Network impairment matrix from `bench-harness-spec.md` (RTT/jitter/loss
  emulation; clumsy/netem-equivalent userspace shaper in the harness).
- Failure drills: disconnect before first token, mid-stream, verifier loss,
  coordinator loss → explicit cancellation/fallback (never silent corruption).

## Study notes from prior art (matrix cross-ref)

- LocalAI: token-gated DHT overlay + mDNS fallback — simple membership
  pattern worth studying for micro-swarm sessions.
- p2ptokens/Hyperspace/KwaaiNet: relay+DCUtR baseline patterns.
- Pooled: TURN/TLS-443 without relaying weights — irrelevant to Windows
  native but noted for completeness.
