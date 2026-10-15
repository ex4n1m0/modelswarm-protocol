//! F2B (ADR-014): the circuit-relay v2 CLIENT over the raw-QUIC backend —
//! reservation (listener side) and `/p2p-circuit` dialing (dialer side),
//! hand-rolled per the F2a honest-boundary note: RESERVE/HOP/STOP are
//! multistream-select-negotiated protocols and this backend opens raw QUIC
//! streams, so the client carries its own minimal codec instead of pulling
//! the whole `libp2p-relay` behaviour stack (the swarm migration is the
//! separately sanctioned F2b increment).
//!
//! Wire compatibility is against rust-libp2p 0.57's relay — the exact
//! behaviour `modelswarm-relay` hosts — and is proven in-crate by tests
//! that run a REAL relay double in-process:
//!
//! - multistream-select 1.0.0: uvarint-length-prefixed strings; the stream
//!   OPENER sends `/multistream/1.0.0\n` + its protocol, the responder
//!   echoes both (or answers `na\n`).
//! - hop/stop messages: protobuf (the proto2 schema implemented by
//!   libp2p-relay 0.22), each uvarint-length-prefixed, max 4096 bytes —
//!   the announced length is validated BEFORE any body allocation (the
//!   same size-before-allocation discipline as the JSON frame codec).
//!
//! Honest security posture: a DIRECT dial's PeerId is verified by the QUIC
//! TLS handshake (ADR-020). A RELAYED session's far-end PeerId is
//! **relay-vouched**: the QUIC crypto terminates at the relay, and the relay
//! binds the circuit to the destination PeerId via its reservation table —
//! a malicious relay can impersonate the far end or read relayed frames,
//! the same trust rust-libp2p's own relay client accepts. Callers label
//! relayed paths as such ([`crate::libp2p_backend::DialPath`]) so ADR-014
//! rule 5 (a relay-only peer is never counted direct-capable; relayed paths
//! carry a cost penalty) can be enforced upstream. End-to-end peer
//! authentication over relayed circuits belongs to the msp-v1 §6.1
//! handshake at the session layer — deliberately not redefined here
//! (`protocol/` is not this crate's path).

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::time::Instant;

use futures::AsyncWriteExt as _;
use libp2p::core::muxing::StreamMuxer as _;
use libp2p::core::transport::{DialOpts, PortUse};
use libp2p::identity::Keypair;
use libp2p::multiaddr::Protocol;
use libp2p::quic;
use libp2p::{Multiaddr, PeerId, Transport as _};
use std::future::Future as _;

use crate::error::{invalid_data, TransportError};
use crate::libp2p_backend::Libp2pSession;

/// The circuit-relay v2 hop protocol (frozen upstream name).
pub const HOP_PROTOCOL: &str = "/libp2p/circuit/relay/0.2.0/hop";
/// The circuit-relay v2 stop protocol (frozen upstream name).
pub const STOP_PROTOCOL: &str = "/libp2p/circuit/relay/0.2.0/stop";
/// Maximum protobuf message size (libp2p-relay's `MAX_MESSAGE_SIZE`).
const MAX_PROTO_MESSAGE: usize = 4096;
/// multistream-select's frame bound (2-byte uvarint length prefix).
const MAX_MSS_FRAME: usize = 16 * 1024;
/// Bound for one reservation (or renewal) exchange.
const RESERVE_DEADLINE: Duration = Duration::from_secs(15);
/// Bound for one inbound stop-stream handshake (the circuit dialer's
/// CONNECT is waiting on the far side meanwhile).
const STOP_HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);
/// Renew at 3/4 of the reservation lifetime — rust-libp2p's cadence.
const RENEWAL_FRACTION: u64 = 4;
/// Floor for the renewal interval (very short leases renew at least this
/// often, never a hot loop).
const MIN_RENEWAL_INTERVAL: Duration = Duration::from_secs(1);

fn relay_err(context: &'static str, detail: String) -> TransportError {
    TransportError::Io(invalid_data(context, detail))
}

// ---------------------------------------------------------------------------
// uvarint + buffered stream reads (cancel-safe; size-before-allocation)
// ---------------------------------------------------------------------------

fn put_uvarint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            return;
        }
    }
}

/// Reads ONE uvarint from `bytes` starting at `pos`; returns the value and
/// advances `pos`. `None` on truncation or an over-long (> 10 byte) varint.
fn take_uvarint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    for index in 0..10 {
        let byte = *bytes.get(*pos + index)?;
        value |= u64::from(byte & 0x7f).checked_shl(u32::try_from(7 * index).ok()?)?;
        if byte & 0x80 == 0 {
            *pos += index + 1;
            return Some(value);
        }
    }
    None
}

/// A `quic::Stream` plus its read buffer for handshake-phase reads. Bytes a
/// single chunk read pulled past the current handshake message stay buffered
/// and seed the resulting session's partial frame buffer
/// ([`HandshakeBuf::into_parts`]), so payload data coalesced with the
/// handshake is never lost.
struct HandshakeBuf {
    stream: quic::Stream,
    buf: Vec<u8>,
    pos: usize,
}

