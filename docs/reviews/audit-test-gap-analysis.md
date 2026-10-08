# Test-Gap Audit — CI and Test Estate vs Master Prompt §13

- Date: 2026-10-09
- Auditor: Test and Release Engineer
- Branch: `audit/master-prompt-2026-10-09` (frozen baseline `730cebf`, workspace version 0.2.20)
- Scope: all five CI workflows, all 17 Rust crates, `apps/modelswarm-sim`, `apps/tracker` test suite, `protocol/vectors`, `docs/verification` evidence inventory, feature-gating CI coverage, release-gate posture.
- Method: read-only inspection plus `gh` queries against GitHub Actions history. No cargo/npm runs started by this audit (a full local freeze-validation suite ran concurrently; snapshot below). No production systems touched.

---

## 1. Local validation suite snapshot (from `target/audit-logs/summary.txt`)

Suite run by `target/audit-logs/run-freeze-validation.sh` on this machine, branch/head as above. Read at 2026-10-09T07:14+08:00 — **complete, all green**:

| Step | Result | Duration |
|---|---|---|
| `cargo fmt --all --check` | PASS | <1 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS | 11 s (warm target) |
| `cargo test --workspace` | PASS | ~2 m |
| clippy with `--features modelswarm-node/libp2p-backend` | PASS | ~1 s (warm) |
| tests with `--features modelswarm-node/libp2p-backend` | PASS | ~1 m 50 s |
| `cargo test -p modelswarm-desktop --features tauri-shell` | PASS | 11 s |
| tracker `npm run typecheck` / `test` / `build` / `validate:vectors` | PASS (each) | 3 s / 2 s / 21 s / 2 s |

Two caveats on reading this snapshot:

1. **The desktop tauri-shell tests passed locally only because this dev machine had the pinned engine staged** under `crates/modelswarm-desktop/engine/` (gitignored). The identical CI step failed on clean runners at audit start — resolved during the audit, see §2.1 (RESOLVED-during-audit).
2. Local tracker `npm test` runs 110/112 cases; the 2 `pg-enrollment` PgStore cases skip without `DATABASE_URL` (`14 passed | 1 skipped` files). Only CI's `integration-postgres` job exercises them against real Postgres.

---

## 2. CI inventory

### 2.1 `rust.yml`

