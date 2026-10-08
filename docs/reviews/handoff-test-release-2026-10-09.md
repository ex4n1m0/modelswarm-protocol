# Handoff — Test and Release Engineer — 2026-10-09

## 1. Changed files and why

- `docs/reviews/audit-test-gap-analysis.md` (NEW; amended post-audit) — the test-gap audit deliverable required by master prompt §8: CI inventory (all five workflows), Rust/tracker test-estate enumeration, §13 coverage matrix, feature-gating CI hole analysis, `docs/verification` evidence inventory, severity-ranked gaps mapped to blocked M-phases, and a timestamped snapshot of the concurrent local freeze-validation suite. Amended 2026-10-09 after the coordinator's resolution notice: G1 folded in as RESOLVED-during-audit (commit `63b98e1`) with root-cause mechanics and a residual-risk assessment of the `TAURI_CONFIG` overlay approach.
- `docs/reviews/handoff-test-release-2026-10-09.md` (this file) — AGENTS.md handoff format; risks/next-task sections updated for the same resolution.

Nothing else was created, modified, committed, or deployed. No cargo/npm commands were started by this agent (target/ lock discipline); the validation results below are reads of the concurrently running freeze suite and of GitHub Actions via `gh`.

## 2. Exact commands run and outcomes

Read/inspection only (grep/sed/ls/git/gh):

- `gh run list --limit 10` — OK (authenticated). Revealed main's last `rust` run FAILED.
- `gh run view 37796497342 [--log-failed / --log]` — OK. Root cause: `Desktop tests (tauri-shell feature)` → tauri build script error `glob pattern engine/* path not found` (clean runner lacks the gitignored staged engine).
- `gh run view 37792918516` — OK. Failed one step earlier (`Tests (libp2p-backend feature)` — the sim wall-clock flake that `730cebf` fixed).
- `gh run list --workflow tracker.yml / desktop-installer.yml / real-model-e2e.yml` — OK. wire-compat last green 2026-10-08T14:29Z; real-model-e2e has ZERO completed runs (only broken-YAML-era push failures).
- `gh run view 37792918339` — OK: tracker check + integration-postgres + wire-compat all green.
- Read: all five `.github/workflows/*.yml`; `zcode-master-prompt.md` (§0/§5/§13); `docs/reviews/expanded-mission-roadmap-review-2026-10-08.md` §10; all 17 crates' Cargo.toml feature sections and test corpora; `apps/tracker/package.json`, `vitest.config.ts`, `scripts/validate-vectors.mjs`, tests/; `apps/modelswarm-sim` (lib + scenarios); `crates/modelswarm-node/tests/live_tracker.rs`; `protocol/vectors/`; `docs/verification/*.md`; `target/audit-logs/summary.txt` + `run-freeze-validation.sh`.
- grep sweeps for `proptest|quickcheck` (1 crate), `cargo-fuzz|fuzz/` (none), `#[cfg(feature` (10 files), `#[ignore` (6 tests), `soak|battery|thermal` (zero in code).

## 3. Test evidence

Concurrent local freeze-validation suite (`target/audit-logs/summary.txt`), branch `audit/master-prompt-2026-10-09`, head `730cebf` — **ALL-DONE 2026-10-09T07:13:45+08:00, every step PASS**:

- fmt PASS; clippy-default PASS; test-default PASS (≈2 m); clippy-libp2p PASS; test-libp2p PASS; test-desktop-shell PASS; tracker typecheck/test/build/validate:vectors all PASS.
- Tracker vitest locally: 110 passed, 2 skipped (PgStore cases; CI-only via Postgres service).

Test-estate counts (static enumeration): ≈326 Rust test functions across 17 crates (all crates covered; thinnest: winjob 1, relay 2) + 7 sim tests; 112 tracker vitest cases in 15 files; 6 env-gated ignored Rust tests (3 CI-executed, 3 manual-only); 5 golden vectors; proptest in exactly 1 crate; zero cargo-fuzz.

GitHub Actions (remote truth): main `rust` workflow RED at baseline `730cebf` (desktop-test step, clean-runner engine staging); tracker wire-compat GREEN (pre-baseline commit); real-model-e2e NEVER run to completion.

## 4. Assumptions

- The freeze suite's logs are authoritative for local state at the timestamps quoted; warm `target/` explains sub-second clippy re-runs.
- Local desktop tauri-shell tests pass because this machine has the engine staged under the gitignored `crates/modelswarm-desktop/engine/`; CI does not. Verified by the runner log's glob error, not by reproducing a clean runner.
- wire-compat validity at `730cebf` is inferred (paths filter did not re-trigger it; the intervening commit changed sim-only code). Low risk, recorded as inference.
- Branch protection / required-check configuration is not readable with the `gh` scopes used; "what blocks merge" is assessed from workflow definitions only.
- Test-function counts come from grep enumeration (`#[test]`/`#[tokio::test]` + integration files), not a `cargo test` listing; ±2 around ~326 is possible.

## 5. Unresolved risks

- **G1: RESOLVED during audit by `63b98e1` on main (2026-10-09, workflow-only)** — root cause: tauri-build 2.7.1 validates the `bundle.resources` globs (`engine/*`, `engine-vulkan/*`) in the build script; engines are gitignored and only installer workflows stage them; the custom `MSP_ALLOW_NO_ENGINE` gate is release-only and not involved. Fix: `TAURI_CONFIG='{"bundle":{"resources":[]}}'` overlay on the test step; proven by local bare-runner repro (exit 101, same glob error) then 7/7 pass; CI run 37858753428 was in progress at audit close. **Residual risk**: the overlay removes all pre-merge resource-config validation (a future glob regression now merges red-free and is caught only post-merge by desktop-installer's engine-presence + per-file-hash checks); recommended zero-cost closure is a `guards`-job assertion freezing `bundle.resources` to exactly `["engine/*", "engine-vulkan/*"]`. See audit §2.1.
- **G2 (critical)**: the manual-dispatch-only real-model e2e gate has never been dispatched; releases have no recorded real-model CI evidence. Process risk, not code risk.
- **G4**: the ADR-024 GPU-variant production path has no automated executor anywhere (same class as the 2026-10-08 libp2p-backend hole that was just fixed).
- §12 floor: no fuzzing of network parsers (transport frames, GGUF, canonical JSON, lease verification).
- Six §13 test classes entirely absent (multi-user, resource-pressure, battery/thermal, update-rollback, physical WAN, soak); hostile-peer coverage stops at the session layer.
- Tracker CI hygiene: no concurrency cancel/job timeouts on two jobs; three workflows float `@stable` toolchain while rust.yml pins 1.99.0 for documented reasons.

## 6. Suggested next task for the integrator

G1 is fixed on main (`63b98e1`) — confirm run 37858753428 went green, and adopt the recommended `guards`-job resource-set assertion to close its residual pre-merge validation hole. Then dispatch real-model-e2e once and record the result (G2), extending it with the vulkan variant for G4 in the same edit. Those are small workflow-only changes owned by me that close the M0 evidence gate; everything else in the remediation order (§10 of the audit) rides the architecture gate.
