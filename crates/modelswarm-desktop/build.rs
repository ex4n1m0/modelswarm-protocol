//! Tauri build script (only active with the `tauri-shell` feature).

fn main() {
    // The bundle's `resources: ["engine/*"]` glob fails SILENTLY when the
    // pinned engine was never staged into `engine/` — v0.2.2 shipped that
    // way and every fresh install was dead. Shout during release builds so
    // it cannot happen quietly again (staging: CI fetch step or
    // `installer/stage-engine-windows.mjs` locally).
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
        if !staged.is_file() {
            println!(
                "cargo:warning=modelswarm-desktop: engine/{name} is NOT staged — the installer \
                 will ship WITHOUT the pinned engine and every fresh install will be dead. Stage \
                 it first (CI does this automatically; locally run \
                 installer/stage-engine-windows.mjs)."
            );
        }
    }
    #[cfg(feature = "tauri-shell")]
    tauri_build::build();
    #[cfg(not(feature = "tauri-shell"))]
    println!("cargo:rerun-if-changed=ui/index.html");
}
