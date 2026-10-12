//! F11 (Phase F): a minimal rust-libp2p backend behind the same surface
//! [`crate::SignedFrameTransport`] exposes — QUIC (peer authentication is
//! built into QUIC's libp2p TLS handshake), framed send/recv of the same
//! JSON messages, plus the ADR-018 amended-staging raw-JSON passthrough
//! ([`Libp2pSession::send_json`]/[`Libp2pSession::recv_json`]) with the
//! same stash discipline as the staged backend's [`crate::Session`], so
//! cooperative extension families (bare JSON frames, e.g. ADR-032
//! VerifyDrafts) ride QUIC without joining the frozen §6 typed surface.
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

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures::AsyncReadExt as _;
use futures::AsyncWriteExt as _;
use futures::StreamExt as _;
use libp2p::core::muxing::StreamMuxer as _;
use libp2p::core::transport::{DialOpts, ListenerId, PortUse, TransportEvent};
use libp2p::identity::Keypair;
use libp2p::quic;
use libp2p::{Multiaddr, Transport};
use modelswarm_identity::InstallationIdentity;
use serde_json::Value;

use crate::error::{invalid_data, TransportError};
use crate::frame::MAX_FRAME_BYTES;
use crate::message::{Control, WireMessage};
use crate::transport::Stats;

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
    peer_id: libp2p::PeerId,
    /// Typed frames intercepted by [`Libp2pSession::recv_json`] while waiting
    /// for a raw passthrough frame (delivered by the next
    /// [`Libp2pSession::recv`]).
    stashed_typed: VecDeque<WireMessage>,
    /// Raw passthrough frames intercepted by [`Libp2pSession::recv`] while
    /// waiting for a typed message (delivered by the next
    /// [`Libp2pSession::recv_json`]).
    stashed_raw: VecDeque<Value>,
    /// Bytes already pulled off the QUIC stream toward the NEXT frame but not
    /// yet reassembled into one (prefix + body share the buffer; the u32-BE
    /// prefix is the first 4 bytes). Recv deadlines on this backend are
    /// WAKE-UPS (polling loops depend on it), so `futures`' `read_exact` —
    /// which silently loses partially-read bytes when its future is dropped —
    /// cannot be used: a timeout mid-frame keeps these bytes and the next
    /// recv continues the same frame instead of desynchronizing the stream.
    partial: Vec<u8>,
}

/// One decoded inbound frame: a typed [`WireMessage`] or, when the payload is
/// valid JSON but not a typed message, the raw passthrough value delivered to
/// [`Libp2pSession::recv_json`] (extension namespaces, ADR-018 amended
/// staging — mirrors the staged backend's reader task).
enum InboundFrame {
    Typed(WireMessage),
    Raw(Value),
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
    /// The remote's QUIC-authenticated PeerId (lease binding, F3).
    pub fn remote_peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    fn new_client(stream: quic::Stream, peer_id: libp2p::PeerId) -> Self {
        Self {
            role: Role::Client { stream },
            peer_id,
            stashed_typed: VecDeque::new(),
            stashed_raw: VecDeque::new(),
            partial: Vec::new(),
        }
    }

