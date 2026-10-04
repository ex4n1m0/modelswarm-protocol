//! ModelSwarm Windows node daemon (host + client + scheduler host process).
//!
//! Phase 0 scaffold: reports identity and exits. Real behavior arrives with
//! Phase 2 (runtime), Phase 3 (P2P), and Phase 4 (gateway).

fn main() {
    println!(
        "modelswarm-node {} — MSP protocol v{} (Phase 0 scaffold; no runtime behavior yet)",
        env!("CARGO_PKG_VERSION"),
        ms_core::MSP_PROTOCOL_VERSION
    );
}
