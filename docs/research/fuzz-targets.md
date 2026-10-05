# Fuzz & Adversarial Test Targets (design in Phase A; grow per phase)

Owner: Security Engineer · Threat model: `docs/threat-model.md` (extended
below) · Rule: fuzzers run in CI from Phase B onward for decoders.

## Decoder fuzz targets (Phase B)

- Canonical-JSON parser: arbitrary bytes → must reject or produce canonical
  round-trip-identical output; no panics, no unbounded allocation.
- Manifest deserializer: mutated `protocol/vectors` corpora (bit flips,
  truncation, key reordering, duplicate keys, deep nesting) → schema
  violations, never type confusion.
- Envelope/lease signature decoders: wrong lengths, non-canonical encodings,
  high-bit garbage.

## Property targets (Phase B gate, ADR-015)

Session-state machine: stale / duplicated / reordered / cross-profile /
cross-epoch messages can never advance state. Commit idempotence: replaying a
signed commit N times yields exactly one state advance.

## Adversarial scenarios (Phases D–F; from the revision's threat list)

1. Fabricated token proposals with plausible logprobs (verifier must catch;
   acceptance contract holds).
2. Lying about proposal probabilities → verification prevents output
   corruption; liar's receipts accumulate evidence → suspension policy.
3. Prefix equivocation: two peers claiming different prefixes for the same
   round → reject both, flag.
4. Replay of old commits/proposals across sessions and reconnects.
5. Withholding results after prefill (resource theft) → deadlines, slot
   release, reputation cost.
6. Latency-measurement manipulation → scheduler uses its own measurements
   (requester-measured RTT dominates; ADR-013).
7. Coordinator/verifier abuse: malicious verifier accepting garbage →
   auditor re-verification of sampled rounds catches divergence; Byzantine
   collusion assumptions documented per round, not assumed away.
8. Flood: registration, lookup, session-open storms → rate limits (msp-v1
   §3.4) + per-IP caps (Phase-1 acceptance F5–F7).
9. Malformed frames / oversized payloads at every P2P entry point.
10. Prompt-retention attempts by serving peers (logging assertions on both
    sides; redaction gates from phase acceptance index).

## Threat-model addendum (cooperative adversaries)

Add to `docs/threat-model.md` adversary set when Phase D begins: **H.
Byzantine swarm participant** (fabricates proposals, equivocates on prefixes,
colludes with other participants or a verifier, withholds work, attempts
prompt extraction/retention). Mitigations: per-message prefix/profile/
generation-param binding, random redundant verification, audit epochs,
receipt evidence, quarantine scoring — each priced for performance cost per
the revision's requirement to measure security overhead.

## Phase F results (2026-10-04, Security engineer)

Suite locations and outcomes, per `docs/acceptance/phase-f.md`:

- **F1 fabricated proposals** —
  `crates/modelswarm-session/tests/adversarial.rs::f1_*` (50 full sessions
  against a proposer answering every request with plausible garbage):
  verifier accepted ≤ 2 draft tokens per session (pure 1/256 coincidences,
  never a pattern), acceptance collapse tripped the bounded fallback on all
  50 seeds, output token-exact vs single decode everywhere.
- **F2 lying proposer invariants** — `f2_*`: wrong `parent_prefix_hash`,
  stale and future rounds → typed `cancel_round/parent_mismatch`; mutated
  tokens → honest `verification_result` (empty accepted prefix, correction =
  the true next token); unknown session → explicit `unknown session`
  failure; state advanced only from the properly signed commit.
- **F3 prefix equivocation** — `f3_*`: two attacker verifiers claiming two
  different `new_prefix_hash` values for the same round; the coordinator's
  own hash validation failed both with the typed
  `new_prefix_hash does not match the committed tokens` error and emitted no
  commit to either attacker.
- **F4 replay** — `f4_*`: captured signed commit replayed over a new
  connection → `duplicate` (benign no-op); commit naming a never-prefilled
  session → explicit wrong-session-style rejection; re-signed commit with a
  mutated token list carrying the honest hash → `hash_mismatch`.
- **F5 withholding** — `f5_*` (two variants): two-peer mode pays exactly one
  bounded round deadline then falls back `peer_lost` and stays exact;
  multi-proposer mode (3 proposers, 1 withholding) completes every round
  from the remaining two, counts the withholder a straggler on every round
  (`straggler_rounds == rounds`, `blocks_arrived == 0`), honest proposers
  serve all rounds.