- **Triggers**: push to `main` + PR, path-filtered (`crates/**`, `apps/**`, root manifests, `protocol/**`, `catalog/**`, `runtime-pins.json`, workflow file); `workflow_dispatch`. Concurrency group with `cancel-in-progress`.
- **Job `check`** (windows-latest, 90 min, pinned toolchain `dtolnay/rust-toolchain@1.99.0` matching `rust-toolchain.toml`): fmt → clippy `-D warnings` (all targets) → workspace tests → **clippy + tests again with `--features modelswarm-node/libp2p-backend`** → desktop tests with `--features tauri-shell`.
- **Job `guards`** (ubuntu): no committed binaries (git-tracked `.exe/.dll/.gguf/.deb/.AppImage/.dmg/.msi/.zip/.7z/.bin` fail the build); version single-sourced (tauri.conf.json must not set `version`; workspace version must be plain semver; `apps/tracker/app/page.tsx` download filenames must embed the exact version).
- **libp2p-backend fix CONFIRMED**: lines 40–43 compile and test the serving bridge, lease gate (ADR-026), and `RemoteExecutor`. This is the direct fix for the 2026-10-08 lease interop break that shipped because the feature was never compiled in CI. The fix is real and covers the known hole.
- **Holes**:
  - **(H1) RESOLVED during audit — clean-runner desktop-test failure (commit `63b98e1`, 2026-10-09).**
    - **As found**: the `Desktop tests (tauri-shell feature)` step had never been green on a clean runner since its introduction in `c4399a8` (both runs 37792918516 and 37796497342 red; the latter is baseline `730cebf` itself, 2026-10-08T15:11Z). Root cause: tauri-build 2.7.1 (tauri-utils `ResourcePaths`) validates the `tauri.conf.json` `bundle.resources` globs — `["engine/*", "engine-vulkan/*"]` — inside the build script; engines are gitignored and only `desktop-installer.yml` stages + hash-verifies them, so on a clean runner the globs match nothing and the build script exits 101 with `glob pattern engine/* path not found or didn't match any files`. Local runs passed only because the dev machine had both engine dirs staged. The custom `build.rs` `MSP_ALLOW_NO_ENGINE` gate is NOT involved — it is release-only (`!cfg!(debug_assertions)`) and `cargo test` builds in debug.
    - **Fix (verified in `origin/main` `63b98e1`, workflow-only, +6 lines)**: the step now sets `TAURI_CONFIG: '{"bundle":{"resources":[]}}'`, stripping bundle resources for the test-only build. Local proof per the fix commit: both engine dirs renamed away reproduces the exact CI failure (exit 101, same glob error); with the overlay, 7/7 desktop tests pass. GitHub run 37858753428 (push of `63b98e1`) was **in progress** at last read (2026-10-09T07:2x+08:00); the local bare-runner reproduction is the acceptance evidence recorded here.
    - **Residual risk of the overlay (assessed)**: (a) the test-only build now differs from the shipping build in one config dimension — `bundle.resources` — so this step no longer validates the resource globs at all; a future resource-config regression (typo, dropped `engine-vulkan/*`, path change) merges red-free and is caught only post-merge by `desktop-installer.yml`, whose "Verify every pinned engine file" + "engine INSIDE each built artifact" steps check real presence AND per-file hashes inside built bundles — strictly stronger than glob validation, but push-to-main only (never PR). (b) none of the 7 current desktop tests touch resource resolution (they cover quant-filename parsing, HF repo validation, runtime descriptor shape, fit/score, no-GPU ranking, vulkan list-devices parsing, conservative requirements), so no existing assertion is weakened; but a future resource-path test would silently run against an empty resource set. (c) the release-only `build.rs` engine gate still protects local release builds. **Recommended cheap hardening**: add a `guards`-job assertion that `tauri.conf.json` `bundle.resources` equals exactly `["engine/*", "engine-vulkan/*"]`, restoring pre-merge config-drift detection at zero runner cost.
  - Path filter misses `tests/**` (repo root; currently README-only, benign) and `installer/**` (staging script changes don't re-run the job whose breakage they'd fix).
  - No `timeout-minutes` on `guards` default is fine, but the workflow has no branch-protection/required-check evidence (could not be verified read-only).

### 2.2 `tracker.yml`

- **Triggers**: push `main` + PR, path-filtered (tracker app, `protocol/vectors/**`, `catalog/**`, tracker-api/identity/eligibility crates, `crates/modelswarm-node/tests/**`, toolchain, workflow). No concurrency group (superfluous runs queue instead of cancelling — minor cost, no correctness issue).
- **`check`** (ubuntu, node 22): `npm ci`, typecheck, build, `validate:vectors` (Ajv validation of every `manifest-*.json` against `catalog/schema-v2.json` + recomputed canonical-JSON profile id).
- **`integration-postgres`**: disposable Postgres 16 service; `db:migrate` → `npm test` (full vitest suite incl. env-gated PgStore tests) → second `db:migrate` must be a no-op (migration idempotency, A1).
- **`wire-compat`** (45 min): starts a REAL built tracker over real Postgres with a deterministic CI-only signing seed (`MSP_HUB_SEED`, public half pinned in `protocol/vectors/lease-hubkey-1.json`), seeds and promotes a real catalog candidate, then runs `cargo test -p modelswarm-node --features libp2p-backend --test live_tracker -- --ignored` with `MSP_LIVE=1`. The harness exercises every signed route the desktop uses, in order: device enroll → register → heartbeat → lookup → challenge_start → challenge_complete → request_lease → **ADR-026 Rust serving-gate verification of the TS-issued lease** → drain. This is the structural fix for the shipped wire-bug class (envelope casing, query-string signing, null-body GET digest, 2026-10-08 lease-issuance shape) and it is correctly wired.
- **Holes**: wire-compat last ran green 2026-10-08T14:29Z (run 37792918339); the baseline commit `730cebf` did not re-trigger it (path filter — it changed only sim code, so risk is low, but the "green at baseline" claim rests on inference, not a run). No job timeouts on `check`/`integration-postgres`. Secrets handling is clean (deterministic seed, never a production secret; `DEVICE_AUTO_APPROVE=1` scoped to the disposable DB).

