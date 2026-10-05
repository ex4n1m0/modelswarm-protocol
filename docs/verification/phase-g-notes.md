# Phase G Verification Notes — Node Composition + Desktop Shell Attempt

Date: 2026-10-04/05 · Owner: Windows Product Engineer · Paths touched (only):
`crates/modelswarm-node`, `crates/modelswarm-desktop`, `installer/tauri/nsis-notes.md`,
this file. Acceptance gates: `docs/acceptance/phase-g.md`.

## 1. What ran (executable evidence)

### Node self-test (G-gate, ADR-019 honesty)

Command (exact):

```
cargo run -q -p modelswarm-node --bin modelswarm-node --features node-selftest -- self-test
```

Observed stdout (single JSON line, after the stderr mock-runtime warning banner):

```
{"self_test":"ok","tokens":2,"redacted_logs":true}
```

- 24 requested tokens stream as 2 SSE token-delta chunks (16-token batching,
  mirroring the session executor's per-round batches), terminal `[DONE]`
  present, content-type `text/event-stream`.
- The prompt carries the canary `MSP-SELFTEST-CANARY-PROMPT-7F3A9B2C`; after
  graceful shutdown the node's own `logs/node.jsonl` is read back from disk
  and the canary is asserted ABSENT (the self-test hard-fails otherwise).
  Independently re-checked by the unit test on the file, not the report.
- Without the feature, the same subcommand refuses (exit 2):

```
cargo run -q -p modelswarm-node --bin modelswarm-node -- self-test
→ self-test refused: this build has no self-test support. … exit=2
```

### Unit/integration tests in the node crate

```
cargo test -p modelswarm-node
→ 9 passed; 0 failed
```

Coverage: `self_test_end_to_end` (SSE tokens + canary-absent-in-logs +
identity seed persisted 32 bytes + store opened + store privacy columns
re-audited over `sqlite_master`/`PRAGMA table_info`),
`gateway_binds_loopback_only_and_refuses_non_loopback`,
`identity_survives_restart_and_is_recorded_in_store`,
`corrupt_seed_is_refused_not_replaced`, executor contract tests
(no-runtime→`NoPeer`, wrong-profile→`profile_mismatch`,
event-stream shape/batching), config/URL-guard tests.

### `run` smoke (manual, loopback only)

```
cargo run -q -p modelswarm-node --bin modelswarm-node --features node-selftest -- \
  run --port 39471 --data-dir <tmp> --profile msp1:smoke --allow-mock-runtime
```

- `GET /v1/models` → `{"data":[{"id":"msp1:smoke",…}],"object":"list"}`
- non-stream chat completion answered (mock toy bytes — TEST-ONLY);
- `<data_dir>/{identity.seed,state.sqlite,logs/node.jsonl}` created;
  log shows redacted structured events (`runtime.selected` carries
  `[REDACTED:HEXBLOB]` for the build hash — the redactor working);
  a second canary (`SMOKE-CANARY-XYZ`) greps 0 hits in the log;
- process killed after the check (no strays — verified at the end).

### Node composition facts

- Gateway port default **11435** (`modelswarm_gateway::DEFAULT_PORT`);
  loopback-only is structural: `Node::start` builds the `127.0.0.1` socket
  itself and calls `assert_loopback` before binding (test asserts the bound
  address is `127.0.0.1`; `0.0.0.0`/`192.168.1.5`/`8.8.8.8`/`::` refused).
- Telemetry: `Telemetry::with_sink(FileSink)` at `<data_dir>/logs/node.jsonl`;
  every field passes the redactor (`prompt`/`token`/`secret` names dropped,
  secret-shaped values scrubbed, oversized values truncated).
- Store: `Store::open(<data_dir>/state.sqlite)` — migrations applied;
  `upsert_installation(id, pub_key)` recorded at boot.
- Identity: load-or-create at `<data_dir>/identity.seed` (exactly 32 bytes;
  a wrong-length seed is REFUSED, not replaced — lost key = re-enroll,
  ADR-004). **Seed-file hardening note:** 0600 via `PermissionsExt` on Unix;
  on Windows the write relies on the user-profile directory ACL — the
  ADR-004 destination is **DPAPI (`CryptProtectData`) / Windows Credential
  Manager, recorded as the Phase F hardening replacement**. The seed never
  appears in logs, `NodeHandle`, or the webview.
- Tracker: `TrackerClient` constructed only when `--tracker URL` is given;
  **no calls at startup**; the only call path is the CLI's opt-in
  `--check-tracker` (read-only `GET /health`).
- Runtime selection (ADR-019): mock only under `cfg(test)` or the
  `node-selftest` feature, gated further by `--allow-mock-runtime`; active
  mock prints a loud stderr banner + `runtime.mock_active` warn log and is
  **local-loopback-only by policy** — no transport listener is constructed,
  no registration path exists, so a mock node cannot join any roster.
  Non-mock: `LlamaCppAdapter` when BOTH `MSP_LLAMACPP_URL` (loopback URLs
  only) and `MSP_LLAMACPP_TOKEN` are set; otherwise the node starts
  runtime-less and the gateway answers honest `no_peer`/503.
- Executor: `SingleLocalExecutor` (local `decode_stream` → batched
  `ExecutorEvent`s, same event shape as `session::spec`'s executor).
  **The full swarm executor (`SpeculativeExecutor` + transport peer
  rotation) is a later wiring** — documented in the crate docs, not faked.

## 2. Desktop shell attempt (G1/G2/G3/G4) — outcome: SUCCEEDED

Contrary to the expected offline/compile-weight risk, **Tauri 2 compiled,
linked, and ran** in this environment (network reachable; tauri 2.x crates
already in the local registry cache). Two configuration iterations were
needed, both project-asset issues, not dependency-tree failures:

1. Attempt 1 — `cargo check -p modelswarm-desktop --features tauri-shell`
   failed: `generate_context!` → "OUT_DIR env var is not set, do you have a
   build script?" → added `build.rs` + optional `tauri-build` build-dep.
2. Attempt 2 — build script failed: `icons/icon.ico not found; required for
   generating a Windows Resource file` → generated a minimal valid
   placeholder `icons/icon.ico` (32×32, scripted) and set
   `bundle.icon = ["icons/icon.ico"]`.

Results:

```
cargo check -p modelswarm-desktop --features tauri-shell   → exit 0
cargo build  -p modelswarm-desktop --features tauri-shell   → exit 0
                                                            (modelswarm-desktop.exe, 12.3 MB debug)
run for 8 s → process ALIVE (window created, no panic), then killed; log empty
```

- The shell stays behind the off-by-default `tauri-shell` feature so the
  plain workspace build never needs the tauri/wry tree; the fallback stub
  (feature off) compiles and prints the intended flow. **Both paths green.**
- `ui/index.html`: static, dependency-free (no JS frameworks, zero JS at
  all), implements the G2 state model — hosting on/off (CSS-only switch),
  profile-id placeholder, artifact-verification + serving-health fields,
  earned eligibility (lease + capacity class), peers + NAT-path table
  (example rows incl. an explicit direct-connect-failure row), per-request
  mode, fallback events list, measured speedup/slowdown placeholder with the
  "no theoretical numbers" note; G3 privacy disclosures (first-run warning
  that peers can read prompts, cooperative roster mode+count pre-request
  disclosure, opt-in diagnostics exclusion, no-telemetry default); G4
  degraded-state chips (`tracker unreachable / ineligible / draining /
  updating / model mismatch / direct-fail`) as CSS-only `data-state`
  states; 40 aria/role attributes for accessibility (G gate); header shows
  `INTERNAL TEST BUILD — UNSIGNED`. HTML validated well-formed
  (parser check; browser-screenshot verification unavailable in this
  subagent session — noted, not blocking for a static page).
- **Not wired yet (honest):** no Tauri IPC commands exist; the node-daemon
  lifecycle supervision from the shell and live state are future wiring.

## 3. IPC surface audit (G1) — planned commands + design rule

The webview is static in this pass, so there are ZERO exposed commands today.
The planned surface (asserted as the design rule for the wiring phase):

| Command | Returns (G2 state fields only) |
|---|---|
| `get_status` | hosting on/off, profile id, artifact verification state, serving health, lease status + expiry, capacity class, peer list + NAT path, last request mode, degraded-state flags |
| `set_hosting(enabled)` | new hosting state (no secrets echoed) |
| `list_models` | profile ids + verification/download state |
| `download_progress` | bytes/total + paused state per profile |
| `drain` | draining state flag |

**Design rule (enforced by review, G1):** keys, tokens, lease secrets, the
identity seed, and prompt/completion content NEVER cross into the webview.
Commands return only the G2 state fields above; anything secret-carrying
stays in the node process. `tauri.conf.json` pins a restrictive CSP
(`default-src 'self'`) with no remote origins.

## 4. Installer + signing (G5/G6)

See `installer/tauri/nsis-notes.md` (staged): exact command
(`cargo tauri build --bundles nsis`), the `-internal-test-unsigned` version
suffix + splash label requirement, per-user/no-elevation flags, uninstall
expectations (app files + user-consented model dir; identity seed removal is
an explicit opt-in meaning re-enrollment). **STOP honored: no signing
certificate exists — no installer artifact was produced in this phase.** No
NSIS toolchain was downloaded; nothing was published.

## 5. Clean-VM pending-hardware list (from phase-g.md, honest boundary)

Requires a clean Windows VM harness; recorded with the exact commands to run
automatically if a VM becomes available:

1. Fresh install: run the (future, labeled) `setup.exe` → assert per-user
   install path, no elevation prompt, Start-menu entry, no services/tasks.
2. Upgrade drill: install `0.1.0-internal-test-unsigned` then a newer build
   → assert settings/state (`state.sqlite`, `identity.seed`) survive.
3. Offline launch: disable networking → shell must open, node must start,
   degraded `tracker unreachable` chip renders, loopback self-test still OK.
4. Firewall recovery: enable outbound block → P2P attempts fail explicitly
   (`direct-connect failure` visible), gateway loopback unaffected.
5. Crash recovery: kill the node process mid-stream → explicit
   `interrupted` SSE error (no fabricated continuation, ADR-007), shell
   restart path works.
6. Uninstall: run uninstaller → app files gone, model dir kept by default
   (removed only with consent), no residual services/tasks/autostart.

## 6. Deviations / interpretations recorded

1. `Node::start` spawns the gateway via `build_router` + `axum::serve` on a
   node-constructed `127.0.0.1` socket (re-asserting `assert_loopback`)
   rather than the gateway's `spawn_loopback`, because `spawn_loopback` is
   ephemeral-port-only and `run --port N` needs a fixed port. Same guard,
   same router; test proves the bind is loopback.
2. Mock+tracker is allowed at construction (no calls are made; mock never
   registers anywhere because no registration path exists yet) and logs a
   `tracker.mock_mode` warning. A hard refusal would be dead-code theater
   this phase; revisit when registration lands.
3. Self-test token count = SSE token-delta chunks (2 for 24 tokens at
   16-token batching), excluding the empty role chunk — documented in
   `selftest.rs`.
4. The desktop `tauri-shell` feature is OFF by default (workspace build
   weight), unlike ADR-008's implied always-on shell; enabling is one
   feature flag. Recorded as a deliberate deviation for CI portability.
5. Placeholder `icons/icon.ico` is a generated 32×32 placeholder, not brand
   art; final icon ships with the installer phase.

## 7. Risks / open items

- Node-daemon lifecycle supervision from the shell (start/stop/crash-restart,
  G1's second half) is not yet wired — the shell is a static state model.
- The llama.cpp sidecar is pointed-at (env vars) but not supervised;
  download/spawn is Runtime Engineer territory (later phase).
- The full swarm executor (`session::spec::SpeculativeExecutor`) is not yet
  wired into the node (documented in `modelswarm-node` crate docs).
- Browser-based visual verification of `ui/index.html` was not possible in
  this agent session (browser tooling unavailable in subagents); the page is
  parser-validated and opens in any browser.
- Windows seed-file protection currently relies on profile-dir ACLs; DPAPI/
  Credential Manager migration is the recorded Phase F hardening (ADR-004).
