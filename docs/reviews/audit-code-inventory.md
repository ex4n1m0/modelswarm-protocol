# Code Inventory Audit — ModelSwarm Protocol

Date: 2026-10-09 · Auditor: code-auditor subagent (master prompt §8)
Baseline: `730cebf` on `audit/master-prompt-2026-10-09`, product v0.2.20.
Method: static analysis only (Read / git grep / ls). No cargo/npm commands
were run (validation suite holds the target lock); no files modified except
this audit's two outputs. Performance statements below are labeled by
environment: **mock / loopback / simulator / LAN / WAN / production**.

Scope: all 17 `crates/` members, both apps (`apps/tracker`,
`apps/modelswarm-sim`), `protocol/`, `catalog/`, `installer/`, `scripts/`,
`experiments/`, `tests/`, `.github/workflows/`, `runtime-pins.json`.

---

## 1. Top findings (severity-ranked)

**CRITICAL — none found.** No prompt/completion leak path, no mock in the
production inference path, no unsafe outside the sanctioned winjob crate, no
committed secrets, no unbounded production data structure found.

### HIGH

- **H1 — "Streaming" on the whole serving path is not streaming (production,
  local + LAN).** `SingleLocalExecutor::execute` awaits the *entire*
  `decode_stream` before it returns any event
  (`crates/modelswarm-node/src/executor.rs:136-195`: events are pre-collected
  into `stream::iter`), and `LlamaCppAdapter::decode_stream` decodes **one
  token per HTTP POST** (`/v1/completions` with `max_tokens:1`,
  `crates/modelswarm-runtime/src/llamacpp.rs:510-535` via `decode_step`
  `:411-506`), re-sending the growing prefix each step (O(n²) request bytes
  on loopback for n tokens; the engine's `stream:true` SSE mode is never
  used). Consequences: TTFT through the local OpenAI API equals *full
  completion time*; SSE chunks arrive as a burst after completion; remote
  peers (serving bridge forwards executor events, `serving.rs:375-409`)
  receive the same burst. This is the dominant latency/UX defect of the
  shipped product and the natural target for the master prompt's
  "connection-reuse and prefill as the real latency levers" directive.
- **H2 — Peer selection in shipped code is "first QUIC peer found", and the
  scheduler crate is compiled into nothing on the product path.**
  `try_swarm_chat` picks the first roster peer with a `quic-v1` multiaddr
  and free slots (`crates/modelswarm-desktop/src/app.rs:1435-1446`); no EWMA,
  no RTT probing, no cost model (remote.rs:350-354 documents this as
  "v1 policy… no EWMA"). `modelswarm-scheduler` (cost model v2, frozen
  ADR-013 rules, 15 tests) is imported **only** by `modelswarm-bench`
  (`git grep modelswarm_scheduler` — sole hit
  `crates/modelswarm-bench/src/lib.rs:39`). Architecture §10's
  `predicted_ms` scoring exists in code but runs in no shipped binary.
- **H3 — Remote P2P serving is OFF by default in shipped builds.** The QUIC
  listener binds only when env `MSP_LISTENER=1`
  (`crates/modelswarm-desktop/src/app.rs:252-253`;
  `crates/modelswarm-transport/src/libp2p_backend.rs:220-225` refuses
  non-loopback binds without it) and required a manual Windows inbound
  firewall rule on the serving machine (phase-f-lan doc). The UI *displays*
  the state ("remote serve: off (dev listener flag not set)",
  `ui/index.html:404-408`) but offers no toggle. The F0/F1 LAN proof is real,
  but "production-proven" applies to the *code path*, not to default
  behavior of the shipped installer.

### MEDIUM

- **M1 — Identity seed and session material are plaintext files.**
  `identity.seed` is written with plain file permissions on Windows
  (`crates/modelswarm-node/src/lib.rs:608-622`, honesty comment: DPAPI /
  Credential Manager is deferred Phase F hardening per ADR-004);
  `session.token` and `lease.id` are plaintext in the data dir
  (`app.rs:979`, `app.rs:1109`).
- **M2 — Blocking calls inside async.** `detect_hardware` runs
  `std::process::Command…--list-devices` synchronously in an async fn
  (`app.rs:521-539`; the Vulkan engine can take seconds on a cold driver
  cache — it is in a `OnceCell`, so once per process, but it stalls the
  tauri async runtime); `Store::open` (sync rusqlite) and sync `fs` calls
  run inside async IPC commands and `DesktopState::new().await`
  (`app.rs:104-128`, `app.rs:806-808`); `verify_engine_variant` reads whole
  engine files into memory to hash them inside async `Node::start`
  (`engine.rs:184-241`).
