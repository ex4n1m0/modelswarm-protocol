//! MSP circuit-relay v2 host (ADR-014, Phase F2).
//!
//! A standard rust-libp2p relay with the limits the phase-f research
//! mandates: reservations ≥1024 and, BY DEFAULT, **no per-circuit
//! duration/byte caps** — the stock defaults (2 min / 128 KiB) reset a
//! token stream mid-flight, which ADR-007 forbids (no retry after the
//! first output token). The library's anti-abuse creation rates are
//! deliberately kept.
//!
//! Admission bounds (2026-10-15, gate D12 runtime half): an operator may
//! arm per-circuit caps at runtime with `--max-circuit-duration-secs` /
//! `--max-circuit-bytes`; when a flag is absent the posture is EXACTLY the
//! phase-f uncapped one (the live deployment changes nothing until the
//! operator opts in). The startup `--limit-summary` line prints the
//! effective posture honestly. A cap is a real cut: the relay terminates
//! the circuit at the limit, and a cut mid-token-stream is unrecoverable
//! for the requester — which is precisely why uncapped is the default.
//! The PROTOCOL half of D12 (admission tickets, ADR-038) is not here; this
//! binary moves bytes and refuses nothing on identity grounds.
//!
//! The relay is content-blind by construction: it forwards opaque
//! substreams and never parses MSP frames, prompts, or completions.
//! Access control is NOT here — serving peers enforce the ADR-026 lease
//! gate; this host only moves bytes.
//!
//! Node-side support (reserve + dial through `/p2p-circuit` addresses over
//! the raw-QUIC session layer) is the F2b transport work
//! (`modelswarm-transport`); this binary is interoperable with any standard
//! libp2p client today.
//!
//! Bind discipline (F2B, 2026-10-15): the default bind is LOOPBACK
//! (`127.0.0.1:4001`) — the same ADR-018 posture the transport crate's
//! `ListenOptions::allow_off_loopback` enforces. Binding any non-loopback
//! address requires the explicit `--public` operator opt-in at the command
//! line; a silently-open relay is never the default. The refusal is tested
//! (`non_loopback_bind_requires_the_explicit_public_opt_in`).

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use libp2p::relay;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identify, ping};

