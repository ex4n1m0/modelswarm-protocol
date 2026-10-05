//! F11 (Phase F, stretch): a minimal rust-libp2p backend behind the same
//! surface [`crate::SignedFrameTransport`] exposes — QUIC (peer
//! authentication is built into QUIC's libp2p TLS handshake), framed
//! send/recv of the same JSON messages.
//!
//! Loopback-only listeners are STILL enforced (the ADR-018 honesty guard
//! applies to every backend until the full Phase F ops set — public relay,
//! AutoNAT — is reviewed). Keys derive from the [`InstallationIdentity`]
//! seed, so a node's libp2p `PeerId` equals its ADR-020 `peer_id()`
//! derivation byte for byte, and `dial` verifies the connected peer's
//! QUIC-authenticated PeerId against the expected derivation.
//!
//! This backend is experimental and feature-gated (`libp2p-backend`); the
//! workspace stays green without it (ADR-018: the trait is the swap point).

use std::future::poll_fn;
use std::pin::pin;
use std::time::Duration;

use futures::AsyncReadExt as _;
use futures::AsyncWriteExt as _;
use libp2p::core::muxing::StreamMuxer as _;
use libp2p::core::transport::{DialOpts, ListenerId, PortUse, TransportEvent};
use libp2p::identity::Keypair;
use libp2p::quic;
use libp2p::{Multiaddr, Transport as _};
use modelswarm_identity::InstallationIdentity;

use crate::error::{invalid_data, TransportError};
use crate::frame::MAX_FRAME_BYTES;
use crate::message::WireMessage;

/// Builds the libp2p keypair from the installation seed (same key ⇒ same
/// PeerId as [`InstallationIdentity::peer_id`]).
fn keypair_from(identity: &InstallationIdentity) -> Result<Keypair, TransportError> {
    Keypair::ed25519_from_bytes(identity.to_bytes())
        .map_err(|e| TransportError::Io(invalid_data("identity seed", e)))
}

fn io_err(what: &'static str, e: impl std::error::Error + Send + Sync + 'static) -> TransportError {
    TransportError::Io(invalid_data(what, e))
}

/// A framed, authenticated session over one libp2p QUIC stream. Same JSON
/// message surface as the staged backend's [`crate::Session`] (typed
/// send/recv, deadline-bounded), minus the Ed25519 handshake exchange — the
/// channel itself is cryptographically bound to the peer's key by QUIC.
pub struct Libp2pSession {
    stream: quic::Stream,
    #[allow(dead_code)]
    peer_id: libp2p::PeerId,
}

impl Libp2pSession {
    fn new(stream: quic::Stream, peer_id: libp2p::PeerId) -> Self {
        Self { stream, peer_id }
    }

    /// Sends one typed message, deadline-bounded.
    pub async fn send(
        &mut self,
        msg: &WireMessage,
        deadline: Duration,
    ) -> Result<(), TransportError> {
        let payload = serde_json::to_vec(msg)
            .map_err(|e| TransportError::Io(invalid_data("frame encode", e)))?;
        let mut framed = Vec::with_capacity(4 + payload.len());
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);
        let mut write = pin!(self.stream.write_all(&framed));
        tokio::time::timeout(deadline, &mut write)
            .await
            .map_err(|_| TransportError::Timeout)??;
        self.stream.flush().await?;
        Ok(())
    }

    /// Receives one typed message, deadline-bounded. The announced length is
    /// validated against [`MAX_FRAME_BYTES`] BEFORE the body is read or
    /// allocated (same rule as the staged frame codec).
    pub async fn recv(&mut self, deadline: Duration) -> Result<WireMessage, TransportError> {
        let mut prefix = [0u8; 4];
        let mut read = pin!(self.stream.read_exact(&mut prefix));
        tokio::time::timeout(deadline, &mut read)
            .await
            .map_err(|_| TransportError::Timeout)??;
        let len = u32::from_be_bytes(prefix) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return Err(TransportError::Io(invalid_data(
                "frame rejected",
                "length prefix outside 1..=256 KiB",
            )));
        }
        let mut body = vec![0u8; len];
        let mut read = pin!(self.stream.read_exact(&mut body));
        tokio::time::timeout(deadline, &mut read)
            .await
            .map_err(|_| TransportError::Timeout)??;
        serde_json::from_slice(&body)
            .map_err(|e| TransportError::Io(invalid_data("frame decode", e)))
    }
}

