//! ModelSwarm desktop shell entry point (Phase G, ADR-008).
//!
//! Two build modes:
//!
//! - **`tauri-shell` feature ON** — the real Tauri 2 shell: a Builder with
//!   no plugins and no custom IPC commands yet, loading the static,
//!   dependency-free `ui/index.html` state model. The webview is a static
//!   page in this pass; the planned IPC surface (G1 audit) lives in
//!   `docs/verification/phase-g-notes.md` and returns only G2 state fields —
//!   keys, tokens, and lease secrets never cross into the webview.
//! - **feature OFF (default)** — the fallback stub: prints the intended
//!   flow and exits 0, so the workspace stays buildable where the
//!   tauri/wry tree cannot compile. The `ui/index.html` state model is
//!   still the deliverable and opens in any browser.

#[cfg(feature = "tauri-shell")]
fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("modelswarm desktop shell failed to start");
}

#[cfg(not(feature = "tauri-shell"))]
fn main() {
    println!(
        "modelswarm-desktop {} — Tauri shell not compiled in this build.",
        env!("CARGO_PKG_VERSION")
    );
    println!("Intended flow (ADR-008 / Phase G):");
    println!("  1. tauri::Builder loads ui/index.html (static G2/G3/G4 state model).");
    println!("  2. The node daemon lifecycle (modelswarm-node) is supervised from the");
    println!("     shell process; crash-restart policy arrives with the IPC wiring.");
    println!("  3. Planned IPC commands (G1 audit, see docs/verification/phase-g-notes.md):");
    println!("     get_status, set_hosting, list_models, download_progress, drain.");
    println!("     Rule: keys/tokens/lease secrets never enter the webview.");
    println!("Enable with: cargo check -p modelswarm-desktop --features tauri-shell");
}