/// Stream-carrying limits (the whole point of running our own relay).
/// `max_circuit_duration_secs` / `max_circuit_bytes` are the OPERATOR caps
/// (`--max-circuit-duration-secs` / `--max-circuit-bytes`); `None` keeps
/// the phase-f uncapped posture EXACTLY as it was before the flags existed
/// (identical `Config` fields, bit for bit — the absent-flag regression
/// test pins this).
fn relay_config(
    max_circuit_duration_secs: Option<u64>,
    max_circuit_bytes: Option<u64>,
) -> relay::Config {
    // Reservation/circuit CREATION rates keep the library's anti-abuse
    // defaults via struct-update (30 per 2 min per peer) — circuits are per
    // pooled session, not per request, so this does not bound throughput.
    relay::Config {
        max_reservations: 4096,
        max_reservations_per_peer: 8,
        max_circuits: 4096,
        max_circuits_per_peer: 128,
        // Default: a token stream must never be cut by the relay itself.
        // The wire format's duration field is u32 seconds — u64::MAX PANICS
        // the behaviour when it announces the limit (~136 years is the real
        // max), so "uncapped" is u32::MAX seconds, not Duration::MAX.
        max_circuit_duration: Duration::from_secs(
            max_circuit_duration_secs.unwrap_or(u32::MAX as u64),
        ),
        max_circuit_bytes: max_circuit_bytes.unwrap_or(u64::MAX),
        ..relay::Config::default()
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay: relay::Behaviour,
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
    println!("        [--max-circuit-duration-secs N] [--max-circuit-bytes N]");
    println!("  --listen  UDP/QUIC bind address (default 127.0.0.1:4001 — LOOPBACK).");
    println!("            A non-loopback IP additionally requires --public.");
    println!("  --key     persisted ed25519 key file (default msp-relay.key in cwd)");
    println!("            a STABLE key keeps the relay PeerId — and every");
    println!("            /p2p/<relay>/p2p-circuit address — valid across restarts.");
    println!("  --public  EXPLICIT operator opt-in for a non-loopback bind (the");
    println!("            ADR-018 allow_off_loopback discipline: the relay never");
    println!("            silently binds a public interface).");
    println!("  --max-circuit-duration-secs N   per-circuit lifetime cap in seconds.");
    println!("            DEFAULT: UNCAPPED — omitting the flag keeps today's");
    println!("            behavior EXACTLY (the live deployment is unchanged until");
    println!("            the operator arms a cap). HONEST WARNING: a cap is a real");
    println!("            CUT — the relay terminates the circuit at the limit, and a");
    println!("            cut mid-token-stream is unrecoverable for the requester");
    println!("            (ADR-007: no retry after the first output token). Size caps");
    println!("            generously or leave them off. Range 1..=4294967295");
    println!("            (u32::MAX — the circuit-relay-v2 wire format's duration");
    println!("            field); 0 is rejected, not read as \"uncapped\".");
    println!("  --max-circuit-bytes N            per-circuit relayed-byte cap.");
    println!("            DEFAULT: UNCAPPED; same CUT warning as above (size for the");
    println!("            FULL response a peer may stream, plus framing). Range");
    println!("            1..=2^64-1; 0 is rejected — the library treats 0 as \"no");
    println!("            limit\" internally, so accepting it here would silently");
    println!("            mean the opposite of what it says.");
    println!("  At startup the relay prints a --limit-summary line with the");
    println!("  EFFECTIVE posture (reservations, circuits, per-peer counts,");
    println!("  duration/byte caps or UNCAPPED) — the ops surface for the live");
    println!("  deployment's logs.");
}

/// Resolve the advertised LAN IP the way the node does — the UDP-connect
/// trick never sends a packet.
fn lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Parsed command line. `public` is the explicit non-loopback opt-in; the
/// cap fields are `Option` — `None` is the ADR-free default meaning
/// EXACTLY today's uncapped posture (never a silent reinterpretation).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    listen: String,
    key_path: PathBuf,
    public: bool,
    max_circuit_duration_secs: Option<u64>,
    max_circuit_bytes: Option<u64>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:4001".to_string(),
            key_path: PathBuf::from("msp-relay.key"),
            public: false,
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

/// The startup `--limit-summary` ops line: the EFFECTIVE posture the relay
/// is about to run with, as greppable key=value tokens. Duration is
/// reported UNCAPPED at the wire maximum (>= u32::MAX seconds — the value
/// the flag-less config itself uses for "no signaling-grade cap"), bytes
/// UNCAPPED at `u64::MAX`; any operator-armed cap prints as its number.
fn limit_summary(config: &relay::Config) -> String {
    let duration = if config.max_circuit_duration >= Duration::from_secs(u32::MAX as u64) {
        "UNCAPPED".to_string()
    } else {
        format!("{}s", config.max_circuit_duration.as_secs())
    };
    let bytes = if config.max_circuit_bytes == u64::MAX {
        "UNCAPPED".to_string()
    } else {
        config.max_circuit_bytes.to_string()
    };
    format!(
        "--limit-summary reservations={} reservations_per_peer={} circuits={} \
         circuits_per_peer={} max_circuit_duration={} max_circuit_bytes={}",
        config.max_reservations,
        config.max_reservations_per_peer,
        config.max_circuits,
        config.max_circuits_per_peer,
        duration,
        bytes
    )
}

/// The honest warning printed at startup when an operator has armed a cap:
/// the relay WILL cut circuits at the limit, and a cut mid-token-stream is
/// unrecoverable for the requester (ADR-007). `None` when uncapped — the
/// default posture needs no warning.
fn caps_warning(args: &Args) -> Option<String> {
    if args.max_circuit_duration_secs.is_none() && args.max_circuit_bytes.is_none() {
        return None;
    }
    Some(
        "--limit-summary WARNING: operator caps are ARMED — the relay CUTS circuits at \
         max_circuit_duration/max_circuit_bytes, and a cut mid-token-stream is \
         unrecoverable for the requester (ADR-007: no retry after the first output \
         token). Size caps for the full streamed response or leave them off."
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

    let config = relay_config(parsed.max_circuit_duration_secs, parsed.max_circuit_bytes);
    println!("modelswarm-relay: peer id {peer_id}");
    println!("modelswarm-relay: {}", limit_summary(&config));
    if let Some(warning) = caps_warning(&parsed) {
        println!("modelswarm-relay: {warning}");
    }
    println!("modelswarm-relay: content-blind — frames are forwarded, never parsed");

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(|key| Behaviour {
            relay: relay::Behaviour::new(key.public().to_peer_id(), config),
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
        // Same with the flag split form.
        let err = parse_args(["--listen".to_string(), "0.0.0.0:4001".to_string()].into_iter())
            .unwrap_err();
        assert!(err.contains("--public"));

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

    /// Malformed listen values and unknown flags are honest parse failures.
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
    }

    /// Admission-bounds flag parsing (gate D12 runtime half): absent flags
    /// are UNCAPPED (None — exactly today's posture), valid values land in
    /// `Args`, and every dishonest value is refused with a reason that
    /// names the fix (0 is not "uncapped"; the duration field is u32
    /// seconds on the wire).
    #[test]
    fn cap_flags_parse_validate_and_default_to_absent() {
        fn flags(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        }

        // ABSENT: both caps are None — the live deployment's default.
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
                "the 0 refusal must name the uncapped escape: {err}"
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

    /// The absent-flag regression: parsed-from-empty-argv args produce the
    /// config of TODAY'S uncapped relay, field for field — the live
    /// deployment restarts bit-identical until an operator arms a cap.
    #[test]
    fn absent_cap_flags_keep_todays_uncapped_config() {
        let args = parse_args(std::iter::empty()).unwrap();
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        assert_eq!(config.max_reservations, 4096);
        assert_eq!(config.max_reservations_per_peer, 8);
        assert_eq!(config.max_circuits, 4096);
        assert_eq!(config.max_circuits_per_peer, 128);
        assert_eq!(config.max_circuit_bytes, u64::MAX);
        assert_eq!(
            config.max_circuit_duration,
            Duration::from_secs(u32::MAX as u64),
            "uncapped stays at the wire-format maximum, exactly as before the flags"
        );
        // And the pre-flag literal config equals the flag-derived one.
        assert_eq!(
            config.max_circuit_duration,
            relay_config(None, None).max_circuit_duration
        );
        assert_eq!(
            config.max_circuit_bytes,
            relay_config(None, None).max_circuit_bytes
        );
    }

    /// Arming a flag reaches the relay behaviour's config EXACTLY: the cap
    /// values the operator typed are the values the relay will enforce,
    /// and the raised reservation/circuit counts are untouched by them.
    #[test]
    fn armed_cap_flags_reach_the_relay_config_exactly() {
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
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        assert_eq!(config.max_circuit_duration, Duration::from_secs(7200));
        assert_eq!(config.max_circuit_bytes, 5_368_709_120);
        assert_eq!(config.max_reservations, 4096);
        assert_eq!(config.max_circuits, 4096);
        assert_eq!(config.max_circuits_per_peer, 128);
        assert_eq!(config.max_reservations_per_peer, 8);
        // One flag alone leaves the other uncapped.
        let only_duration =
            parse_args(["--max-circuit-duration-secs".to_string(), "60".to_string()].into_iter())
                .unwrap();
        let config = relay_config(
            only_duration.max_circuit_duration_secs,
            only_duration.max_circuit_bytes,
        );
        assert_eq!(config.max_circuit_duration, Duration::from_secs(60));
        assert_eq!(config.max_circuit_bytes, u64::MAX);
    }

    /// The `--limit-summary` ops line reports the EFFECTIVE posture
    /// honestly: full UNCAPPED vocabulary for absent flags, the armed
    /// numbers when set, and the ADR-007 warning only when a cap exists.
    #[test]
    fn limit_summary_reports_the_effective_posture() {
        // Uncapped (the live deployment's restart posture).
        let args = parse_args(std::iter::empty()).unwrap();
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        let summary = limit_summary(&config);
        assert!(summary.starts_with("--limit-summary"), "{summary}");
        for token in [
            "reservations=4096",
            "reservations_per_peer=8",
            "circuits=4096",
            "circuits_per_peer=128",
            "max_circuit_duration=UNCAPPED",
            "max_circuit_bytes=UNCAPPED",
        ] {
            assert!(summary.contains(token), "{summary} must contain {token}");
        }
        assert!(
            caps_warning(&args).is_none(),
            "no warning in the default uncapped posture"
        );

        // Armed: the numbers appear, UNCAPPED disappears for the armed cap.
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
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        let summary = limit_summary(&config);
        assert!(summary.contains("max_circuit_duration=120s"), "{summary}");
        assert!(summary.contains("max_circuit_bytes=2048"), "{summary}");
        assert!(!summary.contains("UNCAPPED"), "{summary}");
        let warning = caps_warning(&args).expect("an armed cap must warn");
        assert!(warning.contains("CUT"), "{warning}");
        assert!(warning.contains("ADR-007"), "{warning}");

        // One cap armed: the other still reports UNCAPPED, warning present.
        let args = parse_args(["--max-circuit-bytes".to_string(), "4096".to_string()].into_iter())
            .unwrap();
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        let summary = limit_summary(&config);
        assert!(summary.contains("max_circuit_bytes=4096"), "{summary}");
        assert!(
            summary.contains("max_circuit_duration=UNCAPPED"),
            "{summary}"
        );
        assert!(caps_warning(&args).is_some());
    }

    /// The stream-carrying contract from the phase-f research: a stock relay
    /// resets token streams at 2 min / 128 KiB; ours must never.
    #[test]
    fn limits_carry_streams_and_scale_reservations() {
        let config = relay_config(None, None);
        assert!(
            config.max_reservations >= 1024,
            "reservations must be >= 1024"
        );
        assert!(config.max_circuits >= 1024, "circuits must be >= 1024");
        assert_eq!(config.max_circuit_bytes, u64::MAX, "no byte cap");
        assert!(
            config.max_circuit_duration >= Duration::from_secs(24 * 60 * 60),
            "no signaling-grade duration cap"
        );
        assert_eq!(
            config.max_circuit_duration,
            Duration::from_secs(u32::MAX as u64),
            "must stay within the wire format's u32-second field"
        );
    }

    /// End-to-end acceptance proof for the relay binary's behaviour stack:
    /// a standard libp2p client reserves through our raised-limit relay,
    /// a second client establishes a circuit to it, and ping frames flow
    /// BOTH ways through the relayed connection. This is the interop level
    /// the F2b node-side client must reach.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn standard_clients_reserve_and_circuit_through_the_relay() {
        let _ = env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .parse_env("RUST_LOG")
            .try_init();
        use libp2p::swarm::SwarmEvent;
        use libp2p::{noise, yamux};

        #[derive(NetworkBehaviour)]
        struct ClientBehaviour {
            relay_client: relay::client::Behaviour,
            ping: ping::Behaviour,
        }

        fn client_swarm() -> (libp2p::Swarm<ClientBehaviour>, libp2p::identity::Keypair) {
            let key = libp2p::identity::Keypair::generate_ed25519();
            let swarm = libp2p::SwarmBuilder::with_existing_identity(key.clone())
                .with_tokio()
                .with_quic()
                .with_relay_client(noise::Config::new, yamux::Config::default)
                .unwrap()
                .with_behaviour(|_key, relay_client| ClientBehaviour {
                    relay_client,
                    ping: ping::Behaviour::new(
                        ping::Config::new().with_interval(Duration::from_secs(1)),
                    ),
                })
                .unwrap()
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build();
            (swarm, key)
        }

        // --- relay (the binary's exact behaviour stack + limits) ---
        let relay_key = libp2p::identity::Keypair::generate_ed25519();
        let relay_peer_id = libp2p::PeerId::from(relay_key.public());
        let mut relay_swarm = libp2p::SwarmBuilder::with_existing_identity(relay_key)
            .with_tokio()
            .with_quic()
            .with_behaviour(|key| Behaviour {
                relay: relay::Behaviour::new(key.public().to_peer_id(), relay_config(None, None)),
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
        relay_swarm
            .behaviour_mut()
            .relay
            .set_status(Some(relay::Status::Enable));
        relay_swarm
            .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
            .unwrap();
        let relay_addr = loop {
            match relay_swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => break address,
                _ => continue,
            }
        };
        // Clients reject a reservation whose response carries no relay
        // addresses (NoAddressesInReservation) — advertise the bound addr
        // like the binary advertises its LAN addr.
        relay_swarm.add_external_address(relay_addr.clone());
        let relay_addr = relay_addr.with(libp2p::multiaddr::Protocol::P2p(relay_peer_id));

        let (report_tx, mut report_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let relay_tx = report_tx.clone();
        tokio::spawn(async move {
            let mut swarm = relay_swarm;
            let tx = relay_tx;
            while let Some(event) = swarm.next().await {
                if let SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::Event::CircuitReqAccepted {
                        src_peer_id,
                        dst_peer_id,
                    },
                )) = event
                {
                    let _ = tx.send(format!("relay circuit {src_peer_id} -> {dst_peer_id}"));
                }
            }
        });

        // --- server client (B): reserve, then receive the circuit ---
        let (server_swarm, server_key) = client_swarm();
        let server_peer_id = libp2p::PeerId::from(server_key.public());
        let circuit_listen = relay_addr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2pCircuit);
        let mut server_swarm = server_swarm;
        server_swarm.listen_on(circuit_listen).unwrap();
        let server_tx = report_tx.clone();
        tokio::spawn(async move {
            let mut swarm = server_swarm;
            while let Some(event) = swarm.next().await {
                match event {
                    SwarmEvent::Behaviour(ClientBehaviourEvent::RelayClient(
                        relay::client::Event::ReservationReqAccepted { .. },
                    )) => {
                        let _ = server_tx.send("server reserved".to_string());
                    }
                    SwarmEvent::Behaviour(ClientBehaviourEvent::Ping(ping::Event {
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

        // --- dialer client (A): circuit to the server, then ping it. The
        // reservation is one localhost round trip; a short grace period
        // before the single dial avoids a denied first attempt (a channel-
        // driven dial re-enters select! with a consumed receiver — hot
        // loop hazard not worth the coordination for a test). ---
        let (dialer_swarm, _) = client_swarm();
        let dial_addr = relay_addr
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(server_peer_id));
        let dialer_tx = report_tx.clone();
        tokio::spawn(async move {
            let mut swarm = dialer_swarm;
            let dial_at = tokio::time::Instant::now() + Duration::from_secs(2);
            let mut dialed = false;
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => match event {
                        SwarmEvent::Behaviour(ClientBehaviourEvent::Ping(ping::Event {
                            result: Ok(_),
                            ..
                        })) => {
                            let _ = dialer_tx.send("dialer ping ok".to_string());
                        }
                        SwarmEvent::ConnectionEstablished {
                            endpoint, ..
                        } if endpoint.is_relayed() => {
                            let _ = dialer_tx.send("dialer circuit out".to_string());
                        }
                        _ => {}
                    },
                    _ = tokio::time::sleep_until(dial_at), if !dialed => {
                        dialed = true;
                        match swarm.dial(dial_addr.clone()) {
                            Ok(()) => { let _ = dialer_tx.send("dialer dialed".to_string()); }
                            Err(e) => { let _ = dialer_tx.send(format!("dialer dial error {e:?}")); }
                        }
                    }
                }
            }
        });

        // --- collect the proof: circuit + both-way pings (the reservation
        // line is informational; the dial is timer-driven) ---
        let overall = tokio::time::timeout(Duration::from_secs(60), async {
            let mut server_pinged = false;
            let mut dialer_pinged = false;
            let mut relay_circuit = false;
            while !server_pinged || !dialer_pinged || !relay_circuit {
                let line = report_rx
                    .recv()
                    .await
                    .expect("all swarm drivers exited before the proof completed");
                match line.as_str() {
                    "server ping ok" => server_pinged = true,
                    "dialer ping ok" => dialer_pinged = true,
                    l if l.starts_with("relay circuit") => relay_circuit = true,
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            overall.is_ok(),
            "reservation + circuit + bidirectional ping through the relay did not complete in time"
        );
    }

    // ------------------------------------------------------------------
    // F2B: honest enforcement proofs — caps and refusal paths against a
    // real relay swarm (standard libp2p relay clients).
    // ------------------------------------------------------------------

    /// A standard relay-client swarm (relay client + ping), as in
    /// `standard_clients_reserve_and_circuit_through_the_relay`.
    fn std_client_swarm() -> (libp2p::Swarm<StdClientBehaviour>, libp2p::identity::Keypair) {
        std_client_swarm_with_ping(Duration::from_secs(30))
    }

    /// [`std_client_swarm`] with a configurable ping interval — the
    /// flag-cap cut proofs need fast pings so a byte cap is crossed
    /// DETERMINISTICALLY (ping traffic never stops, so the relay's
    /// forwarded-byte counter grows without bound and any finite cap is
    /// eventually reached).
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

    #[derive(NetworkBehaviour)]
    struct StdClientBehaviour {
        relay_client: relay::client::Behaviour,
        ping: ping::Behaviour,
    }

    /// Starts an in-process relay (the binary's exact behaviour stack) with
    /// the GIVEN config, on an ephemeral loopback QUIC port. Returns its
    /// `/p2p/`-suffixed address and a report channel carrying one honest
    /// line per relay lifecycle event (the binary's diagnostics
    /// vocabulary).
    async fn start_relay(
        config: relay::Config,
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

    /// The configured `max_reservations` cap is ENFORCED: with a cap of 1,
    /// the first peer's reservation is accepted and the second DISTINCT
    /// peer's is denied with the honest `ResourceLimitExceeded` status on
    /// the wire (this is the machinery our uncapped production config
    /// deliberately sizes at 4096 instead of disabling).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reservation_cap_is_enforced_with_resource_limit_exceeded() {
        let config = relay::Config {
            max_reservations: 1,
            ..relay_config(None, None)
        };
        let (relay_addr, mut reports) = start_relay(config).await;

        // First peer reserves: accepted.
        let (mut first, _) = std_client_swarm();
        first
            .listen_on(
                relay_addr
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2pCircuit),
            )
            .unwrap();
        tokio::spawn(async move {
            loop {
                first.select_next_some().await;
            }
        });

        // Second peer reserves: denied by the cap.
        let (mut second, _) = std_client_swarm();
        second
            .listen_on(
                relay_addr
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2pCircuit),
            )
            .unwrap();
        tokio::spawn(async move {
            loop {
                second.select_next_some().await;
            }
        });

        let mut saw_accepted = false;
        let mut saw_denied = false;
        let overall = tokio::time::timeout(Duration::from_secs(30), async {
            while !(saw_accepted && saw_denied) {
                let line = reports
                    .recv()
                    .await
                    .expect("relay driver exited before both events");
                if line.starts_with("reservation accepted") {
                    saw_accepted = true;
                }
                if line.starts_with("reservation denied") && line.contains("ResourceLimitExceeded")
                {
                    saw_denied = true;
                }
            }
        })
        .await;
        assert!(
            overall.is_ok(),
            "cap of 1 must accept the first reservation and deny the second with ResourceLimitExceeded"
        );
    }

    /// A circuit to a destination with NO reservation is refused with the
    /// honest `NoReservation` status (the peer never reserved, or its lease
    /// expired) — never a silent hang or a partial circuit.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn circuit_to_unreserved_destination_is_denied_no_reservation() {
        let (relay_addr, mut reports) = start_relay(relay_config(None, None)).await;

        let (mut dialer, _) = std_client_swarm();
        let stranger = libp2p::PeerId::random();
        let dial_addr = relay_addr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(stranger));
        dialer.dial(dial_addr).unwrap();
        tokio::spawn(async move {
            loop {
                dialer.select_next_some().await;
            }
        });

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
            "the relay must deny the circuit"
        );
        // The client-side error surface belongs to the client; this test
        // pins the RELAY's honest refusal vocabulary.
    }

    // ------------------------------------------------------------------
    // Gate D12 runtime half: the FLAG PATH end to end — caps armed from a
    // PARSED COMMAND LINE are the caps a real relay enforces against
    // standard libp2p clients.
    // ------------------------------------------------------------------

    /// Drives the dialer side of a cap-cut proof: waits `grace` for the
    /// server's reservation to settle (the timer-driven dial pattern from
    /// `standard_clients_reserve_and_circuit_through_the_relay`), dials
    /// through the relay, then reports the RELAYED connection's lifecycle.
    /// Pings are attributed by `ConnectionId` so only pings that crossed
    /// THE CIRCUIT count — the ping behaviour also pings the dialer's
    /// plain transport connection to the relay itself.
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

    /// Collects the cap-cut proof from the merged report stream (relay
    /// lifecycle lines + dialer lines): the circuit must be SEEN by the
    /// relay, ESTABLISH on the dialer, carry at least one ping THROUGH the
    /// circuit, then CLOSE between `min_lifetime` and `max_lifetime` after
    /// establishment — it demonstrably worked, and the relay (not a
    /// handshake failure) ended it at the operator's cap. An uncapped
    /// circuit would live to the 60s idle timeout, outside every window
    /// used here.
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
    /// REAL relay enforces: the relayed circuit establishes, carries pings
    /// both ways, and is CUT at the ~2s deadline (closed 1-10s after
    /// establishment). This is the flag-path version of the transport
    /// crate's `announced_circuit_duration_limit_cuts_the_stream` — and
    /// the exact behavior the default posture exists to avoid.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flag_armed_duration_cap_cuts_the_circuit() {
        // THE FLAG PATH: the enforced cap comes from parsed argv.
        let args =
            parse_args(["--max-circuit-duration-secs".to_string(), "2".to_string()].into_iter())
                .unwrap();
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        assert_eq!(
            config.max_circuit_duration,
            Duration::from_secs(2),
            "the parsed flag value must be the enforced value"
        );
        let (relay_addr, mut relay_reports) = start_relay(config).await;

        // Server reserves and listens on its circuit address.
        let (mut server, server_key) = std_client_swarm();
        let server_peer = libp2p::PeerId::from(server_key.public());
        server
            .listen_on(
                relay_addr
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2pCircuit),
            )
            .unwrap();
        tokio::spawn(async move {
            loop {
                server.select_next_some().await;
            }
        });

        // Dialer: fast pings so the circuit demonstrably carries frames
        // before the deadline.
        let (dialer, _) = std_client_swarm_with_ping(Duration::from_millis(200));
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
    /// REAL relay enforces: the circuit establishes, carries pings (a
    /// prefix of the stream), and is CUT once the forwarded-byte counter
    /// crosses the cap. Deterministic because the ping traffic never
    /// stops, so the counter grows without bound past any finite cap —
    /// the flag-path version of the transport crate's
    /// `announced_circuit_byte_limit_cuts_the_stream`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flag_armed_byte_cap_cuts_the_circuit() {
        // THE FLAG PATH: the enforced cap comes from parsed argv.
        let args = parse_args(["--max-circuit-bytes".to_string(), "4096".to_string()].into_iter())
            .unwrap();
        let config = relay_config(args.max_circuit_duration_secs, args.max_circuit_bytes);
        assert_eq!(
            config.max_circuit_bytes, 4096,
            "the parsed flag value must be the enforced value"
        );
        let (relay_addr, mut relay_reports) = start_relay(config).await;

        let (mut server, server_key) = std_client_swarm();
        let server_peer = libp2p::PeerId::from(server_key.public());
        server
            .listen_on(
                relay_addr
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2pCircuit),
            )
            .unwrap();
        tokio::spawn(async move {
            loop {
                server.select_next_some().await;
            }
        });

        let (dialer, _) = std_client_swarm_with_ping(Duration::from_millis(150));
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

        // The cut lands once handshake + identify + pings cross 4096
        // forwarded bytes — comfortably after the first ping, well before
        // the 45s proof deadline.
        let proof =
            await_relayed_circuit_cut(&mut rx, Duration::from_millis(500), Duration::from_secs(20))
                .await;
        assert!(
            proof.is_ok(),
            "a 4096-byte flag-armed cap must cut the live circuit: {:?}",
            proof.unwrap_err()
        );
    }
}
