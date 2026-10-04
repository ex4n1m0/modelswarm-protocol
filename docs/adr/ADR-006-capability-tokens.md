# ADR-006: Short-lived signed capability tokens bind hosting to consumption

Status: Accepted (Phase 0)

## Context

The core rule — consume P only while actively hosting P — needs an enforceable
artifact. Peers cannot trust each other's claims; the tracker can attest to
what it recently verified.

## Decision

After a node holds an active lease for P **and** passes a live hosting
challenge (artifact digest verified, runtime started, challenge prompt answered
within deadline, ≥1 serving slot, reachable), the tracker issues a
**short-lived Ed25519-signed capability token** binding: peer id, profile id,
`canHost`, `canConsume`, slots, expiry, nonce. **Token expiry is capped at the
issuing lease's expiry plus a 60 s grace window** — a token can never outlive
active hosting by more than that grace period, so a host that stops hosting
loses consumption within ≤ lease-TTL + 60 s (≤ 150 s worst case). Every
inference request carries one; the serving peer verifies signature, expiry,
profile and peer binding (token `peerId` must equal the Noise-authenticated
libp2p PeerId), and checks hub revocation notices (`revoked_tokens` /
`revoked_peers` delivered on heartbeat) at heartbeat cadence. Tokens are
multi-use within their lifetime; their `nonce` is an issuance-dedup and
revocation handle, while per-request replay protection is the single-use
`requestId`. Pausing/draining hosting stops issuance and consumption within
the grace window.

Tokens prove *recent qualification*, not continuous contribution, and not
cryptographic remote attestation — `docs/threat-model.md` §5 states this limit
in the product documentation.

## Consequences

+ One artifact enforceable at three layers (tracker, requester, serving peer).
+ Lease-capped TTL bounds every stale-state problem (drained host, revoked
  install) to seconds, not minutes.
+ Peer + profile binding defeats copying tokens between peers or profiles.
− Token refresh rides the heartbeat/challenge cadence (bounded: 1 challenge
  per lease renewal), so each request may need a freshly cached token.
− A lying node with a patched runtime can still pass challenges (accepted).

## Alternatives rejected

- **Honor-system accounting**: no enforcement at all.
- **Micro-payment/credit ledgers**: out of scope for v0.1 (non-goal).
- **Long-lived certificates**: stale revocation, worse than short TTLs.
