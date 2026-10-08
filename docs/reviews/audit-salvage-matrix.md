# Audit — Salvage Matrix (Consolidated) — 2026-10-09

Merges the code-inventory sweep (35 rows) with the five wave-2 domain
handoffs. Every row traces to its source audit. Classification
vocabulary per the master prompt: KEEP / KEEP WITH TESTS / REFACTOR /
REPLACE / DELETE / QUARANTINE / NEEDS EXPERIMENT.

**Headline verdict: incremental correction everywhere — zero wholesale
rewrites.** The two REPLACE rows are narrow and interface-preserving; the
one DELETE is a dead IPC command. Nothing is QUARANTINE. Approximate
counts: KEEP ≈ 22 · KEEP WITH TESTS ≈ 21 · REFACTOR ≈ 11 · REPLACE 2 ·
DELETE 1 · NEEDS EXPERIMENT 5.

## Protocol / catalog / types

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `protocol/msp-v1.md` | frozen wire spec | architect handoff | KEEP | needs reconciliation ADR (5 unlabeled endpoints), changelog section, doc-sync pass |
| `catalog/schema-v2.json` + `protocol/vectors/` + keys | identity + parity pins | architect, runtime | KEEP | properly ADR-gated (ADR-011/022 parity locks); add receipts/preset vectors with those ADRs |
| `catalog/schema.json` (v1) | dead-but-referenced | architect, dependency | REPLACE (retire references) | msp-v1 §3.5/§4 still point at it; dropped `licenseId` is an M3 risk |
| `modelswarm-types` (manifest, canonical JSON) | identity derivation, one canonical impl | code-inventory | KEEP WITH TESTS | golden-vector culture strong; `ModelProfileId` validates only the dead v1 `msp:` shape — retire/rename (type-level trap) |
| `modelswarm-session` `spec.rs` SpecMessage | cooperative signed vocabulary | architect, scheduler | KEEP WITH TESTS + ADR | `msp-cooperative-v1.md` dangling; PrefixCommit must bind version/nonce/deadline |

## Identity / eligibility

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `modelswarm-identity` | Ed25519 identity, derivations | security, architect | KEEP WITH TESTS | identity.seed plaintext vs DPAPI claim — fix storage |
| `modelswarm-eligibility` lease | ADR-026 wire + verification | architect §4 | KEEP WITH TESTS | byte-pinned end to end |
| `eligibility/policy.rs` suspension | policy table, no caller | architect + tracker (T2) | KEEP + WIRE | needs migration-0004 (DDL blocks slots-to-zero) + refusal-event feeder; prerequisite for M10 reputation |

## Transport / network

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `transport::frame` / `transport` (staged TCP) / `message` / `handshake` | semantics reference impl | network | KEEP WITH TESTS | ADR-018 staging; hostile-prefix tests; §6 flow-level drift fixed by ADR |
| `transport::libp2p_backend` | production raw-QUIC | network | KEEP WITH TESTS → **REPLACE incrementally at F2b** | migrate to libp2p Swarm (same public surface); 4–6 d + ADR |
| `node::remote` (pooled RemoteExecutor) | requesting side | network | KEEP WITH TESTS | fix retryable:false mapping (F16); pool poisoning: none reachable |
| `node::serving` (bridge) | serving loop + lease gate + admission | network, security | KEEP WITH TESTS | fix error-code drift, cancel-over-pool, drain; admission-before-gate ordering |
| `modelswarm-relay` | standalone relay host | network | KEEP WITH TESTS | harden (auth + per-circuit caps) before internet exposure |
| Multi-peer-per-request transport | does not exist | network | NEEDS EXPERIMENT | then build; prerequisite for HEDGED/BALANCED |
| Peer telemetry EWMA (QUIC RTT/loss) | does not exist on production path | network + scheduler | NEEDS EXPERIMENT (then build) | measurement prerequisite for scheduler/M9 (F15) |
| Network-emulator configs (loss/bandwidth/delay) | essentially none | network | NEEDS EXPERIMENT | required for reproducible WAN/degraded baselines |

## Runtime / model integration

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `runtime` trait + KvCommitment | runtime abstraction | runtime | KEEP WITH TESTS | ADR-019 frozen shape |
| `llamacpp.rs` exact-id recovery | token-exact correctness | runtime | KEEP WITH TESTS | fail-closed id+bytes; streaming-compatible |
| `llamacpp.rs` `decode_stream` + `executor.rs` event materialization | decode path | runtime (measured) | **REFACTOR — the critical one** | true SSE adapter + incremental events; measured 7.3× long-prompt throughput, TTFT 4 ms vs completion-time today (F1); before M9 freeze |
| `runtime/src/mock.rs` | test double | runtime | KEEP | feature-off in production |
| `node/artifact.rs` ArtifactManager | possession + downloads | runtime, windows | KEEP WITH TESTS → REFACTOR | add resume (19 GB restarts), disk precheck, cleanup/rollback (M5) |
| `node/engine.rs` supervisor | engine lifecycle | runtime, windows | KEEP WITH TESTS | fix blocking verify read; backoff nit |
| `runtime-pins.json` + `resolve-candidate.mjs` + `promote-candidates.mjs` | pinning + catalog pipeline | runtime, dependency | KEEP | hash chain exemplary; validate candidate files in vectors script |

