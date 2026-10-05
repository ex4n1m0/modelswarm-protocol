# ModelSwarm Risk Register

Status: Phase 0. Reviewed at every phase gate; risks are closed only with
evidence. Severity = impact × likelihood (H/M/L).

| # | Risk | Sev | Mitigation | Owner | Phase check |
|---|---|---|---|---|---|
| R1 | HF auto-updates split the swarm into incompatible replicas | H | Immutable profile IDs + digest pinning (ADR-005); staged promotion | Architect | Phase 1 catalog tests |
| R2 | Model license forbids use/redistribution | H | Direct-from-HF download only; per-profile license review + recorded acceptance; no weight redistribution | Architect | Phase 2 acceptance |
| R3 | Vercel treated as relay → duration/bandwidth blowup | H | ADR-001 boundary; relay explicitly out of hub; CI check: no prompt fields in hub schemas | Tracker | Phase 1 tests |
| R4 | NAT blocks direct connects; users assume Vercel relays | M | Explicit connect-failure states in protocol + UI; collect failure rates before relay milestone | P2P | Phase 3 sim |
| R5 | Free-riders consume without hosting | H | Capability tokens, live challenges, lease coupling (ADR-006); documented deterrence limits | Rust Core | Phase 5 adversarial tests |
| R6 | Serving peers read/leak prompts | H | Disclosure at onboarding; encrypted transport; invite-only v0.1; redaction audits | QA/Security | Phase 7 audit |
| R7 | Malicious/mislabeled GGUF or runtime artifact | H | Approved catalog only; digest + commit-hash pinning; pinned llama.cpp build checksum | Runtime | Phase 2 verification |
| R8 | Sidecar exposed to LAN | H | Loopback bind + random bearer secret; CI lint forbids 0.0.0.0 | Runtime | Phase 2 tests |
| R9 | Scheduler trusts self-reported metrics | M | Requester-measured EWMA dominates; active probing; circuit breakers | Gateway/Sched | Phase 4 tests |
| R10 | Subagent interface drift breaks integration | M | Frozen schemas; ADR-only changes; owned paths; sequential merges (AGENTS.md) | Integrator | Every phase |
| R11 | SmartScreen blocks unsigned installers | L | Keep builds private + labeled; code-sign before any public release | Windows UX | Phase 6 |
| R12 | Scope creep (marketplace, DHT, KV-transfer, payments) stalls prototype | H | Non-goals in build plan; phase gates require review to expand | Integrator | Every phase |
| R13 | Hub signing key compromise | H | Env-var-only key storage; rotation runbook; short token TTLs bound damage | Tracker | Phase 1 ops doc |
| R14 | Tracker outage degrades swarm | M | Leases expire gracefully; existing streams unaffected; UI degraded state | Tracker | Phase 7 drill |
| R15 | Windows path/AV interference with sidecar + models | M | Test on clean Win10/11 VMs early (Phase 2, not Phase 7); documented exclusions if needed | QA/Security | Phase 2 |
| R16 | Cooperative research starts on an unstable baseline or overclaims speedups | H | Prerequisite gate in `docs/cooperative-plan.md` (original phases complete first); fastest-eligible-single comparator rule; per-phase gates with negative results preserved | Integrator | Follow-up P0 audit |
| R17 | Multi-peer cooperative modes widen prompt exposure (N peers receive the prompt) | H | Privacy disclosure in cooperative UI (P8); invite-only swarm; cooperative prompt delivery documented in `docs/privacy.md` when P0 begins | QA/Security | Follow-up P8 |
| R18 | Insufficient differentiation from prior art (p2ptokens ratio marketplace, LocalAI federation, Petals pipelines, exo LAN clusters) | H | Phase A prior-art matrix + testable differentiation paragraph (ADR-009); exact-profile micro-swarm + lossless speculation as identity | Prior-Art Auditor | Revision A |
| R19 | Three overlapping plans (0–7, P0–P8, A–G) cause cross-implementation and gate confusion | H | ADR-015 supersession map; newest-plan-governs rule in AGENTS.md/README until then | Integrator | Revision A |
| R20 | `speculative_exact` depends on runtime hooks llama.cpp does not expose (vLLM adapter or fork) | H | Phase D entry requires ADR on research runtime; stop condition on "large unsafe fork"; production llama.cpp path untouched | Runtime Engineer | Revision D |
| R21 | Performance-dishonesty creep (average baselines, simulated results, hidden regressions) | H | Fastest-eligible-single comparator rule in AGENTS.md; release gates forbid simulated claims; Scheduler Scientist must publish negative results | Test/Release | Every revision phase |
| R22 | PgStore SQL never executed against live Postgres; serverless rate-limit dilution (per-instance buckets) | M | CI must add postgres service container running the env-gated suite before deploy; Vercel-level rate limiting or shared-store limiter at deployment time | Tracker Engineer | Pre-deploy |
| R23 | peerId placeholder equality (installation_id) shipped in Phase B | M | ADR-020 dual-derivation contract + Phase F migration task (multihash binding); goldens updated then | Network Engineer | Phase F |

## Standing review triggers

- Any new dependency → license + vulnerability check appended here.
- Any new endpoint in `protocol/msp-v1.md` → abuse case row added in
  `docs/threat-model.md`.
- Any red/failed phase acceptance item → risk added or escalated before merge.
