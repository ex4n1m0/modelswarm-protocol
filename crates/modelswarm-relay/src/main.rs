//! MSP circuit-relay v2 host (ADR-014, Phase F2; ADR-038 admission).
//!
//! A standard rust-libp2p relay with the limits the phase-f research
//! mandates, plus the ADR-038 admission gate (the D12 close):
//!
//! - **Admission by tracker-signed ticket** ([`admission`]): the
//!   `/msp/relay-ticket/1` pre-flight verifies a `RelayAdmissionTicket`
//!   against the pinned hub key, and ONE gate limiter prepended to BOTH
//!   rate-limiter vectors refuses every reservation and circuit from a
//!   peer without live admission (`resource_limit_exceeded(201)`) —
//!   fail-closed at the accept path itself, independent of the pre-flight.
//!   No valid ticket ⇒ no resources, ever.
//! - **Finite, ADR-frozen circuit caps** (§3.5): `reservation_duration`
//!   600 s (≤ ticket TTL), `max_circuit_duration` 1800 s,
//!   `max_circuit_bytes` 16 MiB. These replace the F2A/F2B uncapped
//!   posture per ADR-038; the operator flags override them. A cap cut
//!   mid-token-stream is unrecoverable (ADR-007: no retry after the first
//!   output token) — the frozen numbers sit two orders above every
//!   legitimate maximum, and the relay announces them with every
//!   reservation so clients price relayed paths honestly.
//! - The library's anti-abuse creation rates are kept (per-peer/per-IP),
//!   with the gate FIRST so unadmitted peers never consume the per-IP
//!   tokens honest peers behind shared NATs depend on.
//!
//! The relay is content-blind by construction: admission is a PRE-STREAM
//! check of a signed blob, and relayed substreams are forwarded as opaque
//! bytes — never parsed (prompts and completions cannot appear here).
//! Admission state is memory-only; the relay NEVER contacts the tracker
//! (T8). The ticket asserts installation liveness ONLY — serving
//! eligibility stays the ADR-026 lease gate at the peers.
//!
//! Node-side support (reserve + dial through `/p2p-circuit` addresses over
//! the raw-QUIC session layer) is the F2b transport work
//! (`modelswarm-transport`); this binary is interoperable with any standard
//! libp2p client today (which the gate will refuse unless admitted).
//!
//! Bind discipline (F2B, 2026-10-15): the default bind is LOOPBACK
//! (`127.0.0.1:4001`) — the same ADR-018 posture the transport crate's
//! `ListenOptions::allow_off_loopback` enforces. Binding any non-loopback
//! address requires the explicit `--public` operator opt-in at the command
//! line; a silently-open relay is never the default. Since ADR-038 a
//! `--public` bind ADDITIONALLY requires a parseable tracker key (embedded
//! default or `--tracker-key`) — refusal is honest and early (exit 2). The
//! gate itself is unconditional: every mode answers the admission protocol
//! and runs the gate limiters; no ungated mode exists.

mod admission;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use futures::StreamExt;
use libp2p::relay;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identify, ping};

/// The pinned hub Ed25519 public key (protocol/keys/hub-public.hex) — the
/// same root the node build embeds for catalog/lease verification; one key
/// now guards tickets too (ADR-038 §3.4). Overridable for rotation testing
/// by `--tracker-key <PATH>`.
const HUB_PUBLIC_KEY_HEX: &str = include_str!("../../../protocol/keys/hub-public.hex");

/// ADR-038 §3.5 frozen policy: reservation lifetime (≤ ticket TTL —
/// admission can never outlive its ticket beyond the skew grace; renewals
/// re-hit the gate every ≤ 450 s).
pub const FROZEN_RESERVATION_DURATION_SECS: u64 = 600;

/// ADR-038 §3.5 frozen policy: per-circuit lifetime cap.
pub const FROZEN_MAX_CIRCUIT_DURATION_SECS: u64 = 1800;

/// ADR-038 §3.5 frozen policy: per-circuit relayed-byte cap (16 MiB).
pub const FROZEN_MAX_CIRCUIT_BYTES: u64 = 16 * 1024 * 1024;

/// Stream-carrying limits + the admission gate (the whole point of running
/// our own relay). The circuit caps default to the ADR-038 §3.5 FROZEN
/// numbers (which REPLACE the pre-ADR-038 uncapped posture); the operator
/// flags override them. `table` is the shared admission state both the
/// pre-flight and the gate limiters close over.
fn relay_config(
    max_circuit_duration_secs: Option<u64>,
    max_circuit_bytes: Option<u64>,
    table: &Arc<admission::AdmissionTable>,
) -> relay::Config {
    let mut config = relay::Config {
        max_reservations: 4096,
        max_reservations_per_peer: 8,
        // ≤ ticket TTL (ADR-038 §3.5): admission never outlives its ticket
        // beyond the skew grace; renewals re-hit the gate.
        reservation_duration: Duration::from_secs(FROZEN_RESERVATION_DURATION_SECS),
        max_circuits: 4096,
        max_circuits_per_peer: 128,
        // ADR-038 §3.5 frozen defaults (replacing the F2A/F2B uncapped
        // posture): two orders of magnitude above every legitimate maximum;
        // announced with every reservation (T5).
        max_circuit_duration: Duration::from_secs(
            max_circuit_duration_secs.unwrap_or(FROZEN_MAX_CIRCUIT_DURATION_SECS),
        ),
        max_circuit_bytes: max_circuit_bytes.unwrap_or(FROZEN_MAX_CIRCUIT_BYTES),
        // Reservation/circuit CREATION rates keep the library's anti-abuse
        // defaults via struct-update (30 per 2 min per peer) — circuits are
        // per pooled session, not per request, so this does not bound
        // throughput.
        ..relay::Config::default()
    };
    // THE GATE (ADR-038 §3.4): one closure limiter prepended FIRST to BOTH
    // limiter vectors — the pinned libp2p-relay 0.22.0 seam consults every
    // limiter at both resource-grant sites (reservations incl. renewals;
    // circuit sources) and denies ResourceLimitExceeded on the first false,
    // so no valid admission ⇒ no resources, pre-flight or not (T1). FIRST
    // also means unadmitted peers never consume the per-IP tokens honest
    // peers behind shared NATs depend on (`.all()` short-circuits).
    let [gate_reservations, gate_circuits] = admission::gate_limiters(table);
    config
        .reservation_rate_limiters
        .insert(0, gate_reservations);
    config.circuit_src_rate_limiters.insert(0, gate_circuits);
    config
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay: relay::Behaviour,
    admission: admission::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

fn load_or_create_key(path: &PathBuf) -> Result<libp2p::identity::Keypair, String> {
    if path.exists() {
        let mut bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let keypair = libp2p::identity::ed25519::Keypair::try_from_bytes(&mut bytes)
            .map_err(|e| e.to_string())?;
        return Ok(libp2p::identity::Keypair::from(keypair));
    }
    let keypair = libp2p::identity::ed25519::Keypair::generate();
    std::fs::write(path, keypair.to_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(libp2p::identity::Keypair::from(keypair))
}

fn print_usage() {
    println!("usage: modelswarm-relay [--listen 127.0.0.1:4001] [--key msp-relay.key] [--public]");
    println!(
        "        [--tracker-key <PATH>] [--max-circuit-duration-secs N] [--max-circuit-bytes N]"
    );
    println!("  --listen  UDP/QUIC bind address (default 127.0.0.1:4001 — LOOPBACK).");
    println!("            A non-loopback IP additionally requires --public.");
    println!("  --key     persisted ed25519 key file (default msp-relay.key in cwd)");
    println!("            a STABLE key keeps the relay PeerId — and every");
    println!("            /p2p/<relay>/p2p-circuit address — valid across restarts.");
    println!("  --public  EXPLICIT operator opt-in for a non-loopback bind (the");
    println!("            ADR-018 allow_off_loopback discipline: the relay never");
    println!("            silently binds a public interface). Since ADR-038 a");
    println!("            --public bind ALSO requires a parseable tracker key");
    println!("            (embedded default or --tracker-key) — the relay refuses");
    println!("            to start (exit 2) otherwise: no ungated public mode.");
    println!("  --tracker-key <PATH>  overrides the embedded pinned hub Ed25519");
    println!("            key (protocol/keys/hub-public.hex) with a file of 64");
    println!("            lowercase hex chars — the key relay admission tickets");
    println!("            verify against. Rotation/testing surface, not protocol.");
    println!("  --max-circuit-duration-secs N   per-circuit lifetime cap in seconds.");
    println!("            DEFAULT (ADR-038, 2026-10-15): 1800 — the frozen finite");
    println!("            cap that REPLACES the former UNCAPPED posture (the");
    println!("            absent flag now means the ADR default, not uncapped).");
    println!("            HONEST WARNING: a cap is a real CUT — the relay terminates");
    println!("            the circuit at the limit, and a cut mid-token-stream is");
    println!("            unrecoverable for the requester (ADR-007: no retry after");
    println!("            the first output token). The frozen numbers sit two");
    println!("            orders above legitimate traffic; if a legitimate workload");
    println!("            ever measures against them they move by ADR amendment,");
    println!("            never by config tweak. Range 1..=4294967295 (u32::MAX —");
    println!("            the circuit-relay-v2 wire format's duration field); 0 is");
    println!("            rejected, not read as \"uncapped\".");
    println!("  --max-circuit-bytes N            per-circuit relayed-byte cap.");
    println!("            DEFAULT (ADR-038): 16777216 (16 MiB) — same frozen-default");
    println!("            change and CUT warning as above. Range 1..=2^64-1; 0 is");
    println!("            rejected — the library treats 0 as \"no limit\"");
    println!("            internally, so accepting it here would silently mean the");
    println!("            opposite of what it says.");
    println!("  Admission (ADR-038): reservations and circuits are granted only to");
    println!("  peers presenting a valid tracker-signed relay admission ticket");
    println!("  (/msp/relay-ticket/1 pre-flight); the gate is fail-closed at the");
    println!("  accept sites. The admission table is memory-only (4096 PeerIds,");
    println!("  deadline-evicted). reservation_duration is frozen at 600 s (<= the");
    println!("  ticket TTL).");
    println!("  At startup the relay prints a --limit-summary line with the");
    println!("  EFFECTIVE posture (reservations, circuits, per-peer counts,");
    println!("  duration/byte caps) — the ops surface for the live deployment's");
    println!("  logs.");
}

/// Resolve the advertised LAN IP the way the node does — the UDP-connect
/// trick never sends a packet.
fn lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Parsed command line. `public` is the explicit non-loopback opt-in; the
/// cap fields are `Option` — `None` means the ADR-038 §3.5 FROZEN defaults
/// (1800 s / 16 MiB), which supersede the pre-ADR-038 uncapped posture;
/// `tracker_key` overrides the embedded pinned hub key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    listen: String,
    key_path: PathBuf,
    public: bool,
    tracker_key: Option<PathBuf>,
    max_circuit_duration_secs: Option<u64>,
    max_circuit_bytes: Option<u64>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:4001".to_string(),
            key_path: PathBuf::from("msp-relay.key"),
            public: false,
            tracker_key: None,
            max_circuit_duration_secs: None,
            max_circuit_bytes: None,
        }
    }
}

