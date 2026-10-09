# ADR-027: Plan supersession — master roadmap M0–M13 governs

Status: Accepted (2026-10-09) · Owner-approved architecture-gate decision D2
(`docs/reviews/architecture-gate-2026-10-09.md` §C.1, §D, §G). Recorded in
ADR-015's supersession style; it supersedes ADR-015's designation of A–G as
the governing plan, exactly as ADR-015 superseded before.

## Context

Four planning layers now stack: the original prototype plan (phases 0–7),
the cooperative-inference plan (P0–P8), the competitive-revision plan (A–G,
designated governing by ADR-015), and the product roadmap M0–M13 elaborated
by `zcode-master-prompt.md` §10 and audited by the eleven-review gate of
2026-10-09. Without a recorded map, agents will cross-implement numbering
systems — the risk ADR-015 closed once, reopened by the new layer. The gate
recorded owner approval (D2: "Approve") of a single successor plan of record
with an ADR-015-style supersession map.

## Decision

**`docs/implementation/master-roadmap.md` is the plan of record.** Newest
governs where documents conflict; older documents remain authoritative
reference for everything they define that the roadmap does not override.

| Plan | Status under this ADR |
|---|---|
| `docs/build-plan.md` (phases 0–7) | Historical reference (threat-modeling depth, acceptance-matrix rows) |
| `docs/cooperative-plan.md` (P0–P8) | **Remains the gated lossless kernel** — cited, not duplicated; its statistical methodology, correctness-first ordering, and prerequisite gate carry forward unchanged |
| `docs/competitive-revision-plan.md` (A–G) | Historical implementation evidence; superseded as governing plan |
| `docs/implementation/master-roadmap.md` (M0–M13) | **Governs** |

### Carried forward unchanged (restated, per gate §C.1 / architecture-seat amendment)

- The **msp-v1 freeze**: changes to `protocol/msp-v1.md`,
  `protocol/msp-cooperative-v1.md`, `protocol/messages.proto`, and
  `catalog/schema*.json` require an ADR; no implementation may invent fields
  outside them. The freeze itself is NOT reopened by this supersession.
- Phase gates with `docs/verification/` evidence; new phases verify as
  `docs/verification/m<N>-<slug>-<date>.md` (gate §9.3).
- The A–I and F0–F3 verification docs under `docs/verification/` remain
  historical evidence; they are cited, never re-run or renumbered.
- The project rule, AGENTS.md hard constraints 1–8, the ownership map, and
  performance-honesty rules (fastest-eligible-single comparator, negatives
  published as visibly as wins).

### M0–M13 ↔ A–I evidence crosswalk (statuses: gate §C.3, audit-corrected)

| Phase | Status (2026-10-09) | Historical evidence absorbed |
|---|---|---|
| M0 baseline/recovery | **DONE** (CI green restored: `63b98e1` run 37858753428 and `86450aa` run 37860513036 both verified success) | `phase-0.md`, freeze + validation audits |
| M1 hardware profiler | PARTIAL-seed (hw detection exists; profiler crate new) | — |
| M2 resource governor | NEW (governor ADR incl. 70%-vs-90% reconciliation, D11) | winjob seed (kill-on-close + total RAM only) |
| M3 signed catalog | PARTIAL → harden (5 actionable rows) | Phase B (schema v2, ADR-011/022) |
| M4 auto model selection | NEW (top-6 recommender seeded; automatic selection new) | — |
| M5 verified download/lifecycle | PARTIAL (resume/disk/rollback absent) | Phase B/C acquisition work |
| M6 real local inference | **DONE** + P1 streaming refactor attached (pre-M9 condition, D10) | Phases C/D; E0 determinism |
| M7 minimal client | DONE with gaps (copy button, Stop drain) | Phases G/H/I (v0.2.20 client) |
| M8 automatic swarm joining | DONE on LAN (F2b Swarm migration outstanding) | F0/F1 (`phase-f-lan-2026-10-07.md`) |
| M9 single-peer baseline | PARTIAL (WAN outstanding; P1 + F15 prerequisites) | F-series; E0 |
| M10 cooperative inference | NOT STARTED (11-step build order delivered; prerequisites P3/P16/F15) | Phases D/E science (sim/loopback only) |
| M11 deliberation | NOT STARTED (gated) | — |
| M12 federation | NOT STARTED (gated; see below) | — |
| M13 website/release | PARTIAL (beta channel, signed updates, incident process, uninstall drill open) | Phase G releases (v0.2.20) |

### Federation non-goal reversal (gate §C.2, D8 acknowledged)

The expanded-mission review §11 listed mixed-model critique and
mixed-`ModelProfileId` cohorts as non-goals. This ADR records the reversal
as **gated future work (M12)** with preconditions — nothing silent:

1. M12 requires its own ADR and its own correctness contracts.
2. No mixed-`ModelProfileId` cohort exists until its contract does.
3. Mixed-model outputs are never labeled exact lossless unless the target
   verifier enforces it token-by-token.
4. **Every output records which profile performed each role (the M12 exit
   criterion).**
5. M12 begins only after the M10 (cooperative) and M11 (deliberation) gates.

### Related decisions

- ADR-028 (tracker-surface/wire reconciliation), ADR-029 (execution presets
  + `msp-cooperative-v1.md` namespace), ADR-030 (receipts v2) constitute the
  R0 governance batch authorized by the same gate (D3).
- Dependency ordering lives in `docs/implementation/dependency-graph.md`
  (gate §C.4 critical path).

## Consequences

+ One active numbering system (M0–M13); every "which plan wins" question
  resolves by table lookup here.
+ The cooperative plan's P0 gate and lossless contracts stay enforceable
  while M10 executes under the roadmap.
− The roadmap document must be kept current (per-phase status flips) — the
  Integrator owns that upkeep alongside the verification docs.