impl HandshakeBuf {
    fn new(stream: quic::Stream) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            pos: 0,
        }
    }

    /// Pulls at least one more byte into the buffer, deadline-bounded.
    async fn fill(&mut self, deadline: Instant) -> Result<usize, TransportError> {
        let mut chunk = [0u8; 2048];
        use futures::AsyncReadExt as _;
        let read = self.stream.read(&mut chunk);
        match tokio::time::timeout_at(deadline, read).await {
            Ok(Ok(0)) => Err(TransportError::Closed),
            Ok(Ok(n)) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(n)
            }
            Ok(Err(e)) => Err(relay_err("circuit relay stream", e.to_string())),
            Err(_) => Err(TransportError::Timeout),
        }
    }

    fn compact(&mut self) {
        self.buf.drain(..self.pos);
        self.pos = 0;
    }

    /// Waits until the buffer holds at least `want` unconsumed bytes.
    async fn wait_for(&mut self, want: usize, deadline: Instant) -> Result<(), TransportError> {
        while self.buf.len() - self.pos < want {
            self.compact();
            self.fill(deadline).await?;
        }
        Ok(())
    }

    /// Reads one uvarint.
    async fn read_uvarint(&mut self, deadline: Instant) -> Result<u64, TransportError> {
        loop {
            if let Some(value) = take_uvarint(&self.buf, &mut self.pos) {
                return Ok(value);
            }
            if self.buf.len() - self.pos >= 10 {
                return Err(relay_err(
                    "circuit relay wire",
                    "uvarint longer than 10 bytes".to_string(),
                ));
            }
            self.fill(deadline).await?;
        }
    }

    /// Reads one length-prefixed frame. The announced length is validated
    /// against `max` BEFORE the body is waited for — no oversized
    /// allocation on untrusted input.
    async fn read_frame(
        &mut self,
        max: usize,
        deadline: Instant,
    ) -> Result<Vec<u8>, TransportError> {
        let len = self.read_uvarint(deadline).await? as usize;
        if len == 0 || len > max {
            return Err(relay_err(
                "circuit relay wire",
                format!("announced frame length {len} outside 1..={max}"),
            ));
        }
        self.wait_for(len, deadline).await?;
        let frame: Vec<u8> = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(frame)
    }

    /// Writes `bytes` and flushes, deadline-bounded.
    async fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), TransportError> {
        let write = self.stream.write_all(bytes);
        tokio::time::timeout_at(deadline, write)
            .await
            .map_err(|_| TransportError::Timeout)??;
        let flush = self.stream.flush();
        tokio::time::timeout_at(deadline, flush)
            .await
            .map_err(|_| TransportError::Timeout)??;
        Ok(())
    }

    /// FINs the send half (the reserve request's `close()`), keeping the
    /// read half open for the response.
    async fn finish_write(&mut self, deadline: Instant) -> Result<(), TransportError> {
        let close = self.stream.close();
        tokio::time::timeout_at(deadline, close)
            .await
            .map_err(|_| TransportError::Timeout)??;
        Ok(())
    }

    /// Splits into the raw stream and the unconsumed buffered bytes (the
    /// payload head that coalesced with the handshake).
    fn into_parts(mut self) -> (quic::Stream, Vec<u8>) {
        self.compact();
        (self.stream, self.buf)
    }
}

// ---------------------------------------------------------------------------
// multistream-select 1.0.0 (single-protocol negotiation)
// ---------------------------------------------------------------------------

const MSS_HEADER: &[u8] = b"/multistream/1.0.0\n";
const MSS_NA: &[u8] = b"na\n";

fn mss_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(10 + payload.len());
    put_uvarint(&mut frame, payload.len() as u64);
    frame.extend_from_slice(payload);
    frame
}

/// mss protocol messages carry the protocol name WITH a trailing newline
/// inside the frame payload (like the header line does).
fn protocol_with_newline(protocol: &str) -> String {
    format!(
        "{protocol}
"
    )
}

/// The stream OPENER side: propose `protocol`, require its echo.
async fn mss_dial(
    io: &mut HandshakeBuf,
    protocol: &str,
    deadline: Instant,
) -> Result<(), TransportError> {
    let proposal = protocol_with_newline(protocol);
    let mut out = mss_frame(MSS_HEADER);
    out.extend_from_slice(&mss_frame(proposal.as_bytes()));
    io.write_all(&out, deadline).await?;
    let header = io.read_frame(MAX_MSS_FRAME, deadline).await?;
    if header != MSS_HEADER {
        return Err(relay_err(
            "multistream-select",
            format!(
                "unexpected header echo {:?}",
                String::from_utf8_lossy(&header)
            ),
        ));
    }
    let echo = io.read_frame(MAX_MSS_FRAME, deadline).await?;
    if echo != proposal.as_bytes() {
        return Err(relay_err(
            "multistream-select",
            format!(
                "relay refused protocol {protocol}: {:?}",
                String::from_utf8_lossy(&echo)
            ),
        ));
    }
    Ok(())
}

