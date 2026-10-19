# Installer notes — Tauri 2 + NSIS `setup.exe` (Phase G, staged — NOT BUILT)

Owner: Windows Product Engineer · Decision record: ADR-008 · Gate:
`docs/acceptance/phase-g.md` G5/G6.

## Status: STAGED, NO ARTIFACT PRODUCED

No installer is built in Phase G. Two hard stops apply:

1. **No signing certificate exists** (G6: paid/identity process). Unsigned
   installers may exist only as clearly-labeled internal-test builds; the
   decision is to not produce even those until the Tauri shell (`tauri-shell`
   feature, `crates/modelswarm-desktop`) and the node daemon lifecycle are
   wired, so the bundle would ship a stub. Recorded, not attempted.
2. The Tauri **updater is enabled as of 2026-10-12** (owner request: "clients
   update and restart when we ship a new client") — under a settled design
   that does NOT need the code-signing certificate above: the installers
   stay UNSIGNED, and each update PACKAGE carries its own minisign-class
   signature (owner-generated keypair; the PUBLIC key is embedded in
   `tauri.conf.json` `plugins.updater.pubkey`, the private key lives
   outside the repo at local release staging only). Signature verification
   cannot be disabled — still treated as correct behavior. Windows
   installs run passive (`installMode: "passive"`: progress bar only, no
   wizard) with the app restarted by the NSIS `/UPDATE /R` flow; the
   desktop renders its own honest status line (`ui/index.html`
   `#update-note`). See `docs/reviews/handoff-windows-product-updater-2026-10-13.md`
   for the exact release-train signing/manifest steps.

## Exact bundling command (when the stop clears)

From the tauri project root (`crates/modelswarm-desktop` with
`tauri.conf.json`):

```
cargo tauri build --bundles nsis
```

(Requires the tauri CLI: `cargo install tauri-cli --version ^2`. The CLI
downloads the pinned NSIS toolchain on first run. With the shell behind the
`tauri-shell` feature flag the invocation needs the feature enabled in
`tauri.conf.json` or a temporary default-features change; that wiring belongs
to the phase that clears the stop.)

Output: `target/release/bundle/nsis/ModelSwarm_<version>_x64-setup.exe`.

## Unsigned internal-test labeling (G5 requirement)

- **Version string suffix**: every unsigned internal build carries the suffix
  `-internal-test-unsigned` (e.g. `0.1.0-internal-test-unsigned`) in the
  bundle version and in the shell's title/header. The static UI already
  renders `INTERNAL TEST BUILD — UNSIGNED` in its header (`ui/index.html`,
  `role="note"` build tag).
- **Splash text**: the NSIS installer wizard must show the same label. Tauri's
  NSIS template exposes installer strings via `bundle > windows > nsis`
  overrides in `tauri.conf.json` (`displayLanguageSelector`,
  `installerIcon`, custom `template`, and `headerImage`/`sidebarImage`);
  when wiring, either use a custom NSIS template that injects
  `INTERNAL TEST BUILD — UNSIGNED` into the welcome/finish pages, or set the
  product name for internal builds to include the suffix. SmartScreen will
  warn regardless (accepted while private, ADR-008).
- Any artifact produced without the label is a release-gate failure, not a
  packaging nit.

## Per-user install flags

- Target: **per-user install, no elevation** (`perMachine = false` — this is
  Tauri's NSIS default; keep it). Install root: `%LOCALAPPDATA%\ModelSwarm`
  (matches the node's default `data_dir`).
- No services, no scheduled tasks, no drivers in v0.1. The node is a
  user-session process supervised by the shell (crash-restart policy arrives
  with the IPC wiring).
- Firewall: the node's gateway binds `127.0.0.1` only (loopback, no prompt).
  Outbound P2P is user-initiated; the Phase F transport ADR (ADR-014 relay
  roadmap) owns any firewall-interactive behavior. Windows Firewall prompts
  must not appear at install time.

## Uninstall expectations (G5)

- Removes: app binaries (shell + node daemon), pinned llama.cpp runtime
  resources (bundled, ADR-008), Start-menu shortcut, registry uninstall key.
- **Model files are NOT auto-deleted**: the uninstaller asks, and only with
  explicit user consent removes the user-consented model directory (weights
  downloaded from HF at runtime are never bundled). Default answer: keep.
- Leaves behind (by design, user data): `%LOCALAPPDATA%\ModelSwarm`
  (`state.sqlite`, `logs/`, `identity.seed`) unless the user opts into a
  "remove all data" checkbox that includes the identity seed — deleting the
  seed means re-enrollment (ADR-004: lost key = lost identity).
- No residual services/tasks/autostart entries (autostart, when it lands,
  is a per-user Run key the uninstaller removes).

## Signed builds (G6) — STOP

Requires a code-signing certificate (OV/EV, paid, identity process) plus a
key-management decision for updater signatures. **Recorded, not attempted.**
No `.msi` either (deferred until asked for, ADR-008).

## Update-safety hook (2026-10-19 evening incident)

`installer-hooks.nsh` (wired via `bundle.windows.nsis.installerHooks` in
`tauri.conf.json`) defines `NSIS_HOOK_PREINSTALL`: before ANY file is
extracted, kill only `llama-server.exe` processes whose image path is under
`$INSTDIR\engine`, then settle 500 ms. Rationale: an orphaned engine
process (repro: benchmark harness whose cargo parent died) holds
`engine\ggml-base.dll` open and stalls NSIS at "Error opening file for
writing" — live during the v0.2.41 updater run on machine A. Path-scoped
by design: never kill by image name, the owner may run an unrelated
llama.cpp. Validation owed: a local `cargo tauri build --bundles nsis`
must compile the hook (CI will also gate it on the next train).