## Scheduler / science

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `modelswarm-scheduler` formulas | frozen cost model v2 | scheduler | KEEP | ADR-013 core, deterministic |
| scheduler as production selector | not wired | scheduler (S1) | KEEP + WIRE | small, tested-code-only; needs F15 measurements first |
| `modelswarm-speculation` algorithms | pure exact algorithms | scheduler | KEEP WITH TESTS | token-exact contracts hold for any draft policy |
| real two-peer speculative execution | mock/loopback only | scheduler | NEEDS EXPERIMENT (9.6) | engine adapter prerequisite (F17); pass 1 expected-negative is a first-class result |
| TOKEN_TREE verification over llama.cpp HTTP | impossible over HTTP | scheduler | NEEDS EXPERIMENT (expect negative) | linear-verify fallback; publish honestly |
| `modelswarm-bench` | honest mock harness | scheduler | KEEP WITH TESTS | fix model-derived baseline (S4), netem gap (S6) |
| `apps/modelswarm-sim` | contract runner | scheduler, test-release | KEEP WITH TESTS | add rtt-spike-detection case (S5) |
| `experiments/` | empty, schemas frozen | scheduler | KEEP | first real entries with 9.6 |

## Tracker (apps/tracker + tracker-api)

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `lib/schemas/envelope/guard/leases/errors/crypto` | API core | tracker | KEEP | replay/precedence solid |
| `lib/ratelimit.ts` | per-instance limiter | tracker | KEEP WITH TESTS | document semantics; dead `rate_counters` table |
| `lib/store.ts` | Postgres + memory dual impl | tracker | REFACTOR | write-only tables, prune policy, drift risk (T5) |
| `lib/eligibility.ts` + `CHALLENGE_PROMPT` | earn chain | tracker (T1) | REFACTOR / **REPLACE the prompt constant** | nonce-bound + digest-verified canary (rides E0); relabel `verified_capacity` now |
| `catalog/requests` route | community intake | tracker | REFACTOR | route through `guardPublicBody` (uncapped read) |
| other routes / pages / migrations 0001–0003 | surface + storage | tracker | KEEP (migrations WITH TESTS) | extend DDL test scope; migration-0004 pending (suspension) |
| `crates/modelswarm-tracker-api` | Rust client | architect | KEEP WITH TESTS | shapes match post-H1-fix |

## Desktop / node / Windows / installer

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| desktop IPC surface (`app.rs` commands) | product API | windows | KEEP WITH TESTS | add tauri-shell clippy gate (F11 CI hole); extract modules from the 2,301-line monolith |
| hardware detection + `fit_and_score` | M1/M4 seed | windows | REFACTOR | extract platform-neutral crate; multi-source; fix 90%-vs-70% policy gap (F13); fix abort-on-bad-line |
| `select_model` swap / `send_chat`/`try_swarm_chat`/`earn_lease` | UX + swarm chat | windows | KEEP WITH TESTS | clear `chat_log` on switch/stop (F2); pool cleanup on Stop; drain (F5) |
| `download_model` IPC | legacy | windows | **DELETE** | unused by UI, wrong semantics |
| `modelswarm-winjob` | enforcement seed | windows | KEEP WITH TESTS → extend | CPU-rate/memory job-limit classes behind the M2 governor ADR |
| `executor.rs` (local) | ADR-025 anchor | windows, code-inventory | KEEP | regression-pinned |
| `ui/index.html` | the product screen | windows | REFACTOR | copy button (missing), transcript clear, patch-in-place cards, scripted geometry gate, a11y residue |
| installer workflows + stage scripts | release gates | windows, dependency | KEEP | per-file hash verify + engine-inside-artifact checks exemplary |
| `installer/tauri/nsis-notes.md` | packaging doc | windows | REFACTOR | stale pre-data-split claims; F4 uninstall VM test pending |

## Integrator-held crates

| Component | Current role | Evidence | Decision | Reason / dependencies |
|---|---|---|---|---|
| `modelswarm-gateway` | local OpenAI API | code-inventory, security | KEEP WITH TESTS | SSE emit rides the F1 refactor; add Origin/Host checks (DNS rebinding) |
| `modelswarm-telemetry` | redacted logging | security | KEEP WITH TESTS | move redaction from convention to structural (typed) |
| `modelswarm-store` / `modelswarm-node` composition | persistence + daemon | code-inventory | KEEP WITH TESTS | blocking rusqlite in async handlers (time-boxed fix) |
