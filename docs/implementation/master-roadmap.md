# Master Roadmap — M0–M13 (plan of record)

Status: plan of record per ADR-027 (owner-approved architecture gate D2,
2026-10-09 — `docs/reviews/architecture-gate-2026-10-09.md`). Statuses below
are the audit-corrected snapshot from gate §C.3. Each phase ends with a
verification doc `docs/verification/m<N>-<slug>-<date>.md` (commit,
environment, commands, results, failures, limitations, security review,
environment-labeled performance, release recommendation); the next phase does
not start until the previous one is reviewed. A–I and F0–F3 verification
docs remain historical evidence (crosswalk in ADR-027).

## Critical path (gate §C.4, dependency-ordered)

R0 governance ADRs → **P1 streaming** → F15 measurement plumbing →
scheduler shadow-mode wiring → receipts-v2 + granted accounting → suspension
wiring (migration-0004) → fair queueing → reputation chassis → HEDGED
honesty patch → BALANCED light-up → presets per-tier light-up.

Parallel tracks: F2b Swarm migration (Network) · M1/M2/M4/M7-gap product
track (Windows) · M3/M5 hardening (Runtime/Tracker) · P4 fuzzing + P7
test-gate closures (Security/Test) · 9.6 pass 1 after F15 (Scheduler).
WAN + k=4 after 9.6; M11/M12 after M10.

Full ordering edges: `docs/implementation/dependency-graph.md`.

## Phases

| Phase | Status (2026-10-09) | Scope / exit gate | Evidence |
|---|---|---|---|
| **M0** baseline & recovery | **DONE** — validated; CI green restored (`63b98e1` run 37858753428 and `86450aa` run 37860513036 both verified success) | Existing behavior can be rebuilt and restored | freeze + validation audits; `docs/verification/phase-0.md` |
| **M1** hardware profiler | PARTIAL-seed (hardware detection exists; profiler crate NEW) | Platform-neutral profile; validated against a documented machine matrix; failures produce safe conservative values | — |
| **M2** resource governor | NEW (governor ADR incl. 70%-vs-90% reconciliation, D11: recommendation engine may advise up to its own bar; runtime governor enforces 70%) | Foreground stays responsive; memory-exhaustion tests cannot freeze the machine; contribution yields before local chat | winjob seed (kill-on-close + total RAM only) |
| **M3** signed model catalog | PARTIAL → harden (5 actionable rows: license metadata, runtime const enforcement, candidate validation, per-request signing) | Unapproved/tampered profiles cannot be auto-selected; identity consistent Rust/tracker/client | Phase B (schema v2, ADR-011/022) |
| **M4** automatic model selection | NEW (top-6 recommender seeded; automatic selection new) | Zero-configuration selection; fits under stress; explainable in diagnostics | — |
| **M5** verified download & lifecycle | PARTIAL (resume, disk precheck, rollback/cleanup absent) | Interrupted/corrupted downloads never become active; failed upgrades recover automatically | Phase B/C acquisition |
| **M6** real local inference | **DONE** + P1 streaming refactor attached (measured 7.3× long-prompt win, TTFT 4 ms) | Real model answers via local API; no mock in the production path | Phases C/D; E0 determinism |
| **M7** minimal client | DONE with gaps (copy button, Stop drain) | Nontechnical tester installs, starts, chats, copies API address, stops — without documentation | Phases G/H/I (v0.2.20) |
| **M8** automatic swarm joining | DONE on LAN (F2b Swarm migration outstanding) | Two physical machines exchange real inference work | F0/F1 `phase-f-lan-2026-10-07.md` |
| **M9** single-peer production baseline | PARTIAL (WAN outstanding; P1 + F15 prerequisites — no baseline freezes before P1, D10) | Fastest-eligible-host selection, streaming, failover; labeled LAN/WAN baselines P50/P95/P99 | F-series; E0 |
| **M10** cooperative inference + trust substrate | NOT STARTED (11-step build order delivered; prerequisites P3/migration-0004, P16 engine adapter, F15 measurements) | Every mode reproducibly beats FASTEST_SINGLE or falls back; results labeled by environment; granted accounting + fair queueing + reputation chassis live | Phases D/E science (sim/loopback); scheduler handoff §5 |
| **M11** deliberation | NOT STARTED (gated on mode-registry ADR + contract) | Objective benchmark quality justifies the compute; bounded debates only | — |
| **M12** multi-model federation | NOT STARTED (gated; ADR-027 records the reversal + preconditions) | Capability routing explicit; every output records which profile performed each role (exit criterion); cross-model work never labeled exact lossless unless the target verifier enforces it | — |
| **M13** production website & release | PARTIAL (beta channel, signed updates, incident process, uninstall drill open; cert spend D5 pending) | Website → working chat with no manual model/network configuration | Phase G releases (v0.2.20) |

## Standing rules (unchanged by this roadmap)

Project rule (host-to-consume eligibility for the exact immutable profile),
AGENTS.md constraints 1–8, phase gates with verification evidence,
performance honesty (fastest-eligible-single comparator; negatives published
as visibly as wins; every number environment-labeled), shared-schema changes
only via ADR. The cooperative plan (P0–P8) remains the gated lossless kernel
— cited, not duplicated.

## R0 — the governance batch that opened this roadmap (DONE with this batch)

ADR-027 (this plan of record) · ADR-028 (tracker-surface/wire
reconciliation + msp-v1 changelog) · ADR-029 (execution presets +
`protocol/msp-cooperative-v1.md`) · ADR-030 (receipts v2). One R0.5 code
item rides ADR-028: the P2P error-code rename (wire-visible, client+server
together). Post-gate first moves confirmed by the gate sign-offs: P1, P4,
P11.