### 2.3 `desktop-installer.yml`

- **Triggers**: `workflow_dispatch` + push `main` path-filtered (desktop/node crates, `runtime-pins.json`, workflow). **Not on PRs** — installer breakage is detected only post-merge.
- 3-OS matrix (`fail-fast: false`): windows nsis (+ADR-024 vulkan variant), linux deb+appimage, macos-arm64 dmg. Pinned llama.cpp archives SHA-256-verified per OS, every file in `runtime-pins.json` re-hashed, engine-presence-inside-artifact checks for all four bundle formats (the v0.2.2 "silently engine-less installer" regression guard), SHA256SUMS generated, artifacts uploaded (never committed — enforced by rust.yml guards).
- **Holes**: toolchain is floating `dtolnay/rust-toolchain@stable` while rust.yml pins `1.99.0` with a comment documenting that a floating stable broke main before — same drift class in the most expensive job; path filter excludes root `Cargo.toml`, so a pure version bump doesn't rebuild installers; installers are unsigned (M13 gap, known); macos-x64 engine pins exist but nothing builds them (documented).

### 2.4 `real-model-e2e.yml`

- Manual-dispatch only (correct per standing rule). Windows runner; pinned Qwen2.5-0.5B Q4_K_M GGUF + llama.cpp b11407 hashes; runs `real_artifact` (GGUF identity-hash parity vs manifest, `modelswarm-types`) and `real_engine` (real-engine smoke: start/serve/die, `modelswarm-node`).
- **Holes**:
  - **(H2, release-gate)** It has **never completed a successful run**. All 15 recorded runs (2026-10-06) are 0-second failures from the broken one-line `on:` YAML era that fell back to push triggers. Zero `workflow_dispatch` runs exist. v0.2.20 shipped without this gate ever passing in CI — the release checklist either lacked it or it was skipped. The two ignored tests it runs DO pass locally per the freeze suite era evidence, but "CI never dispatched" is an evidence gap, not a code bug.
  - It does **not** run `real_node_prefers_gpu_variant` (the ADR-024 GPU production path: vulkan engine + `-ngl` + telemetry + kill-on-close), even though the workflow's own runner could stage it. See §6.

### 2.5 `windows-package-dry-run.yml`

- Push `main` + dispatch; `cargo build --release -p modelswarm-node` + `--version` smoke on a clean Windows runner; binary uploaded as artifact.
- **Hole (H3)**: it builds **default features — no `libp2p-backend`** — so the "release build works" proof compiles the stub-transport daemon, not the shipping configuration (the desktop build does forward `libp2p-backend` via its optional dependency). Also floating `@stable`.

---

## 3. Rust test estate (17 crates)

Total ≈ **326 Rust test functions** (317 in-crate + 7 sim + 2 relay). Every crate has at least one test; the thin tails are `modelswarm-winjob` (1) and `modelswarm-relay` (2).

