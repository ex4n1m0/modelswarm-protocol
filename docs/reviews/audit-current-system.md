# Audit — Current System (Consolidated) — 2026-10-09

Integrator consolidation of the master-prompt audit (§5–§8). Baseline:
`730cebf` (v0.2.20) on branch `audit/master-prompt-2026-10-09`, tag
`audit-baseline-2026-10-09`; main advanced to `63b98e1` during the audit
(CI fix only). Sources: the freeze record plus the per-domain audit docs
listed in §10. **Status: COMPLETE — all eleven audits (freeze + wave-1
six + wave-2 five) consolidated.**

## 1. System state in one page

ModelSwarm v0.2.20 is a **working single-peer P2P inference product**:
signed catalog (15 active profiles), zero-click enrollment, HF downloads
with hash/identity verification, pinned llama.cpp engines (CPU + Windows
Vulkan variant, per-file hash-verified), a Tauri Windows client (plus
Linux/macOS builds) with Start/Stop + chat + local OpenAI-compatible API
(`127.0.0.1:11435/v1`), hardware-aware top-6 model shortlist, QUIC/Noise
P2P serving with connection reuse, hub-signed lease gating at session
open (ADR-026) with the earn chain live against production, and a live
content-blind tracker at modelswarm.deepflux.space. **Proven on real
machines:** cross-machine LAN serving with real models (F0/F1), greedy
determinism across GPU vendors (E0), relay standalone (F2A). **Proven
mock/sim/loopback only:** every cooperative mode; latency-aware selection.
**Not started:** resource governor, full hardware profiler, automatic
strongest-safe-model selection, relay usage from nodes (F2b), deliberation,
federation. Remote serving ships default-off behind `MSP_LISTENER=1`.

## 2. Validation and CI state (freeze evidence)

- Local suite at `730cebf`: **all ten commands PASS** (fmt; clippy ×2;
  tests 301 default / 313 libp2p-backend / 7 desktop tauri-shell; tracker
  typecheck + 110 vitest + build + golden vectors). Logs:
  `target/audit-logs/`, summary in the freeze record.
- CI at freeze: `rust` workflow **red on its last two runs** — the
  tauri-shell desktop-test step had *never* been green (tauri-build
  validates `bundle.resources` globs; clean runners never stage the
  gitignored engines). Fixed during the audit in `63b98e1`
  (CI-only `TAURI_CONFIG` overlay; proven by bare-runner repro
  101-fail → 7/7-pass). Residual risk + recommended guards-job
  resource-freeze assertion: see `audit-test-gap-analysis.md` §2.1.
- `tracker` / `desktop-installer` / `windows-package-dry-run` green.

## 3. What the audit confirmed about the plan-of-record claims

All eight §0 ground-truth claims of the master prompt were **confirmed
with evidence** (code-inventory audit): scheduler bench-only/unwired
(shipped chat = first QUIC peer); speculation mock/loopback only; relay
F2A-verified but standalone; winjob = kill-on-close + total RAM only;
no mock in the single-peer production path; multi-peer transport
unproven; LAN serving proven; desktop §1 interface shipped. ADR-016 is
a deliberately skipped number (Phase A closed it as not-needed) — no
missing document.

## 4. Consolidated findings — severity view (wave-1)

No CRITICAL code findings. Highest-impact items, deduplicated across
audits:

