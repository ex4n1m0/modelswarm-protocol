# ADR-003: Tracker-first discovery; no DHT/relay in v0.1

Status: Accepted (Phase 0)

## Context

Peers need to find each other. Options: a central tracker, a Kademlia DHT +
Identify, or gossipsub capacity broadcasts. The swarm is small, invite-only,
and profile-partitioned; correctness of the core rule matters more than
decentralization.

## Decision

The Vercel tracker is the **bootstrap authority and only discovery mechanism**
in v0.1: registration, expiring leases, profile-filtered lookup, and
offer/answer signaling for direct QUIC connections. libp2p is still used for
the data plane (transport crypto, streams, ping). Kademlia/gossipsub and a
public relay (Relay/DCUtR) are explicitly deferred until direct connections
are measurable and failure rates are known. Direct-connect failure is reported
as failure — the hub is never a fallback relay.

## Consequences

+ One rendezvous implementation to test; deterministic in E2E runs.
+ Lease semantics enforce the eligibility rule naturally.
+ Honest NAT story: works where QUIC hole punching works; fails visibly
  elsewhere.
− Hub outage blocks *new* discovery (existing streams continue).
− Hard-NAT peers are stranded until the relay milestone.

## Alternatives rejected

- **DHT-first**: discovery correctness would compete with the core-rule work;
  also leaks profile membership to arbitrary DHT walkers.
- **Vercel WebSocket presence**: bounded function lifetimes (ADR-001).
