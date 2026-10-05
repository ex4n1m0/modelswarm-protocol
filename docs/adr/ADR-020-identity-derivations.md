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
2. `peer_id = base58(0x12 0x20 ‖ SHA-256(pubkey))` — wire identity, exactly
   libp2p's identity-key PeerId. Typed `PeerId` arrives with the libp2p
   backend (Phase F).
3. **Binding rule**: registration must present both, and the tracker
   validates `peer_id` = multihash-derivation of the registered `pubKey`
   (same key ⇒ deterministic pair; a mismatch is `400 invalid_body`).
4. Leases and capability records bind BOTH values (fields already exist).
5. Migration task (Phase F): tracker register/lookup validation switches
   from the placeholder equality to the multihash check; Rust
   `peer_id_label()` is replaced by a real libp2p PeerId; goldens updated.

## Consequences

+ No redefinition of either identifier later; hub records are already
  dual-fielded.
− Until Phase F, wire `peerId` values are installation-id strings —
  recorded here so nobody treats the placeholder as the spec.