    fn new_server(connection: quic::Connection, peer_id: libp2p::PeerId) -> Self {
        Self {
            role: Role::Server {
                connection,
                stream: None,
            },
            peer_id,
            stashed_typed: VecDeque::new(),
            stashed_raw: VecDeque::new(),
            partial: Vec::new(),
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
        self.write_framed(&payload, deadline).await
    }

    /// Sends one raw JSON value as a frame (extension-namespace passthrough;
    /// ADR-018 amended staging — the same surface as the staged backend's
    /// [`crate::Session::send_json`]).
    ///
    /// The value is framed exactly as serialized — it is NOT wrapped in a
    /// typed [`WireMessage`], so only a peer using
    /// [`Libp2pSession::recv_json`] can read it. ONE JSON document per frame,
    /// under the same 256 KiB cap (msp-v1 §6.4), refused before any socket
    /// write. A timeout here, as everywhere on this backend, leaves the
    /// session open.
    pub async fn send_json(
        &mut self,
        value: &Value,
        deadline: Duration,
    ) -> Result<(), TransportError> {
        let payload = serde_json::to_vec(value)
            .map_err(|e| TransportError::Io(invalid_data("frame encode", e)))?;
        self.write_framed(&payload, deadline).await
    }

    /// Shared framed write: u32-BE length prefix + payload, deadline-bounded.
    /// Payloads above [`MAX_FRAME_BYTES`] are refused BEFORE any socket
    /// write — the same rule the staged backend's writer task enforces (a
    /// peer's `recv` would reject the prefix anyway; failing locally is the
    /// honest error).
    async fn write_framed(
        &mut self,
        payload: &[u8],
        deadline: Duration,
    ) -> Result<(), TransportError> {
        if payload.len() > u32::MAX as usize || payload.len() > MAX_FRAME_BYTES {
            return Err(TransportError::TooLarge);
        }
        let mut framed = Vec::with_capacity(4 + payload.len());
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(payload);
        let mut stream = self.stream_pin(deadline).await?;
        let mut write = stream.write_all(&framed);
        tokio::time::timeout(deadline, &mut write)
            .await
            .map_err(|_| TransportError::Timeout)??;
        stream.flush().await?;
        Ok(())
    }

    /// Receives one typed application message, deadline-bounded.
    ///
    /// F15: control PINGS are answered transparently in-band and never
    /// delivered to the application — the same contract as the staged
    /// backend's reader task, so a serving loop waiting for the next
    /// `InferenceRequest` doubles as an RTT responder without knowing it.
    /// Pongs and every other frame are delivered. The deadline bounds the
    /// WHOLE call including any ping answering: a ping flood cannot extend
    /// a caller's wait beyond what it asked for.
    ///
    /// Raw JSON passthrough frames (see [`Libp2pSession::send_json`]) are
    /// stashed for [`Libp2pSession::recv_json`] instead of surfacing here
    /// (the same stash discipline as the staged backend's `Session`), and a
    /// deadline expiry is a WAKE-UP: the session stays open, already-read
    /// frame bytes are kept, and no frame is lost.
    pub async fn recv(&mut self, deadline: Duration) -> Result<WireMessage, TransportError> {
        let started = Instant::now();
        loop {
            // Stash first: a frame already received is never hidden behind an
            // expired deadline.
            if let Some(msg) = self.stashed_typed.pop_front() {
                return Ok(msg);
            }
            let remaining = deadline
                .checked_sub(started.elapsed())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err(TransportError::Timeout);
            }
            match self.recv_frame(remaining).await? {
                InboundFrame::Typed(WireMessage::Control(control))
                    if control.kind == Control::PING =>
                {
                    let pong = Control::pong(&control.nonce);
                    self.send(&WireMessage::Control(pong), remaining).await?;
                    continue;
                }
                InboundFrame::Typed(msg) => return Ok(msg),
                InboundFrame::Raw(value) => self.stashed_raw.push_back(value),
            }
        }
    }

