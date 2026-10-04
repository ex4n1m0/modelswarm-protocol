# Phase A Verification Report

Date: 2026-10-04 · Environment: Windows 11 x64, git 2.50.1, Rust 1.98.1,
Node 22.18.0, npm 10.9.3 · Commits: `fa09f41` (ADRs), `f6dbd35`
(restructure), `a8e5246` (schema v2 + vectors), plus this final commit.

## Delivered (per the approved first response + revision Phase A)

| Deliverable | Where | Status |
|---|---|---|
| ADR-009…015 (positioning, restructure, manifest IDs, leases, modes, NAT roadmap, supersession) | `docs/adr/` | done |
| Differentiation paragraph, one test per clause | ADR-009 | done |
| Prior-art matrix, 10 systems, primary-source verified, confidence labels | `docs/research/prior-art-matrix.md` + `-notes.md` appendix | done |
| Workspace restructure (history-preserving `git mv`; ms-catalog dissolved; 5 new stub crates; hub → apps/tracker; tracker renamed) | whole repo, per ADR-010 | done |
| Manifest schema v2 + golden vectors + cross-language validator | `catalog/schema-v2.json`, `protocol/vectors/`, `apps/tracker/scripts/validate-vectors.mjs` | done |
| Fastest-single benchmark harness: spec + network matrix + statistical protocol + frozen record schemas | `docs/research/bench-harness-spec.md`, `experiments/` | frozen (spec) |
| Runtime trait spec + llama.cpp capability/gap map | `docs/research/runtime-trait-spec.md` | done (spec) |
| Transport plan incl. Phase F relay posture | `docs/research/transport-plan.md` | done (spec) |
| Tracker endpoint gap map (session-authorize, lease, receipt, audit, model-catalog) | `apps/tracker/DESIGN.md` | done (spec) |
| Windows Phase G requirements inventory | `docs/research/windows-beta-requirements.md` | done (spec) |
| Fuzz/property/adversarial targets + threat-model addendum queued | `docs/research/fuzz-targets.md` | done (spec) |
| Dependency graph snapshot | `docs/research/dependency-graph.md` | done |
| p2ptokens decision: differentiate + selective study, no fork/interop (MIT verified; no speculative decoding there; commodity overlap already frozen in our contracts) | first response §4; matrix row 1 | decided; ADR-016 not needed (no code reuse adopted) |

## Commands run and results

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (exit 0) |
| `cargo test --workspace` | 4 passed, 0 failed (modelswarm-types); stubs 0 |
| `cd apps/tracker && npm ci` | OK (post-move install verified; lock name synced via `npm install --package-lock-only`) |
| `npm run typecheck` | PASS |
| `npm run build` | ✓ Compiled successfully |
| `npm run validate:vectors` | 2/2 PASS — `manifest-basic` → `msp1:229e17c8…818e933`, `manifest-no-spec` → `msp1:47e080cd…f831672` (both recomputed + asserted) |
| JSON parse of schema-v2 + both experiment schemas | PASS |

Incidents during Phase A (honest log):

1. `git mv hub apps/tracker` initially failed (Permission denied) — two
   orphaned `node.exe` processes from the Phase 0 smoke test held the
   directory; killed, retried, clean.
2. First vector run: `manifest-no-spec` failed schema check — its
   hand-written 63-char `build_hash`; fixed and ID recomputed by tooling
   (exactly the failure mode the no-hand-typed-hashes rule exists for).
3. Validator's first version compiled a subschema in isolation, breaking
   internal `$ref` resolution; fixed by compiling the full schema document
   with a `$ref` pointer.

## Deviations from the first response (recorded)

- Rust round-trip canonical-serialization tests moved to **Phase B**, where
   the serializer itself is a deliverable (avoids implementing Phase B
   inside Phase A). The Node validator is the interim parity oracle; the
   fixtures are already shared.
- "Benchmark runs" were never claimed; the harness is **frozen, not
  executed** (nothing measurable exists until Phases B–C). `docs/acceptance/
  phase-a.md` records this boundary explicitly.
- ADR-016 (conditional p2ptokens code reuse) closed as not-needed: no
  module adoption survived the matrix comparison.

## Gate

Phase A exit criteria (differentiation paragraph testable; ADRs approved
implicitly by the "go" approval of the first response; harness/schemas
frozen) are met. **Phase B (protocol foundation + tracker) requires the
owner's review of this report.**