/// Splits a validated `ip:port` listen value. Returned separately so the
/// bind-time validation and the tests share one parser.
fn split_listen(listen: &str) -> Result<(IpAddr, u16), String> {
    let (ip, port) = listen
        .rsplit_once(':')
        .ok_or_else(|| format!("--listen must be ip:port, got {listen}"))?;
    let ip: IpAddr = ip
        .parse()
        .map_err(|_| format!("--listen ip must parse, got {ip}"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| format!("--listen port must be numeric, got {port}"))?;
    Ok((ip, port))
}

/// `--max-circuit-duration-secs` value: a whole number of seconds in
/// `1..=u32::MAX`. 0 is REFUSED (not reinterpreted): a zero-second cap
/// would cut every circuit at acceptance, which is not "uncapped" and not
/// a usable posture — omit the flag for uncapped. Above `u32::MAX` the
/// circuit-relay-v2 wire format's duration field cannot carry the value
/// and the behaviour PANICS when it announces the limit, so the range is
/// enforced here, at the flag boundary.
fn parse_duration_secs(value: &str) -> Result<u64, String> {
    let secs: u64 = value.parse().map_err(|_| {
        format!("--max-circuit-duration-secs must be a whole number of seconds, got {value}")
    })?;
    if secs == 0 {
        return Err(
            "--max-circuit-duration-secs must be >= 1 — 0 would cut every circuit instantly \
             and is NOT \"uncapped\"; omit the flag to stay uncapped"
                .to_string(),
        );
    }
    if secs > u32::MAX as u64 {
        return Err(format!(
            "--max-circuit-duration-secs may not exceed {} (u32::MAX) — the \
             circuit-relay-v2 wire format's duration field",
            u32::MAX
        ));
    }
    Ok(secs)
}

/// `--max-circuit-bytes` value: a whole number of bytes >= 1. 0 is REFUSED
/// because the library's copy loop treats 0 as "no limit" — accepting it
/// here would let an operator believe a cap is armed while nothing is
/// enforced (and the announced limit would still say 0). Omit the flag for
/// uncapped.
fn parse_circuit_bytes(value: &str) -> Result<u64, String> {
    let bytes: u64 = value
        .parse()
        .map_err(|_| format!("--max-circuit-bytes must be a whole number of bytes, got {value}"))?;
    if bytes == 0 {
        return Err(
            "--max-circuit-bytes must be >= 1 — the relay treats 0 as NO limit \
             internally, so 0 would not cap anything; omit the flag to stay uncapped"
                .to_string(),
        );
    }
    Ok(bytes)
}

/// Parses argv into [`Args`], enforcing the ADR-018 bind discipline:
/// a non-loopback `--listen` without `--public` is REFUSED here, before
/// any socket exists. `Err` strings are honest one-line diagnostics.
fn parse_args<I: Iterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => {
                parsed.listen = args
                    .next()
                    .ok_or_else(|| "--listen needs a value".to_string())?;
            }
            "--key" => {
                parsed.key_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--key needs a value".to_string())?,
                );
            }
            "--public" => parsed.public = true,
            "--tracker-key" => {
                parsed.tracker_key = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--tracker-key needs a value".to_string())?,
                ));
            }
            "--max-circuit-duration-secs" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--max-circuit-duration-secs needs a value".to_string())?;
                parsed.max_circuit_duration_secs = Some(parse_duration_secs(&value)?);
            }
            "--max-circuit-bytes" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--max-circuit-bytes needs a value".to_string())?;
                parsed.max_circuit_bytes = Some(parse_circuit_bytes(&value)?);
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                print_usage();
                return Err(format!("unknown argument: {other}"));
            }
        }
    }
    let (ip, _port) = split_listen(&parsed.listen)?;
    if !ip.is_loopback() && !parsed.public {
        return Err(format!(
            "refusing non-loopback bind {ip}: pass --public to explicitly open the relay \
             beyond loopback (ADR-018 allow_off_loopback discipline)"
        ));
    }
    Ok(parsed)
}

/// Resolves the tracker verification key: `--tracker-key <PATH>` (64
/// lowercase hex, trimmed) or the embedded pinned default
/// ([`HUB_PUBLIC_KEY_HEX`]). The ADR-038 §3.4 boot discipline — a `--public`
/// bind REQUIRES a parseable key — refuses startup on `Err` (exit 2); and
/// because no ungated mode exists, an unresolvable key refuses startup in
/// EVERY mode.
fn resolve_tracker_key(args: &Args) -> Result<VerifyingKey, String> {
    let hex_source: String = match &args.tracker_key {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("--tracker-key {}: {e}", path.display()))?;
            raw.trim().to_string()
        }
        None => HUB_PUBLIC_KEY_HEX.trim().to_string(),
    };
    if hex_source.len() != 64
        || !hex_source
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(format!(
            "tracker key must be 64 lowercase hex chars, got {} char(s){}",
            hex_source.len(),
            if args.tracker_key.is_none() {
                " (the embedded protocol/keys/hub-public.hex is corrupt)"
            } else {
                ""
            }
        ));
    }
    let bytes = hex::decode(&hex_source).map_err(|e| format!("tracker key hex: {e}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "tracker key must decode to 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| format!("tracker key parse: {e}"))
}

/// The startup `--limit-summary` ops line: the EFFECTIVE posture the relay
/// is about to run with, as greppable key=value tokens. The ADR-038 frozen
/// caps (1800 s / 16 MiB) are finite by default; operator overrides print
/// as their numbers.
fn limit_summary(config: &relay::Config) -> String {
    format!(
        "--limit-summary reservations={} reservations_per_peer={} circuits={} \
         circuits_per_peer={} max_circuit_duration={}s max_circuit_bytes={} \
         reservation_duration={}s admission=ADR-038-ticket",
        config.max_reservations,
        config.max_reservations_per_peer,
        config.max_circuits,
        config.max_circuits_per_peer,
        config.max_circuit_duration.as_secs(),
        config.max_circuit_bytes,
        config.reservation_duration.as_secs(),
    )
}

