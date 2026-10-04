//! Local multi-peer simulator: launches several MSP peers on distinct ports to
//! exercise discovery, scheduling, failure injection, and backpressure.
//!
//! Phase 0 scaffold. Built alongside Phase 3 (P2P) by QA/Security.

fn main() {
    println!(
        "modelswarm-sim {} — multi-peer simulator arrives in Phase 3",
        env!("CARGO_PKG_VERSION")
    );
}