- **F6 measured-dominance** — `crates/modelswarm-scheduler/src/lib.rs`
  tests `ten_x_advertised_lies_*` and `nat_path_penalty_*`: 10× advertised
  lies never outrank measured reality (the formula carries no advertised
  token-rate term); ADR-014 rule 4 added as
  `nat_path_penalty_ms` (`RELAYED_PATH_PENALTY_MS` = 40 ms,
  `HOLEPUNCHED_PATH_PENALTY_MS` = 10 ms, direct = 0) — relayed ranks
  strictly below direct all-else-equal and a relayed peer "better" by less
  than the penalty loses the ranking.
- **F7 malicious verifier** — `f7_*` plus the new coordinator-side
  `audit_final_chain` (in `crates/modelswarm-session/src/spec.rs`): a rogue
  verifier returning hash-CONSISTENT results over garbage (defeating the
  per-round chain check) is caught by the coordinator re-verifying the final
  committed chain against its own runtime before emitting; the session
  fails with `auditor divergence at committed token N`, no outcome is
  emitted, the rogue never sees a clean close. Honest control passes the
  same audit. This is the auditor pattern of threat-model addendum #7;
  its per-session cost is one load + `committed.len()` decode steps
  (ADR-013 security overhead, priced by the cost model).
- **F8 flood** — `f8_*`: 50 rapid connections × 20 frames = 1000
  malformed/oversized/valid-JSON-garbage frames at a verifier listener;
  every flood connection rejected with a typed error (no panics); a
  handshaked session fed garbage dies cleanly on the invalid-UTF-8 frame
  (bounded queues: `RECV_CHANNEL_CAPACITY` = 16 by construction); the same
  listener then serves a complete, exact speculative session.
- **F9 malformed-frame fuzz** — `f9_*`: 10 000 seeded xorshift64* mutations
  (truncation, 1–16 bit flips, invalid UTF-8 injection, oversize length
  prefixes, valid-JSON garbage, random soup, pristine controls) over a
  corpus of a real signed handshake, a real signed prefix commit, a wide
  object, and a 200 KiB deep-nesting document (≤ 1 MiB bound) against the
  frame decoder + `WireMessage`/`SpecMessage` deserializers: no panics,
  every successfully read payload ≤ `MAX_FRAME_BYTES` (256 KiB, enforced
  before body allocation), deep nesting rejected cleanly (serde_json
  recursion limit).
- **F10 redaction gate** — `f10_*`: a full adversarial session logged
  through `modelswarm-telemetry` with a prompt canary smuggled under
  forbidden field names (`prompt`, `completion`, `delta`): the canary is
  absent from every sink line; the redactor leaves `[REDACTED:*]` markers
  (events survive, values do not).
- **F12 ADR-020 migration** — `modelswarm-identity` `peer_id_for` +
  `InstallationIdentity::peer_id()` (deprecated `peer_id_label` alias
  forwards to it); handshake build/verify hard-switched to the multihash
  binding; tracker `derivePeerId` (pure TS) validates registration
  (`400 invalid_body` on mismatch, placeholder equality rejected by its own
  test); cross-language golden
  `12D3KooWSrKnMZUcSxK8G7wmBbXdU8nFEfWGhLu6H8xjn8LmCSJb` (seed `[9u8;32]`)
  asserted in Rust and TS. NOTE on ADR-020's shorthand: the literal
  `base58(0x12 0x20 ‖ sha256(pubkey))` produces the CIDv0-style `Qm…`
  digest multihash, NOT a PeerId; the mandated `12D3Koo` check defines the
  contract — the derivation is base58(identity-multihash(protobuf-ed25519-
  pubkey)) = `base58(0x00 0x24 ‖ 0x08 0x01 ‖ 0x12 0x20 ‖ pubkey)` (52
  chars), verified byte-identical to `libp2p::PeerId::from_public_key` by
  the F11 test below. ADR-020's `0x12 0x20` bytes appear as the protobuf
  Data field tag+length.
- **F13 relay path** — `apps/modelswarm-sim` `relay <seed>` scenario
  (`run_relay`/`relay_case` + `relay_path_is_end_to_end_exact` test): three
  nodes A(relay) B(coordinator) C(verifier+proposer); B's two sessions dial
  A's `Listener::accept_raw` loopback listeners, which forward every
  length-prefixed frame verbatim to C; the session completes end-to-end
  token-exact vs single decode with receipts verified (the relay is
  transport-only; the terminal peers' handshake/commit signatures
  authenticate through it). **libp2p Circuit-Relay-v2 replaces this whole
  mechanism at the backend swap** (ADR-014): the local relay exercises the
  scheduler/telemetry consequences (nat_path penalty, relayed-path
  labeling) on the real protocol flow, not the production relay protocol.
