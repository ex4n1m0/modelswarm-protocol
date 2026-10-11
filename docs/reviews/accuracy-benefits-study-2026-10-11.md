# Accuracy-Benefits Study — "many is better than one" beyond latency (2026-10-11)

Scheduler Scientist, per the owner directive recorded at ADR-032
acceptance: the main goal is swarm benefits regardless of engineering
path; if one way fails, find others; the exercises may surface other
creative benefits; **some or any form of accuracy is welcome even when
slower**. This study turns that directive into a ranked, evidence-grounded
research plan and delivers one MEASURED proof-of-concept family
(cross-peer greedy divergence detection) on loopback, on the real
two-machine LAN, and against the real pinned engines — including one
real fault the instrument caught live.

Scope guardrails honored throughout: the lossless `speculative_exact`
contract is not weakened; output-changing modes are NAMED as ADR-gated
future work and nothing of the kind is implemented; the hub stays
content-blind (everything below rides P2P serving requests or local
re-execution); no new wire fields (nothing protocol-side was touched —
anything that would need one is routed to the Architect as a proposal,
not built).

---

## 1. Method

A candidate is a benefit only if it is (a) stated as a measurable claim
with a ground truth or an injectable fault model — never a vibes metric,
(b) honest about negatives (a detection instrument that false-alarms is
a negative result and must be publishable as such), and (c) classified:

- **(a) output-preserving (lossless)** — detection, robustness,
  availability, roster accuracy; compatible with `speculative_exact`
  today (the served output is unchanged; the swarm adds knowledge,
  not tokens);
- **(b) output-changing** — best-of-n, self-consistency, ensembling;
  requires its own ADR + preset disclosure (ADR-029 namespace) and is
  only LISTED here.

Ranking score = benefit value (1–5) x feasibility on the current stack
(1–5) / contract risk (1–5, higher = riskier). Scores are argued, not
secretly computed from wishful numbers; the arithmetic just orders the
argument.

## 2. Candidate enumeration and classification

### A. Output-preserving (lossless)

