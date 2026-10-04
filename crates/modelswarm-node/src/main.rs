//! ModelSwarm Windows node daemon (host + client + scheduler host process).
//!
//! Phase 0 scaffold: reports identity and exits. Real behavior arrives with
//! Phase B (identity, store), Phase C (runtime, P2P, gateway).

fn main() {
    println!(
        "modelswarm-node {} — MSP protocol v{} (Phase 0 scaffold; no runtime behavior yet)",
        env!("CARGO_PKG_VERSION"),
        modelswarm_types::MSP_PROTOCOL_VERSION
    );
}
