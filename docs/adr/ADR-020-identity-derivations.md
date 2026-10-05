# ADR-020: Two derivations from the installation key — installation_id vs libp2p PeerId

Status: Accepted (Phase B, 2026-10-05) · Resolves the ambiguity flagged by
the Tracker implementation.

## Context

msp-v1 §2.1 defines `installationId = base58(SHA-256(public key))` and says
"the libp2p `PeerId` derives from the same key". These are **different**
derivations in libp2p: `PeerId = base58(identity-multihash(public key))` =
`base58(0x12 0x20 ‖ SHA-256(public key))` — the `12D3Koo…` form. During
Phase B (no libp2p on the wire yet, ADR-018), the tracker and Rust node use
`installation_id` everywhere a `peerId` appears, and the tracker validates
`peerId == installation_id`. That equality is a **placeholder**, not the
contract.

## Decision

1. `installation_id = base58(SHA-256(pubkey))` — hub-side identity, envelope
   signer identity (unchanged).
2. `peer_id` — wire identity, libp2p's identity-key PeerId
   (`12D3Koo…`). **Derivation (corrected in Phase F; see below)**:
   `base58(0x00 0x24 ‖ 0x08 0x01 ‖ 0x12 0x20 ‖ pubkey)` — an identity
   multihash wrapping the protobuf-encoded Ed25519 public key. The
   original formula in this ADR (`base58(0x12 0x20 ‖ SHA-256(pubkey))`)
   was **wrong**: it produces a CIDv0 digest (`Qm…`), not a PeerId. The
   corrected bytes are cross-verified against `libp2p::PeerId::
   from_public_key` by a test (which caught and fixed a protobuf
   Data-length bug, 0x24→0x20 in the embedded key, in both the Rust and TS
   implementations — evidence in `docs/research/fuzz-targets.md` Phase F
   results).
3. **Binding rule**: registration must present both, and the tracker
   validates `peer_id` = the corrected derivation of the registered
   `pubKey` (mismatch → `400 invalid_body`). Shipped in Phase F (hard
   switch; nothing was ever deployed).
4. Leases and capability records bind BOTH values (fields already exist).
5. ~~Migration task (Phase F)~~ — done in Phase F: Rust `peer_id()`
   replaces the placeholder `peer_id_label()` (kept as a deprecated
   alias); tracker validation switched; cross-language golden added.

## Consequences

+ No redefinition of either identifier later; hub records are already
  dual-fielded.
− Until Phase F, wire `peerId` values are installation-id strings —
  recorded here so nobody treats the placeholder as the spec.
