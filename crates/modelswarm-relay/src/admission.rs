//! ADR-038: the relay-side admission gate — `/msp/relay-ticket/1`.
//!
//! Two cooperating pieces close gate D12 (no authn/authz on the relay):
//!
//! 1. **The pre-flight** (this module's behaviour): a multistream-select
//!    protocol negotiated on the SAME QUIC connection as the hop exchange,
//!    BEFORE it. Exactly ONE `AdmissionMessage` PRESENT frame in, ONE
//!    `AdmissionStatus` frame out, each uvarint-length-prefixed with the
//!    4096-byte cap validated BEFORE allocation (the hop/stop codec
//!    discipline). Verification runs the frozen §3.4 ten-check order
//!    first-failure-wins using `modelswarm_eligibility::relay_ticket`.
//! 2. **The gate limiters** ([`gate_limiters`]): one closure prepended to
//!    BOTH `reservation_rate_limiters` and `circuit_src_rate_limiters` of
//!    the pinned libp2p-relay 0.22.0 config — the verified upstream seam
//!    that consults every limiter at both resource-grant sites and denies
//!    `ResourceLimitExceeded` on the first `false`. Admission is therefore
//!    fail-closed at the accept path itself: a client that never runs the
//!    pre-flight, or races it, gets NOTHING (T1).
//!
//! The relay stays content-blind: the admission protocol is pre-stream, and
//! relayed traffic is still forwarded opaque bytes. Admission state is
//! MEMORY-ONLY (never persisted; a restart empties the table and every peer
//! re-presents one cheap frame). The relay NEVER contacts the tracker (T8) —
//! the pinned Ed25519 key is the only tracker-derived input.
//!
//! Log vocabulary (frozen, §3.4): `admission accepted <peer> until
//! <rfc3339>` and `admission denied <peer>: {malformed | bad_signature |
//! expired | peer_mismatch | installation_mismatch | table_full}` — nothing
//! else, and NO ticket bytes are ever logged ([`Event`] carries peer, code,
//! reason, and deadline only).

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll};
use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use futures::future::FutureExt as _;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::core::upgrade::ReadyUpgrade;
use libp2p::identity::PublicKey;
use libp2p::swarm::handler::{
    ConnectionEvent, ConnectionHandler, FullyNegotiatedInbound, ListenUpgradeError,
};
use libp2p::swarm::{
    ConnectionDenied, ConnectionHandlerEvent, ConnectionId, FromSwarm, NetworkBehaviour,
    StreamProtocol, SubstreamProtocol, THandler, ToSwarm,
};
use libp2p::{Multiaddr, PeerId};
use time::OffsetDateTime;

use modelswarm_eligibility::relay_ticket::{RelayAdmissionTicket, TicketError};
use modelswarm_identity::installation_id_for;

/// The mss-negotiated admission protocol name (ADR-038 §3.2, frozen).
pub const ADMISSION_PROTOCOL: &str = "/msp/relay-ticket/1";

/// Maximum admission-frame body (the hop/stop `MAX_MESSAGE_SIZE` discipline).
const MAX_ADMISSION_FRAME: usize = 4096;

/// Frozen admission-table capacity (ADR-038 §3.5): 4096 PeerIds.
pub const ADMISSION_TABLE_CAPACITY: usize = 4096;

/// Bound for one PRESENT exchange server-side: a client that opens the
/// protocol and stalls must not hold relay resources.
const PRESENT_DEADLINE: Duration = Duration::from_secs(15);

/// Upstream relay status codes the pre-flight reuses (one vocabulary names
/// one fact; the labels match `modelswarm-transport`'s `status_name`).
const STATUS_OK: i32 = 100;
const STATUS_RESOURCE_LIMIT_EXCEEDED: i32 = 201;
const STATUS_PERMISSION_DENIED: i32 = 202;
const STATUS_MALFORMED_MESSAGE: i32 = 400;

/// The frozen denial reasons (ADR-038 §3.4 log vocabulary) — exactly six
/// values plus the accepted outcome; nothing else is ever logged or reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Malformed,
    BadSignature,
    Expired,
    PeerMismatch,
    InstallationMismatch,
    TableFull,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Reason::Malformed => "malformed",
            Reason::BadSignature => "bad_signature",
            Reason::Expired => "expired",
            Reason::PeerMismatch => "peer_mismatch",
            Reason::InstallationMismatch => "installation_mismatch",
            Reason::TableFull => "table_full",
        }
    }

    fn status_code(self) -> i32 {
        match self {
            Reason::Malformed => STATUS_MALFORMED_MESSAGE,
            Reason::BadSignature
            | Reason::Expired
            | Reason::PeerMismatch
            | Reason::InstallationMismatch => STATUS_PERMISSION_DENIED,
            Reason::TableFull => STATUS_RESOURCE_LIMIT_EXCEEDED,
        }
    }
}

/// The outcome of one PRESENT exchange — everything the relay may log.
/// Deliberately carries NO ticket bytes (redaction, §6.2 test 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub peer: PeerId,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Check 10 passed: admitted until `expires_at + 120 s`.
    Accepted { until: OffsetDateTime },
    /// First failure in the frozen §3.4 order.
    Denied { reason: Reason },
}

