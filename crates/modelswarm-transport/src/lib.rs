//! Peer-to-peer transport for ModelSwarm: the ADR-018 staged backend plus
//! the MSP v1 request/stream message surface (`protocol/msp-v1.md` §6).
//!
//! **Staging decision (ADR-018):** Phases B–E run on
//! [`SignedFrameTransport`] — tokio TCP over loopback with length-prefixed
//! JSON frames and an Ed25519-signed handshake — while the protocol
//! mechanics (sessions, cancellation, deadlines, backpressure, diagnostics)
//! are exercised for real. The libp2p/QUIC/Noise backend lands at Phase F
//! and is the only backend allowed beyond loopback.
//!
//! What the staged backend provides and does not:
//!
//! - Integrity + peer authentication: the §6.1 handshake is signed over the
//!   canonical JSON of its fields and verified against the expected key
//!   (msp-v1 §2.2/§6.6).
//! - No confidentiality (no Noise) — acceptable only on loopback. Listeners
//!   refuse non-loopback binds ([`TransportError::NonLoopbackDenied`]),
//!   enforced and tested.
//! - Post-handshake messages travel inside the established session; per the
//!   §6.1 flow staging they are not individually signed.
//!
//! Wire encoding: this crate renders the frozen `protocol/messages.proto`
//! schema as snake_case JSON inside u32-BE length-prefixed frames (max
//! 256 KiB, msp-v1 §6.4). The Phase 3 framing ADR may change the encoding;
//! the fields never change.
//!
//! Key reuse: identities, canonical JSON, timestamps, and nonces come from
//! [`modelswarm_identity`] (ADR-004) — nothing here re-derives them.

pub mod error;
pub mod frame;
pub mod handshake;
pub mod message;
pub mod transport;

/// Re-exported so callers can name the key type that
/// [`Listener::accept`](transport::Listener::accept) verifies against without
/// depending on ed25519-dalek directly.
pub use ed25519_dalek;
pub use error::{FrameError, HandshakeError, TransportError, P2P_ERROR_CODES};
pub use frame::{read_frame, write_frame, MAX_FRAME_BYTES};
pub use handshake::verify_handshake;
pub use message::{
    Cancel, Cancelled, ChatMessage, Completed, Control, Handshake, HandshakeAck, InferenceRequest,
    Sampling, StreamError, TokenDelta, Usage, WireMessage, MSP_PROTOCOL_VERSION,
};
pub use transport::{
    Listener, Session, SignedFrameTransport, Stats, CANCEL_DEADLINE, RECV_CHANNEL_CAPACITY,
    SEND_CHANNEL_CAPACITY,
};
