# Phase C Verification Report

Date: 2026-10-05 · Commits: `e6834e3` (transport+sim, 1/2), this commit
(runtime/scheduler/gateway/bench, 2/2).

## Delivered

| Component | Evidence |
|---|---|
| Runtime trait + adapters | `InferenceRuntime` (object-safe); MockRuntime behind off-by-default `mock-runtime` feature with `draft_accuracy` knob; LlamaCppAdapter tested against canned loopback HTTP servers (happy/401/bad-JSON/timeout/cancel-404) — 22 tests |
| Scheduler | cost-model v1 + v2 (six documented terms), engage rule with 0.05 margin floor, measured-dominant scoring, bounded micro-swarm selection with deterministic ties, EWMA — 13 tests |
| Gateway | OpenAI-compatible router (loopback-only enforced): exact SSE chunk shapes + `[DONE]`, clamp-don't-reject with `x-msp-clamped` echo, 32 KiB cap, ADR-007 retry machine (≤2 retries pre-first-token; explicit `interrupted` after, no fabricated continuation), test doubles incl. fail-mid-stream + slow-consumer — 19 tests |
| Bench harness | schema-valid records (in-Rust mini-validator against the real `experiments/schemas/`, with negative tests proving the validator bites), deterministic same-seed output, bootstrap CI, fastest-of-N single comparator, honest `fell_back_to_single` records, TEST-ONLY labels on every report line + `runtime.name="mock"` in manifests — 26 tests |
| Transport + sim (1/2) | 35 tests; `kill` drill emits explicit `interrupted`, no hang |

## Integrator-run gates

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS (whole workspace) |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | **201 passed, 0 failed** |
| Feature matrix (`mock-runtime` off/on; `test-doubles` off/on) | compiles both ways |

## Integrator decisions on agent assumptions (all accepted, recorded)

1. Two `RuntimeDescriptor` types stay separate (runtime identity vs catalog
   manifest pin) — do not merge in later phases.
2. MockRuntime proposal contract: proposals continue the *default-sampling*
   continuation; verifiers decode with `SamplingParams::default()`;
   `draft_accuracy` is the only error source. Phase D inherits this.
3. Retry plumbing: `execute` takes no peer hint; executor owns peer
   advancement on re-invocation; `peer_hint` is observability-only.
4. TEST-ONLY labeling: closed schemas can't carry a label field, so mock
   runs are pinned via `run_manifest.runtime.name = "mock"` + literal
   label text on every human-facing line (test-asserted).
5. Synthetic bench coefficients (acceptance 0.8, 3× drafter asymmetry…) are
   documented TEST-ONLY constants to exercise engage/fallback paths; real
   coefficients must be measured (ADR-013).

## Honest boundary

Everything executable this phase runs on loopback with MockRuntime or
canned llama.cpp HTTP responses. **No real-model performance claim is made.**
No model weights or runtime binaries were downloaded (stop condition
intact). The llama.cpp adapter needs a real-sidecar smoke test when an
approved GGUF profile exists. Bench engage/fallback economics are
intentionally borderline; on real hardware most cooperative cells may
honestly fall back — that is the outcome the harness exists to record.
