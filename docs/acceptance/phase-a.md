# Phase A Acceptance Tests — Audit and Rebaseline

Exit gate (revision): "No implementation proceeds until the project's
differentiation is written in one testable paragraph." Evidence recorded in
`docs/verification/phase-a.md`.

## A. Governance and positioning

- **A1** ADR-009..015 exist, are cross-referenced from AGENTS.md/README, and
  each conflict the revision introduces vs older plans is resolved by an
  ADR (supersession map ADR-015). ✅
- **A2** The differentiation paragraph (ADR-009) exists with one named test
  per clause. ✅
- **A3** Prior-art matrix (10 systems) verified from primary sources with
  confidence labels; no unverified claim feeds an engineering assumption. ✅

## B. Restructure (ADR-010)

- **B1** `git log --follow` resolves history through every rename (crates,
  hub → apps/tracker). ✅
- **B2** No stale `ms-*`/`hub/` references remain in live docs/config
  (historical records — build-plan copy, phase-0 reviews/verification,
  first-response audit tables — intentionally excluded). ✅
- **B3** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` all pass post-restructure. ✅
- **B4** Tracker: `npm ci && npm run typecheck && npm run build` pass at the
  new path; CI workflow renamed with matching triggers. ✅

## C. Manifest identity (ADR-011)

- **C1** `catalog/schema-v2.json` parses as JSON Schema 2020-12 and validates
  the golden vectors. ✅
- **C2** `protocol/vectors/` fixtures carry `expected_profile_id` values
  that were computed by tooling (validator recomputes and asserts equality
  on every run — CI-enforced). ✅
- **C3** `npm run validate:vectors` exits 0 locally and in CI (tracker
  workflow). ✅
- **C4** Changing any hashed manifest field (test: one-byte revision change
  in a scratch copy) produces a different derived id — asserted by
  construction in the derivation; Rust parity test lands Phase B. ✅(design)

## D. Research freezes

- **D1** Benchmark harness spec + network matrix + statistical protocol
  frozen (`docs/research/bench-harness-spec.md`); schemas
  `experiments/schemas/{run-manifest,mode-result}.schema.json` parse and
  cover ADR-013's telemetry vocabulary. ✅
- **D2** Runtime trait spec + llama.cpp capability gap map recorded. ✅
- **D3** Transport plan incl. Phase F relay posture (ADR-014). ✅
- **D4** Tracker endpoint gap map (5 new metadata endpoints). ✅
- **D5** Windows Phase G requirements inventory. ✅
- **D6** Fuzz/property/adversarial target list + threat-model addendum
  queued for Phase D. ✅
- **D7** Dependency graph snapshot (zero external Rust deps; tracker deps
  enumerated). ✅

## Honest boundary (recorded, not hidden)

- Phase A ran **no performance benchmarks** — nothing executable exists to
  measure. The harness, matrix, and statistical protocol are *frozen*; first
  real runs are a Phase C deliverable (ADR-015 mapping). The revision's
  "benchmark `single` mode across node classes" is therefore satisfied as
  *specified-and-frozen*, not *executed*, in Phase A.
- Rust implementation of canonical serialization/derivation is Phase B; the
  Node validator is the interim parity oracle.