impl Event {
    /// The frozen log line (§3.4): exactly this vocabulary, no ticket bytes.
    pub fn log_line(&self) -> String {
        match &self.outcome {
            Outcome::Accepted { until } => format!(
                "admission accepted {} until {}",
                self.peer,
                until
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default()
            ),
            Outcome::Denied { reason } => {
                format!("admission denied {}: {}", self.peer, reason.as_str())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The admission table (memory-only, deadline-evicted, virtual-clock ready)
// ---------------------------------------------------------------------------

/// The admission clock — injectable so the expiry proofs advance time past
/// `expires_at + 120 s` on a virtual clock instead of sleeping (§6.2 test 1).
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

/// The real wall clock.
pub fn wall_clock() -> Clock {
    Arc::new(OffsetDateTime::now_utc)
}

/// Memory-only admission state: PeerId → deadline (`expires_at + 120 s`).
/// Capacity-bounded (expired entries evicted first), never persisted, and
/// shared with the gate limiter closures so the accept path and the
/// pre-flight see ONE table.
pub struct AdmissionTable {
    entries: StdMutex<HashMap<PeerId, OffsetDateTime>>,
    clock: Clock,
    capacity: usize,
}

impl AdmissionTable {
    pub fn new(clock: Clock) -> Arc<Self> {
        Arc::new(Self::with_capacity(clock, ADMISSION_TABLE_CAPACITY))
    }

    /// Test seam: a table of arbitrary capacity (the frozen production
    /// capacity is [`ADMISSION_TABLE_CAPACITY`]).
    pub fn with_capacity(clock: Clock, capacity: usize) -> Self {
        Self {
            entries: StdMutex::new(HashMap::new()),
            clock,
            capacity,
        }
    }

    fn now(&self) -> OffsetDateTime {
        (self.clock)()
    }

    /// The gate predicate (§3.4 enforcement): true iff the peer holds live
    /// admission. Lazily evicts the queried peer's expired entry.
    pub fn is_live(&self, peer: &PeerId) -> bool {
        let mut entries = self.entries.lock().expect("admission table lock");
        match entries.get(peer) {
            Some(deadline) if *deadline > self.now() => true,
            Some(_) => {
                entries.remove(peer);
                false
            }
            None => false,
        }
    }

    /// Checks 9-10: store/refresh admission, key = PeerId, deadline =
    /// `expires_at + 120 s` (idempotent — a later PRESENT replaces the
    /// deadline). Returns false (table full) after evicting expired entries.
    pub fn admit(&self, peer: PeerId, deadline: OffsetDateTime) -> bool {
        let now = self.now();
        let mut entries = self.entries.lock().expect("admission table lock");
        entries.retain(|_, until| *until > now);
        if !entries.contains_key(&peer) && entries.len() >= self.capacity {
            return false;
        }
        entries.insert(peer, deadline);
        true
    }

    /// Live-entry count (tests/telemetry only).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().expect("admission table lock").len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The tracker key + table bundle the pre-flight verifies against.
pub struct AdmissionVerifier {
    /// The pinned hub Ed25519 key (embedded default or `--tracker-key`).
    pub key: VerifyingKey,
    pub table: Arc<AdmissionTable>,
}

/// The gate limiter closures (§3.4 enforcement): `|peer,_,_| table.is_live(&peer)`
/// FIRST in BOTH limiter vectors — unadmitted peers never reach the per-peer/
/// per-IP token buckets honest peers behind shared NATs depend on (`.all()`
/// short-circuits on the first `false`; §6.1 S8). Each call to
/// `try_next` is the accept path itself, so a `false` denies with the
/// upstream-frozen `resource_limit_exceeded(201)` — fail-closed, independent
/// of the pre-flight.
pub fn gate_limiters(table: &Arc<AdmissionTable>) -> [Box<dyn libp2p::relay::RateLimiter>; 2] {
    let a = {
        let table = Arc::clone(table);
        Box::new(
            move |peer: PeerId, _addr: &Multiaddr, _now: std::time::Instant| table.is_live(&peer),
        )
    };
    let b = {
        let table = Arc::clone(table);
        Box::new(
            move |peer: PeerId, _addr: &Multiaddr, _now: std::time::Instant| table.is_live(&peer),
        )
    };
    [a, b]
}

// ---------------------------------------------------------------------------
// ADR-020 check-8 preimage: the installation id derived from the PeerId
// ---------------------------------------------------------------------------

/// Extracts the Ed25519 public key embedded in a libp2p identity-multihash
/// PeerId (`0x00 0x24 ‖ protobuf(pubkey)`, ADR-020) and derives
/// `base58(SHA-256(pubkey))` — the exact `installation_id_for` derivation the
/// node and tracker use. `None` for non-Ed25519 / non-identity PeerIds
/// (denied as `installation_mismatch` by the caller — no MSP installation
/// presents such a PeerId).
pub fn derived_installation_id(peer: &PeerId) -> Option<String> {
    let multihash = peer.as_ref();
    if multihash.code() != 0 {
        return None; // not an identity multihash
    }
    let key = PublicKey::try_decode_protobuf(multihash.digest()).ok()?;
    let ed25519 = key.try_into_ed25519().ok()?;
    Some(installation_id_for(&ed25519.to_bytes()))
}

// ---------------------------------------------------------------------------
// The frozen ten-check verification (§3.4), first-failure-wins
// ---------------------------------------------------------------------------

/// Strict-decoded PRESENT frame (check 2's success value) — the ticket
/// string.
type Present = String;

/// Verdict of a PRESENT frame: the pre-flight status plus the local log
/// reason. `deadline` is set iff the outcome is `ok(100)`.
pub struct Verdict {
    pub code: i32,
    pub reason: Option<Reason>,
    pub deadline: Option<OffsetDateTime>,
}

/// Checks 2-10 of the frozen §3.4 table, in order (check 1 — frame length —
/// is enforced by the frame reader before any allocation). `remote` is the
/// connection's Noise-authenticated PeerId; `derived` is the ADR-020
/// installation id re-derived from it (passed in so the golden-vector corpus
/// can pin the order with recorded context; the handler computes the real
/// derivation via [`derived_installation_id`]).
pub fn verify_present(
    body: &[u8],
    remote: &PeerId,
    derived: &str,
    verifier: &AdmissionVerifier,
) -> Verdict {
    let deny = |reason: Reason| Verdict {
        code: reason.status_code(),
        reason: Some(reason),
        deadline: None,
    };
    // Check 2: strict protobuf schema — type PRESENT, ticket bytes present,
    // no unknown or wrongly-typed fields, no duplicates.
    let present = match decode_present(body) {
        Ok(present) => present,
        Err(_) => return deny(Reason::Malformed),
    };
    // Checks 3-4 (eligibility's from_wire): serialized length ≤ 1024, two
    // base64url segments, canonical strict JSON, exact field set, RFC-3339
    // timestamps, 32-hex nonce, exactly-600 s delta.
    let (ticket, signature) = match RelayAdmissionTicket::from_wire(&present) {
        Ok(parsed) => parsed,
        Err(TicketError::Malformed(_)) => return deny(Reason::Malformed),
        // A syntactically-valid ticket whose signature segment is not even
        // decodable is a signature failure, per the frozen row mapping.
        Err(TicketError::InvalidSignature) => return deny(Reason::BadSignature),
        Err(_) => return deny(Reason::Malformed),
    };
    // Checks 5-6 (signature precedes binding — the pinned order): Ed25519
    // against the pinned key, then the ±120 s skew window.
    match ticket.verify(&signature, &verifier.key, verifier.table.now()) {
        Err(TicketError::InvalidSignature) => return deny(Reason::BadSignature),
        Err(TicketError::Expired { .. }) => return deny(Reason::Expired),
        Err(_) => return deny(Reason::Malformed),
        Ok(()) => {}
    }
    // Check 7: the ticket admits only the PeerId it names (T3).
    if let Err(TicketError::PeerMismatch { .. }) = ticket.check_peer_binding(&remote.to_string()) {
        return deny(Reason::PeerMismatch);
    }
    // Check 8: installation_id must re-derive from the authenticated
    // PeerId's embedded key (ADR-020).
    if let Err(TicketError::InstallationMismatch { .. }) = ticket.check_installation(derived) {
        return deny(Reason::InstallationMismatch);
    }
    // Checks 9-10: capacity (expired evicted first), then store/refresh.
    let deadline = match ticket.admission_deadline() {
        Ok(deadline) => deadline,
        Err(_) => return deny(Reason::Malformed),
    };
    if !verifier.table.admit(*remote, deadline) {
        return deny(Reason::TableFull);
    }
    Verdict {
        code: STATUS_OK,
        reason: None,
        deadline: Some(deadline),
    }
}

// ---------------------------------------------------------------------------
// uvarint + protobuf (proto2 wire format; strict per the frozen schema)
// ---------------------------------------------------------------------------

pub(crate) fn put_uvarint(buf: &mut Vec<u8>, mut value: u64) {
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

/// Reads ONE uvarint from `bytes` at `pos`; `None` on truncation or an
/// over-long (> 10 byte) varint.
pub(crate) fn take_uvarint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
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

/// One decoded protobuf field.
enum FieldValue {
    Varint(u64),
    Delimited(Vec<u8>),
}

fn parse_fields(body: &[u8]) -> Result<Vec<(u64, FieldValue)>, ()> {
    let mut fields = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let tag = take_uvarint(body, &mut pos).ok_or(())?;
        let field = tag >> 3;
        if field == 0 {
            return Err(()); // corpus: "field_zero"
        }
        match tag & 0x7 {
            0 => {
                let value = take_uvarint(body, &mut pos).ok_or(())?;
                fields.push((field, FieldValue::Varint(value)));
            }
            2 => {
                let len = take_uvarint(body, &mut pos).ok_or(())? as usize;
                let end = pos
                    .checked_add(len)
                    .filter(|end| *end <= body.len())
                    .ok_or(())?;
                fields.push((field, FieldValue::Delimited(body[pos..end].to_vec())));
                pos = end;
            }
            _ => return Err(()),
        }
    }
    Ok(fields)
}

/// `AdmissionMessage.type` values (frozen §3.2 protobuf).
const TYPE_PRESENT: u64 = 0;
/// `AdmissionStatus.type` values (frozen §3.2 protobuf).
const TYPE_STATUS: u64 = 1;

/// Strict PRESENT decode (check 2): exactly one field-1 varint == PRESENT,
/// exactly one field-2 length-delimited ticket, nothing else — unknown type
/// values, unknown fields, wrong wire types, duplicates, truncations, and
/// length overruns are all malformed.
pub fn decode_present(body: &[u8]) -> Result<Present, ()> {
    let fields = parse_fields(body)?;
    let mut kind: Option<u64> = None;
    let mut ticket: Option<Vec<u8>> = None;
    for (field, value) in fields {
        match (field, value) {
            (1, FieldValue::Varint(value)) => {
                if kind.is_some() || (value != TYPE_PRESENT && value != TYPE_STATUS) {
                    return Err(());
                }
                if value != TYPE_PRESENT {
                    return Err(()); // corpus: "unknown_type_value" (a STATUS in a request)
                }
                kind = Some(value);
            }
            (2, FieldValue::Delimited(bytes)) => {
                if ticket.is_some() {
                    return Err(());
                }
                ticket = Some(bytes);
            }
            _ => return Err(()), // unknown field number or wrongly-typed field
        }
    }
    let ticket = ticket.ok_or(())?; // corpus: "truncated_tag" (field-2 tag alone)
    String::from_utf8(ticket).map_err(|_| ())
}

/// Encodes a PRESENT body (field 1 varint PRESENT, field 2 the ticket bytes)
/// — byte-pinned against `relay-admission-frame-1.json` by test. The relay
/// only DECODES PRESENT frames; this encoder exists so the byte-pin test
/// (and any future second implementation check) shares one frozen source.
#[cfg(test)]
pub fn encode_present(ticket: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + ticket.len());
    put_uvarint(&mut out, 1 << 3);
    put_uvarint(&mut out, TYPE_PRESENT);
    put_uvarint(&mut out, (2 << 3) | 2);
    put_uvarint(&mut out, ticket.len() as u64);
    out.extend_from_slice(ticket.as_bytes());
    out
}

/// Strict STATUS decode (client-side mirror): exactly one field-1 varint ==
/// STATUS, exactly one field-3 varint code, nothing else. The relay only
/// ENCODES STATUS frames; the decoder pins the same schema from the
/// consuming side (vector test).
#[cfg(test)]
pub fn decode_status(body: &[u8]) -> Result<i32, ()> {
    let fields = parse_fields(body)?;
    let mut kind = false;
    let mut code: Option<u64> = None;
    for (field, value) in fields {
        match (field, value) {
            (1, FieldValue::Varint(value)) => {
                if kind || value != TYPE_STATUS {
                    return Err(());
                }
                kind = true;
            }
            (3, FieldValue::Varint(value)) => {
                if code.is_some() {
                    return Err(());
                }
                code = Some(value);
            }
            _ => return Err(()),
        }
    }
    if !kind {
        return Err(());
    }
    let code = code.ok_or(())?;
    if code > i32::MAX as u64 {
        return Err(());
    }
    Ok(code as i32)
}

/// Encodes a STATUS body (field 1 varint STATUS, field 3 the code) —
/// byte-pinned for 100/201/202/400 against the frame vector by test.
pub fn encode_status(code: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    put_uvarint(&mut out, 1 << 3);
    put_uvarint(&mut out, TYPE_STATUS);
    put_uvarint(&mut out, 3 << 3);
    put_uvarint(&mut out, code as u64);
    out
}

/// One uvarint-length-prefixed frame, ready to write.
pub(crate) fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + body.len());
    put_uvarint(&mut out, body.len() as u64);
    out.extend_from_slice(body);
    out
}

// ---------------------------------------------------------------------------
// The PRESENT exchange over one negotiated stream
// ---------------------------------------------------------------------------

/// Deadline-bounded buffered reader for the exchange — same shape as the
/// transport crate's `HandshakeBuf` reads: the announced length is validated
/// against the cap BEFORE the body is waited for (size-before-allocation on
/// untrusted input; §6.1 S7).
pub(crate) struct ExchangeBuf<S> {
    stream: S,
    buf: Vec<u8>,
    pos: usize,
}

impl<S: AsyncRead + AsyncWrite + Unpin> ExchangeBuf<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            pos: 0,
        }
    }

    async fn fill(&mut self, deadline: tokio::time::Instant) -> Result<(), ExchangeError> {
        let mut chunk = [0u8; 2048];
        let read = self.stream.read(&mut chunk);
        match tokio::time::timeout_at(deadline, read).await {
            Ok(Ok(0)) => Err(ExchangeError::Closed),
            Ok(Ok(n)) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(())
            }
            Ok(Err(_)) => Err(ExchangeError::Io),
            Err(_) => Err(ExchangeError::Timeout),
        }
    }

    async fn read_uvarint(&mut self, deadline: tokio::time::Instant) -> Result<u64, ExchangeError> {
        loop {
            if let Some(value) = take_uvarint(&self.buf, &mut self.pos) {
                return Ok(value);
            }
            if self.buf.len() - self.pos >= 10 {
                return Err(ExchangeError::Malformed);
            }
            self.fill(deadline).await?;
        }
    }

    /// Check 1: the announced length must be in `1..=4096`, validated before
    /// the body is waited for or allocated.
    pub(crate) async fn read_frame(
        &mut self,
        deadline: tokio::time::Instant,
    ) -> Result<Vec<u8>, ExchangeError> {
        let len = self.read_uvarint(deadline).await? as usize;
        if len == 0 || len > MAX_ADMISSION_FRAME {
            return Err(ExchangeError::Malformed);
        }
        while self.buf.len() - self.pos < len {
            self.buf.drain(..self.pos);
            self.pos = 0;
            self.fill(deadline).await?;
        }
        let body: Vec<u8> = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(body)
    }

    pub(crate) async fn write_frame(
        &mut self,
        bytes: &[u8],
        deadline: tokio::time::Instant,
    ) -> Result<(), ExchangeError> {
        let write = self.stream.write_all(bytes);
        tokio::time::timeout_at(deadline, write)
            .await
            .map_err(|_| ExchangeError::Timeout)?
            .map_err(|_| ExchangeError::Io)?;
        let flush = self.stream.flush();
        tokio::time::timeout_at(deadline, flush)
            .await
            .map_err(|_| ExchangeError::Timeout)?
            .map_err(|_| ExchangeError::Io)?;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum ExchangeError {
    Closed,
    Timeout,
    Io,
    Malformed,
}

/// Runs ONE PRESENT exchange on an already-negotiated admission stream:
/// read one frame (check 1), verify (checks 2-10), write exactly one STATUS
/// frame. Transport errors become `malformed_message(400)` denials — the
/// pre-flight always answers, never hangs (§3.4's "every refusal is
/// labeled, never silent").
async fn exchange<S>(stream: S, peer: PeerId, verifier: Arc<AdmissionVerifier>) -> Event
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let deadline = tokio::time::Instant::now() + PRESENT_DEADLINE;
    let mut io = ExchangeBuf::new(stream);
    let verdict = match io.read_frame(deadline).await {
        // Check 1 (frame length) or a transport failure → malformed.
        Err(_) => Verdict {
            code: STATUS_MALFORMED_MESSAGE,
            reason: Some(Reason::Malformed),
            deadline: None,
        },
        Ok(body) => {
            let derived = derived_installation_id(&peer).unwrap_or_default();
            verify_present(&body, &peer, &derived, &verifier)
        }
    };
    // Answer even when the verdict is a denial; a write failure after a
    // successful check 10 still leaves the admission stored — the gate, not
    // the acknowledgment, is the enforcement (T1).
    let _ = io
        .write_frame(&frame(&encode_status(verdict.code)), deadline)
        .await;
    Event {
        peer,
        outcome: match (verdict.reason, verdict.deadline) {
            (None, Some(until)) => Outcome::Accepted { until },
            (Some(reason), _) => Outcome::Denied { reason },
            (None, None) => Outcome::Denied {
                reason: Reason::Malformed,
            },
        },
    }
}

