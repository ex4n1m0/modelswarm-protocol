# Expanded-Mission Roadmap Review — ModelSwarm Protocol

**Date:** 2026-10-08 · **Requested by:** owner · **Produced by:** the nine-agent
roster (Protocol Architect, Runtime Engineer, Network Engineer, Scheduler
Scientist, Security Engineer, Tracker Engineer, Client/Product Engineer,
Test and Release Engineer, Prior-Art Auditor) with independent read-only
reviews reconciled by the Integrator.

**Mission under review (owner's words, condensed):** use every beneficial
form of inference-time scaling available from a potentially global pool of
one million endpoints hosting the same exact, cryptographically identified
model profile; never assume all endpoints participate — dynamically pick
the smallest useful set per request; preserve the frozen foundations;
compare every cooperative mode against the fastest eligible single peer and
fall back on loss; no performance claim without real-model evidence.

**Status of this document:** a PROPOSAL. Nothing in it changes the
governing plan, protocol version, or any schema. §15 asks for the owner's
decision. Per the prompt's implementation order, the existing validation
suite was run first (all green, below), each subagent reviewed
independently, disagreements are reconciled explicitly, and the smallest
invalidating experiment is identified (§9.6).

---

## 1. Current-state summary (repository evidence)

**Proven, production, as of today (2026-10-08, commits `1702cfb..c4399a8`,
tracker deployed):**
- Cross-machine P2P serving over QUIC on LAN (F0/F1; two machines,
  `docs/verification/phase-f-lan-2026-10-07.md`).
- ADR-026 hub-signed lease gate at serving-session open + the honest
  earn chain (challenge→lease→gate) working end-to-end against
  production, byte-pinned by a cross-language golden vector
  (`protocol/vectors/lease-hubkey-1.json`) and exercised in CI by a
  Rust↔TS wire-compat job against a real local tracker.
- Wire clamps, admission control (session/per-peer caps + request-id
  dedup), privacy-tight logging (codes/variant names only).
- Greedy determinism (E0): Vulkan byte-identical across GPU vendors;
  CPU identical across thread counts; CPU≠Vulkan diverges.
- CI: fmt + clippy + tests in two feature configurations, desktop
  tests, version/binary guards, tracker Postgres suite, wire-compat.
- Gates at snapshot: **301 + 313 + 7 Rust tests, 110 tracker tests,
  5/5 golden vectors — all green.**

**Exists but unproven on real networks:** latency-aware selection
(scheduler crate is bench-only, unwired), speculative decoding
correctness (proven on mocks/loopback only), multi-proposer trees,
suspension/adversarial machinery beyond the lease gate.

**Does not exist:** relay/NAT (F2 untouched; owner decision: relay on
their second desktop), any multi-peer-per-request transport, WAN
anything, and — decisive for this review — **7 of the 12 expanded modes
have no ADR vocabulary at all** (ADR-013 registers only `single`,
`hedged`, `speculative_exact`, `search_verified`, `map_reduce`).

## 2. Gap analysis vs the expanded mission

| Dimension | Today | Gap |
|---|---|---|
| Modes | 5 registered (1 shipped) | 7 modes ungoverned; approximate trio needs its deferred ADR; HYBRID needs composition rules |
| Cohort | 1 peer per request | No N-peer dialer, no per-request deadlines (a straggler timeout kills the whole QUIC session today), no cohort cancel, no partial-result merge |
| Selection | First-eligible-peer | `select_microswarm` exists but receives **no real measurements** (RTT probe is TCP-only; QUIC backend lacks `measure_rtt` and JSON passthrough); cohort size is a constant, never a decision variable; no marginal-utility function exists |
| Runtime | HTTP adapter, greedy | No logits/batch-verify/tree-verify/KV-rollback over llama.cpp HTTP — speculative speedup over this adapter is **structurally zero** (per-position argmax = k sequential forwards); sampled lossless acceptance needs RNG alignment; lease lacks `cpu-kernel-arch` for cross-machine CPU determinism (E0 open sub-question) |
| Trust | Lease gate + clamps + dedup | Receipts prove a session happened, not honest work; HEDGED is a fast-liar-wins mode as speced; Sybil diversity, selector rules, bit-commitments absent |
| Scale | ~2 peers | Tracker heartbeats ~33-67k/s at 1M peers through per-isolate `pg.Pool(max:5)`; no GIN index on profiles (seq scan); per-lease notice broadcast O(N); lookup ≤50 alphabetical (a fairness bias) |
| Product | swarm/local labels | No mode dial, no consent tiers, no structured per-request receipts |

## 3. Architecture decision matrix — all 12 modes

Verdicts reconcile Runtime/Network/Tracker/Security/Prior-Art. "Engine
work" = needs a runtime adapter beyond the pinned HTTP llama.cpp
(cheapest path: in-process binding to the pinned `llama.dll` C API — no
fork; ADR-021 Option A vLLM is the research alternative).

| Mode | Contract | Transport | Runtime | Tracker/governance | Security precondition | Prior-art caveat |
|---|---|---|---|---|---|---|
| FASTEST_SINGLE | lossless (it IS the baseline) | **works today (LAN)** | works | none | none | payoff unproven — first numbers may be negative |
| HEDGED | same output; retry only pre-token (rule 6) | cohort messaging + per-request deadlines | works | mode in registry | **P0 honesty patch**: spot-verify or background agreement check — else a fast liar wins by construction; Sybil-diversity constraint + requester-billed fan-out | Dean 2013 pattern; ~5% extra load; relay-reset vs ADR-007 conflict |
| SPECULATIVE | lossless (greedy today; sampled needs RNG work) | tight-loop cohort (LAN-class only) | **engine work** (logits/verify) | cooperative doc freeze | bit-commitments vs adapt-after-see | **arXiv:2606.25091: co-located SD dominates distributed SD for single-request latency** — our requester hosts the profile, so the comparator is exactly the losing case at WAN; gate to LAN/<10ms and expect published negatives |
| TOKEN_TREE | lossless tree commit | cohort + straggler policy | **engine work** (tree verify impossible over llama.cpp HTTP) | registry + cohort cap raise (roster schema caps 16 today) | proposer equivocation defenses | SpecInfer/Medusa/EAGLE = commodity; multi-proposer gains **sublinear** (Mamou et al.); "machines as proposers" is the novel-but-unproven bit |
| BEST_OF_N | **approximate — never claim lossless** | scatter-gather | works (n choices) | own ADR (ADR-013 defers it) | scorer must never be a candidate; matched-compute baseline mandatory | compute-optimal allocation is >4× more efficient than naive BoN (Snell); task/model-dependent |
| CONSENSUS | approximate agreement report | scatter-gather | works | own ADR | Sybil rings; correlated errors | **identical profiles = correlated errors** — majority vote is not correctness; diversity unmeasured |
| CRITIQUE_AND_REVISE | approximate; bounded rounds | sequential sessions | works (ChatML already) | own ADR | reward-model gaming | intrinsic self-correction often **degrades** results (Huang et al.); require external verifier or drop |
| MAP_REDUCE | approximate; explicit fallback on shard failure | relay + chunk ADR (256 KiB frames today) | works | partition framing | partial-digest chains (fabrication) | cross-chunk dependency failures documented (2506.16411) |
| BATCHED_MULTI_USER | throughput; per-user attribution | long-lived session pool (per-peer cap today 2) | works (llama-server batches); per-request attribution missing | admission caps become scheduler inputs | cross-user KV isolation; batch timing side channel = new asset class | comparator is goodput, not TTFT |
| PREFIX_AWARE | byte-exact same output | transport-neutral | **partial** (implicit slot reuse only; no pin/observe API — `/slots` is the cheap probe) | digest-only signaling (content-blind) | **prefix oracle is a privacy breach class** — cache-hit timing must never be hub-visible; per-requester namespaces | without KV transfer (banned v0.1) cross-peer sharing degenerates to weak prompt-token dedup |
| AUDITED | detection, not prevention | scatter-gather | works within a backend family (E0) | derive right from `can_host`; bind to `audit_epoch` | indistinguishability of audit tasks; audits injected by REQUESTERS from organic distribution (hub stays blind) | Sarmenta/BOINC spot-checking math; detection curves are first-class results |
| HYBRID | composes only individually-gated modes | follows components | follows components | composition rules last | compounding of all above | mode selection without data = a hidden learned scheduler (banned); deterministic heuristics first |

## 4. Proposed protocol and schema changes (all ADR-gated, none started)

1. **Mode-registry ADR** (extends ADR-013 in place — it is a registry,
   not a wire schema): 12 modes with per-mode correctness contract,
   default-off, cohort bounds, fallback-to-FASTEST_SINGLE mandatory.
2. **msp-cooperative-v1.md** (still does not exist): multi-peer messages
   under the existing `/msp/cooperative/1.0.0` namespace in
   modelswarm-session; `SessionOffer{mode, cohort}`; capabilities
   negotiated from manifest `SpecCapability` + `decoding_abi_version`.
   The frozen msp-v1/messages.proto stay byte-identical. Three
   primitives compose every mode: a v1 single stream, the signed prefix
   commit chain, the two-phase receipt.
3. **Receipts v2 ADR**: add content-blind scalars (modeId, cohort
   digest, per-peer roles, accepted/verified token counts, endpoint-
   seconds, MAP_REDUCE partial-digest chain, proposer bit-commitments).
   Machine-derived outcomes only.
4. **Lease additive fields** (if unavoidable: `cpu_kernel_arch`,
   mode-capability flags) → lease-v2 ADR + new cross-language golden
   vector; old leases keep verifying.
5. **Tracker**: GIN index on `peer_leases.profiles`, randomized bounded
   lookup (alphabetical is a fairness bias), set-based
   session-authorize, region column; observations rollup + notice
   epochs only if 1M is committed.
6. **Cohort transport ADR**: N-peer dialer/pool, per-request deadline
   policy (straggler ≠ session death), cohort cancel fan-out, QUIC
   backend parity (`measure_rtt`, JSON passthrough, TCP+Yamux fallback),
   relay reset-vs-retry semantics reconciled with ADR-007.

## 5. Scheduler design (deterministic; no learned components)

Per-request planner in `modelswarm-scheduler` (extending frozen cost
model v2), fed by measurements that must be plumbed first:

```
plan_request(profile, prompt, budget, demand) -> Plan{mode, cohort, margin}:
  C = eligible_candidates(profile)          # existing filters
  f = min over C of predicted_single_ms     # ALWAYS computed (the baseline)
  for mode in candidate_modes(budget):      # exact modes by default
    for k in 1..=cap(mode):
      p = predicted_mode_ms(mode, k, C, prompt, telemetry)
      if p*(1+margin(mode)) < best.p*(1+margin): best = (mode, k, p)
      stop when marginal_benefit(k-1→k) <= τ(k)   # τ measured, not invented
  if best is single or !engages(best.p, f, margin): FASTEST_SINGLE
```

- Marginal-benefit test: hedged/trees use min-of-k order statistics on
  measured per-peer completion distributions (EWMA p50/p95);
  speculative replaces the synthetic 0.8 acceptance constant with
  measured acceptance EWMA; duplicate-work term charged to the
  requester.
- Observed-loss fallback (executor-side, honoring rule 6): 2
  consecutive rounds with `measured > predicted×(1+margin)` or
  acceptance below floor → abort to FASTEST_SINGLE pre-first-token.
- Demand regimes at the gateway: u<0.5 spend slack on
  speculation/search; 0.5–0.8 shrink cohorts (higher τ); >0.8 one host
  per request + batching, no hedging; >1.0 fair queue
  (deficit-round-robin) + per-installation token bucket + explicit
  rejection. No user monopolizes (BEST_OF_N N capped by quota class).
- **Measurement plumbing precedes everything** (RTT EWMA on QUIC,
  NatPath producer, per-peer completion distributions, measured queue
  depth — advertised values are untrusted inputs, not measurements).
  First wiring is **shadow mode**: the planner logs the plan it would
  choose and never acts.

## 6. Multi-user capacity management

Peers report `queueMs`/`freeSlots` (already in the lease); the tracker
supplies load metadata but **never becomes a scheduler** (content-
blindness discipline; per-request hot path off serverless). Fair-queuing
state is peer-side admission control (today's `Admission` struct is the
seed — caps become scheduler inputs). Quota classes ride the lease;
accounting extends receipts v2. Utilization is computed at the
requesting gateway from in-flight vs slots×capacity. Cross-user
isolation rules: per-requester KV namespaces (PREFIX_AWARE), batch
timing channel documented as a non-guarantee, zero cross-session output
leakage asserted by test.

## 7. Security and privacy analysis (new surfaces)

Highest-risk findings (full detail in the Security section): **HEDGED
is a wrong-answer amplifier as speced** (speed-only selection) — P0
honesty patch before it may ship; **Sybil out-voting** in
BEST_OF_N/CONSENSUS needs cohort diversity constraints (≤k peers per
{installation, subnet/ASN}) + fan-out charged to the requester's lease;
**PREFIX_AWARE is a privacy-breach class** (cache-hit timing = a prefix
oracle; `prefillMs` is already a wire field); **AUDITED needs
indistinguishability** (adversaries must not identify verification
tasks; audits injected by requesters, hub stays blind); mode+timing
metadata must be coarsened to quantile buckets before anything is
hub-visible (content oracle). Reputation becomes necessary for
approximate modes (no verifier to lean on) — hub-aggregated, per-
profile, epoch-bucketed counters (ADR-012 chassis), explicitly NOT a
ledger, with vendetta/whitewashing/self-dealing failure-mode tests
before any enforcement. Threat-model v2 adds one adversary row per mode
plus two new adversary classes (colluding ring; selector/scorer
capture).

## 8. Compatibility and migration

Frozen surfaces stay byte-identical: msp-v1, messages.proto field
numbers, lease signed fields (golden vector pins bytes). Every
addition rides a new ADR; every new signed artifact ships with its own
cross-language golden vector + a wire-compat harness assertion in the
same PR (the standing rule after four shipped wire bugs). Old peers
interop unchanged: only `single` is default-on; mode negotiation is
opt-in via SessionOffer; lease-v2 (if ever) verifies alongside v1.
The wire-compat CI job gains a rollback leg (previous-tag client vs
current tracker) so mode fields stay additive.

## 9. Updated phased roadmap (proposed — not started)

- **R0 Governance + measurement (no user-visible change):** mode-
  registry ADR; threat-model v2; QUIC `measure_rtt`/passthrough parity;
  RTT/NatPath/completion-distribution telemetry into the node; planner
  in shadow mode; tracker GIN index + randomized bounded lookup;
  receipt-v2 ADR. Exit: shadow planner runs on real traffic in logs.
- **R1 F2 relay + WAN honesty (owner-blocked: relay host):** relay
  binary with raised limits; direct-vs-relay split published; reset-vs-
  ADR-007 semantics; first WAN tier evidence. Exit: WAN fastest-single
  numbers vs LAN baseline, labeled.
- **R2 The invalidating experiment (see 9.6) + HEDGED behind the
  honesty patch:** cohort transport layer (N-dial, per-request
  deadlines, cohort cancel); HEDGED ships only with spot-verify +
  diversity constraint; receipts v2 live.
- **R3 Lossless modes, evidence-gated:** engine adapter ADR (in-process
  C API or vLLM research pin) unlocking logits/batch-verify — SPECULATIVE
  at LAN-class RTT first (expect WAN negatives per prior art; publish
  them), TOKEN_TREE only if marginal benefit per endpoint > 0 on real
  runs. Cooperative-plan P0 gate closes here.
- **R4 Approximate trio + AUDITED (own ADRs, labeled approximate,
  matched-compute baselines, never lossless claims):** BEST_OF_N,
  CONSENSUS (with the correlated-errors caveat in the contract),
  CRITIQUE_AND_REVISE (external verifier required or dropped), AUDITED
  with published detection-rate curves.
- **R5 BATCHED_MULTI_USER + PREFIX_AWARE + HYBRID last** (composition
  of individually-gated modes only; consent tiers ship with each).
- Every phase ends with `docs/verification/<phase>.md` in the existing
  evidence pattern; every mode ships default-off until its tier gate
  passes on real-model evidence.

**9.6 The smallest invalidating experiment** (Scheduler Scientist,
runnable on the existing 2-machine LAN testbed, before ANY planner
code): sweep draft window {2,4,8,16} × proposer count {1,2,4} ×
injected RTT {0,5,10,20 ms} with real weights; compare (a) fixed k=2,
(b) planner-chosen k from measured acceptance EWMA + marginal test,
(c) FASTEST_SINGLE. Metrics: vs-fastest-single completion ratio +
endpoint-seconds per accepted token, ≥30 reps, bootstrap CI (stats.rs
already provides both). **Invalidation criterion:** if planner-chosen k
does not beat the best fixed k by more than its CI width in ≥2/3 of
cells, adaptive cohort sizing is dead on real hardware below ~8 peers
and the roadmap must say so. First real entries land in
`experiments/raw|processed|manifests/` (currently empty).

## 10. Tests and acceptance gates

Tiered ship-at-all evidence (full table in the Test/Release section;
summary): T0 single-path (FASTEST_SINGLE real-model LAN + frozen
baseline harness; HEDGED zero-duplicate + TTFT p95 improvement else
auto-disable); T1 lossless (distribution-equivalence properties
unconditionally incl. fallback; loopback accepted-tokens/round; LAN
acceptance for the pinned profile; TOKEN_TREE marginal-benefit-per-
endpoint for k=2,4,7); T2 quality (frozen graded corpus, matched-
compute baselines, per-task breakdowns, MAP_REDUCE merge properties,
BATCHED fairness via Jain index, PREFIX byte-exactness + collision
adversarial test); T3 trust (AUDITED detection ≥0.9 at seeded wrongness
rates with false-accusation <1% and overhead published; HYBRID gate =
union of component gates + exhaustive fallback chains). Testbed ladder:
sim (scheduling/fairness/adversarial ONLY — never quoted as perf) →
2-machine LAN (wire authenticity, 2-peer modes) → k=4 harness (2 owner
machines + 2 cheap VMs — owner approval needed for spend) → first WAN
rung → production. Hostile-peer gauntlet pre-merge for every mode
(ramped-delay slow, draining-stale, seeded-wrongness dishonest,
replay/flood malicious). Soak ≥4h mixed workload before any mode is
default-enabled.

## 11. Explicit non-goals and rejected approaches

No KV-cache transfer in v0.x (constraint 8) — PREFIX_AWARE is request
shaping only. No approximate mode default-on, ever, and no approximate
mode labeled lossless. No learned scheduler before deterministic
heuristics have production evidence. No tracker-side ranking or queue
state (directory, not matchmaker). No relay through Vercel. No
universal NAT-traversal claims (constraint 7). No blockchain/payments/
ledger reputation. No mixed-ModelProfileId cohorts (project rule). No
remote attestation / confidential computing. No mixed-model critique
(same exact profile only). No 1M-endpoint extrapolation from sim.
Rejected: llama.cpp fork (maintenance trap — in-process C API binding
instead); editing frozen proto field numbers; adding lease signed
fields without re-pinning vectors; window-scaling proposer counts
beyond measured marginal benefit.

## 12. Open research questions

(1) Does same-profile remote speculation beat the fastest single host
at <10 ms RTT — and is it structurally negative at WAN (2606.25091
says yes for the general case)? (2) Do cross-machine proposer trees
beat two-peer linear speculation after dedup/prune overhead? (3) Does
identical-profile consensus retain useful diversity (correlated-error
measurement)? (4) Hidden-audit detection math vs adaptive evasion.
(5) Shared-batch timing channels in llama.cpp. (6) Collusion-resistant
reputation without a ledger. (7) Cross-machine CPU kernel determinism
(E0's open sub-question — blocks token-exact cooperation on CPU
between different microarches). (8) RNG alignment for sampled lossless
acceptance. (9) Is marginal benefit ever positive beyond k≈3 for
speculative_exact? (10) Patent search on speculative-decoding
acceptance rules before shipping SPECULATIVE/TOKEN_TREE (industrial
filers exist in that lineage).

## 13. File-by-file implementation impact map (R0–R2 only; R3+ rides ADRs)

- `protocol/`: NEW msp-cooperative-v1.md; msp-v1 §5/§7 additive edits;
  NEW vectors per new signed artifact; keys/ unchanged.
- `crates/modelswarm-transport/src/`: libp2p_backend.rs (measure_rtt,
  send_json/recv_json, idle tuning); NEW cohort.rs (N-peer pool,
  per-request deadline policy, cohort cancel); message.rs additive
  Control extensions.
- `crates/modelswarm-scheduler/src/`: NEW planner.rs (mode+k decision,
  marginal-utility, demand regimes); candidate fields (completion
  distribution, NatPath producer input).
- `crates/modelswarm-node/src/`: remote.rs (cohort dialer + shadow
  planner hook), serving.rs (admission caps → config), lib.rs
  (telemetry plumbing).
- `apps/tracker/`: migrations (GIN index, region column — additive),
  peers route (randomized bounded lookup), session-authorize
  (set-based), wire-compat harness extension.
- `crates/modelswarm-desktop/`: send_chat structured receipts; mode
  dial scaffold (Fast live; others honestly-unavailable with reasons);
  consent tiers; receipt export.
- `experiments/`: first real manifests/raw/processed entries (9.6).
- `docs/`: mode-registry ADR, receipts-v2 ADR, threat-model v2,
  successor plan (upon approval), verification docs per phase.

## 14. Risks ranked (severity × probability × detectability)

1. **SPECULATIVE at WAN is structurally negative** (H×M×L) — mitigate:
   R2 experiment gate; publish negatives; LAN-scoped contract.
2. **HEDGED wrong-answer amplification** (H×M×H) — mitigate: P0 honesty
   patch ships with the mode, not after.
3. **Sybil out-voting in approximate modes** (H×M×M) — diversity
   constraints + reputation chassis before enforcement.
4. **Engine adapter slip** (in-process C API effort) blocks the entire
   lossless track (M×H×H) — de-risk early with a spike in R0/R3.
5. **Tracker seq-scans / notice broadcast at scale** (M×M×H) — GIN +
   epochs before any cohort growth.
6. **Consent/privacy UX debt ships modes users didn't agree to**
   (H×M×H) — consent tiers are release-blocking per docs/privacy.md.
7. **12-mode state-machine explosion** (M×H×M) — three primitives only;
   property tests per mode; HYBRID last.
8. **Single-owner LAN dependency for every T0/T1 gate** (M×H×M) — k=4
   harness decision needed (owner spend approval).

## 15. Recommendation: amend or successor?

The roster split 4–4 (Protocol/Security/Test-Release/Product: successor;
Scheduler/Network/Runtime/Tracker: amend). Reconciled recommendation:

**Create a successor plan — narrowly.** A new governing document (e.g.
`docs/inference-scaling-plan.md`) that supersedes
`docs/competitive-revision-plan.md` exactly the way ADR-015 superseded
before (recorded supersession map; phases A–G/H/I/F remain historical
implementation evidence; the cooperative plan P0–P8 remains the gated
lossless kernel, cited not duplicated), while the actual changes happen
as **targeted amendments to the existing ADR-governed registries**
(ADR-013 mode registry extended, receipts-v2, transport staging, threat
model v2) — which is what the "amend" half of the roster was protecting.
No rewrite of frozen protocol surfaces; no mid-flight phase-gate edits.

**Approval requested (owner decision):**
1. Approve or reject the successor-plan approach above.
2. Approve R0 (governance + measurement, no user-visible change) as the
  next work item — it unblocks everything and touches no frozen surface.
3. Decide the two standing blockers when ready: relay host (already
  decided: owner's second desktop) and the k=4 harness VM spend
  (~2 cheap VMs for T0/T1 gates).
4. The 9.6 experiment can start immediately after R0 on the existing
   2-machine LAN testbed — no spend, no protocol change.

Until approval, none of §4–§9 begins. The next implementation task in
the CURRENT plan is unchanged: gated LAN re-probe on the new builds,
then the F2 relay binary.
