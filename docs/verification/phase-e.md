# Phase E Verification Report

Date: 2026-10-05 · Commits: `248aa05` (trie algorithms, early) + this
commit (protocol wiring, sim, bench).

## Delivered

Multi-proposer token-tree mode over the same signed session protocol:

- `TreeVerifyRequest/Result` (branches as token sequences); verifier
  feature-detects tree vs linear requests; commits ride the identical
  signed `PrefixCommit` path.
- `speculate_multi`: fan-out of seeded `ProposalRequest`s
  (`branch_assignment(session, round, roster, i)`), deadline-anchored
  collection with straggler drop and stale-message draining,
  `CandidateTrie::assemble` with per-proposer trailing-acceptance scores,
  single tree verification, commits forwarded to all live proposers,
  single-decode fallback tail + two-phase receipt. Peer loss is absorbed
  by the roster (fallback only when ALL proposers die).
- Branch distinctness via a runtime factory: `factory(branch_seed)` =
  `MockRuntime::new(branch_seed, accuracy)` — distinct divergence patterns
  deterministically, target-continuation accuracy preserved (documented in
  crate docs); at accuracy 1.0 proposers genuinely agree and the trie
  honestly counts it as duplicate work.
- `spec_multi` sim scenario (90-case matrix: 15 seeds × {2,4,7} proposers
  × {0.3,0.8} accuracy) + `MODELSWARM_SIM_STRAGGLER_MS` injection hook;
  bench `run_cell_multi` emits schema-valid `speculative_exact` records
  from real in-process tree rounds.

## Gates (docs/acceptance/phase-e.md)

| Gate | Evidence |
|---|---|
| E1 branch assignment | Determinism/distinctness unit tests (also in `248aa05`) |
| E2/E3 bounds + dedup | Trie property tests (`248aa05`) + live `duplicate_work` in every multi run (integrator run: 20 duplicate tokens at accuracy 0.8) |
| E4 batch vs sequential | 200-case equivalence: tree verify == max per-branch linear verify (incl. a pruning pass) |
| E5 stragglers | Deadline drop, stale-drain; injected 600 ms straggler → `stragglers_total:1`, round completes, output exact (integrator-verified via env hook) |
| E6 losing branches | Stateless-per-round proposals: non-commit releases capacity (architectural, documented) |
| E7 correctness | 90-case matrix + e2e: `tokens_equal_to_single` ALWAYS true, incl. mid-session proposer kill and all-straggler rounds |
| E8 records | Schema-valid multi records; duplicate/pruned telemetry surfaces through `aggregate_model_tokens` + `bytes_per_accepted_token` (closed schema has no dedicated fields — documented) |
| E9 comparative | Machinery exists (multi vs two-peer vs single records); **no claim made** — all mock-backed, TEST-ONLY (ADR-019). The plan's "beat two-peer on a declared workload" gate cannot be honestly evaluated without real hardware; recorded per the D8 posture |

## Integrator-run gates

`cargo fmt --all --check` PASS · `clippy --workspace --all-targets
-- -D warnings` exit 0 · `cargo test --workspace` **256 passed / 0 failed**
· `sim spec_multi 3 4 0.8` → exact, `duplicate_work:20`, mean acceptance
8.0 · straggler hook → `stragglers_total:1`, still exact.

## Honest boundary

Duplicate-work/pruned have no dedicated schema fields (closed v1 schemas);
they are derivable from the recorded waste-sensitive fields and the sim
JSON. A schema ADR can add explicit fields when real experiments need
them. Bench multi drafts from one deterministic runtime (redundancy IS the
telemetry); branch diversity is exercised via seeded factories in sim/e2e.
No performance claim.