| # | Candidate | What the swarm gains | Classification |
|---|---|---|---|
| A1 | **Cross-peer greedy divergence DETECTION** — two same-backend peers (or requester + peer) decode the same prompt under pinned greedy params; any token-level divergence flags nondeterminism / a hostile peer / a hardware fault | Diagnosable accuracy: the swarm becomes a fault detector a single host cannot be (nobody re-checks themselves) | (a), MEASURED (§3) |
| A2 | **Redundant decode racing (Hedged)** — 2 peers decode, first completion wins | Pure availability/tail latency; output-preserving ONLY modulo the nondeterminism A1 measures (first-to-finish picks the winner's attractor) | (a) conditionally; the mode is already registered (ADR-013 `Hedged`); envelope MEASURED structurally (§3.4) |
| A3 | **Verifier audit generalization** — ADR-032 §3 audit path promoted to a VERIFIED-preset promise: X% of committed windows re-executed, divergence rate reported, fallback on any divergence | Correctness-of-authority: catches the false-but-self-consistent verifier (attack surface 4) that signatures and hash chains cannot | (a), specified in ADR-032, blocked on the VerifyDrafts family + ADR-031 gate |
| A4 | **Acceptance-EWMA peer quality ranking** — measured per-(peer, profile) acceptance replaces the 0.8 constant | Accuracy OF THE ROSTER: better cohort decisions from evidence, not adverts | (a), already wired (`AcceptanceStore`) and measured (pass-1/pass-2 curves) |
| A5 | **Cross-machine determinism certification** (creative, evidence-derived from this study) — periodic canary sweeps across (profile, backend family) pairs; the swarm measures the very fact that gates real-draft pass 3 and ADR-032 B CPU auditing (the E0 open sub-question) | The swarm as measurement instrument: collective, continuously updated determinism certificates no single lab run provides | (a), first data point MEASURED (§3.3) |
| A6 | **KV/prefill offload variants** | Would let a slow-prefill peer borrow another's prefill work | (a) in principle, but **blocked**: AGENTS.md hard constraint 8 forbids KV-cache transfer in v0.1, and re-prefill is the measured pass-2 loss (per-round prefix re-post, realized/predicted 2.8–8.3x). Honest negative: no v0.1 benefit exists on this path without a wire the project has explicitly deferred |
| A7 | **Queue/reliability honesty checks** — advertised `queueMs`/slots vs measured RTT/throughput | Roster accuracy; the scheduler's measured-dominates-advertised rule already implements the scoring half | (a), partially wired (shadow `EnvPolicy`, queue discount); the active probe half is a small extension of A5's sweep machinery |

### B. Output-changing (ADR-gated future work — listed, NOT implemented)

| # | Candidate | Why it needs its own ADR + preset disclosure |
|---|---|---|
| B1 | **Best-of-n with a verifier/judge** | Output is selected, not verified — a new preset (ADR-029) must disclose the selection rule and its cost (n x decode + judge) |
| B2 | **Self-consistency majority voting** | Requires sampled decoding (breaks the greedy-determinism identity the whole E0/ADR-032 edifice leans on); majority vote is an accuracy-vs-latency trade that must be disclosed per preset |
| B3 | **Cross-peer ensembling / logit blending** | Needs logits on the wire (new frozen surface, Protocol Architect's), float-canonicalization rules, and a blending policy that changes output distribution by construction |
| B4 | **Verification-guided reranking** (Phase E search adjacency) | Approximate by design (`SearchVerified` exists in the mode registry); any light-up needs the ADR-013 Phase E gate |

None of B1–B4 is implemented, simulated for product claims, or wired
anywhere by this study.

## 3. What was measured today (the proof-of-concept)

All artifacts under `experiments/raw/ACCURACY-*/2026-10-11` (loopback,
LAN, real-engine), aggregated by script into
`experiments/processed/ACCURACY-2026-10-11-summary.json`. Code landed at
commit `bde7eca` (`crates/modelswarm-bench` divergence module + drivers +
per-push CI pin). Synthetic-executor runs are STRUCTURAL evidence
(protocol machinery: inject -> detect -> report -> de-rank); real-engine
runs carry the engine claims and are labeled as such.

### 3.1 Structural detection, loopback (real QUIC, 4 bridges)

Clean control cell: **0/24 false positives**. Injected cell (pool index
2 carries the divergence fault model): **8/8 checks detect at divergence
position 0**, quorum (reference-free) agrees 8/8, exactly the offender
suspended (agreement EWMA 1.0 -> 0.0), and the selected cohort changes
(V,D,offender) -> (V,D,M2). Median check wall 235–237 ms.

### 3.2 Structural detection, two-machine LAN (machine B serve, A driver)

Identical outcome shape on the real LAN: clean 0/24 false positives;
injected **8/8 detected at position 0** (verifier-reference AND quorum),
offender-only suspension, cohort de-rank, all four dialed peer ids
equal to the pinned pool's derived identities. Median check wall
249–252 ms. Machine B left exactly as found (verified: 0 processes,
task/folder/firewall rule removed, `MSPPass1` Ready, v4 intact).

### 3.3 Real engines (pinned b11407, SmolLM2-135M Q4_K_M, E0 discipline)

- **Cross-machine, cache-cold (the E0 open sub-question): first data
  point — determinism HOLDS on this tuple.** Raptor Lake-HX (A) vs
  Coffee Lake i9-9900K (B), byte-identical engine binaries, fresh
  process per prompt, temperature 0: **8/8 A-vs-B prompt pairs
  byte-identical** (16 decodes total, one digest per prompt across all
  four instances).
- **A real fault caught live:** on prompt p1-long the server's
  request-state/prompt cache flips the greedy continuation between two
  attractors that split at character 3 — first decode of a fresh state
  vs every repeat. Reproduced 3/3 (matrix catch + 2x10 characterization
  + fresh-pair probe; minority digest ALWAYS at request 0, 9/1 split on
  both instances). The instrument flagged it as a within-process
  divergence; E0's single-shot design structurally could not.
- **Design rule extracted (measured, not hypothesized):** divergence
  checks and audit re-executions MUST run cache-cold, or the check
  itself can flip the answer.
- Vulkan control on this corpus: 3/4 byte-identical to CPU at
  temperature 0 (backend pairing remains the sound policy; agreement
  rate is corpus-dependent).

### 3.4 Racing envelope (from the same completions, no extra requests)

Median tail saved by racing two peers vs having bet on the slower one:
69–97 ms across the four structural cells (synthetic speeds —
availability SHAPE, not a product claim). Honest caveat from §3.3:
racing is output-preserving only modulo state-dependent
nondeterminism — first-to-finish silently picks the winner's attractor,
so A1-style certification is a precondition for calling Hedged lossless.

### 3.5 What was NOT measured (labeled honestly)

- No real-engine divergence rates across QUANTS or larger models (one
  model, one quant); no WAN; no k>=3 real-machine swarm; no hostile
  adversary actually attacking (the hostile-peer case is modeled by the
  injected fault, not a real attack).
- No cross-machine Vulkan leg (single GPU machine available).
- A6's negative is analytical (constraint + committed pass-2
  measurements), not a new run.

## 4. Ranking (benefit value x feasibility / contract risk)

| Rank | Candidate | Value | Feasibility | Risk | Score | Basis |
|---|---|---|---|---|---|---|
| 1 | A1 divergence detection | 5 | 5 | 1 | **25** | Measured end-to-end today on all three venues; zero contract surface; catches a real fault class single-host QA misses; prerequisite evidence for ADR-032 B auditing |
| 2 | A5 determinism certification | 5 | 4 | 1 | **20** | Direct continuation with the landed harness; resolves the gating E0 open sub-question pair-by-pair; risk stays 1 while it remains a local experiment table (any eligibility wire field needs an ADR) |
| 3 | A4 acceptance/quality roster | 3 | 5 | 1 | **15** | Already wired and measurably refuses losing engagements (replay pin: 1,979/1,982 refused pre-round); hardening is cheap and contract-free |
| 4 | A3 verifier audit | 4 | 2 | 2 | 8 | High correctness value but blocked on ADR-032 B implementation + ADR-031 productionization gate; design should absorb this study's cache-cold rule |
| 5 | A2 racing (Hedged) | 3 | 4 | 2 | 6 | Mode registered; availability value real but the output-preservation caveat (§3.4) and 2x cost make it a follower, not a leader |
| — | A6 KV/prefill offload | 2 | 1 | 4 | 0.5 | Blocked by hard constraint 8 + measured re-post loss; honest negative stands until a KV-transfer ADR exists |
| — | A7 queue honesty | 2 | 3 | 1 | 6 | Fold into A5's sweep as an extra probe column |
| — | B1–B4 output-changing | n/a | n/a | n/a | — | ADR-gated future work; out of scope here by directive |

## 5. Top-3 experiment proposals

### Proposal 1 — Divergence sentinels (A1 productionization)

**Design.** Each requester (which, per the project rule, hosts the
exact profile P itself) runs a periodic cache-cold canary: decode a
pinned canary prompt locally AND on one cohort-eligible peer, compare
token-for-token (the instrument landed at `bde7eca`:
`first_divergence`/`stream_digest`/`AgreementLedger`/`quorum`), record
into a divergence ledger beside the acceptance store, de-rank on first
divergence with a cache-cold RE-CHECK before suspension (the §3.3
rule: one re-check filters state-cache artifacts from real faults).
Canaries are pinned per profile in the experiment manifests — content
synthetic, hub never sees them.

**Comparators.** Injected-fault sensitivity (today: 8/8, position 0);
clean specificity (today: 0/24); real-engine state-flip catch (today:
caught); cost per sweep (today: ~250 ms per 4-peer check on the LAN
shape, off the serving path).

**Honest-negative criteria.** (1) Any false alarm on a certified
same-backend pair that survives the cache-cold re-check invalidates the
certification premise for that pair and MUST be published (it would
also be the first real cross-machine CPU divergence — a major result
either way). (2) If detection latency (check cadence) cannot be kept
off the serving path within idle budgets, the sentinel degrades to
on-demand-only and the proposal shrinks to A5's sweep form.

### Proposal 2 — Determinism certification matrix (A5)

**Design.** For every (profile, backend-family) pair actually present
in a swarm, run the §3.3 protocol on each machine pair: fresh process
per canary, cache-cold, N canaries (>= 64, including near-tie prompts
of the p1-long class — the class that actually flipped — plus
adversarial ties), repeat-stability probe per peer (same prompt twice
to catch the cache class locally). Output: an experiment-derived local
table `pair -> certified | uncertified | divergent(counts)` that gates
real-draft pass 3 and ADR-032 §3 CPU-family audits. No wire field, no
tracker surface — eligibility stays a local decision until an ADR says
otherwise.

**Comparators.** Within-machine cross-process control (expected agree);
cross-machine pair (the measurement); per-prompt attractor-count
distribution (a prompt with >1 attractor across instances is itself a
finding about the prompt family).

**Honest-negative criteria.** Any cross-machine divergence on the same
backend family = uncertified, published with digests and positions —
including the uncomfortable case where a popular quant diverges
cross-machine (that would overturn the §3.3 tuple's optimism and
re-scope ADR-032 auditing to per-machine verifiers; it must be
published, not buried). If certification cost (model loads) exceeds
idle budgets on minimum-spec hardware, record the cost curve and let
the owner pick the cadence.

### Proposal 3 — Roster-quality hardening (A4 + A7)

**Design.** Two deterministic changes inside my crates: (1) key the
engaged-loss EWMA by (profile, verify-term) — the recorded follow-up
from the 2026-10-11 handoff, so a future batch-verify cohort does not
inherit the sequential wire's loss history; (2) feed the divergence
ledger into candidate admission as a suspension filter (the de-rank
demonstrated today) and add the A7 probe column (advertised vs measured
queue) to the same sweep. No learned policy; deterministic heuristics
only, per the standing rule.

**Comparators.** Replay over the committed pass-2 LAN rows (the
existing pin harness pattern) for gate-keying behavior; counterfactual
cohort quality (before/after suspension) on replayed committed rows
plus fresh sentinel runs.

**Honest-negative criteria.** If suspension filtering measurably
degrades cohort quality on clean swarms (false alarms above ~1% of
checks after the cache-cold re-check), the policy is wrong and the
result gets published; if (profile, verify-term) keying shows no
behavioral difference on replay, say so and keep the simpler key
(occam) — the change ships only with a measured reason.

## 6. Constraint compliance

- `speculative_exact` lossless contract: untouched; every measured
  candidate is output-preserving.
- Output-changing modes: listed (B1–B4), not implemented, not simulated.
- Hub content-blind: canaries and checks ride P2P serving requests /
  local re-execution; artifacts carry digests, positions, timings —
  never token text.
- No new wire fields: everything landed is bench-crate + experiments;
  A3/A5 eligibility surfaces are explicitly NOT built pending an
  Architect ADR.
- Benchmarks record hardware, profile, engine build, corpus, network
  conditions, baseline method, repetitions, variance (per ANALYSIS
  files); raw + exact commands committed; negatives published
  (A6 analytical negative, the cache-flip negative finding, and the
  honest NOT-measured list in §3.5).