| Crate | Unit modules | Integration files | Test fns | Notes |
|---|---|---|---|---|
| modelswarm-types | 4 | — | 23 | canonical JSON, GGUF parsing/hashing, manifest ids, golden vectors; `real_artifact_hashes_match_manifest` (ignored) |
| modelswarm-identity | 3 | — | 16 | ADR-020 peerId derivation, handshake signature coverage |
| modelswarm-transport | 6 | loopback.rs | 36 | framed JSON over real loopback sockets, limits, cancel mid-stream, stale/tampered handshakes, 4 concurrent sessions |
| modelswarm-eligibility | 2 | — | 20 | lease/challenge; `lease.rs` consumes `lease-hubkey-1.json` golden vector |
| modelswarm-session | 2 | 5 files | 45 | spec e2e + multi, **adversarial f1–f10** (fabricated/lying/withholding proposers, prefix equivocation, replay matrix, rogue verifier, flood, malformed-frame fuzz, prompt-canary redaction), proptest state-machine properties |
| modelswarm-speculation | 2 | 2 files | 19 | acceptance + trie properties (hand-rolled deterministic loops, not proptest macros) |
| modelswarm-scheduler | 1 | — | 15 | cost-model v2 formula pins, EWMA bounds, selection determinism/filtering/caps, measured-beats-advertised; crate remains bench-only/unwired |
| modelswarm-bench | 2 | 2 files | 27 | record schemas, bootstrap CI, honesty labeling, fastest-single comparator machinery |
| modelswarm-runtime | 3 | — | 26 | LlamaCppAdapter vs canned loopback servers (happy/401/bad-JSON/timeout/cancel-404); MockRuntime behind off-by-default feature |
| modelswarm-store | 0 | store.rs | 6 | rusqlite persistence, pre-separation migration |
| modelswarm-gateway | 1 | — | 19 | exact SSE shapes, clamp-don't-reject, retry-before-first-token + explicit interruption after, slow consumer, redaction, loopback guard |
| modelswarm-telemetry | 2 | — | 11 | redaction drops/truncates, forbidden fields, prompt-smuggling truncation |
| modelswarm-tracker-api | 1 | client.rs | 8 | signing/envelope shapes |
| modelswarm-node | 7 | live_tracker.rs | 37 | serving bridge over QUIC (refuses leaseless/foreign-profile/foreign-peer), download verify/corruption/redownload, engine tamper fail-closed, GPU launch args + vulkan pins parse, remote fallback honesty, session reuse, self-test; ignored: `real_engine_starts_serves_and_dies`, `real_node_prefers_gpu_variant`, `lan_cross_machine_completion` |
| modelswarm-desktop | 1 | — | 7 | hardware-analysis/model-recommendation units (RTX-3080 vector, no-GPU ranking, vulkan list-devices parsing, conservative requirements) |
| modelswarm-winjob | 1 | — | 1 | kill-on-close job object only |
| modelswarm-relay | 1 (in bin) | — | 2 | config-limits unit (stream-carrying contract) + real libp2p client reserve→circuit→bidirectional ping interop |

Env-gated/ignored Rust tests: 6 total. CI executes 3 of them (live_tracker signed flow via wire-compat; `real_artifact` + `real_engine` via e2e *if dispatched* — never has been). Manual/local-only: production HTTPS probe, **GPU variant path**, LAN cross-machine.

`tests/` at repo root contains only a README mapping to per-crate suites (ADR-010/015 restructure) — no orphaned harnesses.

**Property testing**: `proptest` is a dependency of exactly one crate (modelswarm-session, `session_properties.rs`). Speculation/gguf/manifest/lease/transport parsing have no randomized property or coverage-guided fuzz testing.

**Fuzzing**: **no `fuzz/` directory, no cargo-fuzz setup anywhere.** The only fuzz-shaped tests are `session_fuzz.rs` (deterministic xorshift random frames) and adversarial `f9` (malformed-frame, no-panic/no-unbounded-alloc). §12 states "every network parser is fuzzed" — the transport frame parser, GGUF metadata parser, canonical-JSON parser, and lease/receipt verification have no fuzzing. This is a floor-level gap, not a nice-to-have.

**Simulator (`apps/modelswarm-sim`)**: scenarios `pair`, `mesh` (adjacency + RTT matrix), `kill` (bounded peer-failure with msp §6.5 code), `spec` / `spec_multi` (determinism, straggler, exactness), plus `relay_case` (relayed path token-exact vs single decoding, receipts verified). CI scenario tests assert pair/mesh/kill; the spec/spec_multi/relay cases run as lib tests inside the default suite. Contracts asserted: deterministic identities, exact token equality vs single decode, receipt verification, explicit bounded failure. **Missing vs the testbed ladder (§10 of the expanded-mission review), which reserves sim for "scheduling/fairness/adversarial ONLY": there is no fairness/queueing scenario, no adversarial-peer swarm scenario, and no scheduler/planner scenario.** The sim as built proves wire/mesh/failure/speculative contracts — the classes the ladder says sim must own do not exist yet.

---

## 4. Tracker test estate

