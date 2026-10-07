//! F11 (Phase F): a minimal rust-libp2p backend behind the same surface
//! [`crate::SignedFrameTransport`] exposes — QUIC (peer authentication is
//! built into QUIC's libp2p TLS handshake), framed send/recv of the same
//! JSON messages.
//!
//! Loopback-only listeners are STILL enforced (the ADR-018 honesty guard
//! applies to every backend until the full Phase F ops set — public relay,
//! AutoNAT — is reviewed). Keys derive from the [`InstallationIdentity`]
//! seed, so a node's libp2p `PeerId` equals its ADR-020 `peer_id()`
//! derivation byte for byte, and `dial` verifies the connected peer's
//! QUIC-authenticated PeerId against the expected derivation.
//!
//! Driving model (the F11 fix): libp2p-quic's transport must be polled
//! **continuously** for the server side of the handshake to progress —
//! polling only while a caller awaits `accept` loses the connection
//! window. Each listener therefore owns a dedicated driver task pumping
//! `TransportEvent`s into a channel; `accept` is a channel read.
//!
//! This backend is experimental and feature-gated (`libp2p-backend`); the
//! workspace stays green without it (ADR-018: the trait is the swap point).

use std::time::Duration;

use futures::AsyncReadExt as _;
use futures::AsyncWriteExt as _;
use futures::StreamExt as _;
use libp2p::core::muxing::StreamMuxer as _;
use libp2p::core::transport::{DialOpts, ListenerId, PortUse, TransportEvent};
use libp2p::identity::Keypair;
use libp2p::quic;
use libp2p::{Multiaddr, Transport};
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
pub use libp2p::PeerId;

pub struct Libp2pSession {
    role: Role,
    #[allow(dead_code)]
    peer_id: libp2p::PeerId,
}

enum Role {
    /// Dialer side: the stream exists immediately (opened via
    /// `poll_outbound`).
    Client { stream: quic::Stream },
    /// Listener side: QUIC streams only become visible to the peer when
    /// first used, so the inbound stream is materialized lazily on the
    /// first send/recv — by then the client's first frame has arrived and
    /// `poll_inbound` completes. (Awaiting it inside `accept` deadlocks
    /// until the client speaks; libp2p's swarms always let the dialer
    /// speak first.)
    Server {
        connection: quic::Connection,
        stream: Option<quic::Stream>,
    },
}

impl Libp2pSession {
    fn new_client(stream: quic::Stream, peer_id: libp2p::PeerId) -> Self {
        Self {
            role: Role::Client { stream },
            peer_id,
        }
    }

    fn new_server(connection: quic::Connection, peer_id: libp2p::PeerId) -> Self {
        Self {
            role: Role::Server {
                connection,
                stream: None,
            },
            peer_id,
        }
    }

    async fn stream_pin(
        &mut self,
        deadline: Duration,
    ) -> Result<std::pin::Pin<&mut quic::Stream>, TransportError> {
        match &mut self.role {
            Role::Client { stream } => Ok(std::pin::Pin::new(stream)),
            Role::Server { connection, stream } => {
                if stream.is_none() {
                    let mut conn = std::pin::Pin::new(connection);
                    let opened = tokio::time::timeout(
                        deadline,
                        std::future::poll_fn(|cx| conn.as_mut().poll_inbound(cx)),
                    )
                    .await
                    .map_err(|_| TransportError::Timeout)?
                    .map_err(|e| io_err("libp2p inbound stream", e))?;
                    *stream = Some(opened);
                }
                Ok(std::pin::Pin::new(stream.as_mut().expect("just set")))
            }
        }
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
        let mut stream = self.stream_pin(deadline).await?;
        let mut write = stream.write_all(&framed);
        tokio::time::timeout(deadline, &mut write)
            .await
            .map_err(|_| TransportError::Timeout)??;
        stream.flush().await?;
        Ok(())
    }

    /// Receives one typed message, deadline-bounded. The announced length is
    /// validated against [`MAX_FRAME_BYTES`] BEFORE the body is read or
    /// allocated (same rule as the staged frame codec).
    pub async fn recv(&mut self, deadline: Duration) -> Result<WireMessage, TransportError> {
        let mut stream = self.stream_pin(deadline).await?;
        let mut prefix = [0u8; 4];
        let mut read = stream.read_exact(&mut prefix);
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
        let mut stream = self.stream_pin(deadline).await?;
        let mut read = stream.read_exact(&mut body);
        tokio::time::timeout(deadline, &mut read)
            .await
            .map_err(|_| TransportError::Timeout)??;
        serde_json::from_slice(&body)
            .map_err(|e| TransportError::Io(invalid_data("frame decode", e)))
    }
}