    /// Receives the next raw JSON passthrough frame (see
    /// [`Libp2pSession::send_json`]) — the ADR-018 amended-staging surface
    /// the staged backend's [`crate::Session::recv_json`] defines.
    ///
    /// Typed [`WireMessage`] frames that arrive meanwhile are stashed and
    /// delivered by the next [`Libp2pSession::recv`]. Control PINGs are
    /// answered transparently in-band, never delivered. Deadline policy
    /// mirrors this backend's `recv`: expiry is a WAKE-UP, not a session
    /// failure — the session stays open and mid-frame bytes are reassembled
    /// from the session's partial buffer on the next call — so polling loops
    /// (e.g. a WAIT_COMMIT bound) never lose frames.
    pub async fn recv_json(&mut self, deadline: Duration) -> Result<Value, TransportError> {
        let started = Instant::now();
        loop {
            if let Some(value) = self.stashed_raw.pop_front() {
                return Ok(value);
            }
            let remaining = deadline
                .checked_sub(started.elapsed())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err(TransportError::Timeout);
            }
            match self.recv_frame(remaining).await? {
                InboundFrame::Typed(WireMessage::Control(control))
                    if control.kind == Control::PING =>
                {
                    let pong = Control::pong(&control.nonce);
                    self.send(&WireMessage::Control(pong), remaining).await?;
                    continue;
                }
                InboundFrame::Typed(msg) => self.stashed_typed.push_back(msg),
                InboundFrame::Raw(value) => return Ok(value),
            }
        }
    }

    /// Measures round-trip times with `samples` ping/pong probes over this
    /// QUIC session — F15's requester-side RTT source on the production
    /// path. Every probe (write + reply) is bounded by `deadline`.
    ///
    /// CONTRACT: call only BETWEEN requests on an idle session (the pooled
    /// executor probes after a clean completion, before returning the
    /// session to its pool). Application frames that arrive meanwhile are
    /// skipped and lost — the identical contract to the staged backend's
    /// [`crate::Session::measure_rtt`].
    pub async fn measure_rtt(
        &mut self,
        samples: usize,
        deadline: Duration,
    ) -> Result<Stats, TransportError> {
        let mut durations = Vec::with_capacity(samples);
        for _ in 0..samples {
            let ping = Control::ping();
            let nonce = ping.nonce.clone();
            let started = Instant::now();
            self.send(&WireMessage::Control(ping), deadline).await?;
            loop {
                match self.recv(deadline).await? {
                    WireMessage::Control(control)
                        if control.kind == Control::PONG && control.nonce == nonce =>
                    {
                        break;
                    }
                    // A stale pong from an earlier (lost) probe: skip.
                    _ => continue,
                }
            }
            durations.push(started.elapsed());
        }
        Ok(Stats::from_durations(&durations))
    }

    /// One raw frame off the stream: length prefix validated against
    /// [`MAX_FRAME_BYTES`] BEFORE the body is read or allocated (same rule
    /// as the staged frame codec). The payload decodes as a typed
    /// [`WireMessage`] when it is one, and as a raw JSON value otherwise
    /// (the extension-namespace passthrough); only a payload that is not
    /// valid JSON at all corrupts the session.
    ///
    /// Reads go through the session's cancel-safe partial buffer
    /// (see [`Libp2pSession::partial`]): a deadline expiry mid-frame keeps
    /// the already-read bytes for the next call.
    async fn recv_frame(&mut self, deadline: Duration) -> Result<InboundFrame, TransportError> {
        self.fill_frame_bytes(4, deadline).await?;
        let prefix = [
            self.partial[0],
            self.partial[1],
            self.partial[2],
            self.partial[3],
        ];
        let len = u32::from_be_bytes(prefix) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return Err(TransportError::Io(invalid_data(
                "frame rejected",
                "length prefix outside 1..=256 KiB",
            )));
        }
        self.fill_frame_bytes(4 + len, deadline).await?;
        self.partial.drain(..4);
        // Exactly `len` body bytes belong to THIS frame; anything a single
        // chunk read pulled past it is the START of the next frame and stays
        // buffered (QUIC may coalesce multiple frames into one read).
        let tail = self.partial.split_off(len);
        let body = std::mem::replace(&mut self.partial, tail);
        match serde_json::from_slice::<WireMessage>(&body) {
            Ok(msg) => Ok(InboundFrame::Typed(msg)),
            Err(_) => match serde_json::from_slice::<Value>(&body) {
                Ok(value) => Ok(InboundFrame::Raw(value)),
                Err(e) => Err(TransportError::Io(invalid_data("frame decode", e))),
            },
        }
    }

    /// Cancel-safe frame re-assembly: pulls bytes into the session's partial
    /// buffer until it holds `want` bytes of the current frame. Each single
    /// quinn read is cancel-safe (a `Pending` poll consumes nothing), so a
    /// deadline expiry loses nothing — unlike `read_exact`, whose future
    /// would silently drop partially-read bytes when dropped.
    async fn fill_frame_bytes(
        &mut self,
        want: usize,
        deadline: Duration,
    ) -> Result<(), TransportError> {
        while self.partial.len() < want {
            let mut chunk = [0u8; 8192];
            let mut stream = self.stream_pin(deadline).await?;
            let read = stream.read(&mut chunk);
            match tokio::time::timeout(deadline, read).await {
                Ok(Ok(0)) => {
                    // EOF at a frame boundary is the peer's clean FIN;
                    // mid-frame it is truncation.
                    return Err(if self.partial.is_empty() {
                        TransportError::Closed
                    } else {
                        TransportError::Io(invalid_data(
                            "libp2p stream read",
                            "stream ended mid-frame",
                        ))
                    });
                }
                Ok(Ok(n)) => self.partial.extend_from_slice(&chunk[..n]),
                Ok(Err(e)) => return Err(io_err("libp2p stream read", e)),
                Err(_) => return Err(TransportError::Timeout),
            }
        }
        Ok(())
    }

    /// Graceful session teardown — the QUIC analogue of the OS-level close
    /// the staged TCP backend gets for free. FINs the stream (the FIN is
    /// stream-ordered AFTER every frame written so far, so its delivery
    /// implies their delivery), then waits — bounded by `drain` — for the
    /// peer to observe the end (its own FIN/EOF, a stream error, or the
    /// bound).
    ///
    /// Plain DROP has no such guarantee: quinn's implicit close on the last
    /// handle drop sends CONNECTION_CLOSE immediately, which can overtake
    /// queued stream data and lose the LAST sent frame(s) — the reason the
    /// msp-v1 serving bridge `linger`s after its final frame before
    /// dropping (`modelswarm-node/src/serving.rs`). Callers that cannot
    /// await (a bare `Drop`) either keep this method's contract via a
    /// spawned task or accept the abrupt-drop semantics.
    pub async fn close_graceful(&mut self, drain: Duration) {
        // FIN the send half: instant (quinn's finish only marks the FIN).
        if let Ok(mut stream) = self.stream_pin(Duration::from_millis(500)).await {
            let _ = stream.close().await;
        }
        // Bounded drain: read until the peer's FIN/EOF, a stream error, or
        // the deadline. Bytes read here are discarded (the caller is done).
        let _ = tokio::time::timeout(drain, async {
            let mut scratch = [0u8; 256];
            loop {
                let Ok(mut stream) = self.stream_pin(Duration::from_millis(500)).await else {
                    return;
                };
                match stream.read(&mut scratch).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => continue,
                }
            }
        })
        .await;
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

        // A PONG is a delivered application frame (pings are now answered
        // transparently inside recv, never delivered — see measure_rtt).
        let msg = WireMessage::Control(Control::pong("round-trip"));
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
            let msg = WireMessage::Control(Control::pong("per-session"));
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

    /// ADR-018 amended staging on QUIC: raw-JSON passthrough frames and typed
    /// `WireMessage` frames interleave on ONE session without confusing
    /// either side — `recv` hands over exactly the typed frames (raw ones
    /// stashed) and `recv_json` exactly the raw ones (typed stashed), in
    /// order, both directions.
    #[tokio::test]
    async fn libp2p_json_passthrough_interleaves_with_typed_frames() {
        let server_identity = InstallationIdentity::from_bytes(&[41u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[42u8; 32]);
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
        let mut peer_server = listener.accept(Duration::from_secs(10)).await.unwrap();
        let mut peer_client = dialing.await.unwrap().unwrap();

        // Client sends raw, typed, raw; server asks in a DIFFERENT order —
        // the stash discipline must keep every frame exactly once.
        let raw1 = serde_json::json!({"window_tokens": [1, 2, 3]});
        let typed = WireMessage::Control(Control::pong("interleave"));
        let raw2 = serde_json::json!({"accepted_token_ids": [7], "round": 2});
        peer_client
            .send_json(&raw1, Duration::from_secs(5))
            .await
            .unwrap();
        peer_client
            .send(&typed, Duration::from_secs(5))
            .await
            .unwrap();
        peer_client
            .send_json(&raw2, Duration::from_secs(5))
            .await
            .unwrap();

        // recv (typed) first: it must stash BOTH raw frames and still return
        // the one typed frame.
        assert_eq!(
            peer_server.recv(Duration::from_secs(5)).await.unwrap(),
            typed
        );
        // Then the raw frames, in order, via recv_json.
        assert_eq!(
            peer_server.recv_json(Duration::from_secs(5)).await.unwrap(),
            raw1
        );
        assert_eq!(
            peer_server.recv_json(Duration::from_secs(5)).await.unwrap(),
            raw2
        );

        // Reverse direction: server sends typed-then-raw; client's recv_json
        // stashes the typed frame and returns the raw one.
        peer_server
            .send(&typed, Duration::from_secs(5))
            .await
            .unwrap();
        peer_server
            .send_json(&raw1, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            peer_client.recv_json(Duration::from_secs(5)).await.unwrap(),
            raw1
        );
        assert_eq!(
            peer_client.recv(Duration::from_secs(5)).await.unwrap(),
            typed
        );
    }

    /// The deadline contract the ADR-032 serve loop depends on: a `recv_json`
    /// timeout is a pure WAKE-UP. The session survives, a frame that arrives
    /// afterwards is received intact — even a large frame whose delivery
    /// spans several timeout cycles mid-body (the cancel-safe partial buffer
    /// re-assembles it; `read_exact` cancellation would desync the stream).
    #[tokio::test]
    async fn libp2p_recv_json_timeouts_are_wakeups_and_lose_no_bytes() {
        let server_identity = InstallationIdentity::from_bytes(&[43u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[44u8; 32]);
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
        let mut peer_server = listener.accept(Duration::from_secs(10)).await.unwrap();
        let mut peer_client = dialing.await.unwrap().unwrap();

        // Nothing in flight yet: a short deadline must elapse (a wake-up),
        // not fail the session.
        assert!(matches!(
            peer_server.recv_json(Duration::from_millis(20)).await,
            Err(TransportError::Timeout)
        ));

        // A ~240 KiB JSON document (multi-packet on QUIC): the server spins
        // through several 1 ms wake-ups while it may be mid-body. Whichever
        // call finally completes the frame — a short one mid-flight or the
        // generous one after — the frame must be INTACT (partial bytes
        // survived every cancellation).
        let big = serde_json::json!({"blob": "x".repeat(240_000), "round": 1});
        peer_client
            .send_json(&big, Duration::from_secs(10))
            .await
            .unwrap();
        let mut received = false;
        for _ in 0..16 {
            if let Ok(value) = peer_server.recv_json(Duration::from_millis(1)).await {
                assert_eq!(value, big, "no byte may be lost to a cancelled recv");
                received = true;
                break;
            }
            // Every elapsed deadline was a wake-up: liveness is the loop.
        }
        if !received {
            assert_eq!(
                peer_server
                    .recv_json(Duration::from_secs(10))
                    .await
                    .unwrap(),
                big,
                "the frame must arrive intact across the wake-up cycles"
            );
        }

        // The session still carries a follow-up frame in both directions.
        let follow_up = serde_json::json!({"accepted_token_ids": [9]});
        peer_client
            .send_json(&follow_up, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            peer_server.recv_json(Duration::from_secs(5)).await.unwrap(),
            follow_up
        );
    }

    /// msp-v1 §6.4 on the passthrough: payloads above the 256 KiB frame cap
    /// are refused BEFORE any socket write (the receiving side would reject
    /// the length prefix anyway), and the refusal leaves the session usable.
    #[tokio::test]
    async fn libp2p_send_json_refuses_oversized_payload_before_writing() {
        let server_identity = InstallationIdentity::from_bytes(&[45u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[46u8; 32]);
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
        let mut peer_server = listener.accept(Duration::from_secs(10)).await.unwrap();
        let mut peer_client = dialing.await.unwrap().unwrap();

        let oversized = serde_json::json!({"blob": "x".repeat(300_000)});
        assert!(matches!(
            peer_client
                .send_json(&oversized, Duration::from_secs(5))
                .await,
            Err(TransportError::TooLarge)
        ));

        // Nothing was written: the next frame is frame #1 on the stream.
        let small = serde_json::json!({"round": 1});
        peer_client
            .send_json(&small, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            peer_server.recv_json(Duration::from_secs(5)).await.unwrap(),
            small
        );
    }

    /// F15: `measure_rtt` over real loopback QUIC. The dialer probes; the
    /// LISTENER side never sees a ping — `recv` answers them transparently
    /// (its one `recv` call returns the application frame sent AFTER the
    /// probes, proving pings were consumed in-band), and the dialer gets
    /// sane stats back. Quinn's own RTT/loss estimates are not exposed by
    /// libp2p-quic (see `observe::QUINN_STATS_UNAVAILABLE`) — these probes
    /// are the production RTT source.
    #[tokio::test]
    async fn libp2p_measure_rtt_answers_pings_transparently() {
        let server_identity = InstallationIdentity::from_bytes(&[31u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[32u8; 32]);
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

        // The server must be INSIDE recv while the dialer probes — that is
        // who answers the pings. Its single recv call answers all three
        // transparently and then returns the application frame.
        let server_recv =
            tokio::spawn(
                async move { session_server.recv(Duration::from_secs(10)).await.unwrap() },
            );

        // Probe batch BEFORE any application traffic (the documented
        // between-requests contract).
        let stats = session_client
            .measure_rtt(3, Duration::from_secs(5))
            .await
            .expect("rtt probes complete over loopback quic");
        assert!(stats.p50_ms > 0.0, "loopback quic rtt is positive");
        assert!(stats.p95_ms >= stats.p50_ms);
        assert!(stats.jitter_ms.is_finite() && stats.jitter_ms >= 0.0);

        // Application traffic still flows on the same session, and the
        // server's recv hands the APP frame to the application — the pings
        // were answered inside recv, never delivered.
        let msg = WireMessage::Control(Control::pong("app-marker"));
        session_client
            .send(&msg, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            server_recv.await.unwrap(),
            msg,
            "pings were transparent; the app frame arrives intact"
        );
    }
}
