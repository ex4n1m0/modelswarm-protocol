# Phase A First Response — Audit, Redesign Map, and Proposed ADRs

This is the first response required by `docs/competitive-revision-plan.md`
("First ZCode response required"). It claims no implementation and begins no
later phase. Date: 2026-10-04 · Repo state: commit `c2cd8d2`.

## 1. Repository audit summary

What exists (verified, per `docs/verification/phase-0.md`):

- **Governance**: `AGENTS.md` (ownership, handoffs, phase gates), 8 ADRs,
  17-risk register, per-phase acceptance index, two integrated follow-up
  plans (`docs/build-plan.md` original, `docs/cooperative-plan.md` P0–P8,
  both gated).
- **Frozen contracts**: `protocol/msp-v1.md` (signed envelopes, hub REST,
  catalog envelope, lease-capped capability tokens, P2P stream protocol,
  two-phase receipts — reviewed by two independent agents, 12 blocking
  findings fixed pre-freeze), `protocol/messages.proto`, `catalog/schema.json`
  v1.
- **Code**: Cargo workspace of 9 stub crates + 3 stub apps; only `ms-core`
  has logic (protocol constant, validated typed IDs, 4 unit tests). Hub
  skeleton: one real route (`GET /api/v1/health`, smoke-tested `200`). CI
  authored, never run (no Git remote).
- **Checks green locally**: `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` (4/4), hub
  `typecheck` + `build`.

Honest reality check: the revision document assumes a working prototype to
audit. What actually exists is **Phase 0 design + scaffold** — no runtime, no
tracker behavior, no benchmarks have ever run. The audit below therefore maps
*architecture and contracts*, not running code, and Phase A's
"fastest-single-host benchmark" can be **frozen** (harness spec, metric
schemas, corpus, statistical protocol) but not yet **executed**; execution
lands after Phases B–C deliver runtime + transport. This is recorded rather
than papered over.

## 2. Components that can remain unchanged

| Component | Why it survives the revision |
|---|---|
| Governance process (AGENTS.md rules, ADR discipline, HANDOFF, verification reports, phase gates) | The revision mandates the same discipline; roster names change, rules do not |
| ADR-001 content-blind Vercel control plane | Revision *strengthens* it (content blindness is a first-class property) |
| ADR-002 llama.cpp pinned sidecar | Retained as baseline runtime; "runtime trait" was already the crate design |
| ADR-004 Ed25519 identity + PeerId binding | Already required by msp-v1 §2.1/§6.6 |
| ADR-005 immutability *principle* | The mechanism changes (§3 below), the principle does not |
| ADR-007 retry-before-first-token / honest interruption | Core `single`-mode semantics, unchanged |
| ADR-008 Tauri/NSIS packaging | Phase G adds signed MSI+EXE on top |
| Core eligibility rule + three-layer enforcement | Unchanged; token→lease is a rename + field extension |
| Privacy model, redaction architecture, prompt-lifecycle guarantees | Retained; multi-peer extension already flagged in `docs/privacy.md` |
| Threat-model structure + adversary set | Retained; extended with audit collusion/equivocation |
| Hub skeleton, CI workflows, check gates | Retained; paths/names updated |
| `ms-core` types (HfRevision, Sha256Digest, canonical-JSON rules, validation style) | Move into `modelswarm-types` as-is (ModelProfileId semantics change) |

## 3. Components requiring redesign