/// The honest warning printed at startup when an operator has armed a cap
/// OVERRIDE: the frozen ADR-038 defaults are already cuts (that is their
/// point); an override moves the cut, which is an operator decision the
/// log must show. ADR-007 interplay: a cut mid-token-stream is
/// unrecoverable for the requester.
fn caps_warning(args: &Args) -> Option<String> {
    if args.max_circuit_duration_secs.is_none() && args.max_circuit_bytes.is_none() {
        return None;
    }
    Some(
        "--limit-summary WARNING: operator cap overrides are ARMED (ADR-038 frozen \
         defaults are 1800s / 16777216 bytes) — the relay CUTS circuits at \
         max_circuit_duration/max_circuit_bytes, and a cut mid-token-stream is \
         unrecoverable for the requester (ADR-007: no retry after the first output \
         token). Caps move by ADR amendment with measurements, never by tweak."
            .to_string(),
    )
}

#[tokio::main]
async fn main() {
    let parsed = parse_args(std::env::args().skip(1)).unwrap_or_else(|e| {
        eprintln!("modelswarm-relay: {e}");
        std::process::exit(2);
    });
    let (bind_ip, bind_port) = split_listen(&parsed.listen).unwrap_or_else(|e| {
        eprintln!("modelswarm-relay: {e}");
        std::process::exit(2);
    });
    let bind_port = bind_port.to_string();

    let keypair = load_or_create_key(&parsed.key_path).unwrap_or_else(|e| {
        eprintln!("modelswarm-relay: identity: {e}");
        std::process::exit(1);
    });
    let peer_id = libp2p::PeerId::from(keypair.public());

    // ADR-038 §3.4 boot discipline: fail-closed at startup. A --public bind
    // REQUIRES a parseable tracker key; and because the gate is
    // unconditional (no ungated mode exists), an unresolvable key refuses
    // startup in every mode.
    let tracker_key = resolve_tracker_key(&parsed).unwrap_or_else(|e| {
        eprintln!(
            "modelswarm-relay: refusing to start{}: {e}",
            if parsed.public {
                " (--public requires a valid tracker key: embedded default or --tracker-key)"
            } else {
                " (no ungated mode exists; fix the tracker key)"
            }
        );
        std::process::exit(2);
    });
    let table = admission::AdmissionTable::new(admission::wall_clock());
    let verifier = Arc::new(admission::AdmissionVerifier {
        key: tracker_key,
        table: Arc::clone(&table),
    });

    let config = relay_config(
        parsed.max_circuit_duration_secs,
        parsed.max_circuit_bytes,
        &table,
    );
    println!("modelswarm-relay: peer id {peer_id}");
    println!("modelswarm-relay: {}", limit_summary(&config));
    if let Some(warning) = caps_warning(&parsed) {
        println!("modelswarm-relay: {warning}");
    }
    println!(
        "modelswarm-relay: admission gate ARMED — /msp/relay-ticket/1 pre-flight + \
         fail-closed reservation/circuit limiters; admission state is memory-only"
    );
    println!("modelswarm-relay: content-blind — frames are forwarded, never parsed");

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(|key| Behaviour {
            relay: relay::Behaviour::new(key.public().to_peer_id(), config),
            admission: admission::Behaviour::new(verifier),
            identify: identify::Behaviour::new(identify::Config::new(
                "/msp-relay/1".to_string(),
                key.public(),
            )),
            ping: ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(30))),
        })
        .unwrap_or_else(|e| {
            eprintln!("modelswarm-relay: behaviour build failed: {e}");
            std::process::exit(1);
        })
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(300)))
        .build();

    // The relay starts DISABLED and would auto-enable only once it observes
    // its own external address via identify — a fresh bind rejects every
    // reservation until then. A dedicated relay host advertises HOP always.
    swarm
        .behaviour_mut()
        .relay
        .set_status(Some(relay::Status::Enable));

    let bind_family = if bind_ip.is_ipv4() { "ip4" } else { "ip6" };
    let multiaddr: libp2p::Multiaddr = format!("/{bind_family}/{bind_ip}/udp/{bind_port}/quic-v1")
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("modelswarm-relay: bad listen multiaddr: {e}");
            std::process::exit(2);
        });
    swarm.listen_on(multiaddr).unwrap_or_else(|e| {
        eprintln!("modelswarm-relay: listen {} failed: {e}", parsed.listen);
        std::process::exit(1);
    });

    // Honest advertising: a loopback bind advertises exactly the loopback
    // address (the lan_ip() trick would advertise an interface the relay
    // never bound); only a --public bind advertises the LAN address.
    let advertised_ip = if bind_ip.is_loopback() {
        bind_ip
    } else {
        lan_ip().unwrap_or(bind_ip)
    };
    let family = if advertised_ip.is_ipv4() {
        "ip4"
    } else {
        "ip6"
    };
    if let Ok(addr) = format!("/{family}/{advertised_ip}/udp/{bind_port}/quic-v1").parse() {
        swarm.add_external_address(addr);
    }
    let posture = if bind_ip.is_loopback() {
        "loopback-only"
    } else {
        "PUBLIC (--public)"
    };
    println!(
        "modelswarm-relay: listening on /{family}/{advertised_ip}/udp/{bind_port}/quic-v1 ({posture}, relay {peer_id}"
    );
    println!("modelswarm-relay: ready — Ctrl-C stops the relay");

    use libp2p::swarm::SwarmEvent;
    loop {
        tokio::select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(BehaviourEvent::Admission(event)) => {
                    // The frozen ADR-038 §3.4 vocabulary, exactly — peer id,
                    // code, reason, deadline; never any ticket bytes.
                    println!("modelswarm-relay: {}", event.log_line());
                }
                SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::Event::ReservationReqAccepted { src_peer_id, renewed },
                )) => println!(
                    "modelswarm-relay: reservation {} by {src_peer_id}",
                    if renewed { "renewed" } else { "accepted" }
                ),
                SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::Event::ReservationReqDenied { src_peer_id, status },
                )) => println!(
                    "modelswarm-relay: reservation denied for {src_peer_id}: {status:?}"
                ),
                SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::Event::CircuitReqAccepted { src_peer_id, dst_peer_id },
                )) => println!("modelswarm-relay: circuit {src_peer_id} -> {dst_peer_id}"),
                SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::Event::CircuitReqDenied { src_peer_id, dst_peer_id, status },
                )) => println!(
                    "modelswarm-relay: circuit denied {src_peer_id} -> {dst_peer_id}: {status:?}"
                ),
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("modelswarm-relay: bound {address}");
                }
                _ => {}
            },
            _ = tokio::signal::ctrl_c() => {
                println!("modelswarm-relay: shutting down");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::core::muxing::StreamMuxer as _;
    use libp2p::core::transport::{DialOpts, PortUse};
    use libp2p::core::Transport as _;
    use libp2p::quic;
    use modelswarm_eligibility::relay_ticket::sign_ticket;
    use modelswarm_eligibility::relay_ticket::RelayAdmissionTicket;
    use time::OffsetDateTime;

    // ------------------------------------------------------------------
    // Bind-discipline / flag parsing (ADR-018 + ADR-038 command surface)
    // ------------------------------------------------------------------

    /// F2B bind discipline: the default is LOOPBACK, a non-loopback bind is
    /// refused without the explicit `--public` opt-in, and the opt-in lifts
    /// exactly that refusal (ADR-018 allow_off_loopback discipline — never a
    /// silently-open relay).
    #[test]
    fn non_loopback_bind_requires_the_explicit_public_opt_in() {
        // Defaults: loopback, not public.
        let defaults = parse_args(std::iter::empty()).unwrap();
        assert_eq!(defaults, Args::default());
        assert_eq!(defaults.listen, "127.0.0.1:4001");
        assert!(!defaults.public);

        // Non-loopback without --public: refused, with the opt-in named.
        for listen in ["0.0.0.0:4001", "192.168.1.10:4001", "10.0.0.5:9000"] {
            let err =
                parse_args(["--listen".to_string(), listen.to_string()].into_iter()).unwrap_err();
            assert!(
                err.contains("--public") && err.contains("refusing non-loopback bind"),
                "refusal must name the opt-in: {err}"
            );
        }

        // With --public the non-loopback bind parses.
        let public = parse_args(
            [
                "--listen".to_string(),
                "0.0.0.0:4001".to_string(),
                "--public".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(public.public);
        assert_eq!(public.listen, "0.0.0.0:4001");

        // Loopback never needs the opt-in.
        let loopback =
            parse_args(["--listen".to_string(), "127.0.0.1:0".to_string()].into_iter()).unwrap();
        assert!(!loopback.public);
    }

    /// Malformed listen values, unknown flags, and valueless flags are
    /// honest parse failures.
    #[test]
    fn parse_args_rejects_malformed_input() {
        for bad in [
            "noip",
            "1.2.3.4",
            "1.2.3.4:notaport",
            "1.2.3.4:99999",
            "1.2.3.4:-1",
        ] {
            assert!(
                parse_args(["--listen".to_string(), bad.to_string()].into_iter()).is_err(),
                "{bad}"
            );
        }
        assert!(parse_args(["--nope".to_string()].into_iter()).is_err());
        assert!(parse_args(["--listen".to_string()].into_iter()).is_err());
        assert!(parse_args(["--key".to_string()].into_iter()).is_err());
        assert!(parse_args(["--tracker-key".to_string()].into_iter()).is_err());
    }

    /// The `--tracker-key` flag: absent means the embedded pinned key; a
    /// value lands in `Args`; a valueless flag fails.
    #[test]
    fn tracker_key_flag_parses() {
        let absent = parse_args(std::iter::empty()).unwrap();
        assert_eq!(absent.tracker_key, None);

        let with_path =
            parse_args(["--tracker-key".to_string(), "fixture-hub.hex".to_string()].into_iter())
                .unwrap();
        assert_eq!(
            with_path.tracker_key,
            Some(PathBuf::from("fixture-hub.hex"))
        );

        // Combines with the other flags.
        let combined = parse_args(
            [
                "--listen".to_string(),
                "0.0.0.0:4001".to_string(),
                "--public".to_string(),
                "--tracker-key".to_string(),
                "/keys/hub.hex".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(combined.tracker_key, Some(PathBuf::from("/keys/hub.hex")));
    }

    /// Boot discipline (ADR-038 §3.4, §6.2 test 4): a `--public` bind
    /// without a resolvable tracker key refuses startup (the exit-2 path —
    /// `resolve_tracker_key`'s Err is what main turns into exit 2); the
    /// embedded default key resolves; a valid `--tracker-key` file
    /// overrides it; garbage (wrong length, uppercase, non-hex, missing
    /// file) is refused honestly. The loopback default is unchanged.
    #[test]
    fn public_boot_requires_a_resolvable_tracker_key() {
        // The embedded default resolves (both modes).
        let args = parse_args(std::iter::empty()).unwrap();
        assert!(resolve_tracker_key(&args).is_ok(), "embedded key parses");
        let public = parse_args(
            [
                "--listen".to_string(),
                "0.0.0.0:4001".to_string(),
                "--public".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(
            resolve_tracker_key(&public).is_ok(),
            "--public boots with the embedded key"
        );

        // A --tracker-key file overrides the embedded key.
        let fixture = std::env::temp_dir().join("msp-relay-test-hub-key.hex");
        std::fs::write(
            &fixture,
            "76d81b33ba721df89b8e0b0ef26c34eb72e978951044798cd608a4f6dbf2e6d6\n",
        )
        .unwrap();
        let with_file = parse_args(
            [
                "--listen".to_string(),
                "0.0.0.0:4001".to_string(),
                "--public".to_string(),
                "--tracker-key".to_string(),
                fixture.to_string_lossy().to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        let key = resolve_tracker_key(&with_file).expect("fixture key file resolves");
        let expected: [u8; 32] =
            hex::decode("76d81b33ba721df89b8e0b0ef26c34eb72e978951044798cd608a4f6dbf2e6d6")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(key.as_bytes(), &expected);
        let _ = std::fs::remove_file(&fixture);

        // Missing file: honest refusal naming the path.
        let missing = parse_args(
            [
                "--tracker-key".to_string(),
                "C:/definitely/not/here.hex".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        let err = resolve_tracker_key(&missing).unwrap_err();
        assert!(err.contains("--tracker-key"), "{err}");

        // Garbage contents: refused naming the required shape.
        for garbage in [
            "",
            "abcd",
            "ABCDABCDABCDABCDABCDABCDABCDABCDABCDABCDABCDABCDABCDABCDABCDABCD",
            "zz",
        ] {
            let garbage_path = std::env::temp_dir().join("msp-relay-test-hub-garbage.hex");
            std::fs::write(&garbage_path, garbage).unwrap();
            let bad = parse_args(
                [
                    "--listen".to_string(),
                    "0.0.0.0:4001".to_string(),
                    "--public".to_string(),
                    "--tracker-key".to_string(),
                    garbage_path.to_string_lossy().to_string(),
                ]
                .into_iter(),
            )
            .unwrap();
            let err = resolve_tracker_key(&bad).unwrap_err();
            assert!(
                err.contains("64 lowercase hex"),
                "the refusal must name the shape: {err}"
            );
            let _ = std::fs::remove_file(&garbage_path);
        }
    }

    /// Admission-bounds flag parsing: absent flags mean the ADR-038 frozen
    /// defaults (1800 s / 16 MiB), valid values land in `Args`, and every
    /// dishonest value is refused with a reason that names the fix.
    #[test]
    fn cap_flags_parse_validate_and_default_to_the_frozen_policy() {
        fn flags(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        }

        // ABSENT: both caps are None — meaning the FROZEN defaults.
        let absent = parse_args(std::iter::empty()).unwrap();
        assert_eq!(absent.max_circuit_duration_secs, None);
        assert_eq!(absent.max_circuit_bytes, None);
        assert_eq!(absent, Args::default());

        // VALID: values land, alone and combined with the other flags.
        let duration =
            parse_args(flags(&["--max-circuit-duration-secs", "3600"]).into_iter()).unwrap();
        assert_eq!(duration.max_circuit_duration_secs, Some(3600));
        assert_eq!(duration.max_circuit_bytes, None);
        let bytes = parse_args(flags(&["--max-circuit-bytes", "104857600"]).into_iter()).unwrap();
        assert_eq!(bytes.max_circuit_duration_secs, None);
        assert_eq!(bytes.max_circuit_bytes, Some(104857600));
        let combined = parse_args(
            flags(&[
                "--listen",
                "0.0.0.0:4001",
                "--public",
                "--max-circuit-duration-secs",
                "120",
                "--max-circuit-bytes",
                "2048",
            ])
            .into_iter(),
        )
        .unwrap();
        assert_eq!(combined.max_circuit_duration_secs, Some(120));
        assert_eq!(combined.max_circuit_bytes, Some(2048));
        assert!(combined.public);

        // Duration boundary: u32::MAX is the last legal second (the wire
        // format's field); one past it is refused naming the bound.
        let edge =
            parse_args(flags(&["--max-circuit-duration-secs", "4294967295"]).into_iter()).unwrap();
        assert_eq!(edge.max_circuit_duration_secs, Some(u32::MAX as u64));
        let err = parse_args(flags(&["--max-circuit-duration-secs", "4294967296"]).into_iter())
            .unwrap_err();
        assert!(
            err.contains("u32::MAX") && err.contains("wire format"),
            "the over-range refusal must name the wire bound: {err}"
        );

        // 0 is REFUSED for both — it is not "uncapped" (duration) and not
        // a cap at all (the library reads 0 bytes as no limit).
        for zero in [
            &flags(&["--max-circuit-duration-secs", "0"]),
            &flags(&["--max-circuit-bytes", "0"]),
        ] {
            let err = parse_args(zero.iter().cloned()).unwrap_err();
            assert!(
                err.contains("omit the flag"),
                "the 0 refusal must name the escape: {err}"
            );
        }

        // Non-numeric, fractional, negative, and missing values fail.
        for bad in [
            &flags(&["--max-circuit-duration-secs", "soon"]),
            &flags(&["--max-circuit-duration-secs", "1.5"]),
            &flags(&["--max-circuit-duration-secs", "-1"]),
            &flags(&["--max-circuit-bytes", "many"]),
            &flags(&["--max-circuit-bytes", "1kiB"]),
            &flags(&["--max-circuit-bytes", "-5"]),
        ] {
            assert!(
                parse_args(bad.iter().cloned()).is_err(),
                "must refuse {bad:?}"
            );
        }
        assert!(
            parse_args(flags(&["--max-circuit-duration-secs"]).into_iter()).is_err(),
            "a valueless duration flag must fail"
        );
        assert!(
            parse_args(flags(&["--max-circuit-bytes"]).into_iter()).is_err(),
            "a valueless bytes flag must fail"
        );
    }

    /// The ADR-038 frozen policy table (§3.5) is the absent-flag config,
    /// field for field: reservation_duration 600 s (≤ ticket TTL), circuit
    /// caps 1800 s / 16 MiB, the raised counts unchanged, and the gate
    /// limiter prepended to BOTH limiter vectors. This REPLACES the
    /// pre-ADR-038 uncapped posture — an honest, ADR-recorded change.
    #[test]
    fn absent_flags_mean_the_frozen_adr038_policy() {
        let args = parse_args(std::iter::empty()).unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        assert_eq!(config.max_reservations, 4096);
        assert_eq!(config.max_reservations_per_peer, 8);
        assert_eq!(config.max_circuits, 4096);
        assert_eq!(config.max_circuits_per_peer, 128);
        assert_eq!(
            config.reservation_duration,
            Duration::from_secs(600),
            "reservation_duration is frozen at the ticket TTL (§3.5)"
        );
        assert_eq!(
            config.max_circuit_duration,
            Duration::from_secs(1800),
            "the frozen 30-minute circuit cap (§3.5)"
        );
        assert_eq!(
            config.max_circuit_bytes,
            16 * 1024 * 1024,
            "the frozen 16 MiB circuit cap (§3.5)"
        );
        // The gate limiter is FIRST in both vectors (fail-closed; §3.4):
        // exactly one MORE limiter than the library default, in front.
        assert_eq!(
            config.reservation_rate_limiters.len(),
            relay::Config::default().reservation_rate_limiters.len() + 1
        );
        assert_eq!(
            config.circuit_src_rate_limiters.len(),
            relay::Config::default().circuit_src_rate_limiters.len() + 1
        );
    }

    /// The gate limiters actually GATE: with an empty admission table every
    /// peer is refused by the FIRST limiter in both vectors (probed through
    /// the config objects the relay would run).
    #[test]
    fn gate_limiters_refuse_unadmitted_peers() {
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let mut config = relay_config(None, None, &table);
        let peer = libp2p::PeerId::random();
        let addr: libp2p::Multiaddr = "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap();
        let now = std::time::Instant::now();
        // The gate is FIRST in both vectors and refuses the unadmitted
        // peer; the stock token buckets behind it individually would say
        // yes (tokens free) — the ORDER is the fail-closed property
        // (upstream `.all()` short-circuits on the gate's false).
        assert!(
            !config.reservation_rate_limiters[0].try_next(peer, &addr, now),
            "the gate is the FIRST reservation limiter"
        );
        assert!(
            !config.circuit_src_rate_limiters[0].try_next(peer, &addr, now),
            "the gate is the FIRST circuit-src limiter"
        );
        // Admit the peer: the gate passes; the whole vector passes.
        table.admit(
            peer,
            OffsetDateTime::now_utc() + time::Duration::seconds(120),
        );
        assert!(config.reservation_rate_limiters[0].try_next(peer, &addr, now));
        assert!(config.circuit_src_rate_limiters[0].try_next(peer, &addr, now));
    }

    /// Arming a flag overrides the frozen defaults EXACTLY; the frozen
    /// reservation duration and raised counts are untouched by them.
    #[test]
    fn armed_cap_flags_override_the_frozen_defaults_exactly() {
        let args = parse_args(
            [
                "--max-circuit-duration-secs".to_string(),
                "7200".to_string(),
                "--max-circuit-bytes".to_string(),
                "5368709120".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        assert_eq!(config.max_circuit_duration, Duration::from_secs(7200));
        assert_eq!(config.max_circuit_bytes, 5_368_709_120);
        assert_eq!(config.reservation_duration, Duration::from_secs(600));
        assert_eq!(config.max_reservations, 4096);
        assert_eq!(config.max_circuits, 4096);
        assert_eq!(config.max_circuits_per_peer, 128);
        assert_eq!(config.max_reservations_per_peer, 8);
        // One flag alone leaves the other at its frozen default.
        let only_duration =
            parse_args(["--max-circuit-duration-secs".to_string(), "60".to_string()].into_iter())
                .unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            only_duration.max_circuit_duration_secs,
            only_duration.max_circuit_bytes,
            &table,
        );
        assert_eq!(config.max_circuit_duration, Duration::from_secs(60));
        assert_eq!(config.max_circuit_bytes, 16 * 1024 * 1024);
    }

    /// The `--limit-summary` ops line reports the EFFECTIVE posture
    /// honestly: the frozen finite numbers by default, the operator's
    /// numbers when overridden, and the ADR-007 warning only when an
    /// override is armed.
    #[test]
    fn limit_summary_reports_the_effective_posture() {
        // Frozen defaults (the live deployment's restart posture).
        let args = parse_args(std::iter::empty()).unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        let summary = limit_summary(&config);
        assert!(summary.starts_with("--limit-summary"), "{summary}");
        for token in [
            "reservations=4096",
            "reservations_per_peer=8",
            "circuits=4096",
            "circuits_per_peer=128",
            "max_circuit_duration=1800s",
            "max_circuit_bytes=16777216",
            "reservation_duration=600s",
            "admission=ADR-038-ticket",
        ] {
            assert!(summary.contains(token), "{summary} must contain {token}");
        }
        assert!(
            caps_warning(&args).is_none(),
            "the frozen default posture carries no override warning"
        );

        // Armed: the operator numbers appear, warning fires naming the
        // frozen defaults it moved.
        let args = parse_args(
            [
                "--max-circuit-duration-secs".to_string(),
                "120".to_string(),
                "--max-circuit-bytes".to_string(),
                "2048".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        let summary = limit_summary(&config);
        assert!(summary.contains("max_circuit_duration=120s"), "{summary}");
        assert!(summary.contains("max_circuit_bytes=2048"), "{summary}");
        let warning = caps_warning(&args).expect("an armed override must warn");
        assert!(warning.contains("CUT"), "{warning}");
        assert!(warning.contains("ADR-007"), "{warning}");
        assert!(
            warning.contains("1800s"),
            "{warning} names the frozen default"
        );

        // One cap overridden: the other stays frozen, warning still fires.
        let args = parse_args(["--max-circuit-bytes".to_string(), "4096".to_string()].into_iter())
            .unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        let summary = limit_summary(&config);
        assert!(summary.contains("max_circuit_bytes=4096"), "{summary}");
        assert!(summary.contains("max_circuit_duration=1800s"), "{summary}");
        assert!(caps_warning(&args).is_some());
    }

    /// The stream-carrying contract, post-ADR-038: caps are FINITE but two
    /// orders above every legitimate maximum (64 max-size frames per
    /// direction; ≥ 15 maximum-length request deadlines per circuit).
    #[test]
    fn limits_carry_streams_and_scale_reservations() {
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(None, None, &table);
        assert!(
            config.max_reservations >= 1024,
            "reservations must be >= 1024"
        );
        assert!(config.max_circuits >= 1024, "circuits must be >= 1024");
        assert!(config.max_circuit_bytes >= 16 * 1024 * 1024, "16 MiB cap");
        assert!(
            config.max_circuit_duration >= Duration::from_secs(15 * 120),
            "the duration cap covers >= 15 maximum-length request deadlines"
        );
        assert!(
            config.max_circuit_duration <= Duration::from_secs(u32::MAX as u64),
            "must stay within the wire format's u32-second field"
        );
    }

    // ------------------------------------------------------------------
    // In-process gated-relay infrastructure (fixture key + virtual clock)
    // ------------------------------------------------------------------

    /// The recorded fixture signing key (the vectors' 0x38 seed — never the
    /// production hub key; §6.2 fixture discipline).
    fn fixture_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[0x38u8; 32])
    }

    fn rfc3339(t: OffsetDateTime) -> String {
        t.format(&time::format_description::well_known::Rfc3339)
            .expect("rfc3339")
    }

    fn parse_rfc3339(s: &str) -> OffsetDateTime {
        OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).unwrap()
    }

    /// A fixture ticket minted for `keypair`'s PeerId (real ADR-020
    /// derivations — the in-process proofs exercise checks 7-8 against
    /// real keys), valid from `now` under the frozen 600 s TTL.
    fn mint_ticket_for(keypair: &libp2p::identity::Keypair, now: OffsetDateTime) -> String {
        let peer = libp2p::PeerId::from(keypair.public());
        let installation = admission::derived_installation_id(&peer).expect("ed25519 peer derives");
        let ticket = RelayAdmissionTicket {
            installation_id: installation,
            peer_id: peer.to_string(),
            issued_at: rfc3339(now - time::Duration::seconds(60)),
            expires_at: rfc3339(now + time::Duration::seconds(540)),
            nonce: "0123456789abcdef0123456789abcdef".to_string(),
        };
        sign_ticket(&ticket, &fixture_signing_key())
    }

    /// The raw-QUIC pre-flight presenter (the F2B-era client discipline:
    /// dial the relay leg, offer `/msp/relay-ticket/1` over
    /// multistream-select 1.0.0, send ONE PRESENT frame, read ONE STATUS
    /// frame). Test-only mirror of `modelswarm-transport`'s client half.
    async fn present_ticket(
        keypair: &libp2p::identity::Keypair,
        relay_addr: &libp2p::Multiaddr,
        ticket: &str,
    ) -> Result<i32, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut transport = quic::tokio::Transport::new(quic::Config::new(keypair));
        let dial = transport
            .dial(
                relay_addr.clone(),
                DialOpts {
                    role: libp2p::core::Endpoint::Dialer,
                    port_use: PortUse::New,
                },
            )
            .map_err(|e| format!("dial: {e}"))?;
        let (_peer_id, mut connection) = tokio::time::timeout_at(deadline, dial)
            .await
            .map_err(|e| format!("dial: {e:?}"))?
            .map_err(|e| format!("dial: {e}"))?;
        let stream = {
            let opened =
                std::future::poll_fn(|cx| std::pin::Pin::new(&mut connection).poll_outbound(cx));
            tokio::time::timeout_at(deadline, opened)
                .await
                .map_err(|e| format!("stream open: {e:?}"))?
                .map_err(|e| format!("stream open: {e}"))?
        };

        let mut io = admission::ExchangeBuf::new(stream);
        // mss offer: header + protocol, then read both echoes.
        let protocol = format!("{}\n", admission::ADMISSION_PROTOCOL);
        let mss_frame = |payload: &[u8]| -> Vec<u8> {
            let mut frame = Vec::new();
            admission::put_uvarint(&mut frame, payload.len() as u64);
            frame.extend_from_slice(payload);
            frame
        };
        let mut offer = Vec::new();
        offer.extend_from_slice(&mss_frame(b"/multistream/1.0.0\n"));
        offer.extend_from_slice(&mss_frame(protocol.as_bytes()));
        io.write_frame(&offer, deadline)
            .await
            .map_err(|e| format!("mss offer write: {e:?}"))?;
        let _header = io
            .read_frame(deadline)
            .await
            .map_err(|e| format!("mss header: {e:?}"))?;
        let echo = io
            .read_frame(deadline)
            .await
            .map_err(|e| format!("mss echo: {e:?}"))?;
        if echo != protocol.as_bytes() {
            return Err(format!("mss refused: {}", String::from_utf8_lossy(&echo)));
        }
        // ONE PRESENT frame, ONE STATUS frame (§3.2).
        let body = admission::encode_present(ticket);
        io.write_frame(&admission::frame(&body), deadline)
            .await
            .map_err(|e| format!("present write: {e:?}"))?;
        let status_body = io
            .read_frame(deadline)
            .await
            .map_err(|e| format!("status read: {e:?}"))?;
        admission::decode_status(&status_body).map_err(|e| format!("status decode: {e:?}"))
    }

    /// A standard relay-client swarm (relay client + ping).
    fn std_client_swarm() -> (libp2p::Swarm<StdClientBehaviour>, libp2p::identity::Keypair) {
        std_client_swarm_with_ping(Duration::from_secs(30))
    }

    fn std_client_swarm_with_ping(
        interval: Duration,
    ) -> (libp2p::Swarm<StdClientBehaviour>, libp2p::identity::Keypair) {
        use libp2p::{noise, yamux};
        let key = libp2p::identity::Keypair::generate_ed25519();
        let swarm = libp2p::SwarmBuilder::with_existing_identity(key.clone())
            .with_tokio()
            .with_quic()
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .unwrap()
            .with_behaviour(|_key, relay_client| StdClientBehaviour {
                relay_client,
                ping: ping::Behaviour::new(ping::Config::new().with_interval(interval)),
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();
        (swarm, key)
    }

    /// A standard relay-client swarm with a FIXED keypair (same key ⇒ same
    /// PeerId ⇒ one admission covers the presenter connection AND the
    /// swarm's — the table is keyed by PeerId, §3.4 check 10).
    fn std_client_swarm_with_key(
        keypair: libp2p::identity::Keypair,
    ) -> libp2p::Swarm<StdClientBehaviour> {
        use libp2p::{noise, yamux};
        libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .unwrap()
            .with_behaviour(|_key, relay_client| StdClientBehaviour {
                relay_client,
                ping: ping::Behaviour::new(
                    ping::Config::new().with_interval(Duration::from_secs(1)),
                ),
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build()
    }

    #[derive(NetworkBehaviour)]
    struct StdClientBehaviour {
        relay_client: libp2p::relay::client::Behaviour,
        ping: ping::Behaviour,
    }

    /// A settable clock shared with the relay's admission verifier.
    fn settable_clock(
        now: OffsetDateTime,
    ) -> (admission::Clock, Arc<std::sync::Mutex<OffsetDateTime>>) {
        let shared = Arc::new(std::sync::Mutex::new(now));
        let clock: admission::Clock = {
            let shared = Arc::clone(&shared);
            Arc::new(move || *shared.lock().unwrap())
        };
        (clock, shared)
    }

    /// Starts an in-process relay (the binary's exact behaviour stack —
    /// relay + admission + identify + ping) with the GIVEN config and
    /// admission verifier, on an ephemeral loopback QUIC port. Returns its
    /// `/p2p/`-suffixed address and a report channel carrying one honest
    /// line per relay/admission lifecycle event (the binary's diagnostics
    /// vocabulary, admission lines included).
    async fn start_gated_relay(
        config: relay::Config,
        verifier: Arc<admission::AdmissionVerifier>,
    ) -> (
        libp2p::Multiaddr,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        use libp2p::swarm::SwarmEvent;
        let relay_key = libp2p::identity::Keypair::generate_ed25519();
        let relay_peer_id = libp2p::PeerId::from(relay_key.public());
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(relay_key)
            .with_tokio()
            .with_quic()
            .with_behaviour(|key| Behaviour {
                relay: relay::Behaviour::new(key.public().to_peer_id(), config),
                admission: admission::Behaviour::new(verifier),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/msp-relay-test/1".to_string(),
                    key.public(),
                )),
                ping: ping::Behaviour::new(
                    ping::Config::new().with_interval(Duration::from_secs(30)),
                ),
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();
        // Fresh relays are HOP-disabled until identify reports an external
        // address; force it on like the binary does.
        swarm
            .behaviour_mut()
            .relay
            .set_status(Some(relay::Status::Enable));
        swarm
            .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
            .unwrap();
        let bound = loop {
            match swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => break address,
                _ => continue,
            }
        };
        // Clients reject reservations whose response carries no relay
        // addresses; advertise the bound addr like the binary does.
        swarm.add_external_address(bound.clone());

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(event) = swarm.next().await {
                let line = match event {
                    SwarmEvent::Behaviour(BehaviourEvent::Admission(event)) => {
                        Some(event.log_line())
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay::Event::ReservationReqAccepted {
                            src_peer_id,
                            renewed,
                        },
                    )) => Some(format!(
                        "reservation {} by {src_peer_id}",
                        if renewed { "renewed" } else { "accepted" }
                    )),
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay::Event::ReservationReqDenied {
                            src_peer_id,
                            status,
                        },
                    )) => Some(format!("reservation denied for {src_peer_id}: {status:?}")),
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay::Event::CircuitReqAccepted {
                            src_peer_id,
                            dst_peer_id,
                        },
                    )) => Some(format!("circuit {src_peer_id} -> {dst_peer_id}")),
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay::Event::CircuitReqDenied {
                            src_peer_id,
                            dst_peer_id,
                            status,
                        },
                    )) => Some(format!(
                        "circuit denied {src_peer_id} -> {dst_peer_id}: {status:?}"
                    )),
                    _ => None,
                };
                if let Some(line) = line {
                    let _ = tx.send(line);
                }
            }
        });
        (
            bound.with(libp2p::multiaddr::Protocol::P2p(relay_peer_id)),
            rx,
        )
    }

    /// A gated relay under the wall clock: ONE verifier (fixture key) feeds
    /// BOTH the pre-flight and the gate limiters — the config is built from
    /// the verifier's own table so the two can never diverge (the exact
    /// wiring main() runs).
    async fn wall_clock_gated_relay(
        max_circuit_duration_secs: Option<u64>,
        max_circuit_bytes: Option<u64>,
    ) -> (
        libp2p::Multiaddr,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let verifier = Arc::new(admission::AdmissionVerifier {
            key: fixture_signing_key().verifying_key(),
            table: admission::AdmissionTable::new(admission::wall_clock()),
        });
        let config = relay_config(
            max_circuit_duration_secs,
            max_circuit_bytes,
            &verifier.table,
        );
        start_gated_relay(config, verifier).await
    }

    /// Admits a client (mint + present + assert ok(100)). `now` is the
    /// RELAY's admission clock reading (wall now for wall-clock relays, the
    /// virtual t0 for the expiry proof).
    async fn admit_client(
        keypair: &libp2p::identity::Keypair,
        relay_addr: &libp2p::Multiaddr,
        now: OffsetDateTime,
    ) {
        let ticket = mint_ticket_for(keypair, now);
        let code = present_ticket(keypair, relay_addr, &ticket)
            .await
            .expect("pre-flight exchange completes");
        assert_eq!(code, 100, "fixture-key ticket admits the real peer");
    }

    /// Drives one std client's swarm forever.
    fn drive(swarm: libp2p::Swarm<StdClientBehaviour>) {
        tokio::spawn(async move {
            let mut swarm = swarm;
            loop {
                swarm.select_next_some().await;
            }
        });
    }

    // ------------------------------------------------------------------
    // §6.2 test 1: the fail-closed proofs against the REAL relay stack
    // ------------------------------------------------------------------

    /// T1, the D12 close: a standard libp2p client that NEVER runs the
    /// pre-flight is refused — reservation AND circuit denied
    /// `ResourceLimitExceeded`; presenting a fixture-key ticket then admits
    /// the same peer (reserve → circuit → bidirectional ping, the F2B proof
    /// extended); admission expiry on the virtual clock (past
    /// `expires_at + 120 s`) re-closes BOTH gates.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gate_fails_closed_admits_on_ticket_and_recloses_on_expiry() {
        let _ = env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .parse_env("RUST_LOG")
            .try_init();
        use libp2p::multiaddr::Protocol;
        use libp2p::swarm::SwarmEvent;

        // A virtual clock so expiry needs no sleeping.
        let t0 = parse_rfc3339("2026-10-15T12:00:00Z");
        let (clock, now) = settable_clock(t0);
        let verifier = Arc::new(admission::AdmissionVerifier {
            key: fixture_signing_key().verifying_key(),
            table: admission::AdmissionTable::new(clock),
        });
        let table = Arc::clone(&verifier.table);
        let config = relay_config(None, None, &table);
        let (relay_addr, mut reports) = start_gated_relay(config, verifier).await;

        // --- Phase 1: ticketless refusal (reservation AND circuit). ---
        let server_key = libp2p::identity::Keypair::generate_ed25519();
        let server_peer = libp2p::PeerId::from(server_key.public());
        let mut server = std_client_swarm_with_key(server_key.clone());
        server
            .listen_on(relay_addr.clone().with(Protocol::P2pCircuit))
            .unwrap();
        // One merged proof stream: relay lifecycle lines + ping results.
        let (proof_tx, mut proof_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let relay_fwd = proof_tx.clone();
        tokio::spawn(async move {
            while let Some(line) = reports.recv().await {
                let _ = relay_fwd.send(line);
            }
        });
        let server_tx = proof_tx.clone();
        tokio::spawn(async move {
            let mut swarm = server;
            loop {
                match swarm.select_next_some().await {
                    SwarmEvent::Behaviour(StdClientBehaviourEvent::Ping(ping::Event {
                        result: Ok(_),
                        ..
                    })) => {
                        let _ = server_tx.send("server ping ok".to_string());
                    }
                    SwarmEvent::ConnectionEstablished { endpoint, .. } if endpoint.is_relayed() => {
                        let _ = server_tx.send("server circuit in".to_string());
                    }
                    _ => {}
                }
            }
        });

        // The circuit dialer (also ticketless in phase 1).
        let dialer_key = libp2p::identity::Keypair::generate_ed25519();
        let dialer = std_client_swarm_with_key(dialer_key.clone());
        let dial_addr = relay_addr
            .clone()
            .with(Protocol::P2pCircuit)
            .with(Protocol::P2p(server_peer));
        let dialer_tx = proof_tx.clone();
        let phase1_dial_addr = dial_addr.clone();
        tokio::spawn(async move {
            let mut swarm = dialer;
            let dial_at = tokio::time::Instant::now() + Duration::from_secs(1);
            let mut dialed = false;
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => {
                        if let SwarmEvent::Behaviour(StdClientBehaviourEvent::Ping(
                            ping::Event { result: Ok(_), .. },
                        )) = event
                        {
                            let _ = dialer_tx.send("dialer ping ok".to_string());
                        }
                    }
                    _ = tokio::time::sleep_until(dial_at), if !dialed => {
                        dialed = true;
                        let _ = swarm.dial(phase1_dial_addr.clone());
                    }
                }
            }
        });

        // Phase 1 proof: BOTH gates refuse with ResourceLimitExceeded —
        // the reservation denial from the server's listen, the circuit
        // denial from the dialer's dial. The circuit denial ALSO pins that
        // the gate limiter runs before the NoReservation lookup.
        let mut reservation_denied = false;
        let mut circuit_denied = false;
        let phase1 = tokio::time::timeout(Duration::from_secs(30), async {
            while !(reservation_denied && circuit_denied) {
                let line = proof_rx
                    .recv()
                    .await
                    .expect("relay driver exited before phase 1 completed");
                if line.starts_with("reservation denied") && line.contains("ResourceLimitExceeded")
                {
                    reservation_denied = true;
                }
                if line.starts_with("circuit denied") && line.contains("ResourceLimitExceeded") {
                    circuit_denied = true;
                }
            }
        })
        .await;
        assert!(
            phase1.is_ok(),
            "ticketless clients must be refused at BOTH grant sites with ResourceLimitExceeded"
        );
        assert!(reservation_denied && circuit_denied);

        // --- Phase 2: present fixture tickets; the SAME peers (same keys,
        // = same PeerIds) are admitted. libp2p's relay client CLOSES its
        // listener after a denied reservation (no retry), so the admitted
        // phase runs on FRESH swarms with the SAME keys — the admission is
        // keyed to the PeerId, not the connection (§3.4 check 10).
        admit_client(&server_key, &relay_addr, t0).await;
        admit_client(&dialer_key, &relay_addr, t0).await;

        let admitted_server = std_client_swarm_with_key(server_key.clone());
        let admitted_relay_addr = relay_addr.clone();
        let server_tx = proof_tx.clone();
        tokio::spawn(async move {
            let mut swarm = admitted_server;
            swarm
                .listen_on(admitted_relay_addr.with(Protocol::P2pCircuit))
                .unwrap();
            loop {
                match swarm.select_next_some().await {
                    SwarmEvent::Behaviour(StdClientBehaviourEvent::Ping(ping::Event {
                        result: Ok(_),
                        ..
                    })) => {
                        let _ = server_tx.send("server ping ok".to_string());
                    }
                    SwarmEvent::ConnectionEstablished { endpoint, .. } if endpoint.is_relayed() => {
                        let _ = server_tx.send("server circuit in".to_string());
                    }
                    _ => {}
                }
            }
        });

        let redial = std_client_swarm_with_key(dialer_key.clone());
        let redial_tx = proof_tx.clone();
        let redial_addr = dial_addr.clone();
        tokio::spawn(async move {
            let mut swarm = redial;
            let dial_at = tokio::time::Instant::now() + Duration::from_secs(2);
            let mut dialed = false;
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => {
                        if let SwarmEvent::Behaviour(StdClientBehaviourEvent::Ping(
                            ping::Event { result: Ok(_), .. },
                        )) = event
                        {
                            let _ = redial_tx.send("dialer ping ok".to_string());
                        }
                    }
                    _ = tokio::time::sleep_until(dial_at), if !dialed => {
                        dialed = true;
                        let _ = swarm.dial(redial_addr.clone());
                    }
                }
            }
        });

        // Phase 2 proof: reservation accepted + circuit + BOTH-way pings.
        let mut reserved = false;
        let mut circuit = false;
        let mut server_pinged = false;
        let mut dialer_pinged = false;
        let phase2 = tokio::time::timeout(Duration::from_secs(60), async {
            while !(reserved && circuit && server_pinged && dialer_pinged) {
                let line = proof_rx
                    .recv()
                    .await
                    .expect("drivers exited before phase 2 completed");
                match line.as_str() {
                    l if l.starts_with("reservation accepted") => reserved = true,
                    l if l.starts_with("circuit ") && l.contains(" -> ") => circuit = true,
                    "server ping ok" => server_pinged = true,
                    "dialer ping ok" => dialer_pinged = true,
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            phase2.is_ok(),
            "admitted peers must reserve, circuit, and ping both ways through the relay"
        );

        // --- Phase 3: virtual clock past expires_at + 120 s re-closes BOTH gates. ---
        // Ticket issued at t0-60, expires t0+540, deadline t0+660.
        *now.lock().unwrap() = t0 + time::Duration::seconds(700);
        assert!(!table.is_live(&server_peer), "server admission expired");

        // Fresh connections from the SAME PeerIds (same keys) are refused.
        let mut expired_server = std_client_swarm_with_key(server_key.clone());
        expired_server
            .listen_on(relay_addr.clone().with(Protocol::P2pCircuit))
            .unwrap();
        drive(expired_server);
        let expired_dialer = std_client_swarm_with_key(dialer_key.clone());
        let expired_dial_addr = dial_addr.clone();
        tokio::spawn(async move {
            let mut swarm = expired_dialer;
            let dial_at = tokio::time::Instant::now() + Duration::from_secs(1);
            let mut dialed = false;
            loop {
                tokio::select! {
                    _ = swarm.select_next_some() => {}
                    _ = tokio::time::sleep_until(dial_at), if !dialed => {
                        dialed = true;
                        let _ = swarm.dial(expired_dial_addr.clone());
                    }
                }
            }
        });

        let mut reservation_denied_again = false;
        let mut circuit_denied_again = false;
        let phase3 = tokio::time::timeout(Duration::from_secs(30), async {
            while !(reservation_denied_again && circuit_denied_again) {
                let line = proof_rx
                    .recv()
                    .await
                    .expect("drivers exited before phase 3 completed");
                if line.starts_with("reservation denied") && line.contains("ResourceLimitExceeded")
                {
                    reservation_denied_again = true;
                }
                if line.starts_with("circuit denied") && line.contains("ResourceLimitExceeded") {
                    circuit_denied_again = true;
                }
            }
        })
        .await;
        assert!(
            phase3.is_ok(),
            "past expires_at + 120 s both gates must re-close (ResourceLimitExceeded)"
        );
    }

    // ------------------------------------------------------------------
    // Flag-path cap proofs (the machinery the frozen numbers ride), now
    // behind the admission gate.
    // ------------------------------------------------------------------

    /// Drives the dialer side of a cap-cut proof (the F2B shape): waits
    /// `grace` for the server's reservation to settle, dials through the
    /// relay, then reports the RELAYED connection's lifecycle.
    async fn drive_circuit_dialer(
        mut swarm: libp2p::Swarm<StdClientBehaviour>,
        dial_addr: libp2p::Multiaddr,
        grace: Duration,
        report: tokio::sync::mpsc::UnboundedSender<String>,
    ) {
        use libp2p::swarm::SwarmEvent;
        let dial_at = tokio::time::Instant::now() + grace;
        let mut dialed = false;
        let mut relayed_connection: Option<libp2p::swarm::ConnectionId> = None;
        loop {
            tokio::select! {
                event = swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(StdClientBehaviourEvent::Ping(ping::Event {
                        connection,
                        result: Ok(_),
                        ..
                    })) if Some(connection) == relayed_connection => {
                        let _ = report.send("dialer ping ok".to_string());
                    },
                    SwarmEvent::ConnectionEstablished {
                        connection_id, endpoint, ..
                    } if endpoint.is_relayed() => {
                        relayed_connection = Some(connection_id);
                        let _ = report.send("dialer circuit established".to_string());
                    },
                    SwarmEvent::ConnectionClosed { endpoint, .. } if endpoint.is_relayed() => {
                        let _ = report.send("dialer circuit closed".to_string());
                        return;
                    },
                    _ => {}
                },
                _ = tokio::time::sleep_until(dial_at), if !dialed => {
                    dialed = true;
                    if let Err(e) = swarm.dial(dial_addr.clone()) {
                        let _ = report.send(format!("dialer dial error {e:?}"));
                    }
                }
            }
        }
    }

    /// Collects the cap-cut proof from the merged report stream.
    async fn await_relayed_circuit_cut(
        reports: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
        min_lifetime: Duration,
        max_lifetime: Duration,
    ) -> Result<(), String> {
        let mut established_at: Option<tokio::time::Instant> = None;
        let mut relayed_pings = 0usize;
        let mut relay_saw_circuit = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            let line = match tokio::time::timeout_at(deadline, reports.recv()).await {
                Ok(Some(line)) => line,
                Ok(None) => return Err("drivers exited before the proof completed".into()),
                Err(_) => return Err("timed out before the circuit was cut".into()),
            };
            match line.as_str() {
                "dialer circuit established" => {
                    established_at = Some(tokio::time::Instant::now());
                }
                "dialer ping ok" => relayed_pings += 1,
                "dialer circuit closed" => {
                    let at = established_at.ok_or("closed before established")?;
                    if !relay_saw_circuit {
                        return Err("the relay never reported the circuit".into());
                    }
                    if relayed_pings == 0 {
                        return Err("the circuit was cut before carrying a single ping".into());
                    }
                    let lifetime = at.elapsed();
                    if lifetime < min_lifetime {
                        return Err(format!(
                            "connection died at {lifetime:?}, too early to be the armed cap"
                        ));
                    }
                    if lifetime > max_lifetime {
                        return Err(format!("connection lived {lifetime:?}, past the armed cap"));
                    }
                    return Ok(());
                }
                l if l.starts_with("circuit ") && l.contains(" -> ") => {
                    relay_saw_circuit = true;
                }
                l if l.starts_with("dialer dial error") => {
                    return Err(l.to_string());
                }
                _ => {}
            }
        }
    }

    /// `--max-circuit-duration-secs 2` from a command line arms a cap a
    /// REAL gated relay enforces: the relayed circuit establishes, carries
    /// pings both ways, and is CUT at the ~2s deadline.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flag_armed_duration_cap_cuts_the_circuit() {
        // THE FLAG PATH: the enforced cap comes from parsed argv.
        let args =
            parse_args(["--max-circuit-duration-secs".to_string(), "2".to_string()].into_iter())
                .unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        assert_eq!(
            config.max_circuit_duration,
            Duration::from_secs(2),
            "the parsed flag value must be the enforced value"
        );
        drop(config);
        let (relay_addr, mut relay_reports) = wall_clock_gated_relay(Some(2), None).await;

        // Server reserves and listens on its circuit address (admitted).
        let (server, server_key) = std_client_swarm();
        let server_peer = libp2p::PeerId::from(server_key.public());
        admit_client(&server_key, &relay_addr, OffsetDateTime::now_utc()).await;
        let mut server = server;
        server
            .listen_on(
                relay_addr
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2pCircuit),
            )
            .unwrap();
        drive(server);

        // Dialer: fast pings so the circuit demonstrably carries frames
        // before the deadline.
        let (dialer, dialer_key) = std_client_swarm_with_ping(Duration::from_millis(200));
        admit_client(&dialer_key, &relay_addr, OffsetDateTime::now_utc()).await;
        let dial_addr = relay_addr
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(server_peer));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let relay_fwd = tx.clone();
        tokio::spawn(async move {
            while let Some(line) = relay_reports.recv().await {
                let _ = relay_fwd.send(line);
            }
        });
        tokio::spawn(drive_circuit_dialer(
            dialer,
            dial_addr,
            Duration::from_secs(2),
            tx,
        ));

        let proof =
            await_relayed_circuit_cut(&mut rx, Duration::from_secs(1), Duration::from_secs(10))
                .await;
        assert!(
            proof.is_ok(),
            "a 2s flag-armed duration cap must cut the live circuit: {:?}",
            proof.unwrap_err()
        );
    }

    /// `--max-circuit-bytes 4096` from a command line arms a byte cap a
    /// REAL gated relay enforces: the circuit establishes, carries pings,
    /// and is CUT once the forwarded-byte counter crosses the cap.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flag_armed_byte_cap_cuts_the_circuit() {
        let args = parse_args(["--max-circuit-bytes".to_string(), "4096".to_string()].into_iter())
            .unwrap();
        let table = admission::AdmissionTable::new(admission::wall_clock());
        let config = relay_config(
            args.max_circuit_duration_secs,
            args.max_circuit_bytes,
            &table,
        );
        assert_eq!(
            config.max_circuit_bytes, 4096,
            "the parsed flag value must be the enforced value"
        );
        drop(config);
        let (relay_addr, mut relay_reports) = wall_clock_gated_relay(None, Some(4096)).await;

        let (server, server_key) = std_client_swarm();
        let server_peer = libp2p::PeerId::from(server_key.public());
        admit_client(&server_key, &relay_addr, OffsetDateTime::now_utc()).await;
        let mut server = server;
        server
            .listen_on(
                relay_addr
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2pCircuit),
            )
            .unwrap();
        drive(server);

        let (dialer, dialer_key) = std_client_swarm_with_ping(Duration::from_millis(150));
        admit_client(&dialer_key, &relay_addr, OffsetDateTime::now_utc()).await;
        let dial_addr = relay_addr
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(server_peer));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let relay_fwd = tx.clone();
        tokio::spawn(async move {
            while let Some(line) = relay_reports.recv().await {
                let _ = relay_fwd.send(line);
            }
        });
        tokio::spawn(drive_circuit_dialer(
            dialer,
            dial_addr,
            Duration::from_secs(2),
            tx,
        ));

        let proof =
            await_relayed_circuit_cut(&mut rx, Duration::from_millis(500), Duration::from_secs(20))
                .await;
        assert!(
            proof.is_ok(),
            "a 4096-byte flag-armed cap must cut the live circuit: {:?}",
            proof.unwrap_err()
        );
    }

    /// A circuit to a destination with NO reservation is refused with
    /// `NoReservation` — from an ADMITTED source (the honest
    /// unreserved-destination vocabulary, distinct from the gate's
    /// ResourceLimitExceeded).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admitted_circuit_to_unreserved_destination_is_denied_no_reservation() {
        let (relay_addr, mut reports) = wall_clock_gated_relay(None, None).await;

        let (mut dialer, dialer_key) = std_client_swarm();
        admit_client(&dialer_key, &relay_addr, OffsetDateTime::now_utc()).await;
        let stranger = libp2p::PeerId::random();
        let dial_addr = relay_addr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(stranger));
        dialer.dial(dial_addr).unwrap();
        drive(dialer);

        let mut saw_denied = false;
        let overall = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let line = reports
                    .recv()
                    .await
                    .expect("relay driver exited before the denial");
                if line.starts_with("circuit denied") && line.contains("NoReservation") {
                    saw_denied = true;
                    break;
                }
            }
        })
        .await;
        assert!(
            overall.is_ok() && saw_denied,
            "the relay must deny the circuit with NoReservation"
        );
    }
}