- 15 vitest files, **112 cases** (local: 110 pass, 2 skip without Postgres; CI integration-postgres runs all against a real DB). Coverage by route group: enrollment (12), admin (9), abuse (7), hardening (5), lifecycle (6), model-requests (7), tokens (6), stats (4), public/meta (14), phase-b regression (10), schema-ddl (11 — migration/DDL shape), pg-enrollment (2, env-gated), crypto-vectors (19 — includes `lease-hubkey-1.json`), downloads (7).
- `validate:vectors` (CI `check` job): validates every `manifest-*.json` against `catalog/schema-v2.json` and recomputes `ModelProfileId` from canonical JSON — run with Ajv 2020, allErrors, manifest-scoped `$ref`. `lease-hubkey-1.json` is deliberately excluded there (it is a signed-lease vector) and is instead consumed by BOTH `crypto-vectors.test.ts` (TS side) and `modelswarm-eligibility`'s golden test (Rust side), plus end-to-end by the wire-compat job against the CI seed. Cross-language vector parity is genuinely three-sided (schema validator + TS crypto + Rust gate).
- Missing: no tests for the admin catalog promote/candidates route beyond what admin.test.ts covers via wire-compat seeding; no load/soak tests; no tests of Vercel-deploy-specific behavior (out of scope for CI by design).

---

## 5. §13 test-class coverage matrix

