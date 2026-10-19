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
| **M10** cooperative inference + trust substrate | **IN PROGRESS — trust substrate LIVE, science remains** (2026-10-19 audit: build-order steps 1–9 ALL complete — R0 ADR-027..030 · F15 measurements · shadow planner · 9.6 pass 1 negative recorded · scheduler wiring · engine adapter spike + FULL ADR-031 §5 productionization gate closed 2026-10-11 incl. Vulkan + exactness pins, speculative A/B 1.45–1.57× loopback · receipts v2 in production · fair queueing + reputation chassis SHIPPED v0.2.40; ADR-032 light-up tier live since v0.2.21). Remaining: **9.6 pass 2 over the capi-composed path** (repeat/P50 protocol, loopback then owner-gated LAN — the every-mode-beats-or-falls-back evidence) + final preset tiers per build-order step 11 | Every mode reproducibly beats FASTEST_SINGLE or falls back; results labeled by environment; granted accounting + fair queueing + reputation chassis live | Phases D/E science (sim/loopback); scheduler handoff §5 |
| **M11** deliberation | NOT STARTED (gated on mode-registry ADR + contract) | Objective benchmark quality justifies the compute; bounded debates only | — |
| **M12** multi-model federation | NOT STARTED (gated; ADR-027 records the reversal + preconditions) | Capability routing explicit; every output records which profile performed each role (exit criterion); cross-model work never labeled exact lossless unless the target verifier enforces it | — |
| **M13** production website & release | PARTIAL (beta channel, signed updates, incident process, uninstall drill open; cert spend D5 pending) | Website → working chat with no manual model/network configuration | Phase G releases (v0.2.20) |
| **Directives 2026-10-19** (owner) | **1. Outbound DNS via 1.1.1.1 — DONE in `dde91e3`**: DNS-over-HTTPS (`crates/modelswarm-dns`, bootstrap-free TLS endpoint) wired into every outbound client (tracker, HF downloads, cloud peers, advisory, desktop surfaces); honest counted system fallback; `MSP_DNS=system` opt-out, `MSP_DOH_URL` override; live-verified zero-fallback. **2. Huihui Qwen3.5 9B abliterated download — RESOLVED**: the owner-named upstream repo ships safetensors only (no GGUF — what the app runs); the mradermacher Q4_K_M GGUF was hash-verified (sha256 ea1858ef…6e4b3), resolved through the ADR-022 pipeline, promoted LIVE; this machine pre-staged byte-identical. Roadmap note: the resolver script's manifest self-test needed the ADR-030/038-vector filter fix (same commit) | Outbound resolution never depends on ISP DNS; catalog additions ride the pinned-digest pipeline | — |
| **Update-safety: engine-file locks during install** (Integrator, 2026-10-19 evening incident, Windows Product seat) | **FIX STAGED, BUILD VALIDATION OWED**. Live repro during the v0.2.41 updater run on machine A: any `llama-server.exe` alive from the engine dir (here: an orphaned pass-2 experiment child whose cargo parent died at session restart; an app-supervised engine dies cleanly with the app) holds `engine\ggml-base.dll` etc. open → the NSIS installer stalls at "Error opening file for writing" until Retry. Recovered by killing the orphan (PID-pinned, command-line-verified against the engine dir) and clicking Retry; install then completed and the updater relaunched the app normally. Fix now in `installer/tauri/installer-hooks.nsh` (`NSIS_HOOK_PREINSTALL`, wired via `bundle.windows.nsis.installerHooks`): kill only processes whose image path is under `$INSTDIR\engine` (never image-name kills — the owner may run an unrelated llama.cpp), settle 500 ms, then extract. Owed: one local `cargo tauri build --bundles nsis` to prove the hook compiles (next train's CI also gates it) | An update never stalls on a locked engine file; the installer is idempotent against orphaned engine processes | — |

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