| # | Finding | Severity | Source | Fix shape |
|---|---|---|---|---|
| F1 | **Fake streaming end-to-end — CRITICAL, measured**: one HTTP POST per token with the prefix re-sent (`llamacpp.rs:510`); executor awaits full decode then emits a prebuilt Vec (`executor.rs:136-195`); gateway SSE + desktop `drain_chat` drain-after-completion. Measured on the pinned engine (135M, CPU): per-token loop 9.11 ms/tok at a 16-token prefix rising to 24.73 ms/tok at 1,215 (O(prefix) per token, O(n²) per completion); native `stream:true` on the same endpoint: **TTFT 4.0 ms, ITL p50 3.9 ms, 7.3× throughput at long prompts**. Streamed chunks carry `logprobs.content[]` with `id`+`bytes`, so exact-id recovery is streaming-compatible — the recorded reason for non-streaming does not hold | CRITICAL | code-inventory + runtime (measured) | true SSE adapter (~2–3 d) + incremental executor (~1 d) + desktop per-delta UI (~0.5 d) — **before any M9 baseline freeze** |
| F2 | **Hosting challenge is self-attested** (triangulated by 3 audits): two fabricated integers earn a consume lease; fixed public prompt replayable across installs; `capacityFromTimings` turns a claimed number into `verified_capacity` | HIGH | security + architect + tracker | honest relabel now; then nonce-bound prompt + pinned greedy-canary possession digest (hash-only, content-blind, rides proven E0 determinism); hub-side probes rejected (ADR-001) |
| F3 | **Scheduler unwired — selection is alphabetical**: cost model v2 exists; production chat picks the first roster peer with a free QUIC slot and tracker lookup returns alphabetical order, so shipped selection = alphabetical-first-with-capacity. Wiring `select_microswarm`+EWMA is small, tested-code-only work — but blocked by F15 (no measurements to feed it) | HIGH | code-inventory + scheduler | planner per expanded-mission §5: measurement plumbing first, shadow mode before action |
| F4 | **No fuzzing of any network parser** — §12 floor clause FAIL | HIGH | security + test-gaps | cargo-fuzz targets for transport frames, GGUF, canonical JSON, lease |
| F5 | **real-model-e2e never completed a run** (zero dispatches; broken-YAML era) | CRITICAL (test gap G2) | test-gaps | dispatch + record; add GPU-variant execution (G4) |
| F6 | P2P error codes diverge from msp-v1 §6.5 registry (`duplicate_request`/`over_limit` vs `replayed_request`/`overloaded`) — only client-visible wire mismatch | HIGH (protocol) | architect | rename or amend §6.5 via the reconciliation ADR |
| F7 | Five live tracker endpoints + `capacityClass` on the wire with no msp-v1 §3 presence and no ADR (`session-authorize`, `/receipt`, `/audit`, `/stats`, `/download`); `session-authorize.mode` unvalidated free string | HIGH (protocol) | architect | one retroactive tracker-surface reconciliation ADR |
| F8 | **ADR-012 suspension machinery ~1/3 implemented and structurally blocked**: policy fn has no caller; epoch bump exists but enforced only at lease refresh; `blockPeer`/`revokeToken` have store methods with no routes; **slots-to-zero is impossible (DDL `CHECK max_slots 1..8`)**; no refusal counters or test vectors; de-facto revocation bound = token TTL ≤ ~135 s. The M10 reputation chassis cannot "ride existing machinery" — it needs migration-0004 + wiring | HIGH (design input to gate) | architect + tracker | suspension ADR (migration-0004) + refusal-event feeder (Tracker + Security review) |
| F14 | Tracker hygiene cluster: write-only/unbounded tables (`peer_observations` per-heartbeat never read; sessions/device_codes/capability_tokens/session_authorizations); `DEVICE_APPROVAL_CAP=250` counts DISTINCT installs EVER — zero-click enrollment ends permanently once hit; `/api/download` counter writes unthrottled; `unknown_installation` envelopes hit Postgres with no per-IP limit; dead `rate_counters` table; wire-compat CI covers the core lease chain but NOT rendezvous, session-authorize, receipt/job-result, `/catalog/requests`, notices, manual approval, epoch suspension; M3 catalog: 5 actionable rows (license metadata, runtime const enforcement, unvalidated candidate files, per-request signing) | MEDIUM | tracker | §4 hygiene batch + migration; extend wire-compat job |
| F15 | **Zero RTT/loss telemetry on the production QUIC path** — `measure_rtt` exists only on the staged TCP backend; no EWMA store anywhere; blocks scheduler wiring (F3), M9 baselines, and HEDGED | HIGH (prerequisite) | network + scheduler | measurement plumbing first (QUIC RTT EWMA, completion distributions, measured queue depth) per expanded-mission §5 |
| F16 | **Pre-first-token failover broken in practice**: stale-pool / early transport failures map to `retryable:false` (`remote.rs:140-151`, `serving.rs:419-430`) and the gateway refuses retry, though ADR-007 permits failover before the first token; server's 300 s between-requests window vs TTL-less client pool makes stale sessions real | HIGH | network | map transport-failure classes to retryable=true pre-first-token; pooled-session health check |
| F17 | **Engine adapter blocks every lossless cooperative mode**: per-token prefix re-posting (F1) makes speculative speedup structurally ≤ 0 until an in-process C-API engine adapter exists; llama.cpp HTTP cannot tree-verify (honest fallback: linear verify + publish negatives). The 9.6 invalidating experiment is NOT READY (no real-peer bench runner, no RTT probe, no acceptance-EWMA store, no frozen prompt corpus) — pass 1 with the current adapter is cheap, honest, expected-negative, and that negative is a first-class result. Engine-adapter spike must land inside R0/R3 | HIGH (design input to gate) | scheduler + runtime | spike in R0/R3; 9.6 pass 1 scheduled as an honest negative if needed |
| F9 | MEDIUM cluster: relay open to anyone + uncapped per-circuit bytes; admission-before-lease-gate starvation (8×300 s); identity.seed plaintext vs DPAPI claim; loopback API DNS-rebinding (no Origin/Host checks); uncapped `/catalog/requests` body read; blocking-in-async (`--list-devices`, rusqlite, engine probe, `std::fs::read` in engine verify); plaintext identity/session tokens; msp-v1 §6.1 handshake skip unrecorded; §6.3 `accepted`/§7 receipts spec'd-but-undeferred; dual reqwest stacks + ed25519-dalek major split in the desktop binary; CI actions tag-pinned not commit-pinned; `@stable` toolchain in 3 workflows; `chat_log` never cleared on model switch/stop (cross-model thread bleed); GPU detection aborts on one bad stdout line; no tracker drain/deregister on Stop (M7 Stop-sequence gap; network audit concurs: no drain, no Cancel over pooled sessions); fixed 30 s frame deadline kills long-prefill requests; listener death undetectable (driver drops ListenerClosed/Error); cancellation is deadline-only (client abort not propagated); no download resume (19 GB artifacts restart from zero); `prefill_ms` hardcoded 0.0; hedged baseline `fastest_single_actual_ms` is model-derived, not an independent run; sim disables rtt-spike detection (multiplier 1e6); empty `StreamError.request_id`; quinn keepalive (5 s) vs 10 s idle-timeout interplay unverified (30-min experiment queued) | MEDIUM | security / architect / dependency / windows-product / network / runtime / scheduler | individually time-boxed; see per-audit docs |
| F10 | **§1 product-spec deviations**: local-API **copy button missing** (index.html:175-180); Peers/Setup/diagnostics on the primary surface (beta-justified, hide post-M4); steps 4–5 (auto analysis + auto selection) seeded via top-6 recommender but not automatic (the genuine M4 work) | MEDIUM (product) | windows-product | small UI batch + M4 in the gate plan |
| F11 | **tauri-shell feature is never clippy-linted** — app.rs (2,301 lines, the UI orchestration monolith) compiles for tests only; same CI-hole class as the fixed tauri-shell test step and the unexecuted GPU-variant test | HIGH (test gap) | windows-product + test-gaps | add a clippy pass with `--features modelswarm-desktop/tauri-shell` |
| F12 | **Uninstaller vs data-dir split unverified**: `%LOCALAPPDATA%\ModelSwarm\Data` sits inside the install-root family; NSIS notes stale; uninstall may delete identity/models — needs a clean-VM test | MEDIUM (blocked on verification) | windows-product | schedule VM drill (M13) |
| F13 | **Resource-policy ceiling deviation**: `fit_and_score` gates at 90% RAM/VRAM; master prompt §2 mandates 70% ceiling — the M2 governor ADR must reconcile (recommendation engine vs runtime ceiling are different knobs) | DESIGN INPUT to gate | windows-product | governor ADR (M2) |