| Current | Revised | Nature of change |
|---|---|---|
| Crates `ms-core/ms-crypto/ms-p2p/…` | `modelswarm-types/identity/transport/tracker-api/eligibility/runtime/scheduler/session/speculation/bench/node/desktop` | Mechanical rename + 4 new crates (tracker-api, eligibility, session, speculation, bench); history-preserving `git mv` |
| `hub/` Next.js app | `apps/tracker/` with endpoint modules (register, heartbeat, candidates, session-authorize, lease, receipt, audit, model-catalog) | Move + new endpoints (session-authorize, lease, receipt, audit, model-catalog); stateless hub + external durable store |
| `ModelProfileId` = human-minted `msp:<family>:<quant>:vN` | Derived from canonical `ModelProfileManifest` serialization (artifact_hashes[], tokenizer/chat-template/architecture hashes, quantization, runtime build hash, decoding_abi_version, speculative_capabilities) | **Breaking change to frozen catalog schema v1 + msp-v1 §4** → schema v2 + ADR + acceptance-test migration |
| Possession = file digest | Digest + randomized chunk challenges + live inference challenge | Extends §3.3 challenge flow |
| Capability token (ADR-006) | `EligibilityLease` (+`verified_capacity`, `audit_epoch`) | Rename + fields; lease-capped TTL (lease+60 s) retained; bootstrap allowance is new policy surface (scoped off by default) |
| Execution = single stream (msp-v1 §6) | Five-mode registry: `single`, `hedged`, `speculative_exact`, `search_verified`, `map_reduce`, each with a correctness contract | msp-v1 §6 becomes the `single` mode; cooperative namespace `/msp/cooperative/1.0.0` with `SwarmMessage` set (supersedes cooperative-plan's `CooperativeMessage` naming; adds AuditChallenge/AuditResult/PeerReplace/CommitAck) |
| Scheduler formula v1 (msp-v1 §10 arch) | v2: prefill+proposal+verification+synchronization+rollback+failure-risk vs fastest-single + conservative confidence margin | Amends architecture.md §10 |
| NAT posture: direct-only, relay "later" (ADR-003) | Relay/DCUtR/AutoNAT promoted to Phase F deliverable as a **separate service** (never Vercel) | Amends ADR-001/003 boundary wording |
| Phase numbering: 0–7 and P0–P8 | A–G governs; older systems mapped (ADR-015) | Documentation supersession |

## 4. p2ptokens recommendation

**Differentiate; study selectively; do not fork; no interop in v0.1.**

Verified against the repository (2026-10-04): MIT license; Rust + libp2p +
Tokio + axum + Tauri; early-stage (28 commits); v1 coordinator is in-memory,
no DB; peers serve via Ollama/OpenAI-compatible backends; access governed by
BitTorrent-style upload/download ratio with newcomer grace, optimistic
unchoke, and signed co-receipts ("neither side can lie by more than one
chunk"); fan-out modes (single/racing/quorum/ensemble) send a prompt to
several peers independently. **No speculative decoding, no cooperative
acceleration of a single generation**; Petals-style sharding deferred to v2+.

Reasoning:

1. The overlap with our work is commodity infrastructure (heartbeats, signed
   receipts, NAT, retries, content-blind coordinator) that we have *already
   frozen in our own contracts* — adopting their code would import a product
   shaped around a different thesis (ratio marketplace, Ollama-centric,
   in-memory coordinator) and force rework of frozen interfaces.
2. Our differentiators do not exist there: manifest-derived exact-profile
   identity with chunk+inference possession challenges; micro-swarm
   cooperative execution with lossless correctness contracts;
   fastest-single-comparison with automatic fallback.
3. MIT permits code reuse with attribution, so **pattern-level adoption is
   licensed**; concrete code reuse is deferred to a Phase A comparison
   finding a specific module that is a clear net win (ADR-016 if it happens).
4. Study items worth harvesting: co-receipt settlement mechanics for our job
   receipts; newcomer grace / optimistic unchoke for eligibility bootstrap
   design; their NAT/retry test cases.

## 5. Proposed ADR list (Phase A deliverables)

- **ADR-009** Revised positioning & differentiation (testable paragraph below;
  marketplace framing demoted).
- **ADR-010** Workspace restructure (`ms-*` → `modelswarm-*`, `hub/` →
  `apps/tracker/`; history-preserving moves; CI path updates).
- **ADR-011** Manifest-derived `ModelProfileId`, catalog schema v2,
  chunk+inference possession challenges, canonical-serialization + shared
  golden-vector format.
- **ADR-012** `EligibilityLease` (amends ADR-006; bootstrap allowance scoped
  as research policy, default off until Phase F evidence).
- **ADR-013** Execution-mode registry, per-mode correctness contracts,
  scheduler cost model v2 + conservative margin + fallback trigger.
- **ADR-014** NAT traversal roadmap (Phase F: relay/DCUtR/AutoNAT as a
  separate service; amends ADR-001/003).
- **ADR-015** Plan supersession map (A–G governs; original 1–7 and
  cooperative P0–P8 mapped to A–G; acceptance-doc homes).
- **ADR-016** (conditional) p2ptokens reuse decisions, if any concrete module
  adoption survives the comparison.

Draft differentiation paragraph (the Phase A exit-gate artifact, each clause
mapped to a test):

> ModelSwarm admits a peer to a swarm only for the exact immutable profile it
> verifiably hosts — identity is derived from a canonical manifest and proven
> by randomized chunk challenges plus live inference [test: mismatched
> profile/hash/revision peers are excluded]. It executes one request in a
> network-aware micro-swarm of at most eight peers selected from a population
> of any size [test: bounded cohort regardless of registry size]. Its
> cooperative modes are lossless speculative decoding and verified candidate
> trees with explicit correctness contracts [test: greedy golden equality;
> sampled distribution tests]. And it cooperates only when an explainable
> cost model predicts beating the measured fastest eligible single host,
> reverting automatically at the break-even point [test: fallback triggers on
> injected RTT/acceptance collapse]. No audited prior art — p2ptokens (ratio
> marketplace, fan-out), LocalAI (static federation), Petals (layer
> pipelines), exo (LAN partitioning) — combines these four properties.

## 6. Phase A task graph by subagent

Order: stage 1 parallel (Auditor, Architect), ADR review gate, stage 2
parallel specs, Integrator reconciliation. No speculation code.

| Subagent | Phase A deliverable | Owned output path |
|---|---|---|
| Prior-Art Auditor | Verified 10-system × 8-property matrix (the revision doc's table is the hypothesis, sources re-verified); overlap/gap report | `docs/research/prior-art-matrix.md` |
| Protocol Architect | Manifest canonical-serialization design; profile-id derivation; schema v2 draft; golden-vector format | drafts ADR-011; `protocol/` drafts |
| Runtime Engineer | Runtime trait spec (tokenize/prefill/decode/logits/verify/KV-commitment/rollback/cancel) + llama.cpp capability map; gaps documented | `crates/modelswarm-runtime/` docs |
| Network Engineer | Transport design for renamed crate; NAT/relay inventory; test topology (direct/hole-punch/relay) | drafts ADR-014 input |
| Scheduler Scientist | Fastest-single benchmark harness spec; frozen metric schemas (from cooperative-plan P0 list); cost model v2; negative-result format | drafts ADR-013; `apps/modelswarm-bench/` spec |
| Security Engineer | Extended adversary matrix (fabricated proposals, equivocation, audit collusion, prompt retention); challenge/audit flow design; fuzz-target list | extends `docs/threat-model.md` |
| Tracker Engineer | `apps/tracker/` endpoint map vs msp-v1 §3 (gaps: session-authorize, lease, receipt, audit, model-catalog); stateless-hub + external store plan | tracker design doc |
| Windows Product Engineer | Phase G requirements inventory (signed MSI+EXE, benchmark view, mode/peer/path visibility, privacy warnings) | spec only |
| Test and Release Engineer | CI extension plan (property tests, golden vectors, network-emulation matrix); reproducible-benchmark metadata format | CI + `docs/acceptance/phase-a.md` draft |

Exit gate: the differentiation paragraph is written and each clause maps to
an acceptance test; ADR-009…015 approved; benchmark harness + metric
schemas + corpus + statistical protocol frozen (execution deferred to Phase C
by explicit note, since no runtime exists yet).

## 7. Risks and unknowns

1. **Three-plan supersession ambiguity** (R19): original 1–7, P0–P8, and A–G
   all exist. Without ADR-015's explicit map, agents will cross-implement.
2. **Benchmark-before-runtime**: fastest-single harness is freezable but not
   runnable until B–C land; the Phase A gate must say "frozen", not "run".
3. **Runtime hooks gap**: llama.cpp exposes no proposal/verification hooks
   today; `speculative_exact` depends on a pinned vLLM adapter or a
   controlled fork — the revision's own stop condition ("large unsafe fork")
   is live for Phase D.
4. **Bootstrap allowance vs no-economy stance**: eligibility grace for new
   peers must be off by default until Phase F evidence (conflict with
   original plan's non-goals).
5. **Manifest hash inputs**: `chat_template_hash`/`architecture_hash` may not
   be stably derivable for every HF artifact; the resolver must record
   evidence or fail closed (never guess — original plan rule).
6. **Golden-vector format across TS/Rust** undecided (canonical JSON vs
   CBOR) — resolved inside ADR-011.
7. **Phase F relay hosting** unsourced; paid-resource stop rule applies.
8. **CI still never executed** on a real runner (no remote configured).

## 8. Exact commands and tests Phase A intends to run

- Restructure (after ADR-010 approval): `git mv` operations, then
  `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D
  warnings`; `cargo test --workspace`.
- Tracker move: update `hub.yml` paths; `cd apps/tracker && npm ci && npm
  run typecheck && npm run build`.
- Schema v2 vectors: JSON-Schema validation of manifest test fixtures
  (small Node script or ajv CLI in tracker tests).
- Canonical-serialization property: Rust round-trip tests in
  `modelswarm-types` — serialize→deserialize→re-serialize byte equality;
  sorted-key canonical JSON tests.
- Golden-vector format check: same fixture file consumed by a Rust test and
  a Node test (parity harness, per `hub/tests/README.md` precedent).
- Property tests *designed* in A, implemented at Phase B start: stale,
  duplicated, reordered, and cross-profile messages cannot advance session
  state.
- **No performance runs in Phase A** (nothing exists to measure); benchmark
  harness ships with unit tests for its metric/report serialization only.
