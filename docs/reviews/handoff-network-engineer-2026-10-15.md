# Handoff — Network Engineer, 2026-10-15 (F2B: relay client + honest fallback)

Session record for F2B per ADR-014, on top of the F2A relay (a8d8a63,
docs/verification/f2a-relay-2026-10-08.md). Context that drove the work:
the owner's first three-machine swarm from OUTSIDE the A/B LAN failed the
data plane exactly per v0.1 rule 7 (cross-NAT direct QUIC dial failed, app
honestly fell back local: "family: not engaged (elected_fellback)").
MSP peers can now USE the relay.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-relay/src/main.rs` | (commit 86f9ba4) Loopback-bind default + explicit `--public` opt-in — the ADR-018 `allow_off_loopback` discipline reached the binary (the F2A default was `0.0.0.0:4001`, a silently-open bind). Honest advertising fixed to match the posture (loopback bind advertises loopback; only `--public` advertises the LAN addr; v6 multiaddr family handled). Parser extracted into testable `parse_args`. (commit a1df415) Shared test helpers + enforcement proofs: reservation cap denial (`ResourceLimitExceeded`), unreserved-destination circuit refusal (`NoReservation`). |
| `crates/modelswarm-transport/src/relay_client.rs` | NEW (commit bf57931). The circuit-relay v2 CLIENT, hand-rolled per the F2A honest-boundary note: multistream-select 1.0.0 + the libp2p-relay 0.22 protobuf schema (uvarint-length-prefixed, 4096-byte cap validated BEFORE body allocation). Reserve (+3/4-lifetime auto-renewal driver, stop-stream accept), CONNECT dial with dial-side relay PeerId verification, relay status vocabulary surfaced verbatim (`no_reservation(204)` etc.). |
| `crates/modelswarm-transport/src/libp2p_backend.rs` | (bf57931) `Role::Ready` + `Libp2pSession::new_relayed` (payload stream with the partial frame buffer pre-seeded by handshake-coalesced bytes); `Libp2pTransport::reserve_relay`; `Libp2pTransport::dial_with_fallback` (direct-first, then relay candidates; per-attempt honest labels; impostor circuit addresses skipped, never dialed; winner labeled `DialPath::{Direct, Relayed}`); candidate classification (circuit addrs ride the EXISTING multiaddr list). (a1df415) deadline/byte-cap cut tests. (bd469ce) blackhole de-flake. |
| `crates/modelswarm-transport/src/lib.rs` | Module registration + docs. |
| `crates/modelswarm-transport/Cargo.toml` | dev-dependency libp2p with the `relay` feature — the in-process test double is the relay binary's exact rust-libp2p stack; the production client shares no code with it. |

## 2. Exact commands and outcomes

- `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy -p modelswarm-relay --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — **622 passed / 0 failed / 6 ignored** (78 suites).
- `cargo test --workspace --features modelswarm-node/libp2p-backend` — **645 passed / 0 failed / 7 ignored**.
- `cargo test -p modelswarm-relay` — 6/6 (x2 stable): limits contract, standard-clients reserve→circuit→bidirectional ping, both parser tests, cap denial, no-reservation refusal.
- `cargo test -p modelswarm-transport` (default features) — 22+12 green.
- `cargo test -p modelswarm-transport --features libp2p-backend` — 46+12 green; lib suite 3x consecutive stable.
- No winjob flake surfaced in this session's batteries (known ~1-in-3; re-run, not fix).

One flake was observed and FIXED (not rerun-away): the first workspace
battery failed `direct_failure_falls_back_to_relay_and_streams_both_ways`
once (freed-port race under parallel load) — commit bd469ce replaces the
freed port with a HELD UDP socket (true blackhole; also the honest
cross-NAT simulation). Both full batteries above are green AFTER that fix.

## 3. Test evidence (the F2B proofs)

All against a REAL relay (the double is the relay binary's exact
rust-libp2p relay behaviour, in-process on ephemeral loopback QUIC):

- `direct_failure_falls_back_to_relay_and_streams_both_ways` — the direct
  candidate is a blackhole socket; the relay candidate wins; path labeled
  `Relayed`; typed frames stream BOTH ways; the frame sent immediately
  after dial arrives intact (coalesced-with-handshake bytes preserved);
  exactly one circuit is opened.
- `direct_success_never_touches_the_relay` — direct wins, relay circuit
  counter stays 0 (ADR-014 direct-first).
- `relay_dial_to_unreserved_peer_is_refused_with_status_label` — relay
  refuses with `no_reservation(204)` surfaced verbatim.
- `circuit_address_for_another_destination_is_skipped` — a circuit addr
  naming a different destination is skipped with an impostor label, zero
  circuits (dial-side PeerId verification discipline).
- `announced_circuit_duration_limit_cuts_the_stream` /
  `announced_circuit_byte_limit_cuts_the_stream` — the limit machinery
  provably cuts streams at announced caps, which is exactly why
  modelswarm-relay ships duration/bytes UNCAPPED (an ADR-007
  unrecoverable interruption otherwise).
- `reservation_cap_is_enforced_with_resource_limit_exceeded` /
  `circuit_to_unreserved_destination_is_denied_no_reservation` (relay
  crate, standard libp2p clients).
- Codec units: uvarint round trips + overlong rejection; hop/stop
  protobuf round trips; malformed protobuf rejected (field 0, overrun,
  wire types, truncation, unknown types); upstream status names; renewal
  interval (3/4 + floor); `--public` parser contract.

## 4. Assumptions

