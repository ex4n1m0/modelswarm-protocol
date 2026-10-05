# Phase F Verification Report

Date: 2026-10-05 · This commit.

## Delivered

**Adversarial suite** (10 tests, `session/tests/adversarial.rs`, over real
loopback sockets):

- F1 fabricated proposals (50 seeds, always-wrong drafts): output remains
  token-exact; ≤2 coincidental-vocab acceptances per session, never
  systematic.
- F2 lying invariants: wrong parent hash / stale round / wrong session /
  mutated tokens → typed rejections, zero state advance.
- F3 prefix equivocation (two lying verifiers): mismatch caught, nothing
  committed, explicit failure.
- F4 replay across reconnect/sessions/mutations: `duplicate` /
  unknown-session / `hash_mismatch`.
- F5 withholding: silent proposer absorbed (straggler path) in both
  two-peer and multi modes; no coordinator stall.
- F7 malicious verifier: coordinator-side `audit_final_chain` (full local
  re-decode before emitting) catches a rogue verifier's garbage hash;
  control sessions pass. Cost: one decode per session — priced security
  overhead (ADR-13 rule honored).
- F8 flood: 1000 rapid malformed/oversized frames — bounded memory, clean
  rejections, server serves a legitimate session afterward.
- F9 malformed-frame fuzz: 10 000 seeded mutations (truncation, bit flips,
  oversize, invalid UTF-8, deep nesting) — no panics, no unbounded
  allocation.
- F10 redaction gate: canary prompt string absent from every telemetry
  sink line across a full adversarial session.

**ADR-020 migration (F12)**: real libp2p PeerId derivation
(`12D3Koo…` identity-key form) in Rust + tracker TS, hard-switched
(placeholder removed, alias deprecated), cross-language golden, plus a
libp2p cross-check test — which **found and fixed the ADR's own wrong byte
formula** (literal reading produced a CIDv0 `Qm…`; corrected to the
identity-multihash-of-protobuf-key form; ADR amended with the correction).
Tracker suite green (81/81) after the switch.

**Relay path (F13)**: sim `relay` scenario — B→A→C over two TCP sessions
with A forwarding frames; 59 frames relayed; output token-exact; receipts
verified; output itself carries the stand-in note (Circuit-Relay-v2
replaces it at the backend swap). Scheduler: relayed-path penalty (40 ms)
and hole-punched penalty (10 ms) in `predicted_single_ms` (ADR-014 rule),
tested.

**F11 libp2p backend — PARTIAL, honestly recorded**: dependency resolution
and compilation succeeded (libp2p 0.57.0, 5 iterations against the
classic Transport/StreamMuxer traits); the ADR-020 cross-check and
loopback-refusal tests pass; the **loopback round-trip times out** (no
`TransportEvent::Incoming` within 10 s on the announced QUIC addr) — the
test is `#[ignore]`d at the pre-agreed 25-minute bound. The workspace is
green with and without the `libp2p-backend` feature. Verdict: the backend
compiles and its identity semantics are verified, but "D/E suites re-run
over it" is **unmet**; the version-pin matrix and swarm-path diagnosis are
recorded in `docs/research/fuzz-targets.md`. This is the one open
engineering item Phase F leaves.

## Integrator-run gates

`cargo fmt --all --check` PASS · `clippy --workspace --all-targets
-- -D warnings` exit 0 · `cargo test --workspace` **271/0** · tracker
`npm test` **81/81**, `validate:vectors` 2/2 · `sim relay 13` exact with
59 relayed frames.

## Hard stops (unchanged, honored)

Public relay hosting (paid), DNS/production credentials, code signing —
none attempted.

## Addendum 2026-10-05 (integrator follow-up): F11 RESOLVED

The libp2p round-trip is fixed (both root causes and the fix recorded in
`docs/research/fuzz-targets.md` §F11 resolution): dedicated listener driver
task + lazy server-side stream materialization. Gates re-run by the
integrator: feature tests **36/0** (round trip + two-session reuse in
<0.2 s), workspace **271/0**, clippy `-D warnings` clean **with and
without** the `libp2p-backend` feature. F11 status moves from
partial → **functional at the transport surface**; reparameterizing the
D/E session suites over this backend remains recorded follow-up wiring
(identical message codec proven by the round-trip tests).
