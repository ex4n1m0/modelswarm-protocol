//! Tauri build script (only active with the `tauri-shell` feature).

fn main() {
    #[cfg(feature = "tauri-shell")]
    tauri_build::build();
    #[cfg(not(feature = "tauri-shell"))]
    println!("cargo:rerun-if-changed=ui/index.html");
}
