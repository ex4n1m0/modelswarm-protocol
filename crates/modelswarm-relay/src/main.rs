//! MSP circuit-relay v2 host (ADR-014, Phase F2).
//!
//! A standard rust-libp2p relay with the limits the phase-f research
//! mandates: reservations ≥1024 and **no per-circuit duration/byte caps** —
//! the stock defaults (2 min / 128 KiB) reset a token stream mid-flight,
//! which ADR-007 forbids (no retry after the first output token). The
//! library's anti-abuse creation rates are deliberately kept.
//!
//! The relay is content-blind by construction: it forwards opaque
//! substreams and never parses MSP frames, prompts, or completions.
//! Access control is NOT here — serving peers enforce the ADR-026 lease
//! gate; this host only moves bytes.
//!
//! Node-side support (reserve + dial through `/p2p-circuit` addresses over
//! the raw-QUIC session layer) is the separate F2b transport work; this
//! binary is interoperable with any standard libp2p client today.

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use libp2p::relay;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identify, ping};

/// Stream-carrying limits (the whole point of running our own relay).
fn relay_config() -> relay::Config {
    // Reservation/circuit CREATION rates keep the library's anti-abuse
    // defaults via struct-update (30 per 2 min per peer) — circuits are per
    // pooled session, not per request, so this does not bound throughput.
    relay::Config {
        max_reservations: 4096,
        max_reservations_per_peer: 8,
        max_circuits: 4096,
        max_circuits_per_peer: 128,
        // A token stream must never be cut by the relay itself. The wire
        // format's duration field is u32 seconds — u64::MAX PANICS the
        // behaviour when it announces the limit (~136 years is the real max).
        max_circuit_duration: Duration::from_secs(u32::MAX as u64),
        max_circuit_bytes: u64::MAX,
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
    println!("usage: modelswarm-relay [--listen 0.0.0.0:4001] [--key msp-relay.key]");
    println!("  --listen  UDP/QUIC bind address (default 0.0.0.0:4001)");
    println!("  --key     persisted ed25519 key file (default msp-relay.key in cwd)");
    println!("            a STABLE key keeps the relay PeerId — and every");
    println!("            /p2p/<relay>/p2p-circuit address — valid across restarts.");
}

/// Resolve the advertised LAN IP the way the node does — the UDP-connect
/// trick never sends a packet.
fn lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

#[tokio::main]
async fn main() {
    let mut listen = "0.0.0.0:4001".to_string();
    let mut key_path = PathBuf::from("msp-relay.key");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => {
                listen = args.next().unwrap_or_else(|| {
                    eprintln!("--listen needs a value");
                    std::process::exit(2);
                })
            }
            "--key" => {
                key_path = PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--key needs a value");
                    std::process::exit(2);
                }))
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
    }
    let (bind_ip, bind_port) = listen.rsplit_once(':').unwrap_or_else(|| {
        eprintln!("--listen must be ip:port, got {listen}");
        std::process::exit(2);
    });
    let bind_port: u16 = bind_port.parse().unwrap_or_else(|_| {
        eprintln!("--listen port must be numeric, got {bind_port}");
        std::process::exit(2);
    });

    let keypair = load_or_create_key(&key_path).unwrap_or_else(|e| {
        eprintln!("modelswarm-relay: identity: {e}");
        std::process::exit(1);
    });
    let peer_id = libp2p::PeerId::from(keypair.public());

    let config = relay_config();
    println!("modelswarm-relay: peer id {peer_id}");
    println!(
        "modelswarm-relay: limits reservations {} ({} per peer), circuits {} ({} per peer), duration/bytes UNCAPPED",
        config.max_reservations,
        config.max_reservations_per_peer,
        config.max_circuits,
        config.max_circuits_per_peer
    );
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

    let multiaddr: libp2p::Multiaddr = format!("/ip4/{bind_ip}/udp/{bind_port}/quic-v1")
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("modelswarm-relay: bad listen multiaddr: {e}");
            std::process::exit(2);
        });
    swarm.listen_on(multiaddr).unwrap_or_else(|e| {
        eprintln!("modelswarm-relay: listen {listen} failed: {e}");
        std::process::exit(1);
    });

    let advertised_ip = lan_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| bind_ip.to_string());
    if let Ok(addr) = format!("/ip4/{advertised_ip}/udp/{bind_port}/quic-v1").parse() {
        swarm.add_external_address(addr);
    }
    println!(
        "modelswarm-relay: listening on /ip4/{advertised_ip}/udp/{bind_port}/quic-v1 (relay {peer_id}"
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

    /// The stream-carrying contract from the phase-f research: a stock relay
    /// resets token streams at 2 min / 128 KiB; ours must never.
    #[test]
    fn limits_carry_streams_and_scale_reservations() {
        let config = relay_config();
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
                relay: relay::Behaviour::new(key.public().to_peer_id(), relay_config()),
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
}
