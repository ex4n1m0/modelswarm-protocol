# ADR-010: Workspace restructure per the competitive revision

Status: Accepted (Phase A, 2026-10-04)

## Context

The revision (`docs/competitive-revision-plan.md` §Repository changes)
specifies a `modelswarm-*` crate naming scheme and a tracker under
`apps/tracker/`. Our Phase 0 layout used `ms-*` crates and `hub/`. Renaming
now is cheap (stubs only); renaming later would rewrite history and CI.

## Decision

History-preserving renames (`git mv`), effective Phase A:

| Phase 0 | Phase A |
|---|---|
| `crates/ms-core` | `crates/modelswarm-types` |
| `crates/ms-crypto` | `crates/modelswarm-identity` |
| `crates/ms-p2p` | `crates/modelswarm-transport` |
| `crates/ms-runtime` | `crates/modelswarm-runtime` |
| `crates/ms-scheduler` | `crates/modelswarm-scheduler` |
| `crates/ms-store` | `crates/modelswarm-store` |
| `crates/ms-gateway` | `crates/modelswarm-gateway` |
| `crates/ms-telemetry` | `crates/modelswarm-telemetry` |
| `crates/ms-catalog` | **dissolved** — manifest types → `modelswarm-types`; HF acquisition → `modelswarm-runtime` |
| `hub/` | `apps/tracker/` |
| `apps/modelswarm-node` | `crates/modelswarm-node` (revision lists node/desktop under `crates/`) |
| `apps/modelswarm-desktop` | `crates/modelswarm-desktop` |

New interface-freeze stubs: `modelswarm-tracker-api`, `modelswarm-eligibility`,
`modelswarm-session`, `modelswarm-speculation`, `modelswarm-bench`.
`apps/modelswarm-sim` remains under `apps/` (test tooling, not in the
revision's crate list).

Service identity: the tracker's `service` string changes
`modelswarm-hub` → `modelswarm-tracker` in `protocol/msp-v1.md` §3.1,
`docs/acceptance/phase-1.md` B1, and `lib/version.ts`. The public hostname
stays `modelswarm.deepflux.space` (lowercase canonical). All doc references
to `hub/` and `ms-*` paths are updated in the same commit; CI workflow paths
follow.

## Consequences

+ Names now match the governing plan; zero runtime code to migrate.
+ `git log --follow` preserves file history through the renames.
− Every doc that named the old paths is touched once, now, deliberately.

## Alternatives rejected

- Keep `ms-*` (avoid churn): permanent divergence between plan and repo —
  exactly the R10 drift risk.
- Restructure per cooperative plan (`ms-coop-protocol`…): superseded by this
  revision's layout (ADR-015).
