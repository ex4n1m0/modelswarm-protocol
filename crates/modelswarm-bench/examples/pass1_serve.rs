//! LAN pass-1 serve side (OWNER-GATED — machine B; see
//! docs/reviews/handoff-scheduler-scientist-2026-10-09.md §9.6 harness
//! for the full command list). Spawns N REAL serving bridges over QUIC
//! backed by the TEST-ONLY synthetic token executor (loopback-equivalent
//! engine; the real-runtime variant rides the node daemon once the LAN
//! pass is scheduled), mints experiment leases for the driver's peer id,
//! and prints the `MSP_BENCH_PEER_*` connection lines the driver on
//! machine A consumes.
//!
//! Usage:
//! ```text
//! cargo run -p modelswarm-bench --features quic-runner --example pass1_serve -- \
//!     --profile <msp1:...64hex> --client-peer <driver PeerId> \
//!     --bridges 2 --inject-rtt-ms 0 --bind-ip 0.0.0.0
//! ```

use std::sync::Arc;
use std::time::Duration;

use modelswarm_bench::runner::{spawn_bridge, BridgeConfig};

/// Env-gated stderr telemetry sink (`MSP_SERVE_LOG=1`): the 2026-10-09
/// LAN bug was invisible on the serve side because its telemetry went to
/// a memory sink only — no subscriber existed, so `RUST_LOG` did nothing.
/// This surfaces serving lifecycle events (spawn/stop/refuse/ended)
/// without adding a logging dependency; field payloads stay redacted by
/// the telemetry layer (privacy rule 5).
struct StderrSink;

impl modelswarm_telemetry::Sink for StderrSink {
    fn write_line(&self, line: &str) {
        eprintln!("# pass1-serve telemetry: {line}");
    }
}