/// The stream RESPONDER side (stop streams): echo the header, then echo the
/// protocol iff it is the one expected, else answer `na` and fail.
async fn mss_listen(
    io: &mut HandshakeBuf,
    expected: &str,
    deadline: Instant,
) -> Result<(), TransportError> {
    let header = io.read_frame(MAX_MSS_FRAME, deadline).await?;
    if header != MSS_HEADER {
        return Err(relay_err(
            "multistream-select",
            "missing /multistream/1.0.0 header".to_string(),
        ));
    }
    io.write_all(&mss_frame(MSS_HEADER), deadline).await?;

    let wanted = protocol_with_newline(expected);
    let proposed = io.read_frame(MAX_MSS_FRAME, deadline).await?;
    let agreed = proposed == wanted.as_bytes();
    let reply = if agreed {
        mss_frame(wanted.as_bytes())
    } else {
        mss_frame(MSS_NA)
    };
    io.write_all(&reply, deadline).await?;
    if !agreed {
        return Err(relay_err(
            "multistream-select",
            format!(
                "unexpected stream protocol {:?} (expected {expected})",
                String::from_utf8_lossy(&proposed)
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// circuit-relay v2 protobuf (proto2, the libp2p-relay 0.22 schema)
// ---------------------------------------------------------------------------

// Status codes — frozen upstream enum values.
const STATUS_OK: i32 = 100;
const STATUS_RESERVATION_REFUSED: i32 = 200;
const STATUS_RESOURCE_LIMIT_EXCEEDED: i32 = 201;
const STATUS_PERMISSION_DENIED: i32 = 202;
const STATUS_CONNECTION_FAILED: i32 = 203;
const STATUS_NO_RESERVATION: i32 = 204;
const STATUS_MALFORMED_MESSAGE: i32 = 400;
const STATUS_UNEXPECTED_MESSAGE: i32 = 401;

/// The stable, honest name of a relay status code (the relay protocol's own
/// upstream vocabulary — not an invented telemetry label).
fn status_name(code: i32) -> String {
    let name = match code {
        STATUS_OK => "ok",
        STATUS_RESERVATION_REFUSED => "reservation_refused",
        STATUS_RESOURCE_LIMIT_EXCEEDED => "resource_limit_exceeded",
        STATUS_PERMISSION_DENIED => "permission_denied",
        STATUS_CONNECTION_FAILED => "connection_failed",
        STATUS_NO_RESERVATION => "no_reservation",
        STATUS_MALFORMED_MESSAGE => "malformed_message",
        STATUS_UNEXPECTED_MESSAGE => "unexpected_message",
        _ => return format!("unknown_status({code})"),
    };
    format!("{name}({code})")
}

/// Reservation terms from a reserve response (`Reservation` message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationTerms {
    /// Unix-epoch seconds at which the relay will drop the reservation.
    pub expire: u64,
    /// The relay's advertised addresses (raw multiaddr bytes), for building
    /// `p2p-circuit` addresses.
    pub addrs: Vec<Vec<u8>>,
}

/// Circuit limits the relay announced (`Limit` message); `None` per field
/// means uncapped — recorded so callers can price relayed paths honestly
/// (our own relay runs uncapped by design; third-party relays may not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CircuitLimits {
    pub duration_secs: Option<u64>,
    pub bytes: Option<u64>,
}

/// One decoded protobuf field.
enum FieldValue {
    Varint(u64),
    Delimited(Vec<u8>),
}

fn parse_fields(body: &[u8]) -> Result<Vec<(u64, FieldValue)>, TransportError> {
    let bad = |what: &str| relay_err("circuit relay protobuf", what.to_string());
    let mut fields = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let tag = take_uvarint(body, &mut pos).ok_or_else(|| bad("truncated field tag"))?;
        let field = tag >> 3;
        if field == 0 {
            return Err(bad("field number 0 is invalid"));
        }
        match tag & 0x7 {
            0 => {
                let value =
                    take_uvarint(body, &mut pos).ok_or_else(|| bad("truncated varint value"))?;
                fields.push((field, FieldValue::Varint(value)));
            }
            2 => {
                let len =
                    take_uvarint(body, &mut pos).ok_or_else(|| bad("truncated length"))? as usize;
                let end = pos
                    .checked_add(len)
                    .filter(|end| *end <= body.len())
                    .ok_or_else(|| bad("length-delimited field overruns the message"))?;
                fields.push((field, FieldValue::Delimited(body[pos..end].to_vec())));
                pos = end;
            }
            other => return Err(bad(&format!("unsupported wire type {other}"))),
        }
    }
    Ok(fields)
}

fn field_varint(fields: &[(u64, FieldValue)], number: u64) -> Option<u64> {
    fields
        .iter()
        .rev()
        .find_map(|(n, v)| match (*n == number, v) {
            (true, FieldValue::Varint(value)) => Some(*value),
            _ => None,
        })
}

fn field_bytes(fields: &[(u64, FieldValue)], number: u64) -> Option<&[u8]> {
    fields
        .iter()
        .rev()
        .find_map(|(n, v)| match (*n == number, v) {
            (true, FieldValue::Delimited(bytes)) => Some(bytes.as_slice()),
            _ => None,
        })
}

fn field_all_bytes(fields: &[(u64, FieldValue)], number: u64) -> Vec<&[u8]> {
    fields
        .iter()
        .filter_map(|(n, v)| match (*n == number, v) {
            (true, FieldValue::Delimited(bytes)) => Some(bytes.as_slice()),
            _ => None,
        })
        .collect()
}

fn put_field_varint(out: &mut Vec<u8>, field: u64, value: u64) {
    put_uvarint(out, field << 3);
    put_uvarint(out, value);
}

fn put_field_bytes(out: &mut Vec<u8>, field: u64, body: &[u8]) {
    put_uvarint(out, (field << 3) | 2);
    put_uvarint(out, body.len() as u64);
    out.extend_from_slice(body);
}

/// One uvarint-length-prefixed protobuf message, ready to write.
fn proto_frame(body: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(10 + body.len());
    put_uvarint(&mut frame, body.len() as u64);
    frame.extend_from_slice(body);
    frame
}

/// The hop-protocol messages this client emits or consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HopMessage {
    Reserve,
    Connect {
        dst: Vec<u8>,
    },
    Status {
        code: i32,
        reservation: Option<ReservationTerms>,
        limit: Option<CircuitLimits>,
    },
}

impl HopMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            HopMessage::Reserve => put_field_varint(&mut out, 1, 0),
            HopMessage::Connect { dst } => {
                put_field_varint(&mut out, 1, 1);
                let mut peer = Vec::new();
                put_field_bytes(&mut peer, 1, dst);
                put_field_bytes(&mut out, 2, &peer);
            }
            HopMessage::Status {
                code,
                reservation,
                limit,
            } => {
                put_field_varint(&mut out, 1, 2);
                if let Some(terms) = reservation {
                    let mut res = Vec::new();
                    put_field_varint(&mut res, 1, terms.expire);
                    for addr in &terms.addrs {
                        put_field_bytes(&mut res, 2, addr);
                    }
                    put_field_bytes(&mut out, 3, &res);
                }
                if let Some(limits) = limit {
                    let mut lim = Vec::new();
                    if let Some(duration) = limits.duration_secs {
                        put_field_varint(&mut lim, 1, duration);
                    }
                    if let Some(bytes) = limits.bytes {
                        put_field_varint(&mut lim, 2, bytes);
                    }
                    put_field_bytes(&mut out, 4, &lim);
                }
                put_field_varint(&mut out, 5, *code as u64);
            }
        }
        out
    }

    fn decode(body: &[u8]) -> Result<Self, TransportError> {
        let bad = |what: String| relay_err("circuit relay protobuf", what);
        let fields = parse_fields(body)?;
        let kind = field_varint(&fields, 1).ok_or_else(|| bad("missing type".into()))?;
        match kind {
            0 => Ok(HopMessage::Reserve),
            1 => {
                let peer =
                    field_bytes(&fields, 2).ok_or_else(|| bad("connect without peer".into()))?;
                let peer_fields = parse_fields(peer)?;
                let dst = field_bytes(&peer_fields, 1)
                    .ok_or_else(|| bad("connect peer without id".into()))?
                    .to_vec();
                Ok(HopMessage::Connect { dst })
            }
            2 => {
                let code = field_varint(&fields, 5)
                    .filter(|code| *code <= i32::MAX as u64)
                    .ok_or_else(|| bad("status without code".into()))?
                    as i32;
                let reservation = match field_bytes(&fields, 3) {
                    Some(bytes) => {
                        let res_fields = parse_fields(bytes)?;
                        let expire = field_varint(&res_fields, 1)
                            .ok_or_else(|| bad("reservation without expire".into()))?;
                        let addrs = field_all_bytes(&res_fields, 2)
                            .into_iter()
                            .map(<[u8]>::to_vec)
                            .collect();
                        Some(ReservationTerms { expire, addrs })
                    }
                    None => None,
                };
                let limit = match field_bytes(&fields, 4) {
                    Some(bytes) => {
                        let lim_fields = parse_fields(bytes)?;
                        Some(CircuitLimits {
                            duration_secs: field_varint(&lim_fields, 1),
                            bytes: field_varint(&lim_fields, 2),
                        })
                    }
                    None => None,
                };
                Ok(HopMessage::Status {
                    code,
                    reservation,
                    limit,
                })
            }
            other => Err(bad(format!("unknown hop message type {other}"))),
        }
    }
}

