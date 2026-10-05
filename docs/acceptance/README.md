# Acceptance tests, per phase

Index of exact acceptance criteria. Each phase's file is written at the start
of that phase (or before, when the contract is frozen) and must pass with
recorded evidence in `docs/verification/<phase>.md` before the next phase
starts.

| File | Scope | Status |
|---|---|---|
| `phase-1.md` | Tracker hub: schemas, signed envelopes, leases, tokens, abuse limits | Frozen in Phase 0 |
| `phase-2.md` | Node: identity, store, HF download/verify, llama.cpp supervision — **must include a log-redaction assertion once any code path touches license/challenge prompts** | To write in Phase 2 |
| `phase-3.md` | P2P: malformed frames, replay, wrong signatures, wrong profile, expired leases, oversized prompts, cancellation, disconnects, slow consumers — **must include a prompt-redaction gate (prompt text first flows here)** | To write in Phase 3 |
| `phase-4.md` | Gateway/scheduler: routing determinism, retry semantics, circuit breaker — **second prompt-redaction gate** | To write in Phase 4 |
| `phase-5.md` | Capability enforcement: token copy/forgery/expiry, clock skew, stale leases, patched metrics | To write in Phase 5 |
| `phase-6.md` | Installer, UX states, privacy disclosures | To write in Phase 6 |
| `phase-7.md` | Full release matrix from `docs/build-plan.md` | To write in Phase 7 |

## Revision track: phases A–G (governs going forward)

The competitive-learnings revision (`docs/competitive-revision-plan.md`)
defines phases A–G with hard exit gates and its own release-gate list.
Mapping to the older numbering (formalized in ADR-015):

| Revision phase | Contains (from older plans) | Acceptance home |
|---|---|---|
| A — Audit & rebaseline | Prior-art matrix, ADR-009…015, manifest schema v2, fastest-single harness *frozen* | `phase-a.md` — **executed 2026-10-04, see `docs/verification/phase-a.md`** |
| B — Protocol foundation | Original Phase 1 (tracker) + canonical serialization, leases, golden vectors; property gates | `phase-b.md` — **executed 2026-10-05, see `docs/verification/phase-b.md`** |
| C — Single & hedged | Original Phases 2–4 (runtime, transport, gateway) + racing | `phase-c.md` — **executed 2026-10-05, see `docs/verification/phase-c.md`** |
| D — Exact two-peer speculation | Cooperative P1–P3 (internals lab, local spec, two-node protocol) | `phase-d.md` — **executed 2026-10-05, see `docs/verification/phase-d.md`** |
| E — Multi-proposer trees | Cooperative P4 (+`search_verified` from P6 behind its own gate) | `phase-e.md` |
| F — Public hostile swarm | Cooperative P7 + NAT/relay (amends ADR-003) + original Phase 5 hardening | `phase-f.md` |
| G — Windows beta | Original Phases 6–7 + cooperative P8; signed installers | `phase-g.md` — **executed 2026-10-05 (composition+shell; signing/VM stops recorded), see `docs/verification/phase-g.md`** |

Nothing in A–G has started. The Phase A exit gate (differentiation paragraph
+ approved ADRs + frozen harness/schemas) is defined in
`docs/reviews/phase-a-first-response.md`.

## Follow-up track: cooperative inference (gated)

Phases P0–P8 are specified in `docs/cooperative-plan.md` with per-phase gates
and their own acceptance table. They have **not started**: the prerequisite
gate (original prototype complete with baseline metrics, eligibility, and
privacy verification) is unmet while only Phase 0 exists. When P0 begins,
per-phase acceptance files `phase-p0.md`…`phase-p8.md` are authored here
under the same rule below, plus the follow-up's scientific baseline rule
(fastest eligible single host as comparator).

Rule: a phase's acceptance file may only be authored from the frozen contract
documents (`protocol/`, `catalog/`, ADRs) — never from an implementation.