## 5. Security floor verdict (§12)

**3 PASS / 4 PARTIAL / 1 FAIL** (8 clauses): PASS content-blind hub,
loopback-by-default local API, secrets out of source, authenticated
capability checks (ADR-026 gate verified in the production serving path).
PARTIAL: log redaction (convention-based), signed updates (catalog/engines
signed; installers unsigned), size/time/rate limits (transport solid;
three gaps), plus one structural clause. FAIL: parser fuzzing (none).
Owner calibration applied: findings ranked by measured exploitability ×
reachability with time-boxed fixes; no new cryptography proposed where a
structural fix is cheaper. Detail: `audit-security-findings.md`.

## 6. Protocol/schema/ADR state

Frozen core **byte-consistent end-to-end** (lease wire §5, envelope
signing §2.3, manifest identity ADR-011/022, catalog v2, ADR-026 gate;
wire-compat CI with a real tracker+Postgres and byte-pinned golden
vector). ADR coverage: 001–010, 015, 020, 022–026 implemented clean;
011 missing chunk challenges; 012 two partials (suspension unwired;
self-reported capacity); 013 correct-but `session-authorize.mode`
unvalidated; 014 delivered early; 018 ahead of its text. msp-v1.md has
**no changelog**. Preset layer: no SessionOffer surface today; the
additive-and-freezable path (msp-cooperative-v1.md namespace + receipts
v2 + registry-tightened session-authorize) is enumerated with 7 ADR
touchpoints. Golden vectors: 3-sided manifest + lease vectors strong;
no vectors for receipts, handshake payload, catalog envelope, SpecMessage,
relay signaling. Detail: `handoff-protocol-architect-2026-10-09.md`.

