# Phase G Verification Report

Date: 2026-10-05 · Agent notes: `docs/verification/phase-g-notes.md` ·
This commit.

## Delivered

**Node daemon** (`crates/modelswarm-node`) — real composition: FileSink
telemetry (redaction-enforced), SQLite store, load-or-create Ed25519
identity (file-based now; DPAPI/Credential-Manager migration recorded for
Phase F per ADR-004), TrackerClient only when configured (zero startup
calls), runtime selection with the mock path feature-gated and
loopback-only-by-policy, gateway on a node-built loopback socket with the
guard re-asserted, `SingleLocalExecutor`, ctrl-c graceful shutdown, and a
`self-test` subcommand that plants a **canary string in a real prompt and
asserts its absence from the log file**.

**Desktop shell** (`crates/modelswarm-desktop`) — Tauri 2 shell **built
and ran** (not the fallback): `cargo check/build --features tauri-shell`
clean (12.3 MB exe; window created, no panic). Static dependency-free UI
(40 aria/role attributes) implementing the G2 state model, G3 privacy
disclosures (first-run warning + cooperative roster note), G4 degraded
chips, and the INTERNAL TEST BUILD header. Two project-asset fix-ups were
needed (tauri-build/OUT_DIR; generated placeholder icon) — recorded
verbatim in the notes.

**Installer staging** (`installer/tauri/nsis-notes.md`) — exact bundle
command, unsigned labeling requirement, per-user flags, uninstall
expectations. **Signing STOP honored: no artifact produced.**

## Integrator-run gates

| Command | Result |
|---|---|
| `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` | PASS / exit 0 |
| `cargo test -p modelswarm-node` | 9/0 |
| `cargo run -p modelswarm-node --features node-selftest -- self-test` | `{"self_test":"ok","tokens":2,"redacted_logs":true}` exit 0 |
| self-test without the feature | loud refusal, exit 2 (agent-verified) |
| `cargo check -p modelswarm-desktop --features tauri-shell` | exit 0 |
| `run` smoke + canary grep of `logs/node.jsonl` | OK, 0 hits (agent-run; recorded) |

## Gates (docs/acceptance/phase-g.md)

- G1: node lifecycle composed; **IPC audit recorded** (planned commands
  enumerated; secrets-never-in-webview rule documented; live wiring is
  Phase F+ work — honest boundary). G2/G3/G4: implemented in the UI state
  model; visual browser check pending (parser-validated only).
- G5: NSIS staging only — unsigned-internal-test path documented;
  **G6 signing STOP recorded**, no artifact.
- Environment-dependent items (clean-VM install/firewall/offline/upgrade
  drills): pending-hardware list with exact commands, per the acceptance
  doc's pre-declared boundary.

## Honest boundaries

- Shell↔node supervision and live IPC not yet wired (static audited UI);
  the swarm `SpeculativeExecutor` is not yet composed into the node
  (documented in-crate) — Phase F integration work.
- llama.cpp sidecar is pointed-at, not supervised; no GGUF profile exists
  (stop conditions intact — nothing downloaded).
- Identity seed relies on profile-dir ACLs until the Phase F DPAPI
  migration.