| §13 class | Status | Evidence |
|---|---|---|
| unit | PRESENT | ~326 Rust + 112 TS fns; all 17 crates covered |
| integration | PRESENT | session e2e/adversarial, transport loopback, tracker-api client, store, bench harness, node bridge tests |
| property | PARTIAL | proptest in session only; speculation properties are hand-rolled fixed-seed loops; none for gguf/manifest/lease/transport |
| golden vectors | PRESENT | 5 vectors in `protocol/vectors/`, dual-language consumption + wire-compat |
| protocol compatibility | PRESENT | wire-compat CI job (real tracker + real Rust client, full signed-route chain incl. lease gate); production probe manual |
| fuzz | ABSENT (tooling) / PARTIAL (spirit) | no cargo-fuzz; session_fuzz + adversarial f9 only |
| concurrency | PARTIAL | concurrent sessions/logging/session-reuse/mesh; no stress or loom-style scheduling tests |
| failure-injection | PRESENT | kill scenario, withholding/lying peers, mid-stream fail, tamper/corrupt engine+downloads |
| network simulation | PARTIAL | pair/mesh/kill/spec/spec_multi/relay-stand-in on loopback; missing the ladder's fairness/adversarial/scheduling scenarios |
| hostile-peer | PARTIAL | session-layer f1–f10 strong; serving-gate refusal tests; NOTHING at accounting/receipts layer (doesn't exist yet — M10), no sybil/whitewash/ring tests, no hostile-tracker tests |
| multi-user | ABSENT | admission is admit-or-reject; wire `queue_position`/`eta_ms` and §6 10 s queue deadline untested |
| resource-pressure | ABSENT | winjob has 1 test (kill-on-close); no RAM/VRAM pressure, no memory-exhaustion, no responsiveness probe |
| battery and thermal | ABSENT | zero occurrences of battery/thermal in any crate |
| installer | PARTIAL | 3-OS build + engine-inside-artifact + hash sums in CI; no install→run→uninstall, no upgrade, no channel/beta test |
| update and rollback | ABSENT | no signed-update, no failed-upgrade recovery, no previous-profile rollback, no uninstall-cleanup test |
| physical LAN | PRESENT | `docs/verification/phase-f-lan-2026-10-07.md` (2 physical machines, production tracker, real weights); `lan_cross_machine_completion` kept for re-runs |
| physical WAN | ABSENT | acknowledged outstanding (R1); no k=4 harness (owner spend pending) |
| long-running soak | ABSENT | no soak harness anywhere; §10 requires ≥4 h mixed workload before any mode is default-on |

Score: 6 present, 6 partial, 6 absent (of 18 classes).

---

## 6. Feature-gating CI holes (the libp2p-backend lesson, applied generally)

1. **`modelswarm-node/libp2p-backend` + `modelswarm-transport/libp2p-backend`: FIXED.** rust.yml compiles and tests both (the transport feature rides the node's optional dep, which forwards it — node Cargo.toml line 24). The desktop's `tauri-shell` also forwards `libp2p-backend` (desktop Cargo.toml line 43), so the SHIPPING installer build compiles the real serving path. The wire-compat job runs the live harness with the feature on. No remaining hole for this pair.
2. **`modelswarm-desktop/tauri-shell`: step exists but is BROKEN on clean runners (H1)** — the exact "green locally, red where it matters" pattern the libp2p fix was meant to end. Main is RED at the audit baseline because of it.
3. **ADR-024 GPU variant (`engine-vulkan`)**: production resolution code is NOT feature-gated (compiled everywhere), but its real-path test `real_node_prefers_gpu_variant` is ignored AND not run by any workflow — real-model-e2e stages only the CPU engine. The GPU production path (variant verify + spawn `-ngl` + `backend=vulkan` telemetry + kill-on-close) has never executed in CI in any form. Same bug class as the lease break: shipped-behind-a-gate code with no automated executor. Recommendation: extend real-model-e2e to stage the vulkan archive and run it (it already knows how to hash-verify archives).
4. **`windows-package-dry-run` builds default features (H3)** — the "release daemon" it proves is the stub-transport binary. One flag (`--features modelswarm-node/libp2p-backend`) closes it.
5. `mock-runtime` / `node-selftest`: correctly test-only, off by default, enabled via dev-dependency in test builds — no production leakage. No action.
6. Floating `@stable` toolchain in desktop-installer / real-model-e2e / windows-package-dry-run vs pinned `1.99.0` in rust.yml — the drift risk rust.yml's own comment warns about, present in three other workflows.

---

## 7. docs/verification evidence inventory (real-machine vs otherwise)

13 documents. Environment honesty is good throughout — each labels its tier.

- **Real cross-machine (LAN)**: `phase-f-lan-2026-10-07.md` only (2 physical machines, production tracker, real weights — F0/F1).
- **Real single-machine hardware**: `e0-determinism-2026-10-07.md` (RTX 5080 Laptop GPU, CPU-vs-vulkan pinned engines, greedy determinism).
- **Real engines, loopback/local**: `phase-h.md` (real-model selection incl. GGUF parity), `f2a-relay-2026-10-08.md` (relay; in-process/loopback libp2p clients, not cross-machine), `phase-f.md` (loopback QUIC), `phase-d.md` (loopback two-peer speculative).
- **Local-machine unit/integration + loopback sims**: phase-0/a/b/c/e/g/i.
- **Absent**: WAN anything, k>2 testbeds, soak, resource-pressure runs — all consistent with the roadmap (R1/k=4 pending owner decisions); recorded here as evidence inventory, not new findings.

---

## 8. Severity-ranked gaps with blocked phase

| # | Sev | Gap | Blocks | Notes |
|---|---|---|---|---|
| G1 | ~~CRITICAL~~ **RESOLVED-during-audit** (`63b98e1`, 2026-10-09) | Main `rust` CI RED at baseline: desktop tauri-shell test step fails on clean runners (unstaged `engine/*` globs validated by tauri-build); the step had never been green in CI | **M0** (exit: suite green, no unexplained failures) | Fixed via `TAURI_CONFIG` resources-stripping overlay for the test-only build; local bare-runner repro + 7/7 pass; CI run 37858753428 in progress at audit time. Residual: overlay removes all pre-merge resource-config validation — see §2.1 hardening recommendation (guards-job frozen-set assertion). Downgraded but not fully closed until the guards assertion (or equivalent) lands. |
| G2 | CRITICAL | `real-model-e2e` has never completed a run (0 dispatches; only broken-YAML-era push failures); v0.2.20 shipped without it | **M0/M6** evidence discipline | Add dispatch + recorded result to the release checklist; do not treat "manual-only" as "optional". |
| G3 | HIGH | No fuzz tooling for network parsers (transport frames, GGUF, canonical JSON, lease verification) — §12 floor violation | §12 floor; **M10** hostile-peer posture | cargo-fuzz targets + CI smoke runs; session_fuzz shows the in-process pattern is viable. |
| G4 | HIGH | ADR-024 GPU-variant production path never tested in CI (ignored test not wired into any workflow) | **M6/M7** verification on shipped builds | Extend real-model-e2e (G2) with the vulkan archive + `MSP_ENGINE_BACKEND`. |
| G5 | HIGH | Resource-pressure, battery, thermal tests entirely absent; governor surface (winjob) has 1 test | **M2** (exit gate: memory-exhaustion cannot crash/freeze; foreground responsive) | Needs the governor crate first, but the TEST design must be in the M2 plan now. |
| G6 | HIGH | Multi-user/queueing and fair-queueing tests absent (wire fields exist, unused) | **M10** (receipts v2 + DRR queueing) | Sim is the designated tier per ladder §10 — needs new sim scenarios. |
| G7 | HIGH | No soak harness (§10: ≥4 h mixed workload before any mode default-on) | **M10** default-enable gates | Build as CI scheduled/cron or long local run with artifact evidence. |
| G8 | MEDIUM | Update/rollback/uninstall tests absent (signed updates, failed-upgrade recovery, previous-profile rollback, uninstall cleanup, interrupted-download resume beyond unit redownload) | **M5, M13** | Node has corruption/redownload units (good seed); lifecycle-level tests missing. |
| G9 | MEDIUM | windows-package-dry-run builds stub-transport default features | M0 hygiene | One-line feature flag fix. |
| G10 | MEDIUM | desktop-installer not PR-gated; version-bump-only pushes don't trigger it; floating `@stable` in 3 workflows | M13 release discipline | Cost/balance: full installer matrix per PR is expensive; consider a compile-only PR job. |
| G11 | MEDIUM | Sim lacks fairness/adversarial/scheduling scenarios the testbed ladder reserves for it | **M10**, scheduler shadow-mode (R2) | The sim today proves wire/mesh/failure/spec contracts only. |
| G12 | MEDIUM | Hostile-peer tests stop at the session layer; nothing for accounting/reputation enforcement ladder (sybil rings, whitewash, seeded wrongness) | **M10** gauntlet (pre-merge for every mode) | Depends on receipts v2; gauntlet must be designed with it, not after. |
| G13 | LOW | Property coverage narrow (proptest in 1 crate) | M3/M5 robustness | Extend to manifest/gguf/lease/transport. |
| G14 | LOW | tracker.yml: no concurrency cancel, no job timeouts on 2 jobs; wire-compat not re-run at baseline commit (path filter) | hygiene | Low risk; note in next tracker CI touch. |
| G15 | LOW | No `cargo audit`/`cargo deny` dependency scanning in any workflow | supply-chain floor (§12 "dependency compromise") | Cheap to add; false-positive triage is the only cost. |
| G16 | LOW | Installer/test artifacts unsigned; no beta channel machinery | **M13** (known, roadmap-tracked) | Recorded for completeness. |

---

## 9. What is genuinely good (do not rebuild)

- The wire-compat CI job is the correct structural fix for the shipped wire-bug class and covers the full signed-route + lease-gate chain against a real tracker and Postgres.
- Golden-vector discipline is three-sided (schema + TS crypto + Rust gate) with a pinned CI signing seed whose public half is itself a vector.
- The session adversarial suite (f1–f10) is a real hostile-peer harness at the protocol layer, including replay matrices and a prompt-canary redaction gate.
- Release guards (no committed binaries, version single-sourcing, filename↔version match, engine-inside-artifact) encode shipped-regression lessons directly into CI.
- Environment honesty in `docs/verification/` matches the master prompt's labeling rules; no sim numbers found promoted as physical results.

## 10. Recommended remediation order

1. ~~G1~~ RESOLVED during audit (`63b98e1`); remaining follow-up: the zero-cost `guards`-job resource-set assertion (residual-risk closure).
2. G2 + G4 together (dispatch e2e once, then extend it with the GPU variant).
3. G9 (one-line), G10 toolchain pins.
4. G3 fuzz targets for the four parsers (floor).
5. G6/G7/G11 sim + soak scaffolding ahead of M10 design; G5 test design inside the M2 governor ADR.
