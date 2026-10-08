//! Tauri build script (only active with the `tauri-shell` feature).

fn main() {
    // The bundle's `resources: ["engine/*"]` glob fails SILENTLY when the
    // pinned engine was never staged into `engine/` — v0.2.2 shipped that
    // way and every fresh install was dead. A warning scrolled past once
    // already, so this is a release GATE: panic in release builds unless
    // MSP_ALLOW_NO_ENGINE=1 explicitly opts out (stub-only dev builds).
    #[cfg(feature = "tauri-shell")]
    if !cfg!(debug_assertions) {
        let name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        let staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("engine")
            .join(name);
        if !staged.is_file() && std::env::var("MSP_ALLOW_NO_ENGINE").as_deref() != Ok("1") {
            panic!(
                "modelswarm-desktop: engine/{name} is NOT staged — the installer would ship \
                 WITHOUT the pinned engine and every fresh install would be dead. Stage it first \
                 (CI does this automatically; locally run \
                 installer/stage-engine-windows.mjs) or set MSP_ALLOW_NO_ENGINE=1 for a \
                 stub-only build."
            );
        }
    }
    #[cfg(feature = "tauri-shell")]
    tauri_build::build();
    #[cfg(not(feature = "tauri-shell"))]
    println!("cargo:rerun-if-changed=ui/index.html");
}