- **F11 libp2p backend (bounded stretch attempt)** — see the next section.


### F11 resolution (2026-10-05, integrator follow-up)

FIXED — the round trip now completes in <0.2 s (tests `libp2p_loopback_round_trip_framed_json`
and `libp2p_listener_serves_two_sessions`, feature `libp2p-backend`, `#[ignore]` removed).
Root causes, found by instrumented diagnosis rather than the version matrix:

1. **The listener transport must be polled continuously.** libp2p-quic's
   server-side handshake only progresses while `Transport::poll` is being
   driven; polling merely while a caller awaits `accept` loses the window
   (a racing `select!` proved the handshake finishes in <100 ms whenever an
   idle poller exists). Fix: each listener owns a dedicated driver task
   pumping `TransportEvent`s through a channel; `accept` is a channel read.
2. **QUIC streams are lazily visible to the peer.** The server's
   `poll_inbound` cannot complete until the client's first frame arrives,
   so awaiting it inside `accept` deadlocks (the dialer speaks first in
   every real libp2p protocol). Fix: `accept` returns on connection
   upgrade; the server session materializes its inbound stream lazily on
   first send/recv (`Role::Server`).

The version-pin matrix below is therefore moot. Remaining honest scope:
the D/E session suites still bind to `SignedFrameTransport` inside
`spec.rs`; reparameterizing them over this backend is recorded as the
follow-up wiring task (the message codec is already identical — the
backend round-trips the same framed `WireMessage`s).

--- (original bounded-attempt record below)

### F11 libp2p backend attempt (verbatim outcome)

Attempted: optional `libp2p-backend` feature on `modelswarm-transport`
(`crates/modelswarm-transport/src/libp2p_backend.rs`, `Libp2pTransport` /
`Libp2pListener` / `Libp2pSession`), `libp2p = "0.57.0"` with features
`tokio, quic, noise, yamux, identify, macros, ping` (noise/yamux/identify/
macros/ping resolve but are unused by the minimal QUIC composition).

- Dependency resolution: **SUCCESS** — `cargo fetch` of the libp2p 0.57.0
  tree completed in 8 s (network available).
- Compilation: **SUCCESS** — `cargo check`/`clippy` clean after five
  iterations against the libp2p-core 0.44 classic `Transport` trait
  (poll-based `listen_on`/`dial` + `TransportEvent`, `StreamMuxer::
  poll_inbound/poll_outbound` for streams; libp2p-quic 0.14 `Connection`).
- Tests: `libp2p_peer_id_equals_the_adr020_derivation` **PASSES** (the
  F12 derivation equals `libp2p::PeerId::from_public_key` exactly — this
  cross-check is what caught the protobuf Data-length bug above) and
  `libp2p_listen_refuses_non_loopback` **PASSES**. The end-to-end round
  trip **TIMES OUT** and is committed `#[ignore]`d with its reason:
  `called Result::unwrap() on an Err value: Timeout` — the listener never
  yields `TransportEvent::Incoming` within 10 s while the dial-side future
  is polled concurrently against the announced
  `/ip4/127.0.0.1/udp/<port>/quic-v1` address (bound addr verified
  announced via `NewAddress`). Marked ignored at the ~25-minute
  build-fighting bound; the default-feature workspace is unaffected.
- Version pin matrix to try next (recorded per the stop rule):
  1. libp2p `0.56.0` (last version docs.rs built — same core trait; may
     behave differently on Windows loopback QUIC).
  2. libp2p `0.55.x` with `quic` + `tokio` only (isolate whether the
     unused `noise`/`yamux` features perturb endpoint config).
  3. Swap the manual `poll_fn` driving for the `SwarmBuilder` swarm path
     (the maintained usage; the classic Transport composition may miss
     required endpoint housekeeping on the accept side).
  4. If still timing out: test the same dial/listen pair on a Linux CI
     runner to rule out a Windows loopback quinn quirk (quinn UDP
     socket options on loopback).
- The trait boundary (ADR-018) remains the swap point: `SignedFrameTransport`
  and `Libp2pTransport` both expose loopback-only `listen`, `dial`, and
  deadline-bounded framed JSON sessions; the D/E correctness suites re-run
  over the libp2p backend when the round trip is fixed (Phase F gate).
