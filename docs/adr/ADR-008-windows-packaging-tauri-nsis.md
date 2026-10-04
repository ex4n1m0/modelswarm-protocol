# ADR-008: Windows packaging via Tauri 2 + NSIS setup.exe

Status: Accepted (Phase 0)

## Context

The node is a Windows 10/11 x64 daemon plus a desktop UI. We need an installer,
clean uninstall, tray integration, and a credible update path. Tauri 2
produces NSIS `setup.exe` (and optional MSI via WiX), wraps a Rust backend
with a small webview UI, and enforces signed updates.

## Decision

Ship an NSIS `setup.exe` built by Tauri 2 (Phase 6). The installer bundles the
node daemon, desktop shell, and the pinned llama.cpp runtime resources; it does
not bundle model weights (downloaded at runtime from HF). No elevation unless
proven necessary (per-user install is the default target). Early builds are
**unsigned and clearly labeled internal-test**; public builds get code signing
before distribution. The Tauri updater is designed for but not enabled until
signed artifacts + key management exist — its signature verification cannot be
disabled, which we treat as correct behavior.

## Consequences

+ One toolchain for shell + installer; tray + autostart well-supported.
+ Update channel (release_channels table) aligns with staged rollouts.
− SmartScreen will warn on unsigned builds (accepted while private).
− MSI (enterprise-friendly) deferred until asked for.

## Alternatives rejected

- **egui all-Rust UI**: weaker webview-style UI + no installer story.
- **WiX-first MSI**: more setup cost; NSIS covers v0.1.
- **Portable zip**: no tray/autostart/uninstall hygiene.
