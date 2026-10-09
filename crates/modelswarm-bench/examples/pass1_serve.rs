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

#[tokio::main]
async fn main() {
    let mut profile = String::new();
    let mut client_peer = String::new();
    let mut bridges: u64 = 2;
    let mut inject_ms: u64 = 0;
    let mut bind_ip = "0.0.0.0".to_string();
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

    // Machine B's harness identity (documented experiment seed).
    let identity = modelswarm_identity::InstallationIdentity::from_bytes(&[0xB1; 32]);
    let telemetry = Arc::new(modelswarm_telemetry::Telemetry::memory().0);
    let metrics = Arc::new(modelswarm_node::measure::PeerMetrics::memory(Arc::clone(
        &telemetry,
    )));

    println!("# pass1-serve: {bridges} bridges, inject {inject_ms} ms, profile {profile}");
    println!("MSP_BENCH_POLICY_NOTE=leases are throwaway-hub-signed (experiment-only)");
    for index in 0..bridges {
        // Asymmetric pool per the dry run (mid, slow, fast-drafter cycle).
        let config = match index % 3 {
            0 => BridgeConfig::new(0x11 + index, 0.30, 1.5),
            1 => BridgeConfig::new(0x22 + index, 0.20, 1.0),
            _ => {
                let mut drafter = BridgeConfig::new(0x33 + index, 2.00, 4.0);
                drafter.advertised_queue_ms = 300;
                drafter.capacity_class = "gpu_high";
                drafter
            }
        };
        let bridge = spawn_bridge(
            &profile,
            &identity,
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
                .replace("/ip4/0.0.0.0/", &format!("/ip4/{bind_ip}/")),
            bridge.peer_id,
            bridge.token,
            bridge.roster.advertised_queue_ms.unwrap_or(0),
            bridge.roster.capacity_class.as_deref().unwrap_or("cpu"),
        );
    }
    println!("# serving until Ctrl+C");
    tokio::signal::ctrl_c().await.expect("ctrl-c");
}