- **M3 — Desktop opens a second telemetry FileSink on the same file.**
  `license_accepted()` calls `open_node_telemetry` per invocation
  (`app.rs:147-159`) while `DesktopState` holds "the ONE process telemetry
  sink" (`app.rs:46-49`) — violates the crate's own stated invariant
  (append-mode makes it benign, but it is the pattern the comment forbids).
- **M4 — Protocol document vs implementation drift on the P2P protocol id
  and handshake.** `protocol/msp-v1.md:249` specifies protocol id
  `/msp/infer/1` and §6.1 a handshake exchange; the production libp2p backend
  opens **raw QUIC streams without multistream-select and no protocol id**
  (`libp2p_backend.rs:1-36`; confirmed by the F2A doc: "our transport opens
  raw QUIC streams without multistream-select"). Handshake/HandshakeAck
  frames exist and are used only by the staged TCP backend
  (`transport/src/handshake.rs`, sim-only). Also blocks F2b relay use
  (RESERVE/HOP are negotiated protocols).
- **M5 — Artifact downloads cannot resume.** `ensure_artifact` streams to a
  `.part` file with atomic rename and full sha256+GGUF-identity
  verification, but "No resume in v1 — a dropped connection restarts the
  transfer" (`crates/modelswarm-node/src/artifact.rs:15-17`). A dropped
  multi-GB download restarts from zero; no disk-space precheck either.
- **M6 — The relay is unusable by MSP nodes (F2b missing).** `modelswarm-relay`
  is a complete, verified standalone binary, but no node-side client exists
  (`docs/verification/f2a-relay-2026-10-08.md` "Honest boundary": "MSP nodes
  cannot yet USE it"). NAT traversal beyond same-LAN direct connect is
  therefore absent; AGENTS.md constraint 7 honesty holds.
- **M7 — ChatML is hardcoded in the serving executor** (`executor.rs:93-104`,
  ADR-025): only ChatML-family profiles may join the catalog; a non-ChatML
  profile would be silently mis-templated. Documented, but a correctness
  cliff for catalog growth.

### LOW

- **L1** — `eprintln!` diagnostics in library code on the serving path
  (`crates/modelswarm-node/src/remote.rs` — 6 sites) are invisible in the
  windowed desktop; `serving.rs` correctly uses telemetry. Inconsistent.
- **L2** — Gateway `audit` is a process-global ring (4096 lines,
  `gateway/src/lib.rs:235-258`) with no production consumer (only tests
  drain it). Bounded, but dead weight in production.
- **L3** — `FailoverExecutor` (`remote.rs:355-387`) is production-dead: the
  desktop drives `RemoteExecutor` directly with its own fallback
  (`app.rs:1395-1401` says so explicitly); only tests construct it.
- **L4** — Duplicated `#[cfg(feature = "libp2p-backend")]` attribute pairs
  (`crates/modelswarm-node/src/lib.rs:54-59`) and two independent
  `include_str!` copies of the pinned hub key
  (`node/src/serving.rs:939`, `node/src/catalog.rs:17`).
- **L5** — `catalog/candidate-profiles/README.md:12` still points candidate
  validation at `catalog/schema.json` (v1); the resolver actually validates
  against `schema-v2.json` (`scripts/resolve-candidate.mjs:360`). Stale doc
  reference; v1 schema is superseded (ADR-011) and referenced nowhere in code.
- **L6** — Root `tests/` is a README pointing at per-crate tests
  (documented ADR-010 consequence, not an issue).
- **L7** — Desktop UI shows hardcoded `http://127.0.0.1:11435/...` while
  hosting is off (`ui/index.html:416`); it switches to the real
  `gateway_addr` (which may be an ephemeral fallback port) once hosting is
  on (`app.rs` `set_hosting` port-conflict fallback, `node/lib.rs:255-263`).
  Correct when it matters; cosmetic otherwise.

---

## 2. Ground-truth claim verification (master prompt §0)

1. **Cross-machine P2P serving over QUIC on LAN with real models —
   CONFIRMED, with one caveat.** Evidence chain: desktop
   `set_hosting` → `bind_serving_listener` (opt-in `MSP_LISTENER=1`,
   `app.rs:248-294`) → `modelswarm_node::serving::serve_sessions` attached to
   the node's executor (`app.rs:911-931`) → `SingleLocalExecutor` →
   `LlamaCppAdapter` → supervised pinned llama-server
   (`engine.rs:242-345`). Requester side: `send_chat` → `try_swarm_chat` →
   `RemoteExecutor` over `Libp2pTransport::dial` with PeerId verification
   (`remote.rs:188-237`, `libp2p_backend.rs:286-321`). Live two-machine proof
   with real Qwen2.5-7B weights on the production tracker:
   `docs/verification/phase-f-lan-2026-10-07.md` (24 remote tokens, 1.6 s).
   Feature wiring: shipped installer builds `--features tauri-shell`
   (`.github/workflows/desktop-installer.yml:118`) which enables
   `modelswarm-node/libp2p-backend` (desktop Cargo.toml dep). **Caveat:
   default-off** (see H3); CI covers the feature explicitly
   (`.github/workflows/rust.yml:40-47`).
2. **modelswarm-scheduler is bench-only and unwired — CONFIRMED.** Sole
   importer is `modelswarm-bench` (`crates/modelswarm-bench/src/lib.rs:39`).
   Neither `modelswarm-node`, `modelswarm-gateway`, nor `modelswarm-desktop`
   depend on it (workspace manifests); the shipped gateway/desktop select
   first-found peer (H2). Cost model v2 (`predicted_swarm_ms`,
   `should_engage_cooperative`, `select_microswarm`,
   `scheduler/src/lib.rs:133-258`) exists, is frozen-rule-compliant, and has
   15 unit tests — but executes only inside the mock bench harness.
3. **modelswarm-speculation is mocks/loopback only — CONFIRMED.** Consumers:
   `modelswarm-session/src/spec.rs` (sim-gated), `modelswarm-bench`,
   `apps/modelswarm-sim`. No SpecMessage ever crosses the production QUIC
   path; the sim's spec scenarios run against `MockRuntime` over the staged
   TCP transport (`sim/src/lib.rs:29-56`). `LlamaCppAdapter::propose` exists
   (`llamacpp.rs:553+`) but is called by nothing in production.
4. **modelswarm-relay exists, F2A-verified 2026-10-08 — CONFIRMED, and it is
   SEPARATE, not shipped, not opt-in.** Standalone binary crate (deps: libp2p
   only; no workspace crate imports it). Deployed manually on the owner's
   second desktop per the F2A doc. Nodes cannot use it at all (F2b client
   missing — M6/M4). Classification: infrastructure waiting for its client.
5. **modelswarm-winjob = kill-on-close job objects + total RAM — CONFIRMED.**
   `assign_child`/`assign_kill_on_close_job`
   (`winjob/src/lib.rs:75-135`) + `total_ram_bytes` via `GlobalMemoryStatusEx`
   (`:49-58`). Used by `node/engine.rs:441` (engine kill-on-close) and
   `desktop/app.rs:483` (RAM detection for model fit). No CPU, priority,
   VRAM, thermal, or battery control — the M2 governor seed, exactly as
   claimed.
6. **Shipped client surface — CONFIRMED.** Start/Stop per model card
   (`ui/index.html:256-277`), chat (`send_chat`), state chips (hosting,
   tracker, listener, enrollment view), local API address with copyable curl
   snippet (`ui/index.html:179,416`); default
   `http://127.0.0.1:11435/v1` (`DEFAULT_PORT = 11_435`,
   `gateway/src/lib.rs:52`; node binds it, desktop falls back to ephemeral on
   conflict and the UI then shows the real address).
7. **No mock in the production inference path — CONFIRMED.** `MockRuntime`
   compiles only under the off-by-default `mock-runtime` feature;
   `Node::start` refuses `mock:true` without `cfg(test)`/`node-selftest`
   (`node/src/lib.rs:501-537`, loud banner otherwise); the desktop hardcodes
   `mock: false` (`app.rs:895`). The production chain is
   gateway → SingleLocalExecutor → LlamaCppAdapter → pinned llama-server,
   all real. Every cooperative mode (speculative, multi-proposer trees,
   receipts-v2 receipts in-session) runs only in sim/bench with the mock
   runtime.
8. **Multi-peer-per-request transport exists but is unproven on real
   networks — CONFIRMED.** `speculate_multi` (2..=7 proposers, bounded
   `CandidateTrie`, straggler drop) exists in `session/src/spec.rs` and is
   exercised only by `apps/modelswarm-sim` (`spec_multi` scenario, loopback
   TCP). No real-network run recorded in `docs/verification/`.

**ADR-016 note:** absent from `docs/adr/` (001–015, 017–026). It is
referenced only by `docs/reviews/phase-a-first-response.md:94,116` and
`docs/verification/phase-a.md:23,59`, which record it as the *conditional*
p2ptokens code-reuse ADR, "closed as not-needed (no code reuse adopted)".
**No code references it.** The gap in the numbering is documented
in-repo; not an anomaly, though a tombstone line in the ADR index would
make that obvious to future readers.

---

## 3. Workspace dependency map (who is actually shipped)

Production path (compiled into the v0.2.20 Windows installer):
`modelswarm-desktop` (tauri-shell) → `modelswarm-node` (libp2p-backend) →
{`modelswarm-gateway` → `modelswarm-runtime`/llamacpp; `modelswarm-transport`
(libp2p QUIC); `modelswarm-identity`, `modelswarm-eligibility` (lease gate),
`modelswarm-store`, `modelswarm-telemetry`, `modelswarm-tracker-api`,
`modelswarm-types`} + `modelswarm-winjob` (Windows target dep).

Bench/sim-only (compiled by `cargo test --workspace` but into no shipped
binary): `modelswarm-scheduler` (bench), `modelswarm-speculation`
(session/bench/sim), `modelswarm-session` (sim), `modelswarm-bench`,
`apps/modelswarm-sim`, `modelswarm-runtime::mock`.

Standalone: `modelswarm-relay` (separate host binary); `apps/tracker`
(Vercel control plane).

---

## 4. Per-component inventory

### 4.1 Rust crates (17)

| Crate (module) | Actual purpose (evidence) | Production wiring | Tests (count, quality) | Placeholders / risks | Decision |
|---|---|---|---|---|---|
| `modelswarm-types` (canonical, gguf, manifest) | Frozen protocol types; canonical JSON; ADR-011 manifest→`msp1:` id derivation; GGUF v3 metadata + ADR-022 identity hashes re-derivation (`manifest.rs`, `gguf.rs`) | Imported by 10 crates; catalog verification depends on it | 23; strong incl. golden-vector parity | None | **KEEP** |
| `modelswarm-identity` (installation, envelope, canonical, timestamp) | Ed25519 identity, signed envelopes, nonce/replay windows, ADR-020 peer-id derivation (`lib.rs:1-20`) | Every signed hub call + QUIC PeerId | 16; good | Seed handling delegated to node (see M1) | **KEEP** |
| `modelswarm-eligibility` (lease, policy, canonical) | Hub-signed EligibilityLease issue/verify; suspension ladder (`lib.rs:1-17`) | Serving-side lease gate (`serving.rs:941-1001`); tracker issues leases | 20; good | Bootstrap allowance experiment flag off by default | **KEEP** |
| `modelswarm-transport` (frame, message) | 4-byte length-prefixed JSON frames, ≤256 KiB validated before allocation (`libp2p_backend.rs:145-161`); frozen wire vocabulary `message.rs` | Both backends + sim | 36 total; good | — | **KEEP** |
| `modelswarm-transport` (handshake + staged `SignedFrameTransport`, TCP loopback) | ADR-018 Phase C staging transport with Ed25519 handshake | **Sim/test only** (ADR-018 swap point); no shipped binary path | loopback tests | Superseded by libp2p for production; retained deliberately | **KEEP** (sim harness) |
| `modelswarm-transport::libp2p_backend` | QUIC transport + listener with dedicated driver task; PeerId = ADR-020 derivation verified at dial (`libp2p_backend.rs:1-36, 286-321`) | Production P2P path (feature-gated, shipped via desktop) | loopback round-trip, refusal, peer-id parity | Raw streams, no multistream/protocol-id (M4); no relay/DCUtR | **KEEP WITH TESTS** |
| `modelswarm-runtime` (trait `lib.rs`) | Runtime abstraction: load/tokenize/detokenize/prefill/decode_step/decode_stream/propose/cancel + metrics | Production | 26 total | `cancel` best-effort only | **KEEP** |
| `modelswarm-runtime::llamacpp` | HTTP adapter to pinned llama-server; exact token-id recovery w/ vocab + fail-closed disagreement tripwire (`llamacpp.rs:411-506`; F2A fix) | Production baseline runtime | strong (regression tests for special tokens) | **H1**: 1 token/POST, prefix re-send, batch-at-end streaming | **REFACTOR** |
| `modelswarm-runtime::mock` | Deterministic TEST-ONLY decoder (ADR-019), feature-gated off | None (test/bench/sim) | covered via consumers | None — discipline proven (claim 7) | **KEEP** |
| `modelswarm-gateway` (http, lib) | OpenAI-compatible loopback API; §6.2 normalization; §6.4 clamp-don't-reject with `x-msp-clamped`; ADR-007 retry state machine (≤2 retries, only pre-first-token); SSE | Production local API (`node/lib.rs:388-420`) | 19; excellent (SSE shape, retry, privacy sentinels) | L2 audit ring; `serve`/`spawn_loopback` test helpers | **KEEP WITH TESTS** |
| `modelswarm-gateway::doubles` | StaticExecutor/QueueStream test doubles, `test-doubles` feature | None external (feature currently unused outside crate) | own tests | — | **KEEP** |
| `modelswarm-node` (lib, engine, catalog) | Daemon composition: telemetry/store/identity/tracker/engine supervision (hash-verified pins, GPU fallback visible, bounded restarts, kill-on-close), loopback bind, data-dir migration (`lib.rs`, `engine.rs`) | Production core (desktop runs it in-process; CLI binary also exists) | 37; excellent (incl. env-gated real-engine + LAN tests, privacy column audit) | M2 blocking engine-file hashing; L4 dup cfg attrs | **KEEP WITH TESTS** |
| `modelswarm-node::executor` (SingleLocalExecutor) | Local executor: ChatML render (ADR-025), honest NoPeer, closed error-code table, batched deltas | Production serving + local | good | **H1**: full-decode-then-emit; M7 ChatML hardcode | **KEEP WITH TESTS** (rework rides H1) |
| `modelswarm-node::serving` | Serving bridge: lease gate at session open + per request (ADR-026), §6.4 clamps, admission caps (8 sessions/2 per peer), replay dedup window, pooled sessions | Production (feature-gated; enabled by shipped desktop) | QUIC round-trips incl. foreign-lease/leaseless refusals | Session cap constants are prototype-sized | **KEEP WITH TESTS** |
| `modelswarm-node::remote` (RemoteExecutor, FailoverExecutor) | Requester side: pooled QUIC session reuse, ADR-007 error mapping; FailoverExecutor remote-first/local-fallback | RemoteExecutor: production (swarm chat). FailoverExecutor: **dead** (L3) | reuse + failover tests | L1 eprintln; L3 | **KEEP WITH TESTS** |
| `modelswarm-node::artifact` (ArtifactManager) | HF download → streamed sha256 + GGUF identity re-derivation → atomic rename → store row; re-verified on every start | Production (download/host flows) | canned-host tests | **M5** no resume, no disk-space precheck | **REFACTOR** |
| `modelswarm-node::selftest` | ADR-019 no-network self-test (mock, node-selftest feature) | None (diagnostic) | exercised by node tests | — | **KEEP** |
| `modelswarm-desktop` (app.rs, main.rs, ui/) | Tauri 2 shell: 14 IPC commands (state-only returns), model picker w/ per-machine fit scoring (`fit_and_score`), hardware detection (RAM FFI + `--list-devices`), swarm-first chat with honest mode labels, HF picker (ADR-023), enrollment heartbeat task, single-instance | Production UI (v0.2.20 installer) | 7 unit + tauri-shell CI job; UI logic largely untested (static HTML/JS) | M1-M3, H2, H3; `withGlobalTauri: true` broadens webview surface (CSP is tight) | **KEEP WITH TESTS** |
| `modelswarm-scheduler` | Pure selection/scoring: §10 v1 formula, cost model v2, engage rule w/ margin floor, EWMA, NAT-path penalties (ADR-014) | **Bench-only** (claim 2) | 15; frozen-rule tests incl. 10×-liar dominance | Unwired (H2) — keep for M9/M10 planner | **KEEP WITH TESTS** (awaiting wiring) |
| `modelswarm-session` (lib = commit-gate state machine, spec = Phase D/E speculative sessions) | Signed commit chain, rollback, receipts; `speculate`/`speculate_multi` (2..=7 proposers, tries, stragglers, bounded fallback) | **Sim-only** (cooperative-plan P-gate holds) | 45 incl. adversarial, fuzz, property, multi-e2e | Greedy-only on wire; sampled rules unit-level | **KEEP WITH TESTS** (sim-gated) |
| `modelswarm-speculation` (acceptance, trie, rng, metrics) | Pure lossless acceptance rules (greedy + sampled full-q), candidate tries, SplitMix64 | Via session (sim) + bench | 19 incl. property tests (token-exactness under adversarial drafts) | Needs runtime with distributions for sampled wiring | **KEEP WITH TESTS** |
| `modelswarm-bench` (lib, records, stats) | ADR-013 harness: fastest-single comparator, network matrix, Ajv-validated records, mock labeling (`TEST_ONLY_MOCK_LABEL`), negatives recorded | Bench-only | 27 incl. schema validation of emitted records | All numbers mock/synthetic by design | **KEEP WITH TESTS** |
| `modelswarm-store` | SQLite, append-only migrations, privacy-shaped schema w/ structural audit | Production | 6 integration tests incl. forbidden-column sweep | Sync rusqlite in async contexts (M2) | **KEEP WITH TESTS** |
| `modelswarm-telemetry` (lib, redact) | JSONL/mem sinks, mandatory redactor, in-process metrics | Production | 11 | — | **KEEP** |
| `modelswarm-tracker-api` | Typed client: signed envelopes, enrollment, register/heartbeat/drain, challenge/lease, verified catalog fetch (`fetch_catalog_verified`) | Production (node+desktop) + wire-compat harness (`node/tests/live_tracker.rs`) | 8 | — | **KEEP WITH TESTS** |
| `modelswarm-winjob` | The workspace's only `unsafe_allow` crate: kill-on-close jobs + total RAM | Production (engine child + RAM probe) | 1 real-process test | Exactly the M2 seed (claim 5) | **KEEP** (extend in M2) |
| `modelswarm-relay` | Standard libp2p circuit-relay v2 host; stream-carrying limits (no duration/byte caps); persistent key | **Separate binary**; unusable by nodes until F2b (M6) | 2 incl. real in-process circuit test | No access control by design (content-blind mover) | **KEEP WITH TESTS** |

### 4.2 Apps

| App | Purpose vs docs | Production | Tests | Decision |
|---|---|---|---|---|
| `apps/tracker` (Next.js 15 + Postgres) | Control plane: 12 API route groups (auth, peers, catalog+admin, rendezvous, lease/challenge, session-authorize, receipts, stats, downloads), MemoryStore/PgStore, zod-strict, Ed25519 envelopes, bounded rendezvous mailboxes, content-blind (prompt-field rejection at edge) | Live at modelswarm.deepflux.space via Vercel CLI | 120 vitest tests in 15 files + PG integration job in CI (`tracker.yml`) + golden-vector wire parity (`validate:vectors`, `crypto-vectors.test.ts`) — the best-tested component in the repo | **KEEP WITH TESTS** |
| `apps/modelswarm-sim` | Deterministic loopback scenario runner: pair/mesh/kill/spec/spec_multi/relay (TCP stand-in) | None (Test & Release owned) | 7 scenario tests | **KEEP WITH TESTS** |

### 4.3 Protocol, catalog, scripts, installer, CI, experiments

| Component | Purpose / state | Evidence | Decision |
|---|---|---|---|
| `protocol/msp-v1.md` (339 lines) | Frozen v1 contracts: envelopes, hub REST, leases, P2P stream, receipts | Implemented with two drifts: protocol-id/handshake (M4) and receipts — Rust wire intentionally omits receipt frames (`message.rs:140-151`), tracker accepts `/events/job-result` + `/receipt` | **KEEP WITH TESTS** (close M4 via ADR when F2b lands) |
| `protocol/vectors/` (6 files) | Golden vectors: manifests (real + synthetic), lease-hubkey byte-pinned (ADR-026), validated in Rust tests + TS `validate:vectors` + CI Rust↔TS wire-compat job | byte-pinned, CI-enforced | **KEEP** |
| `protocol/keys/hub-public.hex` + `apps/tracker/keys/` | Pinned hub verifying key; seed lives only in Vercel env | `keys/README.md` | **KEEP** |
| `protocol/messages.proto` | Proto mirror of wire messages (receipts included) | advisory; Rust uses JSON frames | **KEEP** |
| `catalog/schema-v2.json` | ADR-011 manifest schema; enforced by Rust type + TS zod + Ajv in resolver | resolver `:360` | **KEEP** |
| `catalog/schema.json` (v1) | Pre-ADR-011 catalog shape; superseded; only stale README reference remains (L5) | `candidate-profiles/README.md:12` | **KEEP** (historical) + fix README |
| `catalog/candidate-profiles/` (15 manifests) | Resolver output awaiting promotion; all schema-v2, fail-closed resolver | `scripts/resolve-candidate.mjs` | **KEEP** |
| `scripts/resolve-candidate.mjs`, `promote-candidates.mjs` | GGUF→manifest resolution w/ identity hashes; promotion path to tracker | used for catalog ladder | **KEEP WITH TESTS** (add dry-run CI if absent) |
| `installer/stage-engine-windows.mjs` | Fail-closed engine staging (sha256 vs pins, CPU+Vulkan sets); "a release built without this ships a dead installer" | header `:1-14`; CI verifies engine inside artifacts (`desktop-installer.yml:125+`) | **KEEP WITH TESTS** |
| `installer/tauri/nsis-notes.md`, README | Packaging notes (Phase 0 text retained) | — | **KEEP** |
| `.github/workflows/rust.yml` | windows-latest: fmt, clippy -D warnings, tests, **plus libp2p-backend feature job and tauri-shell desktop tests**; guards: no committed binaries, single-sourced version, download-filename parity | `:28-47, 52-85` | **KEEP** |
| `.github/workflows/tracker.yml` | typecheck/build/vectors + disposable-Postgres integration | `:14-60` | **KEEP** |
| `.github/workflows/desktop-installer.yml` | 4-OS matrix, pinned engine fetch+verify, `--features tauri-shell` build, engine-inside-artifact check | `:38-125` | **KEEP** |
| `.github/workflows/real-model-e2e.yml`, `windows-package-dry-run.yml` | Env-gated real-model smoke (identity-hash parity, determinism); node release smoke | workflow_dispatch | **KEEP** |
| `experiments/schemas/` | mode-result + run-manifest JSON schemas, Ajv-validated in bench tests | `bench/tests/harness.rs:35-69` | **KEEP** |
| `runtime-pins.json` | b11407 pins, canonical build hash, 4 platforms + vulkan variant; embedded at compile time; fail-closed | `engine.rs:32-104`, tests | **KEEP** |
| `tests/` (README) | Pointer doc to per-crate tests (ADR-010) | — | **KEEP** |

---

## 5. Cross-cutting sweep results

- **Placeholders:** zero `TODO`/`FIXME`/`todo!`/`unimplemented!` in non-test
  code. `unreachable!()` appears 7× — all provably-unreachable match arms in
  tests/bench (`bench/lib.rs:949`, `serving.rs` tests, `session/spec.rs:1605`,
  `handshake.rs:184,231`). Mock/stub/placeholder vocabulary is confined to
  the ADR-019-gated mock runtime and two doc comments (tracker Phase B–E
  placeholder note, desktop stub main). The codebase is unusually clean here.
- **unsafe:** exactly one crate (`modelswarm-winjob`), which sets
  `unsafe_code = "allow"` locally against the workspace `forbid`
  (winjob Cargo.toml comment documents the design). Nothing else.
- **unwrap/expect:** hundreds, overwhelmingly in `#[cfg(test)]` modules.
  Production-path spots audited (serving/remote/engine/executor) use
  `expect` only on lock acquisition (`serving.rs:194` "admission lock") and
  provably-valid statics (`LeasePolicy::production` on the pinned key). No
  caller-controlled input hits an unwrap on the serving path.
- **Unbounded resource use:** none found in production paths — admission
  caps + replay window with hard cap 8192 (`serving.rs:222-238`), 20-turn
  chat log cap (`app.rs:1682-1685`), 10-min artifact size cache, audit ring
  4096, MemSink ring. Frame size ≤256 KiB enforced pre-allocation.
- **Cancellation/timeouts/backpressure:** deadlines on every wire IO
  (30 s IO, 10 s dial, 300 s between-requests, 120 s gateway deadline clamp);
  engine health timeout + bounded restarts (3); graceful shutdown via watch
  channel wired to engine kill, listener, and gateway. Cancellation of an
  in-flight decode is best-effort (`/cancel` tolerated 404) — mostly moot
  today because decode completes before first emit (H1).
- **Duplication:** three deliberate mirrors (staged transport ↔ libp2p
  backend; executor.rs batching ↔ session spec batching; two hub-key
  includes L4). No harmful copy-paste found.
- **Race conditions:** heartbeat task writes `session.token`/`lease.id` while
  other tracker clients read them (benign last-writer-wins, acknowledged in
  code comments). No other shared-state races spotted; admission uses
  std Mutex with no await inside the critical section.
- **Platform assumptions:** Windows-first (NSIS, winjob, CREATE_NO_WINDOW,
  LOCALAPPDATA) with real Linux/macOS pins and CI matrix; desktop total-RAM
  detection has Linux/macOS paths (`app.rs:480-515`).

---

## 6. Salvage matrix (feeds the orchestrator)

| Component | Current role | Evidence | Decision | Reason | Dependencies |
|---|---|---|---|---|---|
| modelswarm-types | Frozen core types, canonical JSON, manifests, GGUF hashes | 10 importers, golden vectors | KEEP | Foundation, correct, tested | — |
| modelswarm-identity | Ed25519 identity, envelopes, peer-id | All signed paths | KEEP | Foundation | types |
| modelswarm-eligibility | Leases, suspension | serving.rs gate | KEEP | Enforces project rule | types |
| modelswarm-transport (staged) | TCP loopback + handshake | sim only | KEEP | Sanctioned sim/test harness | identity |
| modelswarm-transport::libp2p_backend | QUIC P2P | F0/F1 LAN proof | KEEP WITH TESTS | Production transport; needs soak + protocol-id/multistream (M4) | identity |
| modelswarm-runtime::llamacpp | llama.cpp HTTP adapter | executor path | REFACTOR | H1 streaming/1-token-per-POST | types |
| modelswarm-runtime (trait+mock) | Abstraction + test double | — | KEEP | Clean seam (ADR-018/019) | types |
| modelswarm-gateway | OpenAI loopback API + retry machine | node wiring | KEEP WITH TESTS | Solid; audit-ring cleanup optional | runtime |
| modelswarm-node (lib/engine/catalog/selftest) | Daemon composition + engine supervision | desktop in-process | KEEP WITH TESTS | Production core | most crates |
| modelswarm-node::executor | Local serving executor | both paths | KEEP WITH TESTS | Rides H1 rework | gateway, runtime |
| modelswarm-node::serving | Lease-gated serving bridge | F0 | KEEP WITH TESTS | Enforcement point (ADR-026) | transport, eligibility, gateway |
| modelswarm-node::remote (RemoteExecutor) | Requester QUIC executor + pool | F1 + swarm chat | KEEP WITH TESTS | Production requester side | transport, gateway |
| modelswarm-node::remote (FailoverExecutor) | Remote-first fallback wrapper | tests only (app.rs:1398) | KEEP WITH TESTS | Dead in prod; either wire it or fold into planner | gateway |
| modelswarm-node::artifact | HF download + verify | download/host flows | REFACTOR | M5 resume/disk checks | store, types |
| modelswarm-desktop (+ui) | Product UI | v0.2.20 installer | KEEP WITH TESTS | Ship surface; fix M1-M3, H2 selection, H3 listener toggle | node, gateway, tracker-api, winjob |
| modelswarm-scheduler | Selection + cost model v2 | bench only (claim 2) | KEEP WITH TESTS | Unwired but frozen-correct; wire in M9/M10 | types |
| modelswarm-session | Cooperative sessions | sim only | KEEP WITH TESTS | P-gate holds; Phase D/E assets | gateway, transport, speculation, identity |
| modelswarm-speculation | Lossless acceptance algorithms | session/bench/sim | KEEP WITH TESTS | Property-proven pure core | — |
| modelswarm-bench | Mock benchmark harness | bench | KEEP WITH TESTS | Honest, labeled, schema-validated | runtime(mock), scheduler, speculation |
| modelswarm-store | SQLite privacy-shaped state | node+desktop | KEEP WITH TESTS | Schema audit is exemplary | — |
| modelswarm-telemetry | Redacted logs/metrics | everywhere | KEEP | Mandated redactor | — |
| modelswarm-tracker-api | Typed tracker client | node+desktop+LAN test | KEEP WITH TESTS | Wire-compat proven | identity |
| modelswarm-winjob | Job objects + RAM FFI | engine + hardware probe | KEEP | M2 seed (claim 5) | — |
| modelswarm-relay | Circuit-relay v2 host | standalone, F2A-verified | KEEP WITH TESTS | Needs F2b client to matter | libp2p |
| apps/tracker | Control plane | production Vercel | KEEP WITH TESTS | Best-tested component | — |
| apps/modelswarm-sim | Scenario runner | sim | KEEP WITH TESTS | Gate evidence generator | session, transport, runtime(mock) |
| protocol/ (msp-v1, vectors, keys) | Contracts + golden vectors | CI-enforced | KEEP WITH TESTS | Close M4 drift via ADR | — |
| catalog/ (v2 schema, candidates) | Manifest schema + pipeline | resolver/promotion | KEEP | Fix stale v1 README ref (L5) | — |
| installer/ + scripts/ | Engine staging + catalog tooling | release pipeline | KEEP WITH TESTS | Fail-closed, CI-guarded | runtime-pins |
| .github/workflows (5) | CI: feature jobs, guards, PG integration, installers | rust.yml:40-47 | KEEP | The libp2p/tauri feature jobs are load-bearing | — |
| runtime-pins.json / experiments/ / tests/ | Pins, record schemas, pointer | engine verify, bench tests | KEEP | — | — |

Classification counts: KEEP 17 · KEEP WITH TESTS 15 · REFACTOR 3 · REPLACE 0 ·
DELETE 0 · QUARANTINE 0 · NEEDS EXPERIMENT 0 (at component level; the
"needs experiment" items — canary auditing, reciprocity supply — are design
questions already tracked in the gap study, not code components).
