//! Tauri build script (only active with the `tauri-shell` feature).

fn main() {
    // The NSIS bundle's `resources: ["engine/*"]` glob fails SILENTLY when
    // the pinned engine was never staged into `engine/` — v0.2.2 shipped
    // that way and every fresh install reported "engine missing". Shout
    // during release builds so it cannot happen quietly again (staging:
    // `installer/stage-engine-windows.mjs`; other OSes stage their own).
    #[cfg(all(feature = "tauri-shell", target_os = "windows"))]
    if !cfg!(debug_assertions) {
        let staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("engine")
            .join("llama-server.exe");
        if !staged.is_file() {
            println!(
                "cargo:warning=modelswarm-desktop: engine/llama-server.exe is NOT staged — the \
                 NSIS bundle will ship WITHOUT the pinned engine and every fresh install will be \
                 dead. Run installer/stage-engine-windows.mjs first."
            );
        }
    }
    #[cfg(feature = "tauri-shell")]
    tauri_build::build();
    #[cfg(not(feature = "tauri-shell"))]
    println!("cargo:rerun-if-changed=ui/index.html");
}
