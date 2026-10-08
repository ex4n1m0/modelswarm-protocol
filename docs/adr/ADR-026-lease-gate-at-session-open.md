# ADR-026: Hub-signed lease gate at serving-session open

- Status: accepted (2026-10-08)
- Owner: Security Engineer (gate semantics), Windows Product Engineer
  (desktop attach), Network Engineer (transport PeerId binding)
- Reviewers: Protocol Architect (msp-v1 §5 alignment), Test and Release
  (gate tests)

## Context

After the first cross-machine swarm completion (2026-10-07,
`docs/verification/phase-f-lan-2026-10-07.md`), the serving bridge
admitted ANY protocol-speaking session asking for the hosted profile —
the honest F0 TODO. F3's first control closes it.

## Decision

A serving peer admits an `InferenceRequest` only when its
`capability_token` carries a hub-signed `EligibilityLease` (wire form
`base64url(json).base64url(sig)`) that passes [`LeasePolicy::check`]:

1. **Signature** against the pinned hub key (`protocol/keys/hub-public.hex`
   — the same key catalog envelopes verify against).
2. **Freshness + lease-cap TTL** (the eligibility crate's frozen checks).
3. **Profile match**: the lease names THIS node's served profile.
4. **Rights**: `can_consume` with `slots >= 1`.
5. **Peer binding**: `lease.peer_id` equals the remote's
   **QUIC-authenticated** `PeerId` (exposed by the transport session) —
   a lease stolen from another peer is worthless, and the lease cannot
   be replayed across connections of different peers.

Refusal is a `StreamError { code: "invalid_lease" }` with the reason in
`message`, then the session closes. The gate sits BEFORE the executor.

## The requester chain (who can get a lease)

The tracker grants leases only after a **passed hosting challenge**
(`challenge_start` → real generation timings → `challenge_complete`) —
leases are earned by demonstrably hosting the profile, which is the
project's contribute-to-consume rule enforced cryptographically. The
desktop's swarm-chat path will complete this chain (register leaseId →
challenge with real local-engine timings → `request_lease` → present);
the wire pieces exist and are harness-tested up to the challenge
(`live_tracker.rs`), with the challenge step requiring a real serving
peer to time honestly.

## Tests

`gate_accepts_own_and_rejects_foreign_binding` (unit),
`serving_bridge_refuses_leaseless_requests`,
`serving_bridge_refuses_foreign_peer_lease` (real QUIC), plus every
positive path now runs WITH a lease (round trip, remote executor,
failover). Eligibility gains `from_wire`/`to_wire` with round-trip
coverage; `BadWireLease` error added.

## Consequences

- Leaseless/stolen-lease/foreign-profile requests never reach the
  executor.
- Old F0-era requesters (`f0-unverified`/empty tokens) are refused by
  new builds — intentional breaking change for the internal-test
  population only.
- Revocation semantics (audit_epoch, suspension) inherit from the
  eligibility crate and light up as the tracker starts enforcing them.