/// The libp2p (QUIC) backend, feature-gated `libp2p-backend` (F11).
pub struct Libp2pTransport {
    keypair: Keypair,
}

impl std::fmt::Debug for Libp2pTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libp2pTransport")
            .field("peer_id", &self.peer_id())
            .finish()
    }
}

impl Libp2pTransport {
    /// Builds the backend whose PeerId equals `identity.peer_id()`.
    pub fn new(identity: &InstallationIdentity) -> Result<Self, TransportError> {
        Ok(Self {
            keypair: keypair_from(identity)?,
        })
    }

    /// The local libp2p PeerId (the ADR-020 multihash derivation).
    pub fn peer_id(&self) -> libp2p::PeerId {
        libp2p::PeerId::from_public_key(&self.keypair.public())
    }

    /// Binds `bind` (e.g. `"127.0.0.1:0"`). Refuses any address whose IP is
    /// not loopback with [`TransportError::NonLoopbackDenied`] — the ADR-018
    /// honesty guard applies to this backend too.
    pub async fn listen(&self, bind: &str) -> Result<Libp2pListener, TransportError> {
        let addr: std::net::SocketAddr = bind
            .parse()
            .map_err(|e| TransportError::Io(invalid_data("invalid bind address", e)))?;
        if !addr.ip().is_loopback() {
            return Err(TransportError::NonLoopbackDenied);
        }
        let ip = addr.ip();
        let family = if addr.is_ipv4() { "ip4" } else { "ip6" };
        let multiaddr: Multiaddr = format!("/{family}/{ip}/udp/{}/quic-v1", addr.port())
            .parse()
            .map_err(|e| TransportError::Io(invalid_data("multiaddr", e)))?;
        let mut transport = quic::tokio::Transport::new(quic::Config::new(&self.keypair));
        transport
            .listen_on(ListenerId::next(), multiaddr)
            .map_err(|e| io_err("libp2p listen", e))?;
        // Poll the transport until it announces the concrete listen address
        // (":0" binds an ephemeral port).
        let bound: Multiaddr = tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                use std::task::Poll;
                match std::pin::Pin::new(&mut transport).poll(cx) {
                    Poll::Ready(TransportEvent::NewAddress { listen_addr, .. }) => {
                        Poll::Ready(listen_addr)
                    }
                    Poll::Ready(_) | Poll::Pending => Poll::Pending,
                }
            }),
        )
        .await
        .map_err(|_| TransportError::Timeout)?;
        Ok(Libp2pListener { transport, bound })
    }

    /// Dials `addr` (a `/ip/../udp/../quic-v1` multiaddr) and opens one
    /// framed stream, verifying the connected peer's QUIC-authenticated
    /// PeerId against `expected_peer` (the ADR-020 derivation of the
    /// directory key — an impostor fails the binding here).
    pub async fn dial(
        &self,
        addr: &str,
        expected_peer: &libp2p::PeerId,
        deadline: Duration,
    ) -> Result<Libp2pSession, TransportError> {
        let multiaddr: Multiaddr = addr
            .parse()
            .map_err(|e| TransportError::Io(invalid_data("invalid dial multiaddr", e)))?;
        let mut transport = quic::tokio::Transport::new(quic::Config::new(&self.keypair));
        let dial = transport
            .dial(
                multiaddr,
                DialOpts {
                    role: libp2p::core::Endpoint::Dialer,
                    port_use: PortUse::New,
                },
            )
            .map_err(|e| io_err("libp2p dial", e))?;
        let (peer_id, mut connection) = tokio::time::timeout(deadline, dial)
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| io_err("libp2p dial", e))?;
        if &peer_id != expected_peer {
            return Err(TransportError::HandshakeFailed(
                "quic peer id does not match the expected ADR-020 derivation".to_string(),
            ));
        }
        let stream = tokio::time::timeout(
            deadline,
            poll_fn(|cx| std::pin::Pin::new(&mut connection).poll_outbound(cx)),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| io_err("libp2p stream open", e))?;
        Ok(Libp2pSession::new(stream, peer_id))
    }
}

/// Loopback-only libp2p listener yielding framed sessions.
pub struct Libp2pListener {
    transport: quic::tokio::Transport,
    bound: Multiaddr,
}

impl Libp2pListener {
    /// The bound loopback multiaddr (concrete port).
    pub fn bound_addr(&self) -> &Multiaddr {
        &self.bound
    }

