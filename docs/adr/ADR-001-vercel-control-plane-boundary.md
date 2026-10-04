# ADR-001: Vercel is the control plane, never the data plane

Status: Accepted (Phase 0)

## Context

The tracker must run somewhere cheap, HTTPS-native, and maintenance-free.
Vercel offers that, but its functions have bounded durations; its WebSocket
support (public beta) pins each connection to a function that dies with it.
Model traffic is high-bandwidth, long-lived, and latency-sensitive.

## Decision

`modelswarm.deepflux.space` (Next.js on Vercel + managed Postgres) handles
only: accounts, catalog, peer registration/leases, candidate lookup, brief
rendezvous signaling, and signed accounting events. All inference traffic —
prompts, tokens, cancellation — flows peer-to-peer over encrypted QUIC. Model
downloads flow directly from Hugging Face to nodes. Heartbeats are HTTPS every
15–30 s with 60–90 s leases; nothing requires a hub function to stay alive for
a peer's lifetime. A standalone public relay may be added later, separately
hosted, only after real direct-connect failure rates justify it.

## Consequences

+ No bandwidth/duration pressure on Vercel; costs stay metadata-sized.
+ Lease expiry works without background processes (rows age out).
− No relay in v0.1: peers behind hard NATs cannot connect; the UI must say so.
− Signaling is short-poll/REST-based, slightly slower than a socket.

## Alternatives rejected

- **Vercel as WebSocket relay**: violates function-duration bounds; beta risk.
- **Self-hosted VPS tracker + relay**: more ops burden than the prototype needs.
- **DHT-first discovery**: no bootstrap authority; deferred (ADR-003).
