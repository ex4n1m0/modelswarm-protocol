# ADR-015: Plan supersession map — A–G governs

Status: Accepted (Phase A, 2026-10-04)

## Context

Three planning documents now stack: the original prototype plan (phases
0–7), the cooperative-inference plan (P0–P8), and the competitive-learnings
revision (A–G). Without an explicit map, agents will cross-implement
numbering systems (risk R19).

## Decision

**Newest plan governs where they conflict.** A–G is the execution program;
the older documents remain authoritative reference for everything they
define that A–G does not override (notably: the original plan's threat
modeling depth, acceptance-matrix rows, and the cooperative plan's
statistical methodology and correctness-first ordering).

| A–G phase | Absorbs (older plans) | Notes |
|---|---|---|
| A — Audit & rebaseline | — | Prior-art matrix, ADR-009…016, schema v2, frozen harness |
| B — Protocol foundation | Original 1 (tracker) + cooperative P1 fixtures | Canonical serialization, manifest IDs, leases, golden vectors, tracker endpoints |
| C — Single & hedged | Original 2–4 (runtime, transport, gateway) | First **real** benchmark runs; harness unfrozen→executed |
| D — Exact two-peer speculation | Cooperative P1–P3 | Internals lab → local → two-node |
| E — Multi-proposer trees | Cooperative P4 (+P6 search behind its own gate) | `search_verified` stays separate from lossless |
| F — Public hostile swarm | Cooperative P7 + ADR-014 traversal + original 5 hardening | Relay goes live |
| G — Windows beta | Original 6–7 + cooperative P8 | Signed installers, benchmark view |

Overrides already decided: crate layout (ADR-010), profile identity
(ADR-011), lease naming (ADR-012), mode registry + formula (ADR-013),
traversal (ADR-014). The cooperative plan's `CooperativeMessage` names are
superseded by the revision's `SwarmMessage` set; its statistical protocol,
prerequisite gate (absorbed as C/D entry conditions), and per-phase
acceptance rigor carry forward unchanged.

`docs/acceptance/phase-1.md` remains the tracker's acceptance source for
Phase B (paths amended in place per ADR-010). Per-phase acceptance and
verification docs continue: `phase-a.md`, `phase-b.md`, …

## Consequences

+ One active numbering system; older plans become reference material.
+ Every future "which plan wins" question resolves by table lookup here.