// ---------------------------------------------------------------------------
// The behaviour: inbound `/msp/relay-ticket/1` streams on every connection
// ---------------------------------------------------------------------------

/// Network behaviour accepting admission streams. One instance is shared by
/// every connection; the verifier (key + table) is the same one the gate
/// limiters close over — one admission state for the whole relay.
pub struct Behaviour {
    verifier: Arc<AdmissionVerifier>,
    events: VecDeque<Event>,
}

impl Behaviour {
    pub fn new(verifier: Arc<AdmissionVerifier>) -> Self {
        Self {
            verifier,
            events: VecDeque::new(),
        }
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Handler;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Handler::new(peer, Arc::clone(&self.verifier)))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _addr: &Multiaddr,
        _role_override: libp2p::core::Endpoint,
        _port_use: libp2p::core::transport::PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Handler::new(peer, Arc::clone(&self.verifier)))
    }

    fn on_swarm_event(&mut self, _event: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        event: Event,
    ) {
        self.events.push_back(event);
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, std::convert::Infallible>> {
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(ToSwarm::GenerateEvent(event));
        }
        Poll::Pending
    }
}

/// Per-connection handler: every inbound `/msp/relay-ticket/1` stream
/// becomes one deadline-bounded exchange task; outcomes flow up as
/// [`Event`]s for the frozen log lines.
pub struct Handler {
    peer: PeerId,
    verifier: Arc<AdmissionVerifier>,
    tasks: VecDeque<Pin<Box<dyn Future<Output = Event> + Send>>>,
    queued: VecDeque<Event>,
}