/// The stop-protocol messages this client emits or consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StopMessage {
    Connect {
        src: Vec<u8>,
        limit: Option<CircuitLimits>,
    },
    Status {
        code: i32,
    },
}

impl StopMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            StopMessage::Connect { src, limit } => {
                put_field_varint(&mut out, 1, 0);
                let mut peer = Vec::new();
                put_field_bytes(&mut peer, 1, src);
                put_field_bytes(&mut out, 2, &peer);
                if let Some(limits) = limit {
                    let mut lim = Vec::new();
                    if let Some(duration) = limits.duration_secs {
                        put_field_varint(&mut lim, 1, duration);
                    }
                    if let Some(bytes) = limits.bytes {
                        put_field_varint(&mut lim, 2, bytes);
                    }
                    put_field_bytes(&mut out, 3, &lim);
                }
            }
            StopMessage::Status { code } => {
                put_field_varint(&mut out, 1, 1);
                put_field_varint(&mut out, 4, *code as u64);
            }
        }
        out
    }

    fn decode(body: &[u8]) -> Result<Self, TransportError> {
        let bad = |what: String| relay_err("circuit relay protobuf", what);
        let fields = parse_fields(body)?;
        let kind = field_varint(&fields, 1).ok_or_else(|| bad("missing type".into()))?;
        match kind {
            0 => {
                let peer =
                    field_bytes(&fields, 2).ok_or_else(|| bad("connect without peer".into()))?;
                let peer_fields = parse_fields(peer)?;
                let src = field_bytes(&peer_fields, 1)
                    .ok_or_else(|| bad("connect peer without id".into()))?
                    .to_vec();
                let limit = match field_bytes(&fields, 3) {
                    Some(bytes) => {
                        let lim_fields = parse_fields(bytes)?;
                        Some(CircuitLimits {
                            duration_secs: field_varint(&lim_fields, 1),
                            bytes: field_varint(&lim_fields, 2),
                        })
                    }
                    None => None,
                };
                Ok(StopMessage::Connect { src, limit })
            }
            1 => {
                let code = field_varint(&fields, 4)
                    .filter(|code| *code <= i32::MAX as u64)
                    .ok_or_else(|| bad("status without code".into()))?
                    as i32;
                Ok(StopMessage::Status { code })
            }
            other => Err(bad(format!("unknown stop message type {other}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// relay leg dialing (QUIC to the relay, PeerId verified)
// ---------------------------------------------------------------------------

/// The `/p2p/<peer-id>` suffix PeerId of a relay multiaddr — REQUIRED: the
/// relay leg is dialed with dial-side PeerId verification, exactly like a
/// direct dial.
fn peer_id_suffix(addr: &Multiaddr) -> Result<PeerId, TransportError> {
    addr.iter()
        .find_map(|protocol| match protocol {
            Protocol::P2p(id) => Some(id),
            _ => None,
        })
        .ok_or_else(|| {
            relay_err(
                "circuit relay address",
                "relay multiaddr must carry a /p2p/<relay-peer-id> suffix".to_string(),
            )
        })
}

async fn dial_relay_leg(
    keypair: &Keypair,
    relay_addr: &Multiaddr,
    deadline: Instant,
) -> Result<quic::Connection, TransportError> {
    let expected = peer_id_suffix(relay_addr)?;
    let mut transport = quic::tokio::Transport::new(quic::Config::new(keypair));
    let dial = transport
        .dial(
            relay_addr.clone(),
            DialOpts {
                role: libp2p::core::Endpoint::Dialer,
                port_use: PortUse::New,
            },
        )
        .map_err(|e| relay_err("circuit relay dial", e.to_string()))?;
    let (peer_id, connection) = tokio::time::timeout_at(deadline, dial)
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| relay_err("circuit relay dial", e.to_string()))?;
    if peer_id != expected {
        return Err(TransportError::HandshakeFailed(
            "relay peer id does not match the /p2p/ address suffix".to_string(),
        ));
    }
    Ok(connection)
}

async fn open_outbound_stream(
    connection: &mut quic::Connection,
    deadline: Instant,
) -> Result<quic::Stream, TransportError> {
    let opened = std::future::poll_fn(|cx| std::pin::Pin::new(&mut *connection).poll_outbound(cx));
    tokio::time::timeout_at(deadline, opened)
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| relay_err("circuit relay stream open", e.to_string()))
}

// ---------------------------------------------------------------------------
// dialer side: CONNECT through the relay
// ---------------------------------------------------------------------------

/// The payload stream of one established circuit plus any bytes that
/// coalesced with the handshake (the session's frame-buffer head).
pub(crate) struct CircuitStream {
    pub stream: quic::Stream,
    pub preloaded: Vec<u8>,
}

/// Opens one circuit: dial the relay leg (PeerId-verified), negotiate hop,
/// CONNECT to `dst`. `Ok` means the relay accepted for exactly `dst` — the
/// far-end binding is relay-vouched (see the module docs for the honest
/// trust statement).
pub(crate) async fn dial_circuit(
    keypair: &Keypair,
    relay_addr: &Multiaddr,
    dst: &PeerId,
    deadline: Instant,
) -> Result<CircuitStream, TransportError> {
    let mut connection = dial_relay_leg(keypair, relay_addr, deadline).await?;
    let stream = open_outbound_stream(&mut connection, deadline).await?;
    let mut io = HandshakeBuf::new(stream);
    mss_dial(&mut io, HOP_PROTOCOL, deadline).await?;
    let connect = HopMessage::Connect {
        dst: dst.to_bytes(),
    }
    .encode();
    io.write_all(&proto_frame(&connect), deadline).await?;

    let frame = io.read_frame(MAX_PROTO_MESSAGE, deadline).await?;
    match HopMessage::decode(&frame)? {
        HopMessage::Status {
            code: STATUS_OK, ..
        } => {
            let (stream, preloaded) = io.into_parts();
            Ok(CircuitStream { stream, preloaded })
        }
        HopMessage::Status { code, .. } => Err(relay_err(
            "circuit relay hop",
            format!("relay refused the circuit: {}", status_name(code)),
        )),
        other => Err(relay_err(
            "circuit relay hop",
            format!("unexpected hop message {other:?}"),
        )),
    }
}

// ---------------------------------------------------------------------------
// listener side: RESERVE + inbound stop streams
// ---------------------------------------------------------------------------

/// Runs one RESERVE exchange over a NEW stream on `connection`; returns the
/// relay's terms and announced circuit limits.
async fn reserve_on_connection(
    connection: &mut quic::Connection,
    deadline: Instant,
) -> Result<(ReservationTerms, CircuitLimits), TransportError> {
    let stream = open_outbound_stream(connection, deadline).await?;
    let mut io = HandshakeBuf::new(stream);
    mss_dial(&mut io, HOP_PROTOCOL, deadline).await?;
    io.write_all(&proto_frame(&HopMessage::Reserve.encode()), deadline)
        .await?;
    // Mirror libp2p's client: FIN the request half before awaiting the
    // response (the read half stays open).
    io.finish_write(deadline).await?;

    let frame = io.read_frame(MAX_PROTO_MESSAGE, deadline).await?;
    match HopMessage::decode(&frame)? {
        HopMessage::Status {
            code: STATUS_OK,
            reservation: Some(terms),
            limit,
        } => {
            if terms.expire <= unix_now() {
                return Err(relay_err(
                    "circuit relay reserve",
                    "relay announced an already-expired reservation".to_string(),
                ));
            }
            Ok((terms, limit.unwrap_or_default()))
        }
        HopMessage::Status { code, .. } => Err(relay_err(
            "circuit relay reserve",
            format!("relay refused the reservation: {}", status_name(code)),
        )),
        other => Err(relay_err(
            "circuit relay reserve",
            format!("unexpected hop message {other:?}"),
        )),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn renewal_interval(expire: u64) -> Duration {
    let lifetime = expire.saturating_sub(unix_now());
    Duration::from_secs(lifetime / RENEWAL_FRACTION * 3).max(MIN_RENEWAL_INTERVAL)
}

/// Shared, lock-protected view of the reservation the caller can poll
/// without touching the driver task.
#[derive(Debug)]
struct ReservationState {
    alive: bool,
    terms: ReservationTerms,
    limits: CircuitLimits,
}

/// A live reservation on one relay: the listener-side handle that receives
/// relayed circuits ([`RelayReservation::accept`]) and exposes the relay's
/// honest terms. Dropping it aborts the driver, closing the QUIC connection
/// — the relay then drops the reservation (honest cleanup, no lease leak).
pub struct RelayReservation {
    relay_addr: String,
    own_peer_id: PeerId,
    state: Arc<StdMutex<ReservationState>>,
    inbound: tokio::sync::mpsc::Receiver<Libp2pSession>,
    driver: tokio::task::JoinHandle<()>,
}

impl RelayReservation {
    /// The relay's multiaddr (this reservation's connection target).
    pub fn relay_addr(&self) -> &str {
        &self.relay_addr
    }

    /// This node's advertised circuit address:
    /// `<relay multiaddr>/p2p-circuit/p2p/<own peer id>` — exactly the
    /// multiaddr shape peers put in their tracker `addresses` list, so no
    /// schema change is needed to advertise relay reachability.
    pub fn circuit_addr(&self) -> String {
        format!(
            "{relay}/p2p-circuit/p2p/{own}",
            relay = self.relay_addr,
            own = self.own_peer_id
        )
    }

    /// The relay's current reservation terms (expiry per the LAST accepted
    /// reserve response).
    pub fn terms(&self) -> ReservationTerms {
        self.state
            .lock()
            .expect("reservation state lock")
            .terms
            .clone()
    }

    /// The circuit limits the relay announced with the reservation.
    pub fn announced_limits(&self) -> CircuitLimits {
        self.state.lock().expect("reservation state lock").limits
    }

    /// Whether the reservation is still believed live (connection up, last
    /// renewal accepted). Honest: a relay-side drop is only observed on the
    /// next renewal or traffic.
    pub fn alive(&self) -> bool {
        self.state.lock().expect("reservation state lock").alive
    }

    /// Accepts the next relayed circuit as a session. The session's
    /// [`Libp2pSession::remote_peer_id`] is the relay-vouched SOURCE PeerId
    /// of the circuit (see the module docs for the trust statement).
    pub async fn accept(&mut self, deadline: Duration) -> Result<Libp2pSession, TransportError> {
        if !self.alive() {
            return Err(TransportError::Closed);
        }
        match tokio::time::timeout(deadline, self.inbound.recv()).await {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(TransportError::Closed),
            Err(_) => Err(TransportError::Timeout),
        }
    }
}

impl Drop for RelayReservation {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Reserves on the relay at `relay_addr` and spawns the driver task that
/// (a) answers inbound stop-stream handshakes into sessions on `inbound_tx`
/// and (b) renews at 3/4 of each lease. Returns the caller-facing handle.
pub(crate) async fn reserve(
    keypair: &Keypair,
    relay_addr: &Multiaddr,
    deadline: Instant,
) -> Result<RelayReservation, TransportError> {
    let mut connection = dial_relay_leg(keypair, relay_addr, deadline).await?;
    let at = Instant::now() + RESERVE_DEADLINE;
    let (terms, limits) = reserve_on_connection(&mut connection, at).await?;

    let state = Arc::new(StdMutex::new(ReservationState {
        alive: true,
        terms: terms.clone(),
        limits,
    }));
    let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel::<Libp2pSession>(16);
    let driver = tokio::spawn(reservation_driver(
        connection,
        Arc::clone(&state),
        inbound_tx,
    ));
    Ok(RelayReservation {
        relay_addr: relay_addr.to_string(),
        own_peer_id: PeerId::from(keypair.public()),
        state,
        inbound: inbound_rx,
        driver,
    })
}

/// The per-stop-stream handshake: mss responder side, read the relay's
/// CONNECT (source peer + announced limit), answer `ok`, and hand the
/// payload stream to the reservation as a ready session.
async fn stop_handshake(stream: quic::Stream, inbound: tokio::sync::mpsc::Sender<Libp2pSession>) {
    let deadline = Instant::now() + STOP_HANDSHAKE_DEADLINE;
    let mut io = HandshakeBuf::new(stream);
    let result = async {
        mss_listen(&mut io, STOP_PROTOCOL, deadline).await?;
        let frame = io.read_frame(MAX_PROTO_MESSAGE, deadline).await?;
        match StopMessage::decode(&frame)? {
            StopMessage::Connect { src, limit: _ } => {
                let src = PeerId::from_bytes(&src).map_err(|e| {
                    relay_err(
                        "circuit relay stop",
                        format!("circuit source peer id does not parse: {e:?}"),
                    )
                })?;
                let ok = proto_frame(&StopMessage::Status { code: STATUS_OK }.encode());
                io.write_all(&ok, deadline).await?;
                let (stream, preloaded) = io.into_parts();
                Ok::<(quic::Stream, Vec<u8>, PeerId), TransportError>((stream, preloaded, src))
            }
            other => Err(relay_err(
                "circuit relay stop",
                format!("unexpected stop message {other:?}"),
            )),
        }
    }
    .await;
    if let Ok((stream, preloaded, src)) = result {
        let session = Libp2pSession::new_relayed(stream, src, preloaded);
        // A closed channel means the reservation was dropped: dropping the
        // session closes the stream, which the relay reports upstream.
        let _ = inbound.send(session).await;
    }
}

/// What the reservation driver should do next.
enum DriverStep {
    /// An inbound stream arrived (a relay-initiated stop stream).
    Stream(quic::Stream),
    /// The QUIC connection to the relay failed/closed: reservation dead.
    Dead,
    /// The renewal timer fired.
    Renew,
}

/// Owns the reservation connection: polls inbound stop streams into
/// sessions, and renews the lease at 3/4 of its lifetime. Any leg failure
/// marks the reservation dead (`alive() == false`) and ends the task (the
/// task is aborted on handle drop, which drops the connection — the relay
/// reads that as a released lease).
///
/// Structure note: the connection and the renewal timer are polled from
/// ONE `poll_fn` (two futures borrowing the connection mutably cannot
/// coexist in a `select!`).
async fn reservation_driver(
    mut connection: quic::Connection,
    state: Arc<StdMutex<ReservationState>>,
    inbound: tokio::sync::mpsc::Sender<Libp2pSession>,
) {
    let interval = {
        let guard = state.lock().expect("reservation state lock");
        renewal_interval(guard.terms.expire)
    };
    let mut next_renewal = Box::pin(tokio::time::sleep(interval));
    loop {
        let step = {
            let next_renewal = &mut next_renewal;
            let connection = &mut connection;
            std::future::poll_fn(|cx| {
                use std::task::Poll;
                if next_renewal.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(DriverStep::Renew);
                }
                match std::pin::Pin::new(&mut *connection).poll_inbound(cx) {
                    Poll::Ready(Ok(stream)) => Poll::Ready(DriverStep::Stream(stream)),
                    Poll::Ready(Err(_)) => Poll::Ready(DriverStep::Dead),
                    Poll::Pending => Poll::Pending,
                }
            })
            .await
        };
        match step {
            DriverStep::Stream(stream) => {
                tokio::spawn(stop_handshake(stream, inbound.clone()));
            }
            DriverStep::Dead => {
                state.lock().expect("reservation state lock").alive = false;
                break;
            }
            DriverStep::Renew => {
                let at = Instant::now() + RESERVE_DEADLINE;
                match reserve_on_connection(&mut connection, at).await {
                    Ok((terms, limits)) => {
                        let expire = terms.expire;
                        {
                            let mut guard = state.lock().expect("reservation state lock");
                            guard.terms = terms;
                            guard.limits = limits;
                        }
                        next_renewal = Box::pin(tokio::time::sleep(renewal_interval(expire)));
                    }
                    Err(_) => {
                        state.lock().expect("reservation state lock").alive = false;
                        break;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// tests: codec units (wire interop is proven against a real relay double in
// crates/modelswarm-transport/src/libp2p_backend.rs tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uvarint_round_trips_and_rejects_overlong() {
        for value in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            let mut buf = Vec::new();
            put_uvarint(&mut buf, value);
            let mut pos = 0;
            assert_eq!(take_uvarint(&buf, &mut pos), Some(value));
            assert_eq!(pos, buf.len());
        }
        // 11 continuation bytes: over-long, rejected.
        let mut pos = 0;
        assert_eq!(take_uvarint(&[0x80u8; 11], &mut pos), None);
        // Truncated: rejected, position not advanced past the end.
        let mut pos = 0;
        assert_eq!(take_uvarint(&[0x80], &mut pos), None);
        assert_eq!(pos, 0);
    }

    #[test]
    fn hop_messages_round_trip_through_the_wire_schema() {
        let cases = vec![
            HopMessage::Reserve,
            HopMessage::Connect {
                dst: PeerId::random().to_bytes(),
            },
            HopMessage::Status {
                code: STATUS_OK,
                reservation: Some(ReservationTerms {
                    expire: 1_900_000_000,
                    addrs: vec![b"/ip4/127.0.0.1/udp/4001/quic-v1".to_vec()],
                }),
                limit: Some(CircuitLimits {
                    duration_secs: Some(4_294_967_295),
                    bytes: Some(u64::MAX),
                }),
            },
            HopMessage::Status {
                code: STATUS_NO_RESERVATION,
                reservation: None,
                limit: None,
            },
        ];
        for message in cases {
            assert_eq!(HopMessage::decode(&message.encode()).unwrap(), message);
        }
        // The framed form carries the uvarint length prefix.
        let frame = proto_frame(&HopMessage::Reserve.encode());
        assert_eq!(frame[0] as usize, frame.len() - 1);
    }

    #[test]
    fn stop_messages_round_trip_through_the_wire_schema() {
        let cases = vec![
            StopMessage::Connect {
                src: PeerId::random().to_bytes(),
                limit: Some(CircuitLimits {
                    duration_secs: None,
                    bytes: Some(131_072),
                }),
            },
            StopMessage::Status { code: STATUS_OK },
        ];
        for message in cases {
            assert_eq!(StopMessage::decode(&message.encode()).unwrap(), message);
        }
    }

    #[test]
    fn protobuf_decoder_rejects_malformed_input() {
        // Field number 0.
        let mut body = Vec::new();
        put_uvarint(&mut body, 0x00);
        assert!(HopMessage::decode(&body).is_err());
        // Length-delimited field overrunning the message.
        let mut body = Vec::new();
        put_uvarint(&mut body, (2 << 3) | 2);
        put_uvarint(&mut body, 100);
        body.push(0x01);
        assert!(HopMessage::decode(&body).is_err());
        // Unsupported wire type (3 = start-group).
        let mut body = Vec::new();
        put_uvarint(&mut body, (1 << 3) | 3);
        assert!(HopMessage::decode(&body).is_err());
        // Truncated varint value.
        let mut body = Vec::new();
        put_uvarint(&mut body, 1 << 3);
        body.push(0x80);
        assert!(HopMessage::decode(&body).is_err());
        // Unknown message type.
        let mut body = Vec::new();
        put_field_varint(&mut body, 1, 9);
        assert!(HopMessage::decode(&body).is_err());
    }

    #[test]
    fn status_names_come_from_the_relay_protocol_vocabulary() {
        assert_eq!(status_name(STATUS_OK), "ok(100)");
        assert_eq!(status_name(STATUS_NO_RESERVATION), "no_reservation(204)");
        assert_eq!(
            status_name(STATUS_RESOURCE_LIMIT_EXCEEDED),
            "resource_limit_exceeded(201)"
        );
        assert_eq!(status_name(999), "unknown_status(999)");
    }

    #[test]
    fn renewal_interval_renews_at_three_quarters_of_the_lease() {
        let expire = unix_now() + 3600;
        assert_eq!(renewal_interval(expire), Duration::from_secs(2700));
        // Very short leases clamp to the floor, never a hot loop.
        let expire = unix_now() + 2;
        assert_eq!(renewal_interval(expire), MIN_RENEWAL_INTERVAL);
    }

    #[test]
    fn relay_multiaddr_requires_a_p2p_suffix() {
        let with: Multiaddr = "/ip4/127.0.0.1/udp/4001/quic-v1"
            .parse::<Multiaddr>()
            .unwrap()
            .with(libp2p::multiaddr::Protocol::P2p(PeerId::random()));
        assert!(peer_id_suffix(&with).is_ok());
        let without: Multiaddr = "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap();
        assert!(peer_id_suffix(&without).is_err());
    }
}