#[tokio::main]
async fn main() {
    let mut profile = String::new();
    let mut client_peer = String::new();
    let mut bridges: u64 = 2;
    let mut inject_ms: u64 = 0;
    let mut bind_ip = "0.0.0.0".to_string();
    let mut pool = "pass1".to_string();
    let mut divergent: Vec<usize> = Vec::new();
    let mut print_driver_peer = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().expect("flag needs a value");
        match arg.as_str() {
            "--profile" => profile = value(),
            "--client-peer" => client_peer = value(),
            "--bridges" => bridges = value().parse().expect("--bridges <n>"),
            "--inject-rtt-ms" => inject_ms = value().parse().expect("--inject-rtt-ms <n>"),
            "--bind-ip" => bind_ip = value(),
            "--pool" => pool = value(),
            "--divergent" => {
                divergent = value()
                    .split(',')
                    .map(|v| v.parse().expect("--divergent <comma-separated indexes>"))
                    .collect();
            }
            "--print-driver-peer" => print_driver_peer = true,
            other => panic!("unknown flag {other:?}"),
        }
    }
    // Machine-A helper (the handoff's A1 step): the driver identity is
    // the documented fixed seed, so its PeerId is deterministic — print
    // it for --client-peer on machine B and exit.
    if print_driver_peer {
        let identity = modelswarm_identity::InstallationIdentity::from_bytes(&[0xB0; 32]);
        println!("DRIVER_PEER_ID={}", identity.peer_id());
        return;
    }
    assert!(
        profile.starts_with("msp1:") && profile.len() == 69,
        "--profile must be msp1:<64hex>"
    );
    assert!(
        !client_peer.is_empty(),
        "--client-peer <driver PeerId> is required"
    );

    // The DRIVER identity (documented fixed seed on machine A): leases
    // must bind to ITS derived PeerId, and --client-peer exists to pin
    // exactly that (a typo would otherwise surface only as a runtime
    // invalid_lease refusal per bridge — the 2026-10-09 repro minted
    // leases for the SERVE identity instead, binding them to a peer id
    // no client ever presents). Assert the pin up front.
    let driver_identity = modelswarm_identity::InstallationIdentity::from_bytes(&[0xB0; 32]);
    assert_eq!(
        driver_identity.peer_id(),
        client_peer,
        "--client-peer must equal the driver's PeerId (print it on machine A \
         with --print-driver-peer)"
    );
    let telemetry = Arc::new(if std::env::var("MSP_SERVE_LOG").as_deref() == Ok("1") {
        modelswarm_telemetry::Telemetry::with_sink(Box::new(StderrSink))
    } else {
        modelswarm_telemetry::Telemetry::memory().0
    });
    let metrics = Arc::new(modelswarm_node::measure::PeerMetrics::memory(Arc::clone(
        &telemetry,
    )));

    // `--bind-ip 0.0.0.0` means "bind all interfaces and ADVERTISE the
    // LAN IP" — resolve it the way the desktop heartbeat does (UDP
    // connect picks the default-route source address), so the printed
    // multiaddr is dialable cross-machine. An explicit IP binds+advertises
    // itself. Off-loopback listens need MSP_LISTENER=1 (set by the bat).
    let advertised_ip = if bind_ip == "0.0.0.0" {
        let sock = std::net::UdpSocket::bind("0.0.0.0:0").expect("udp bind");
        sock.connect("8.8.8.8:80")
            .expect("udp connect (no packets sent)");
        sock.local_addr().expect("local addr").ip().to_string()
    } else {
        bind_ip.clone()
    };
    std::env::set_var("MSP_LISTENER", "1");

    println!(
        "# pass1-serve: {bridges} bridges (pool {pool}), inject {inject_ms} ms, profile {profile}, advertise {advertised_ip}"
    );
    println!("MSP_BENCH_POLICY_NOTE=leases are throwaway-hub-signed (experiment-only)");
    assert!(
        pool == "pass1" || pool == "pass2" || pool == "accuracy",
        "--pool must be pass1, pass2, or accuracy (got {pool:?})"
    );
    assert!(
        pool != "accuracy" || divergent.is_empty() || !divergent.contains(&0),
        "--divergent must not include index 0 (the verifier-class reference peer)"
    );
    // HOLD every bridge handle for the process lifetime (the 2026-10-09
    // LAN bug): each LiveBridge owns its serving task's shutdown watch
    // sender; dropping the handle at the end of a loop iteration dropped
    // the sender, which made serve_sessions' shutdown branch
    // permanently ready — every inbound QUIC upgrade was then cancelled
    // mid-handshake and the dialer saw "aborted by peer ... during the
    // handshake". The in-process harness holds bridges the same way.
    let mut live = Vec::with_capacity(usize::try_from(bridges).unwrap_or(0));
    for index in 0..bridges {
        // pass1: the pass-1 speed cycle (mid, slow, fast-drafter).
        // pass2: the engagement pool — V (fastest single: fast prefill,
        //   slow decode), D (drafter: decode 40x V, busy), M/M2 (busy
        //   mids). Indexes beyond 4 repeat the cycle.
        // accuracy: the 2026-10-11 accuracy-divergence pool (same shape,
        //   fixed seeds so the driver can cross-check peer ids), with
        //   --divergent injecting the divergence fault model.
        if pool == "accuracy" {
            let config = modelswarm_bench::runner::accuracy_pool(&divergent)
                [usize::try_from(index).expect("index")]
            .clone();
            let config = BridgeConfig {
                bind_addr: bind_ip.clone(),
                ..config
            };
            let bridge = spawn_bridge(
                &profile,
                &driver_identity,
                &config,
                Duration::from_millis(inject_ms),
                Arc::clone(&telemetry),
                Arc::clone(&metrics),
            )
            .await
            .expect("bridge spawns");
            println!(
                "MSP_BENCH_PEER_{index}={}|{}|{}|{}|{}",
                bridge
                    .addr
                    .replace("/ip4/0.0.0.0/", &format!("/ip4/{advertised_ip}/")),
                bridge.peer_id,
                bridge.token,
                bridge.roster.advertised_queue_ms.unwrap_or(0),
                bridge.roster.capacity_class.as_deref().unwrap_or("cpu"),
            );
            live.push(bridge);
            continue;
        }
        let mut config = match (pool.as_str(), index % 4) {
            ("pass2", 0) => BridgeConfig::new(0x51 + index, 0.10, 3.0),
            ("pass2", 1) => {
                let mut drafter = BridgeConfig::new(0x66 + index, 4.00, 6.0);
                drafter.advertised_queue_ms = 300;
                drafter.capacity_class = "gpu_high";
                drafter
            }
            ("pass2", 2) => {
                let mut mid = BridgeConfig::new(0x77 + index, 0.25, 1.2);
                mid.advertised_queue_ms = 400;
                mid
            }
            ("pass2", 3) => {
                let mut mid = BridgeConfig::new(0x88 + index, 0.20, 1.0);
                mid.advertised_queue_ms = 500;
                mid
            }
            _ => match index % 3 {
                0 => BridgeConfig::new(0x11 + index, 0.30, 1.5),
                1 => BridgeConfig::new(0x22 + index, 0.20, 1.0),
                _ => {
                    let mut drafter = BridgeConfig::new(0x33 + index, 2.00, 4.0);
                    drafter.advertised_queue_ms = 300;
                    drafter.capacity_class = "gpu_high";
                    drafter
                }
            },
        };
        config.bind_addr = bind_ip.clone();
        let bridge = spawn_bridge(
            &profile,
            &driver_identity,
            &config,
            Duration::from_millis(inject_ms),
            Arc::clone(&telemetry),
            Arc::clone(&metrics),
        )
        .await
        .expect("bridge spawns");
        println!(
            "MSP_BENCH_PEER_{index}={}|{}|{}|{}|{}",
            bridge
                .addr
                .replace("/ip4/0.0.0.0/", &format!("/ip4/{advertised_ip}/")),
            bridge.peer_id,
            bridge.token,
            bridge.roster.advertised_queue_ms.unwrap_or(0),
            bridge.roster.capacity_class.as_deref().unwrap_or("cpu"),
        );
        live.push(bridge);
    }
    println!(
        "# serving until Ctrl+C ({} bridge handles held)",
        live.len()
    );
    tokio::signal::ctrl_c().await.expect("ctrl-c");
    drop(live);
    println!("# pass1-serve: bridges released");
}