## 7. Agent/ownership state

The nine `msp-*` definitions: 2 keep, 7 revise — the 2026-10-08 revision
did not fully hold (scheduler definition stalest). Ownership conflicts:
speculation double-claimed; identity def narrower than map; **relay
crate owned by no one**; many shared paths unassigned. Three new agents
recommended: msp-resource-engineer (M1/M2 — genuinely unowned
subsystems), msp-architecture-reviewer (the §9 gate's approval seat),
msp-gateway-engineer (map's own "held pending" item). Detail:
`audit-agent-inventory.md`, `audit-agent-gap-analysis.md`,
`audit-agent-ownership-map.md`.

## 8. Domain deep-dives (wave-2 — fold in as handoffs land)

- **Runtime / model integration** — **IN**
  (`handoff-runtime-engineer-2026-10-09.md`). Floor verdict: the
  exact-token correctness chain is genuinely solid (pinned b11407
  hash-verified end to end; exact id+bytes recovery fail-closed; ADR-022
  identity hashes incl. both null amendments; catalog Ed25519; atomic
  artifact activation; no mocks in production). The performance layer
  above it does not: fake streaming measured and quantified (= F1);
  M5 gaps 3 absent (resume, disk-space precheck, cleanup/deprecation/
  rollback) + 2 partial; M6 gaps: streaming (F1), cancellation
  deadline-only, memory accounting absent, context/prefill metrics
  partial; 8 further M6 items present. Value-equality greedy mapping is a
  protocol decision to record; `prefill_ms` hardcodes 0.0.
- **Network / transport / relay** — **IN**
  (`handoff-network-engineer-2026-10-09.md`). Pool-reuse correctness:
  clean by construction (any error drops the session; only clean
  `Completed` re-pools; `Usage` provably precedes `Completed`). F2b
  verdict: **migrate the serving path to a libp2p Swarm (Option B)**, NOT
  hand-rolled multistream-select + copied protobuf (libp2p-relay
  internals are `pub(crate)`; permanent drift risk; still no DCUtR) —
  4–6 days + half-day ADR folding the §6 drift decisions; ADR-018 gate
  re-runs included; dependency surface grows only via features on the
  existing libp2p 0.57 dep. M8: direct/lease/reuse LAN-proven; drain,
  listener-death signaling, relay fallback, DCUtR missing. M9: WAN
  entirely unproven; hole-punch 60–70% is literature, not our
  measurement. HIGHs = F15/F16.
- **Scheduler / cooperative science** — **IN**
  (`handoff-scheduler-scientist-2026-10-09.md`). Unwired verdict
  CONFIRMED (= F3); cost model v2 lacks jitter/loss/bandwidth, measured
  queue depth, acceptance history, marginal τ, deadline, and any planner
  function (= F15); honest-results culture PASSES (negative-result
  records, TEST-ONLY labels, closed schemas; no average-baseline claims).
  **M10 build order delivered** (11 ADR-gated steps, S/M/L effort): R0
  governance ADRs incl. presets + receipts-v2 → measurement plumbing →
  shadow planner → 9.6 pass 1 on existing LAN → wire scheduler (small) →
  engine adapter spike → receipts v2 + granted accounting → fair
  queueing (bounded wait + DRR) → reputation chassis on ADR-012
  suspension (requires F8 fix first) → HEDGED honesty patch + cohort
  transport → presets light-up per tier gates. 9.6 readiness: NOT READY
  (see F17).
- **Tracker control plane** — **IN** (`handoff-tracker-engineer-2026-10-09.md`).
  Content-blindness holds across routes (artifact-pointer-only schemas; no
  prompt/completion fields; unlogged bodies). Earn chain weakness confirmed
  (T1, = F2). Suspension machinery ~1/3 implemented, slots-to-zero blocked
  by DDL (T2, = F8) — migration-0004 ADR required. M3: approval gating, id
  derivation, immutability HOLD; 5 actionable rows (license metadata, runtime
  const enforcement location, unvalidated candidate files, per-request
  signing). API hygiene + unbounded-table cluster (T4/T5, = F14). Wire-compat
  CI genuinely covers the core lease chain against real tracker+Postgres;
  seven surfaces remain cross-language uncovered. Recommendation on the
  challenge: relabel honestly now, then nonce-bound prompt + greedy-canary
  possession digest (content-blind, rides E0).
- **Windows product / installer** — **IN** (`handoff-windows-product-engineer-2026-10-09.md`).
  §1 verdict: state/Start-Stop/chat/progress/honest errors/11435 all ship;
  copy button MISSING; Peers/Setup/diagnostics beta-justified on the primary
  surface; M4 = making the seeded top-6 recommender automatic. M2 seam: only
  kill-on-close jobs + total-RAM detection exist; engine runs all cores,
  normal priority, fixed `-c 4096`; admission caps + wire clamps are the only
  bounded-work hooks; governor crate shape (EngineBudget, pressure ladder,
  priority classes, CPU-rate/memory job limits) designed in the handoff,
  needs an ADR. UX-study leftovers still open: 15 s innerHTML re-render,
  a11y residue, no scripted geometry gate, chat_log bleed; landed: one-action
  swap, single lifecycle surface, chat gating, keyboard-operable switch.

## 9. Salvage matrix status

Code-inventory delivered 35 component rows (KEEP 17 / KEEP WITH TESTS 15
/ REFACTOR 3 / others 0; best-tested component: apps/tracker). The
consolidated `audit-salvage-matrix.md` (merging wave-2 depth) lands with
§8. Preliminary signal: **no component is REPLACE/DELETE/QUARANTINE** —
the audit supports incremental correction, not a rewrite.

## 10. Audit document index (this batch)

`audit-freeze-2026-10-09.md` (freeze + validation + CI story) ·
`audit-code-inventory.md` + handoff · `audit-dependency-map.md` + handoff ·
`audit-test-gap-analysis.md` + handoff · `audit-security-findings.md` +
handoff · `audit-agent-{inventory,gap-analysis,ownership-map}.md` +
handoff · `handoff-protocol-architect-2026-10-09.md` · the five wave-2
handoffs (runtime, network, scheduler, tracker, windows product) · this
document. Remaining: `audit-salvage-matrix.md`,
`audit-production-risk-register.md`, `audit-rewrite-boundaries.md` →
the §9 architecture decision gate.
