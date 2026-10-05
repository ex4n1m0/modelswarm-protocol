//! Cryptographic services for ModelSwarm nodes: Ed25519 installation identity
//! (ADR-004), the signed request envelope (msp-v1 §2.3), canonical JSON
//! signing form (§2.2), RFC 3339 timestamps, replay windows, and nonces.
//!
//! Capability-token/lease verification lives in `modelswarm-eligibility`
//! (ADR-012). The libp2p `PeerId` type arrives with `modelswarm-transport`;
//! until then [`InstallationIdentity::peer_id_label`] provides the identical
//! base58(SHA-256(pubkey)) string.

pub mod canonical;
pub mod envelope;
pub mod installation;
pub mod timestamp;

pub use canonical::canonical_json;
pub use envelope::SignedEnvelope;
pub use installation::{installation_id_for, InstallationIdentity};
pub use timestamp::{
    empty_body_digest, new_nonce, new_nonce_with, rfc3339_now, within_window, REPLAY_WINDOW_SECS,
};
