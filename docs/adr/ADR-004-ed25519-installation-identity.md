# ADR-004: Per-installation Ed25519 identity

Status: Accepted (Phase 0)

## Context

Peers must authenticate to the hub and to each other without passwords.
Identity must survive reinstalls of the OS only via user re-enrollment, and be
revocable centrally.

## Decision

Each installation generates one **Ed25519 keypair** on first run. The private
half is stored through a Windows secret-storage abstraction (Credential
Manager/DPAPI) — never in the webview, never in plaintext config. The public
half is registered at device enrollment and binds: hub requests (signature
over timestamp + nonce + installation id + body digest) and the libp2p
`PeerId`. The hub can revoke an installation; revocation propagates through
lease expiry and token denial.

## Consequences

+ No passwords anywhere; per-installation granularity for bans.
+ Same key material serves hub auth and P2P identity — one revocation story.
+ Signature payloads are small and fast.
− Lost key = lost identity (acceptable; re-enroll via device auth flow).
− DPAPI ties the key to the Windows profile; roaming across machines is not
  supported by design.

## Alternatives rejected

- **User passwords in the node**: phishing-prone, worse UX.
- **TLS client certs**: heavier lifecycle, awkward with libp2p PeerIds.
- **Hardware keys**: disproportionate for v0.1.