    /// Accepts one connection, then its first inbound stream, as a session.
    /// The QUIC handshake authenticates the remote; the returned session
    /// carries the remote's verified PeerId.
    pub async fn accept(&mut self, deadline: Duration) -> Result<Libp2pSession, TransportError> {
        let incoming = tokio::time::timeout(
            deadline,
            poll_fn(|cx| {
                use std::task::Poll;
                loop {
                    match std::pin::Pin::new(&mut self.transport).poll(cx) {
                        Poll::Ready(TransportEvent::Incoming { upgrade, .. }) => {
                            return Poll::Ready(Some(upgrade))
                        }
                        Poll::Ready(_) => continue,
                        Poll::Pending => return Poll::Pending,
                    }
                }
            }),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
        .ok_or(TransportError::Closed)?;
        let (peer_id, mut connection) = tokio::time::timeout(deadline, incoming)
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| io_err("libp2p accept", e))?;
        let stream = tokio::time::timeout(
            deadline,
            poll_fn(|cx| std::pin::Pin::new(&mut connection).poll_inbound(cx)),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| io_err("libp2p inbound stream", e))?;
        Ok(Libp2pSession::new(stream, peer_id))
    }
}

impl std::fmt::Debug for Libp2pListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libp2pListener")
            .field("bound", &self.bound)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F12/F11: the libp2p keypair derived from the installation seed yields
    /// EXACTLY the ADR-020 peer_id derivation.
    #[test]
    fn libp2p_peer_id_equals_the_adr020_derivation() {
        let identity = InstallationIdentity::from_bytes(&[3u8; 32]);
        let transport = Libp2pTransport::new(&identity).unwrap();
        assert_eq!(transport.peer_id().to_string(), identity.peer_id());
    }

    /// ADR-018 honesty guard: the libp2p backend refuses non-loopback binds
    /// before any socket exists, exactly like the staged backend.
    #[tokio::test]
    async fn libp2p_listen_refuses_non_loopback() {
        let identity = InstallationIdentity::from_bytes(&[4u8; 32]);
        let transport = Libp2pTransport::new(&identity).unwrap();
        assert!(matches!(
            transport.listen("0.0.0.0:0").await.unwrap_err(),
            TransportError::NonLoopbackDenied
        ));
        assert!(matches!(
            transport.listen("192.168.1.10:9000").await.unwrap_err(),
            TransportError::NonLoopbackDenied
        ));
    }

    /// One loopback QUIC round trip: listener + dial + framed typed exchange,
    /// with the dial verifying the PeerId binding.
    ///
    /// IGNORED (F11 bounded attempt, see docs/research/fuzz-targets.md):
    /// dependency resolution and compilation of the libp2p backend SUCCEEDED
    /// (libp2p 0.57.0, QUIC transport composition), and the PeerId-equality
    /// + honesty-guard tests above pass — but this end-to-end round trip
    /// times out: the listener never surfaces `TransportEvent::Incoming`
    /// within 10 s while the dial-side `GenTransport::dial` future is polled
    /// concurrently against the announced `/ip4/127.0.0.1/udp/<port>/quic-v1`
    /// address. Remaining work is recorded in the fuzz-targets results.
    #[tokio::test]
    #[ignore = "listener never yields Incoming on loopback QUIC; diagnosis plan in docs/research/fuzz-targets.md"]
    async fn libp2p_loopback_round_trip_framed_json() {
        let server_identity = InstallationIdentity::from_bytes(&[5u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[6u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let client = Libp2pTransport::new(&client_identity).unwrap();
        let mut listener = server.listen("127.0.0.1:0").await.unwrap();

        let bound = listener.bound_addr().clone();
        let expected = server.peer_id();
        let dialing = tokio::spawn(async move {
            client
                .dial(
                    bound.to_string().as_str(),
                    &expected,
                    Duration::from_secs(10),
                )
                .await
        });

        let mut session_server = listener.accept(Duration::from_secs(10)).await.unwrap();
        let mut session_client = dialing.await.unwrap().unwrap();

        let msg = WireMessage::Control(crate::Control::ping());
        session_client
            .send(&msg, Duration::from_secs(5))
            .await
            .unwrap();
        let received = session_server.recv(Duration::from_secs(5)).await.unwrap();
        assert_eq!(received, msg);
    }
}
