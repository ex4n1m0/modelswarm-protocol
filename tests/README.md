# Tests

The promised contents of these directories moved into the crates as the
plan evolved (ADR-010 restructure; ADR-015's A-G phases superseded the
original 0-7 map — see `docs/architecture.md` §14 note):

- Cross-crate integration coverage lives in each crate's `tests/`
  directory (e.g. `crates/modelswarm-transport/tests/loopback.rs`,
  `crates/modelswarm-tracker-api/tests/client.rs`,
  `crates/modelswarm-node/tests/live_tracker.rs` — the Rust↔TS wire
  harness) and in per-crate adversarial/fuzz/property suites
  (`crates/modelswarm-session/tests/`).
- Windows end-to-end scenarios are the env-gated ignored tests in
  `crates/modelswarm-node` (`real_engine_starts_serves_and_dies`,
  `real_node_prefers_gpu_variant`, `lan_cross_machine_completion`) and
  the stub-harness UI checks described in
  `docs/reviews/desktop-start-stop-ux-study-2026-10-06.md`.
- Multi-node swarm scenarios are driven by `apps/modelswarm-sim`.

Acceptance criteria per phase: `docs/acceptance/`. Evidence:
`docs/verification/`.