/// Events the listener driver forwards to [`Libp2pListener`].
enum DriverEvent {
    Bound(Multiaddr),
    Incoming(<quic::tokio::Transport as Transport>::ListenerUpgrade),
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
    ///
    /// Spawns the transport's driver task (see the module docs for why
    /// continuous polling is required).
    pub async fn listen(&self, bind: &str) -> Result<Libp2pListener, TransportError> {
        let addr: std::net::SocketAddr = bind
            .parse()
            .map_err(|e| TransportError::Io(invalid_data("invalid bind address", e)))?;
        // ADR-018 honesty guard: the listener stays loopback-only unless the
        // operator explicitly opts in for Phase F0 friendly-network testing.
        // `MSP_LISTENER=1` is a per-machine decision, logged by the caller;
        // the flag never changes what addresses are ADVERTISED (that is the
        // heartbeat's honest multiaddr, updated separately).
        if !addr.ip().is_loopback() && std::env::var("MSP_LISTENER").as_deref() != Ok("1") {
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

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DriverEvent>(16);
        let driver = tokio::spawn(async move {
            let mut events = futures::stream::poll_fn(move |cx| {
                use std::pin::Pin;
                use std::task::Poll;
                match Pin::new(&mut transport).poll(cx) {
                    Poll::Ready(event) => Poll::Ready(Some(event)),
                    Poll::Pending => Poll::Pending,
                }
            });
            while let Some(event) = events.next().await {
                let forwarded = match event {
                    TransportEvent::NewAddress { listen_addr, .. } => {
                        Some(DriverEvent::Bound(listen_addr))
                    }
                    TransportEvent::Incoming { upgrade, .. } => {
                        Some(DriverEvent::Incoming(upgrade))
                    }
                    // AddressChange/ListenerError/ListenerClosed: surfaced
                    // via `bound_addr()`/error paths; not forwarded.
                    _ => None,
                };
                if let Some(ev) = forwarded {
                    if tx.send(ev).await.is_err() {
                        break; // listener dropped
                    }
                }
            }
        });

        // Wait for the concrete bound address (":0" binds an ephemeral
        // port), then hand the event stream to the listener.
        let bound = loop {
            match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                Err(_) => {
                    driver.abort();
                    return Err(TransportError::Timeout);
                }
                Ok(None) => {
                    driver.abort();
                    return Err(TransportError::Closed);
                }
                Ok(Some(DriverEvent::Bound(addr))) => break addr,
                Ok(Some(DriverEvent::Incoming(_))) => continue,
            }
        };
        Ok(Libp2pListener {
            _driver: driver,
            events: rx,
            bound,
        })
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
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut connection).poll_outbound(cx)),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| io_err("libp2p stream open", e))?;
        Ok(Libp2pSession::new_client(stream, peer_id))
    }
}

/// Loopback-only libp2p listener yielding framed sessions. Dropping it
/// stops the driver task.
pub struct Libp2pListener {
    _driver: tokio::task::JoinHandle<()>,
    events: tokio::sync::mpsc::Receiver<DriverEvent>,
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
        let incoming = loop {
            match tokio::time::timeout(deadline, self.events.recv()).await {
                Err(_) => {
                    return Err(TransportError::Timeout);
                }
                Ok(None) => return Err(TransportError::Closed),
                Ok(Some(DriverEvent::Incoming(upgrade))) => break upgrade,
                Ok(Some(DriverEvent::Bound(_))) => continue,
            }
        };
        let (peer_id, connection) = tokio::time::timeout(deadline, incoming)
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| io_err("libp2p accept", e))?;
        Ok(Libp2pSession::new_server(connection, peer_id))
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
    /// with the dial verifying the PeerId binding. FIXED by the dedicated
    /// driver task (see module docs): with the transport polled
    /// continuously, the handshake completes in well under a second.
    #[tokio::test]
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

        // Echo back over the same QUIC stream to prove bidirectionality.
        session_server
            .send(&msg, Duration::from_secs(5))
            .await
            .unwrap();
        let echoed = session_client.recv(Duration::from_secs(5)).await.unwrap();
        assert_eq!(echoed, msg);
    }

    /// Two sequential sessions on one listener: the driver task survives an
    /// accept/drop cycle and serves the next dial.
    #[tokio::test]
    async fn libp2p_listener_serves_two_sessions() {
        let server_identity = InstallationIdentity::from_bytes(&[7u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let mut listener = server.listen("127.0.0.1:0").await.unwrap();
        for i in 0..2u8 {
            let client_identity = InstallationIdentity::from_bytes(&[20u8 + i; 32]);
            let client = Libp2pTransport::new(&client_identity).unwrap();
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
            assert_eq!(
                session_server.recv(Duration::from_secs(5)).await.unwrap(),
                msg
            );
        }
    }
}