impl Handler {
    fn new(peer: PeerId, verifier: Arc<AdmissionVerifier>) -> Self {
        Self {
            peer,
            verifier,
            tasks: VecDeque::new(),
            queued: VecDeque::new(),
        }
    }
}

impl ConnectionHandler for Handler {
    type FromBehaviour = std::convert::Infallible;
    type ToBehaviour = Event;
    type InboundProtocol = ReadyUpgrade<StreamProtocol>;
    type OutboundProtocol = ReadyUpgrade<StreamProtocol>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(
            ReadyUpgrade::new(StreamProtocol::new(ADMISSION_PROTOCOL)),
            (),
        )
    }

    fn connection_keep_alive(&self) -> bool {
        !self.tasks.is_empty() || !self.queued.is_empty()
    }

    fn on_behaviour_event(&mut self, _: std::convert::Infallible) {}

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        if let Some(event) = self.queued.pop_front() {
            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(event));
        }
        let mut index = 0;
        while index < self.tasks.len() {
            let task = self.tasks[index].as_mut().poll(cx);
            if let Poll::Ready(event) = task {
                self.tasks.remove(index);
                self.queued.push_back(event);
            } else {
                index += 1;
            }
        }
        if let Some(event) = self.queued.pop_front() {
            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(event));
        }
        Poll::Pending
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<Self::InboundProtocol, Self::OutboundProtocol>,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: stream,
                ..
            }) => {
                let peer = self.peer;
                let verifier = Arc::clone(&self.verifier);
                self.tasks
                    .push_back(exchange(stream, peer, verifier).boxed());
            }
            // An inbound upgrade failure on the admission protocol is a
            // remote transport/protocol violation: the stream is dead and
            // the peer stays unadmitted — nothing to answer.
            ConnectionEvent::ListenUpgradeError(ListenUpgradeError { .. }) => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use modelswarm_eligibility::relay_ticket::sign_ticket;
    use time::Duration as TimeDuration;

    // ------------------------------------------------------------------
    // Vector loading + fixture key (the lease-hubkey discipline)
    // ------------------------------------------------------------------

    fn vector(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../protocol/vectors/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&raw).expect("vector json parses")
    }

    /// The recorded fixture signing key (0x38 seed) — never the production
    /// hub key (§6.2 fixture discipline).
    fn fixture_key() -> SigningKey {
        let doc = vector("relay-ticket-1.json");
        let seed_hex = doc["signing_seed_hex"].as_str().expect("seed");
        let seed = hex::decode(seed_hex).expect("seed hex");
        let bytes: [u8; 32] = seed.try_into().expect("32 seed bytes");
        SigningKey::from_bytes(&bytes)
    }

    fn fixture_verifying_key() -> VerifyingKey {
        fixture_key().verifying_key()
    }

    /// A fixed virtual clock reading `t`.
    fn fixed_clock(t: OffsetDateTime) -> Clock {
        Arc::new(move || t)
    }

    /// A settable virtual clock (the expiry proofs advance it).
    fn settable_clock(t: OffsetDateTime) -> (Clock, Arc<StdMutex<OffsetDateTime>>) {
        let shared = Arc::new(StdMutex::new(t));
        let clock: Clock = {
            let shared = Arc::clone(&shared);
            Arc::new(move || *shared.lock().expect("clock lock"))
        };
        (clock, shared)
    }

    fn base_ticket() -> modelswarm_eligibility::relay_ticket::RelayAdmissionTicket {
        let doc = vector("relay-ticket-1.json");
        modelswarm_eligibility::relay_ticket::RelayAdmissionTicket {
            installation_id: doc["fields"]["installation_id"]
                .as_str()
                .expect("installation_id")
                .to_string(),
            peer_id: doc["fields"]["peer_id"]
                .as_str()
                .expect("peer_id")
                .to_string(),
            issued_at: doc["fields"]["issued_at"]
                .as_str()
                .expect("issued_at")
                .to_string(),
            expires_at: doc["fields"]["expires_at"]
                .as_str()
                .expect("expires_at")
                .to_string(),
            nonce: doc["fields"]["nonce"].as_str().expect("nonce").to_string(),
        }
    }

    fn base_token() -> String {
        vector("relay-ticket-1.json")["token"]
            .as_str()
            .expect("token")
            .to_string()
    }

    /// The corpus's authenticated remote PeerId (relay-ticket-1's peer).
    fn base_peer() -> PeerId {
        base_ticket().peer_id.parse().expect("base peer parses")
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex byte"))
            .collect()
    }

    fn verifier(clock: Clock) -> AdmissionVerifier {
        AdmissionVerifier {
            key: fixture_verifying_key(),
            table: AdmissionTable::new(clock),
        }
    }

    /// Runs checks 2-10 with the corpus's remote/derived context.
    fn check(body: &[u8], verifier: &AdmissionVerifier) -> Verdict {
        verify_present(body, &base_peer(), &base_ticket().installation_id, verifier)
    }

    // ------------------------------------------------------------------
    // §6.2 test 5 (frame vector): byte-pinned codec + malformed corpus
    // ------------------------------------------------------------------

    /// The PRESENT encode reproduces the vector's frame byte for byte, and
    /// the framed wire form carries the uvarint length prefix (402).
    #[test]
    fn present_encode_is_byte_pinned_to_the_frame_vector() {
        let doc = vector("relay-admission-frame-1.json");
        let expected = unhex(
            doc["present_frame_hex"]
                .as_str()
                .expect("present_frame_hex"),
        );
        let token = base_token();
        let body = encode_present(&token);
        assert_eq!(body, expected, "PRESENT body must match the vector");
        // The vector hex is the protobuf BODY; the wire frame is
        // uvarint(body.len()) + body (the vector's length_prefix_rule).
        let framed = frame(&body);
        assert_eq!(framed.len(), 2 + expected.len());
        assert_eq!(&framed[..2], &[0xae, 0x03], "uvarint(430) length prefix");
        // The body's inner structure matches the vector's notes: field 1
        // varint PRESENT, field 2 length-delimited with uvarint length 425
        // carrying the token bytes. (Lengths move with the token: 402/407
        // before the 2026-10-15 regeneration, 425/430 after — the hex
        // above is the pin, these are its annotated structure.)
        assert_eq!(&body[..2], &[0x08, 0x00]);
        assert_eq!(
            &body[2..5],
            &[0x12, 0xa9, 0x03],
            "field-2 tag + uvarint(425)"
        );
        assert_eq!(
            &body[5..],
            token.as_bytes(),
            "the ticket bytes are the token"
        );
    }

    /// The four STATUS encodes (100/201/202/400) are byte-pinned.
    #[test]
    fn status_encodes_are_byte_pinned_to_the_frame_vector() {
        let doc = vector("relay-admission-frame-1.json");
        let statuses = &doc["status_frames_hex"];
        for (key, code) in [
            ("ok_100", 100),
            ("resource_limit_exceeded_201", 201),
            ("permission_denied_202", 202),
            ("malformed_message_400", 400),
        ] {
            let expected = unhex(statuses[key].as_str().expect("status frame"));
            assert_eq!(encode_status(code), expected, "{key}");
            assert_eq!(decode_status(&expected), Ok(code), "{key} round-trips");
        }
    }

    /// The malformed corpus: every row is refused by the strict decoder with
    /// the vector's verdict (malformed), and the full exchange maps it to
    /// `malformed_message(400)` + reason `malformed`.
    #[test]
    fn malformed_corpus_is_refused_with_malformed_verdicts() {
        let doc = vector("relay-admission-frame-1.json");
        for row in doc["malformed_corpus"].as_array().expect("corpus rows") {
            let id = row["id"].as_str().expect("id");
            let bytes = unhex(row["hex"].as_str().expect("hex"));
            assert!(decode_present(&bytes).is_err(), "{id} must be refused");
        }
        // And each refusal is the frozen 400/malformed pair.
        let verifier = verifier(fixed_clock(
            OffsetDateTime::parse(
                "2026-10-15T12:05:00Z",
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap(),
        ));
        for row in doc["malformed_corpus"].as_array().expect("corpus rows") {
            let bytes = unhex(row["hex"].as_str().expect("hex"));
            let verdict = check(&bytes, &verifier);
            assert_eq!(verdict.code, 400, "{:?}", row["id"]);
            assert_eq!(verdict.reason, Some(Reason::Malformed), "{:?}", row["id"]);
        }
    }

    /// Check 1: an announced length outside 1..=4096 is refused by the frame
    /// reader BEFORE the body is waited for or allocated
    /// (size-before-allocation on untrusted input; §6.1 S7).
    #[tokio::test]
    async fn oversized_announced_length_is_refused_before_allocation() {
        /// An in-memory futures-traits duplex serving preloaded chunks.
        struct MemStream {
            chunks: VecDeque<Vec<u8>>,
            written: Vec<u8>,
        }
        impl MemStream {
            fn one(chunk: Vec<u8>) -> Self {
                Self {
                    chunks: VecDeque::from(vec![chunk]),
                    written: Vec::new(),
                }
            }
        }
        impl AsyncRead for MemStream {
            fn poll_read(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &mut [u8],
            ) -> Poll<std::io::Result<usize>> {
                match self.chunks.pop_front() {
                    Some(chunk) => {
                        buf[..chunk.len()].copy_from_slice(&chunk);
                        Poll::Ready(Ok(chunk.len()))
                    }
                    None => Poll::Ready(Ok(0)),
                }
            }
        }
        impl AsyncWrite for MemStream {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                self.written.extend_from_slice(buf);
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
            fn poll_close(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        // uvarint(4097) and NOTHING ELSE ever arriving: the reader refuses
        // on the length alone BEFORE waiting for (or allocating toward)
        // the announced body — a malformed refusal, not a timeout.
        let mut announced = Vec::new();
        put_uvarint(&mut announced, 4097);
        let mut io = ExchangeBuf::new(MemStream::one(announced));
        let err = io
            .read_frame(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .expect_err("4097 must be refused");
        assert!(
            matches!(err, ExchangeError::Malformed),
            "refused on the length, not by timeout: {err:?}"
        );

        // A length-0 announcement is refused the same way.
        let mut io = ExchangeBuf::new(MemStream::one(vec![0x00]));
        let err = io
            .read_frame(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .expect_err("0 must be refused");
        assert!(matches!(err, ExchangeError::Malformed));
    }

    // ------------------------------------------------------------------
    // §6.2 tests 2 (forge corpus) + expiry vector, ORDER pinned
    // ------------------------------------------------------------------

    /// A minted ticket's wire form, fixture-signed (test-only minting).
    fn mint(
        installation_id: &str,
        peer_id: &str,
        issued: OffsetDateTime,
        ttl: i64,
        nonce: &str,
    ) -> String {
        let ticket = RelayAdmissionTicket {
            installation_id: installation_id.to_string(),
            peer_id: peer_id.to_string(),
            issued_at: issued
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
            expires_at: (issued + TimeDuration::seconds(ttl))
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
            nonce: nonce.to_string(),
        };
        sign_ticket(&ticket, &fixture_key())
    }

    fn at(ts: &str) -> OffsetDateTime {
        OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339).unwrap()
    }

    /// The base vector token: checks 1-6 pass at verify_at, checks 7-8 pass
    /// under the corpus's remote/derived context, and the outcome is
    /// `ok(100)` with the vector's recorded admission deadline (12:12:00Z).
    #[test]
    fn relay_ticket_1_vector_admits_with_the_recorded_deadline() {
        let doc = vector("relay-ticket-1.json");
        let verify_at = doc["verify_at"].as_str().expect("verify_at");
        let verifier = verifier(fixed_clock(at(verify_at)));
        let body = encode_present(&base_token());
        let verdict = check(&body, &verifier);
        assert_eq!(verdict.code, 100);
        assert_eq!(verdict.reason, None);
        let deadline = verdict.deadline.expect("accepted carries a deadline");
        let expected = doc["expected_admission_deadline"]
            .as_str()
            .expect("expected_admission_deadline");
        assert_eq!(
            deadline,
            at(expected),
            "admission deadline is expires_at + 120 s"
        );
    }

    /// The expiry vector: the same token at 12:20:00Z (past 12:12:00Z) is
    /// `permission_denied(202)` / `expired` — signature still valid.
    #[test]
    fn relay_ticket_2_expired_vector_is_denied_expired() {
        let doc = vector("relay-ticket-2-expired.json");
        let verify_at = doc["verify_at"].as_str().expect("verify_at");
        let verifier = verifier(fixed_clock(at(verify_at)));
        let verdict = check(&encode_present(&base_token()), &verifier);
        assert_eq!(verdict.code, 202);
        assert_eq!(verdict.reason, Some(Reason::Expired));
    }

    /// The full forge corpus re-derived in code, asserted against the frozen
    /// status+reason pairs, with the first-failure-wins ORDER pinned:
    /// syntax precedes signature; signature precedes binding; capacity is
    /// checked last.
    #[test]
    fn forge_corpus_rows_match_the_frozen_pairs() {
        let verify_at = at("2026-10-15T12:05:00Z");
        let verifier = verifier(fixed_clock(verify_at));
        let base = base_ticket();
        let base_peer_id = base.peer_id.clone();
        let derived = base.installation_id.clone();
        let nonce = base.nonce.clone();
        let issued = at("2026-10-15T12:00:00Z");

        // bad_signature: flip one base64url char of the signature segment.
        let token = base_token();
        let (payload, sig) = token.rsplit_once('.').expect("two segments");
        // Flip the FIRST character of the signature segment (any base64url
        // char changed breaks the signature deterministically).
        let mut chars: Vec<char> = sig.chars().collect();
        if chars[0] == 'A' {
            chars[0] = 'B';
        } else {
            chars[0] = 'A';
        }
        let flipped: String = chars.into_iter().collect();
        let bad_sig = format!("{payload}.{flipped}");
        let verdict = check(&encode_present(&bad_sig), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (202, Some(Reason::BadSignature))
        );

        // wrong_key: a different fixture key's signature over the same
        // canonical payload.
        let other = SigningKey::from_bytes(&[0x77u8; 32]);
        let other_token = sign_ticket(&base, &other);
        let verdict = check(&encode_present(&other_token), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (202, Some(Reason::BadSignature))
        );

        // peer_mismatch: a valid ticket naming peer B on A's connection.
        let b_peer = libp2p::identity::Keypair::generate_ed25519();
        let b_peer_id = libp2p::PeerId::from(b_peer.public()).to_string();
        let for_b = mint(&derived, b_peer_id.as_str(), issued, 600, &nonce);
        let verdict = verify_present(&encode_present(&for_b), &base_peer(), &derived, &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (202, Some(Reason::PeerMismatch))
        );

        // installation_mismatch: valid ticket, wrong installation id.
        let wrong_install = mint("b58-someone-else", &base_peer_id, issued, 600, &nonce);
        let verdict = check(&encode_present(&wrong_install), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (202, Some(Reason::InstallationMismatch))
        );

        // unknown_field: canonical JSON gains an extra field.
        let payload = format!(
            r#"{{"expires_at":"{}","extra_key":1,"installation_id":"{}","issued_at":"{}","nonce":"{}","peer_id":"{}"}}"#,
            base.expires_at, base.installation_id, base.issued_at, base.nonce, base.peer_id
        );
        use base64::Engine as _;
        let unknown_field = format!(
            "{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload),
            base_token().rsplit_once('.').unwrap().1
        );
        let verdict = check(&encode_present(&unknown_field), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // missing_field: canonical JSON drops nonce.
        let payload = r#"{"expires_at":"2026-10-15T12:10:00.000Z","installation_id":"b58-installation-relay-0001","issued_at":"2026-10-15T12:00:00.000Z","peer_id":"12D3KooWSrKnMZUcSxK8G7wmBbXdU8nFEfWGhLu6H8xjn8LmCSJb"}"#;
        let missing = format!(
            "{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload),
            base_token().rsplit_once('.').unwrap().1
        );
        let verdict = check(&encode_present(&missing), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // non_canonical_segment: swapped field order, valid signature.
        let payload = r#"{"installation_id":"b58-installation-relay-0001","expires_at":"2026-10-15T12:10:00.000Z","issued_at":"2026-10-15T12:00:00.000Z","nonce":"a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6","peer_id":"12D3KooWSrKnMZUcSxK8G7wmBbXdU8nFEfWGhLu6H8xjn8LmCSJb"}"#;
        let noncanon = format!(
            "{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload),
            base_token().rsplit_once('.').unwrap().1
        );
        let verdict = check(&encode_present(&noncanon), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // oversized_ticket: > 1024 bytes.
        let oversized = format!("{}.{}", "A".repeat(1100), "B".repeat(86));
        let verdict = check(&encode_present(&oversized), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // wrong_ttl_delta: 601 s, VALIDLY signed — syntax precedes signature.
        let wrong_ttl = mint(&derived, &base_peer_id, issued, 601, &nonce);
        let verdict = check(&encode_present(&wrong_ttl), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // nonce_wrong_charset: 31 chars / uppercase.
        let short_nonce = mint(
            &derived,
            &base_peer_id,
            issued,
            600,
            "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d",
        );
        let verdict = check(&encode_present(&short_nonce), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );
        let upper_nonce = mint(
            &derived,
            &base_peer_id,
            issued,
            600,
            "A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6",
        );
        let verdict = check(&encode_present(&upper_nonce), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // bad_timestamp: plain date.
        let mut plain_date = base.clone();
        plain_date.issued_at = "2026-10-15".to_string();
        let plain = sign_ticket(&plain_date, &fixture_key());
        let verdict = check(&encode_present(&plain), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );

        // single_segment: no '.' separator.
        let verdict = check(
            &encode_present(&"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".repeat(2)),
            &verifier,
        );
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed))
        );
    }

    /// The ORDER pins: a ticket failing BOTH signature and peer binding
    /// reports `bad_signature` (signature precedes binding); a ticket
    /// failing BOTH syntax and signature reports `malformed`.
    #[test]
    fn first_failure_wins_order_is_pinned() {
        let verifier = verifier(fixed_clock(at("2026-10-15T12:05:00Z")));
        let base = base_ticket();
        let issued = at("2026-10-15T12:00:00Z");

        // Bad signature AND wrong peer: bad_signature wins.
        let b_peer = libp2p::identity::Keypair::generate_ed25519();
        let b_peer_id = libp2p::PeerId::from(b_peer.public()).to_string();
        let for_b = RelayAdmissionTicket {
            peer_id: b_peer_id.clone(),
            ..base.clone()
        };
        let other = SigningKey::from_bytes(&[0x66u8; 32]);
        let both_bad = sign_ticket(&for_b, &other);
        let verdict = verify_present(
            &encode_present(&both_bad),
            &base_peer(),
            "b58-installation-relay-0001",
            &verifier,
        );
        assert_eq!(
            (verdict.code, verdict.reason),
            (202, Some(Reason::BadSignature)),
            "signature precedes binding"
        );

        // Wrong TTL (syntax) AND bad signature: malformed wins.
        let wrong_ttl = RelayAdmissionTicket {
            expires_at: "2026-10-15T12:10:01.000Z".to_string(),
            ..base.clone()
        };
        let both_bad = sign_ticket(&wrong_ttl, &other);
        let verdict = check(&encode_present(&both_bad), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (400, Some(Reason::Malformed)),
            "syntax precedes signature"
        );

        // Peer mismatch AND installation mismatch: peer binding (check 7)
        // precedes identity consistency (check 8).
        let both = mint("b58-someone-else", &b_peer_id, issued, 600, &base.nonce);
        let verdict = verify_present(
            &encode_present(&both),
            &base_peer(),
            "b58-installation-relay-0001",
            &verifier,
        );
        assert_eq!(
            (verdict.code, verdict.reason),
            (202, Some(Reason::PeerMismatch)),
            "peer binding precedes installation consistency"
        );
    }

    /// Check 8 with the REAL derivation, CLOSED (the 2026-10-15 vector
    /// regeneration, drift ruling (b)): the vector pair is really derived
    /// — the recorded peer seed's embedded key derives EXACTLY the
    /// vector's installation_id — so the golden token now passes check 8
    /// end-to-end against a real PeerId, and checks 7-8 are replayable
    /// cross-language (peer_identity_seed_hex is carried in the vectors;
    /// the TS consumer re-derives both ids with the tracker's own
    /// crypto primitives).
    #[test]
    fn real_derivation_accepts_the_regenerated_vector_pair() {
        // The vector peer IS the identity recorded as
        // peer_identity_seed_hex (0x51 × 32) in the vectors.
        let keypair = libp2p::identity::Keypair::ed25519_from_bytes(&mut [0x51; 32])
            .expect("seed 0x51 keypair");
        let peer = libp2p::PeerId::from(keypair.public());
        assert_eq!(
            peer,
            base_peer(),
            "the vector peer is the recorded-seed identity"
        );
        let derived = derived_installation_id(&peer).expect("ed25519 identity multihash derives");
        assert_eq!(
            derived,
            base_ticket().installation_id,
            "the vector installation_id IS the real ADR-020 derivation"
        );
        // Cross-check the derivation against modelswarm-identity directly
        // (protobuf key bytes = [0x08 0x01 0x12 0x20 ‖ 32-byte key]).
        let protobuf = keypair.public().encode_protobuf();
        assert_eq!(protobuf.len(), 36);
        assert_eq!(
            derived,
            modelswarm_identity::installation_id_for(&protobuf[4..36])
        );

        // Presenting the golden token under the REAL derivation now
        // passes end-to-end — checks 1-10, the closure the drift ruling
        // asked for.
        let verifier = verifier(fixed_clock(at("2026-10-15T12:05:00Z")));
        let verdict = verify_present(&encode_present(&base_token()), &peer, &derived, &verifier);
        assert_eq!((verdict.code, verdict.reason), (100, None));
    }

    // ------------------------------------------------------------------
    // Table semantics: capacity, eviction, idempotent refresh (checks 9-10)
    // ------------------------------------------------------------------

    #[test]
    fn admission_table_capacity_and_deadline_eviction() {
        let (clock, now) = settable_clock(at("2026-10-15T12:00:00Z"));
        let table = AdmissionTable::with_capacity(clock, 2);
        let a = PeerId::random();
        let b = PeerId::random();
        let c = PeerId::random();
        assert!(!table.is_live(&a));
        assert!(table.admit(a, at("2026-10-15T12:12:00Z")));
        assert!(table.admit(b, at("2026-10-15T12:12:00Z")));
        assert!(table.is_live(&a) && table.is_live(&b));
        // Full: a third DISTINCT peer is refused (check 9).
        assert!(!table.admit(c, at("2026-10-15T12:12:00Z")));
        // Idempotent refresh for an already-live peer still succeeds.
        assert!(table.admit(a, at("2026-10-15T12:30:00Z")));

        // Deadline eviction: past a's deadline it is dead, and the freed
        // slot admits c (expired entries evicted first).
        *now.lock().unwrap() = at("2026-10-15T12:31:00Z");
        assert!(!table.is_live(&a), "a expired at 12:30");
        assert!(!table.is_live(&b), "b expired at 12:12");
        assert!(table.admit(c, at("2026-10-15T12:45:00Z")), "slot freed");
        assert!(table.is_live(&c));
    }

    /// The table-full row: with a capacity-1 table, a valid second ticket is
    /// `resource_limit_exceeded(201)` / `table_full` (checks 9 before 10).
    #[test]
    fn table_full_denies_with_resource_limit_exceeded() {
        let clock = fixed_clock(at("2026-10-15T12:05:00Z"));
        let verifier = AdmissionVerifier {
            key: fixture_verifying_key(),
            table: Arc::new(AdmissionTable::with_capacity(clock, 1)),
        };
        // Fill the single slot with another peer's valid ticket.
        let other_peer = PeerId::random();
        verifier.table.admit(other_peer, at("2026-10-15T12:12:00Z"));
        let verdict = check(&encode_present(&base_token()), &verifier);
        assert_eq!(
            (verdict.code, verdict.reason),
            (201, Some(Reason::TableFull))
        );
    }

    // ------------------------------------------------------------------
    // Redaction (§6.2 test 6, relay half): no ticket bytes anywhere
    // ------------------------------------------------------------------

    #[test]
    fn log_lines_carry_no_ticket_bytes() {
        let token = base_token();
        let accepted = Event {
            peer: base_peer(),
            outcome: Outcome::Accepted {
                until: at("2026-10-15T12:12:00Z"),
            },
        };
        let denied = Event {
            peer: base_peer(),
            outcome: Outcome::Denied {
                reason: Reason::BadSignature,
            },
        };
        for line in [accepted.log_line(), denied.log_line()] {
            assert!(!line.contains(&token), "no serialized ticket in logs");
            assert!(!line.contains("eyJ"), "no base64 ticket payload in logs");
        }
        // The frozen vocabulary, exactly.
        assert!(accepted.log_line().starts_with("admission accepted "));
        assert!(accepted.log_line().contains(" until 2026-10-15T12:12:00"));
        assert_eq!(
            denied.log_line(),
            format!("admission denied {}: bad_signature", base_peer())
        );
    }

    // ------------------------------------------------------------------
    // The gate predicate closes on expiry (unit half; the in-process proof
    // against the real relay stack lives in main.rs's tests)
    // ------------------------------------------------------------------

    #[test]
    fn gate_predicate_is_deadline_bounded() {
        let (clock, now) = settable_clock(at("2026-10-15T12:00:00Z"));
        let table = AdmissionTable::new(clock);
        let peer = PeerId::random();
        // deadline = expires_at + 120 s (T4): admission never outlives its
        // ticket beyond the skew grace.
        table.admit(peer, at("2026-10-15T12:12:00Z"));
        assert!(table.is_live(&peer));
        *now.lock().unwrap() = at("2026-10-15T12:11:59Z");
        assert!(table.is_live(&peer), "still inside the grace");
        *now.lock().unwrap() = at("2026-10-15T12:12:01Z");
        assert!(
            !table.is_live(&peer),
            "past the deadline the gate re-closes"
        );
    }

    #[test]
    fn empty_table_is_not_live_and_starts_empty() {
        let table = AdmissionTable::new(fixed_clock(at("2026-10-15T12:00:00Z")));
        assert!(table.is_empty());
        assert!(!table.is_live(&PeerId::random()));
    }
}
