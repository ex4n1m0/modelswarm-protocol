# Phase G Acceptance Tests — Windows Beta Shell

Gate (revision): installer/uninstall/upgrade/offline/firewall/crash
recovery on supported Windows versions; every performance claim traces to a
reproducible record. Evidence → `docs/verification/phase-g.md`.

## Shell (Tauri 2, ADR-008)

- **G1** Desktop shell builds and runs, wrapping the node daemon lifecycle
  (start/stop/crash-restart) — private keys/tokens never enter the webview
  ( IPC surface audited: list every exposed command in the report).
- **G2** Visible states implemented: hosting on/off, exact `msp1:` profile
  id, artifact verification, serving health, earned eligibility (lease +
  capacity class), peer list + NAT path, per-request mode, fallback events,
  measured speedup **and** slowdown (no theoretical numbers).
- **G3** Privacy disclosures: first-run warning (peers can read prompts);
  cooperative-mode roster disclosure (mode + peer count pre-request);
  opt-in diagnostics export that structurally excludes prompt/completion
  content.
- **G4** Degraded states render correctly: tracker unreachable,
  ineligible, draining, updating, model mismatch, direct-connect failure.

## Installer

- **G5** NSIS `setup.exe` builds via Tauri (unsigned, splash-labeled
  "INTERNAL TEST BUILD — UNSIGNED" when no certificate is present);
  per-user install, no elevation; uninstall removes app + user-consented
  model files; no residual services/tasks.
- **G6** Signed `.msi`/`.exe`: **STOP — requires code-signing certificate
  (paid/identity process). Recorded, not attempted.**

## Environment-dependent items (honest boundary)

- Fresh-VM install, firewall-recovery, offline-launch, and upgrade drills
  require a clean Windows VM harness; recorded as pending-hardware with the
  exact commands to run, executed automatically if a VM becomes available.
- All prior-phase regression suites must be green in the same run.