1. Hand-rolled mss+protobuf client (F2A's sanctioned option) instead of a
   Swarm migration — the incremental Swarm move remains the separately
   sanctioned F2b follow-up (needs its own ADR + parity tests).
2. Relay addresses ride the EXISTING `addresses:[multiaddr]` field in
   peer records (msp-v1 §4/§6): `/…/p2p/<relay>/p2p-circuit[/p2p/<dst>]`
   parses as a multiaddr, so NO tracker/schema change is required.
3. Relayed-session far-end identity is relay-vouched (QUIC terminates at
   the relay) — same trust rust-libp2p's own client accepts; documented
   in-module, not hidden.
4. One relay candidate dial = one fresh relay connection (no relay-leg
   pooling yet; the existing per-peer SESSION pooling sits above this).
5. Renewal follows rust-libp2p's cadence (re-reserve at 3/4 of the lease;
   libp2p-relay 0.22 default lease 60 min).

## 5. Unresolved risks / honest boundaries

1. **Node/desktop do not USE the new surface yet** (cross-path, below) —
   F2B is transport-complete and proven, but RemoteExecutor still dials
   one direct addr only. The three-machine family swarm stays
   direct-until-integrated.
2. **Relayed far-end authentication is relay-vouched**: a malicious relay
   can impersonate the destination or read relayed frames (payload is
   plaintext AT the relay; our relay is content-blind but a hostile one
   need not be). The msp-v1 §6.1 Ed25519 handshake should be run
   end-to-end over relayed circuits — session-layer decision, cross-path.
3. **No DCUtR yet**: relayed paths stay relayed for the whole stream
   (no hole-punch upgrade); ADR-014's cost penalty should price this.
4. Reservation QUIC keep-alive relies on libp2p-quic's 5s default
   keep-alive interval — verified present in libp2p-quic 0.14 defaults,
   not re-tested over real NAT timeouts.
5. Cross-machine/real-network proof still pending (owner-gated; item 6
   below is the deployment shape for it).
6. `measure_rtt` on relayed sessions measures the relayed RTT (correct
   but must be labeled relayed in observations — scheduler input via
   ADR-013 nat-path field, not yet wired).

## 6. Minimal production relay deployment (for the owner's host choice)

- **What**: `modelswarm-relay` (already staged as `relay.exe` on the
  second desktop; rebuild from main for these changes). It is
  content-blind: forwards opaque streams, never parses MSP frames.
- **Port/protocol**: ONE UDP port (default 4001) — QUIC v1. No TCP.
  Allow UDP 4001 inbound on the host firewall AND port-forward it on any
  NAT in front (the relay must be reachable by BOTH peers directly).
- **Run line**: `modelswarm-relay.exe --listen 0.0.0.0:4001 --key
  <stable path>\msp-relay.key --public` (`--public` is now REQUIRED for
  any non-loopback bind — deliberate friction).
- **Key**: keep `msp-relay.key` stable across restarts or every peer's
  advertised `/p2p/<relay>/p2p-circuit` address dies with the old PeerId.
- **Resource shape**: trivial — RAM tens of MB; CPU negligible (pure
  forwarding); bandwidth = the sum of relayed token streams (7B-family
  chat is ~KB/s-scale; budget ~10 Mbps for comfortable multi-peer use).
  A cheapest-tier VPS (~$5/mo, 1 vCPU/1 GB) is far more than enough;
  Fly.io/cheap VPS both work — the standing stop-and-ask rule applies
  before creating anything.
- **Not on Vercel** (ADR-014 rejected; function-duration/bandwidth).
- **Not yet for the public internet** (gate D12): no relay authn/authz
  and no per-circuit caps — reservations are first-come. LAN/trusted
  deployment only until auth + per-circuit caps land.
- **Ops honesty**: logs are the stable `modelswarm-relay:` lines
  (reservation accepted/denied + status, circuit opened/denied + status,
  bind posture, peer id). Nothing else is emitted; no content ever.

## 7. Cross-path requests (no schema change needed; integration requests)

1. **Windows Product (crates/modelswarm-node, modelswarm-desktop)** —
   adopt the F2B surface: (a) serving side: `Libp2pTransport::reserve_relay`
   per configured relay, select over `Libp2pListener::accept` AND
   `RelayReservation::accept`, advertise `reservation.circuit_addr()` in
   the heartbeat `addresses` list ALONGSIDE the direct addr; (b)
   requester side: replace `RemoteExecutor`'s single `dial` with
   `dial_with_fallback(candidates, expected_peer, per_attempt)` — the
   `DialAttempt` report lines slot into the EXISTING `net.failure` /
   stage=`dial` telemetry (no new event names; verified against the
   frozen 91-event inventory — this crate emits nothing). Per-attempt
   budget guidance: short for direct (cross-NAT blackholes burn the whole
   budget), generous for relay. (c) desktop roster filter: the
   "quic-v1"-suffix dialability check ALSO matches circuit addrs —
   classify before display/selection.
2. **Protocol/Session seats** — decide where the §6.1 Ed25519 handshake
   runs over relayed circuits (end-to-end peer authn; closes risk 5.2).
   No new protocol fields are required for F2B itself.
3. **Tracker Engineer** — NO change required (circuit multiaddrs parse
   and ride `addresses`); optional courtesy: `/peers` consumers should
   treat the multiaddr list as mixed direct/relay.

## 8. Suggested next task

Owner-gated real-network proof (the actual F2B exit): deploy the relay per
§6 on a LAN-trusted host (the owner's second desktop qualifies), then the
node/desktop integration of §7.1, then rerun the three-machine family
swarm from outside the LAN and record the first relayed completion in
`docs/verification/f2b-relay-2026-10-1x.md` — including the honest
"relayed, not direct" labeling end-to-end. DCUtR hole-punch upgrade and
per-circuit caps + relay auth (gate D12, public internet) follow after.

— Network Engineer, 2026-10-15
