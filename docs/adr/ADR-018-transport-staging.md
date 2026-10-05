# ADR-018: Transport backend staging — signed-frame TCP now, rust-libp2p at Phase F

Status: Accepted (Phase B, 2026-10-05) · Amends ADR-014's sequencing detail
(not its boundary: the tracker still never relays; public-internet release
still requires the full libp2p stack).

## Context

msp-v1 §6 names libp2p/QUIC/Noise as the transport. Pulling the full
rust-libp2p stack in now would make Phases B–E iterate on dependency weight
and API churn rather than on the protocol mechanics (sessions, commits,
speculation, adversarial behavior) that the phase gates actually test.
Those tests run on loopback/LAN between our own processes.

## Decision

`modelswarm-transport` exposes a small `PeerTransport` trait (connect,
open_stream, framed send/recv with deadlines, cancel, measured RTT).
Implementations:

1. **SignedFrameTransport (Phases B–E)**: tokio TCP + length-prefixed
   frames (256 KiB payload cap, validated before body allocation).
   *Signing staging (amended Phase C, 2026-10-05)*: the **handshake is
   Ed25519-signed** (canonical-JSON payload per msp-v1 §2.2, binding peer
   identity, profile, runtime, ts, nonce); post-handshake frames ride the
   authenticated session, and **state-changing cooperative messages
   (commits, receipts) are additionally signed at the session layer** via
   modelswarm-session/modelswarm-identity machinery in Phases D–E. The
   original "every frame signed" wording is superseded by this staged
   rule; per-message transport signatures arrive with the libp2p/Noise
   backend where the channel itself is cryptographically bound.
   `HandshakeAck` and transport-layer pings are unsigned (loopback-only
   allowance; fixed by Noise at Phase F).
2. **libp2p backend (Phase F, mandatory before any public-internet use)**:
   QUIC + Noise + Identify + AutoNAT + relay/DCUtR per ADR-014. The trait
   is the swap point; the Phase F gate includes re-running the D/E
   correctness suites over the libp2p backend.

**Honesty constraint:** SignedFrameTransport provides authentication at
session establishment and integrity via length-bounded framing, but **not
confidentiality** (no Noise). It is acceptable only on loopback/LAN test
topologies. The product refuses to enable swarm participation on
non-loopback listeners while this backend is active — enforced in code
(`TransportError::NonLoopbackDenied`) and tested. Production releases ship
only with the libp2p backend.

**Framing notes for the Phase-3 wire-encoding ADR:** the 256 KiB cap counts
the announced payload (prefix excluded); JSON external-tag framing is the
staged encoding — field names mirror `protocol/messages.proto` exactly and
survive a later protobuf switch.

## Consequences

+ Protocol work proceeds at full speed without the dependency mountain.
+ The confidentiality gap is structurally impossible to ship by accident.
− One re-verification pass at Phase F over the real backend (planned).
