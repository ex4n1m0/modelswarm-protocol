# ModelSwarm Cooperative Inference: ZCode Follow-Up Plan

## Purpose

This plan follows the original Windows/Rust ModelSwarm prototype. It changes the performance objective from **selecting one fast replica** to determining when **multiple identical replicas can improve one user’s request**.

The work is deliberately experimental. Ordinary autoregressive decoding is sequential, and wide-area synchronization can cost more than the compute it saves. The required result is therefore not “multi-peer mode works,” but a reproducible answer to: **under which models, prompts, hardware classes, network conditions, and algorithms does cooperative inference beat the fastest single peer?**

The first cooperative mechanism should be proposer/verifier speculative decoding. This architecture lets one or more workers propose future token blocks while a target-model verifier accepts a valid prefix and recovers at the first rejection. Existing runtimes expose acceptance metrics such as accepted draft tokens, mean acceptance length, and per-position acceptance, providing useful definitions for ModelSwarm’s instrumentation.[^1][^2][^3]

## Prerequisite gate

Do not start this plan until the earlier prototype has these verified capabilities:

- At least three Windows nodes can host one immutable model profile.
- The tracker returns exact-profile peers.
- Direct encrypted peer streams work.
- The local OpenAI-compatible gateway streams tokens correctly.
- The fastest-peer baseline records time to first token, inter-token latency, throughput, completion time, and network bytes.
- Host-to-consume eligibility works.
- Prompt and completion contents are absent from tracker storage and ordinary logs.

If any prerequisite is incomplete, ZCode must return to the corresponding original phase rather than implementing cooperative inference on unstable foundations.

## Research objective

Add an optional execution planner with five modes:

| Mode | Peers | Objective |
|---|---:|---|
| `single` | 1 | Fastest eligible baseline |
| `hedged` | 2 | Reduce tail latency by racing starts |
| `speculative` | 2–8 | Improve one stream’s accepted tokens per verification step |
| `search` | 4–64 | Improve difficult-answer quality through parallel candidates and verification |
| `map_reduce` | Variable | Accelerate naturally divisible workloads |

Only `single` is enabled by default. Every cooperative mode must automatically fall back to `single` when its predicted or observed cost exceeds the baseline.

## Scientific rule

The performance claim must always compare against the **fastest eligible single host available to that requester at that moment**, not an average or deliberately slow host.

For every experiment, record:

- Model profile, runtime build, quantization, and generation parameters.
- Request/prompt identifier without storing sensitive prompt text.
- Peer hardware and runtime configuration.
- Requester-to-peer RTT, jitter, loss, and measured throughput.
- Time to first token.
- Inter-token latency and tokens per second.
- End-to-end completion time.
- Drafted and accepted tokens.
- Mean acceptance length.
- Verification steps.
- Network bytes per accepted token.
- Total aggregate model tokens computed across all peers.
- Peak participating peers.
- Failure/fallback reason.
- Determinism or distribution-equivalence result where applicable.

vLLM defines mean acceptance length as `1 + accepted draft tokens / speculative steps`; ModelSwarm should use the same convention to make results comparable. Speculative decoding is intended primarily for medium-to-low request rates and memory-bound workloads, so results must not be generalized beyond tested conditions.[^4][^1]

## Architecture extension

```text
Local application
       │
ModelSwarm gateway
       │
Execution planner
       ├── single-peer scheduler
       ├── hedge controller
       ├── cooperative coordinator
       ├── candidate verifier
       └── fallback controller
       │
Tracker lookup + measured network topology
       │
Micro-swarm selected from exact-profile hosts
       ├── proposer A
       ├── proposer B
       ├── proposer C
       └── verifier V
```

The global tracker may know one million peers, but the coordinator should form **small temporary micro-swarms**. Initial limits are two peers for hedging and two to four peers for speculative experiments. Larger groups require evidence that they improve the selected metric.

## Protocol additions

Add a versioned cooperative namespace without breaking the original inference protocol:

```text
/msp/cooperative/1.0.0
```

### Required messages

```rust
pub enum CooperativeMessage {
    Capability(CooperativeCapability),
    SessionOpen(CooperativeSessionOpen),
    PrefillRequest(PrefillRequest),
    PrefillReady(PrefillReady),
    ProposalRequest(ProposalRequest),
    CandidateBlock(CandidateBlock),
    VerificationRequest(VerificationRequest),
    VerificationResult(VerificationResult),
    PrefixCommit(PrefixCommit),
    CreditUpdate(CreditUpdate),
    Cancel(CancelRequest),
    SessionClose(SessionClose),
}
```

### Capability record

```rust
pub struct CooperativeCapability {
    pub profile_id: ModelProfileId,
    pub runtime_build: String,
    pub tokenizer_hash: Hash,
    pub supports_logprobs: bool,
    pub supports_top_k_logits: bool,
    pub supports_batch_verify: bool,
    pub supports_kv_checkpoint: bool,
    pub max_candidate_tokens: u16,
    pub max_candidate_branches: u16,
    pub max_context_tokens: u32,
    pub benchmark_class: String,
}
```

### Candidate block

```rust
pub struct CandidateBlock {
    pub session_id: SessionId,
    pub round: u64,
    pub parent_prefix_hash: Hash,
    pub proposer_peer_id: PeerId,
    pub token_ids: Vec<u32>,
    pub draft_logprobs: Option<Vec<f32>>,
    pub generation_seed: u64,
    pub elapsed_us: u64,
    pub signature: Signature,
}
```

### Prefix commit

```rust
pub struct PrefixCommit {
    pub session_id: SessionId,
    pub round: u64,
    pub previous_prefix_hash: Hash,
    pub accepted_token_ids: Vec<u32>,
    pub new_prefix_hash: Hash,
    pub verifier_peer_id: PeerId,
    pub verification_digest: Hash,
    pub signature: Signature,
}
```

Every message must have strict byte/token limits, deadlines, monotonically increasing rounds, replay protection, parent-prefix validation, and cancellation semantics. Never transmit arbitrary native runtime memory structures over the network.

## Runtime strategy

### Research runtime

Keep the original `llama.cpp` path for the Windows product baseline. Add a separate, explicitly experimental runtime adapter for cooperative research. It may be:

- A pinned vLLM environment on supported test machines.
- A custom `llama.cpp` fork/sidecar exposing proposal and verification hooks.
- A minimal local research engine used only for correctness experiments.

Do not force experimental low-level hooks into the production runtime abstraction until benchmarks demonstrate value. vLLM already reports per-request and aggregate acceptance measurements, making it useful as a reference implementation and metrics oracle. Runtime flags and metric formats remain version-sensitive, so every experiment must pin the runtime version and persist its exact configuration.[^5][^6][^1]

### Profile identity

Cooperative compatibility becomes part of the immutable profile:

```text
base weights
+ quantization
+ tokenizer/chat template
+ context policy
+ runtime build
+ proposer method/version
+ verification algorithm/version
+ sampling contract
= CooperativeProfileId
```

Peers with different cooperative profile IDs may not participate in the same lossless speculative session.

## Subagents

ZCode should add these specialist agents to the original agent roster:

| Agent | Ownership | Deliverable |
|---|---|---|
| Transformer Internals | `research/transformer/`, technical notes | Tiny decoder, KV/logit tracing, correctness explanations |
| Decoding Algorithms | `crates/ms-decode/` | Baseline, hedge, proposal, verification, prefix-commit algorithms |
| Cooperative Protocol | `crates/ms-coop-protocol/` | Versioned messages, state machine, limits, serialization tests |
| Runtime Instrumentation | runtime adapters/forks | Logits, proposal, batch verification, acceptance telemetry |
| Experiment Platform | `apps/ms-bench/`, `experiments/` | Reproducible matrix runner, network impairment, raw results |
| Performance Modeling | `crates/ms-planner/` | Cost model, mode selection, fallback rules |
| Distributed Security | cooperative threat model/tests | Byzantine proposals, replay, coordinator/verifier abuse |
| Integrator | shared contracts and merge control | ADRs, compatibility review, release gates |

Each agent must work in a separate branch or worktree, modify only owned paths, and write `HANDOFF.md` containing assumptions, changed interfaces, commands, raw result locations, failures, and unresolved questions.

## Repository additions

```text
modelswarm/
├── crates/
│   ├── ms-coop-protocol/
│   ├── ms-decode/
│   ├── ms-planner/
│   ├── ms-verification/
│   └── ms-network-model/
├── apps/
│   ├── ms-bench/
│   └── ms-coop-simulator/
├── research/
│   ├── transformer/
│   ├── speculative-decoding/
│   ├── distributed-search/
│   └── correctness/
├── experiments/
│   ├── manifests/
│   ├── schemas/
│   ├── raw/          # ignored if large; CI retains artifacts
│   ├── processed/
│   └── reports/
├── protocol/
│   └── msp-cooperative-v1.md
└── docs/adr/
```

## Follow-up phases

### Phase P0 — freeze baseline

- Tag the last stable prototype commit.
- Build a deterministic benchmark corpus containing public, redistributable prompts and expected test properties.
- Benchmark `single` mode across at least three node classes.
- Add controllable network impairment for RTT, jitter, bandwidth, and packet loss.
- Freeze metric definitions and JSON/CSV schemas.
- Generate machine-readable raw output; do not copy numbers manually.
- Establish statistical methodology, warm-up policy, run count, random seeds, outlier policy, and confidence intervals before comparing algorithms.

**Gate:** No cooperative code until baseline runs are reproducible within a documented tolerance.

### Phase P1 — internals laboratory

- Implement or adapt a tiny transformer that exposes embeddings, attention, residual updates, logits, sampling, and KV cache.
- Create tests demonstrating prefill versus decode behavior.
- Confirm token-by-token equivalence between ordinary decoding and a reference verification algorithm on small deterministic cases.
- Document what data must cross the network: token IDs, probabilities where required, prefix hashes, and control messages.
- Measure the size of logits, top-
`k` logits, tokens, and KV data for the chosen reference profile.

**Gate:** The verification algorithm passes exhaustive small-vocabulary tests and randomized property tests.

### Phase P2 — local speculative baseline

- Implement proposer/verifier speculative decoding on one machine.
- Start with one proposer and one target verifier.
- Support greedy decoding first, then implement sampling only with a mathematically correct acceptance/recovery procedure.
- Compare output token-for-token for greedy mode.
- Compare empirical output distributions for sampled mode using fixed test suites and repeated seeded runs.
- Record acceptance rate, acceptance length, verification time, and end-to-end speed.

**Gate:** Correctness passes, and at least one tested workload improves over ordinary single-runtime decoding without excluding regressions from the report.

### Phase P3 — two-node speculative protocol

- Move proposer and verifier to separate machines.
- Exchange only bounded token/probability/control messages.
- Add round IDs, prefix hashes, deadlines, cancellation, and signed commits.
- Sweep proposal lengths such as 1, 2, 4, 8, and 16 tokens.
- Sweep controlled RTT and bandwidth conditions.
- Add automatic fallback when the measured cooperative estimate exceeds the single-host estimate.

**Gate:** At least one real two-node configuration beats the fastest single eligible host in median end-to-end completion time while preserving the declared correctness contract.

### Phase P4 — multi-proposer token tree

- Add two to four proposers with distinct branch assignments.
- Prevent duplicate proposals through deterministic branch allocation.
- Implement bounded candidate-tree assembly and batch verification.
- Compare breadth, depth, acceptance, verification cost, and network cost.
- Drop stragglers without blocking the verification round.
- Cap speculative compute and branches per accepted token.

**Gate:** Multi-proposer mode must outperform the best two-node mode for at least one predeclared workload; otherwise retain it as experimental or remove it.

### Phase P5 — adaptive planner

- Implement an explainable cost model using prompt length, output budget, peer runtime speed, RTT, jitter, loss, bandwidth, queue, historical acceptance, and verification cost.
- Predict `single`, `hedged`, or `speculative` before execution.
- Re-evaluate after prefill and every bounded number of rounds.
- Fall back to `single` when acceptance collapses or synchronization dominates.
- Never train a learned planner on fabricated data; use only measured experiment records.

**Gate:** On a held-out benchmark set, planner-selected mode should have lower median completion time than always-single without materially worsening the selected tail-latency threshold. Define “materially” before testing.

### Phase P6 — parallel quality mode

- Add `search` mode separately from lossless speed mode.
- Generate diverse complete or partial solutions across peers.
- Add task-specific verifiers: tests for code, exact checks for math, schemas for structured output, citation validation for grounded responses.
- Record pass@N, verified success, total compute, and wall time.
- Clearly label that this mode changes generation behavior and may not preserve the baseline distribution.

Research on multi-agent mixtures indicates that multiple candidate models/agents can improve response quality, but the gain depends on diversity and aggregation rather than replica count alone. Identical replicas therefore need purposeful diversity through seeds, branch constraints, roles, or search policies.[^7]

**Gate:** Enable only for task classes with an objective or separately validated verifier.

### Phase P7 — adversarial resilience

- Test fabricated token proposals, false probabilities, prefix equivocation, replay, delayed commits, denial-of-service branches, and malicious verifier decisions.
- Add redundant verification for sampled rounds.
- Add peer scoring and quarantine based on signed, reproducible mismatches.
- Ensure no single public peer can silently control a high-value answer.
- Measure the performance cost of each security mechanism.

**Gate:** A malicious proposer cannot alter a lossless verified result; verifier trust assumptions are explicit and tested.

### Phase P8 — Windows integration

- Add an Experimental Performance page to the Tauri application.
- Keep modes opt-in.
- Show selected peers, measured RTT range, mode, accepted tokens per round, fallback events, added compute, and observed speedup/slowdown.
- Do not advertise theoretical speedup.
- Add privacy disclosure that several peers may receive the prompt in cooperative mode.
- Preserve the original single-peer path as a stable fallback.

**Gate:** Cooperative mode may be included in a test release only after the same installer passes single-peer regression tests.

## Master ZCode continuation prompt

Paste this into ZCode GLM-5.3 Goal mode at the root of the existing ModelSwarm repository:

```text
You are continuing the existing ModelSwarm project. Do not rebuild or replace the prior Windows/Rust prototype. Inspect the repository, Git history, architecture documents, ADRs, test reports, and current status before changing code.

NEW MISSION
Extend ModelSwarm from fastest-replica routing into an experimental cooperative-inference research platform. Determine when multiple identical model replicas can improve one user's latency or answer quality compared with the fastest eligible single host.

SCIENTIFIC REQUIREMENT
The baseline is always the fastest eligible single peer available under the same model profile, request, network test condition, and generation settings. Never claim a speedup against an average, deliberately slowed, or different-class host. Report slowdowns and failed hypotheses with the same visibility as improvements.

TARGET EXECUTION MODES
1. single: original fastest-peer behavior.
2. hedged: race two starts to reduce tail latency.
3. speculative: proposer/verifier cooperative decoding using 2–8 exact-profile peers.
4. search: parallel candidate generation plus objective verification.
5. map_reduce: divisible workloads only.

NON-NEGOTIABLE CORRECTNESS RULES
- Greedy lossless mode must match baseline accepted tokens exactly.
- Sampled lossless mode must implement a valid rejection-sampling/recovery algorithm and pass empirical distribution tests.
- A mode that changes the target distribution must be labeled quality/search mode, never lossless speed mode.
- Synthetic acceptance is permitted only in isolated benchmark tests and must never produce user-facing output.
- Every participant must share the exact CooperativeProfileId, including model artifact, tokenizer, template, quantization, runtime build, proposer method, verifier algorithm, and sampling contract.
- Every cooperative message is bounded, versioned, signed where trust requires it, replay-protected, and linked to the parent prefix hash.
- Do not transfer raw KV-cache/native pointers or arbitrary runtime memory over the public protocol.
- Never conceal the prompt replication privacy cost of multi-peer modes.

ENGINEERING CONSTRAINTS
- Primary implementation language remains Rust.
- Preserve original tracker, identity, eligibility, model catalog, P2P transport, Windows installer, and single-peer gateway.
- Add cooperative features behind compile-time or runtime experimental flags.
- Keep production llama.cpp baseline untouched until a research adapter proves value.
- A pinned vLLM or custom instrumented runtime may be used as a reference research adapter.
- Keep modelswarm.deepflux.space as control plane only; cooperative token traffic remains peer-to-peer.
- Use small micro-swarms selected from the global peer set; never synchronize all known peers.

SUBAGENTS
Create bounded subagents with separate worktrees/branches:
- Transformer Internals
- Decoding Algorithms
- Cooperative Protocol
- Runtime Instrumentation
- Experiment Platform
- Performance Modeling
- Distributed Security
- Integrator

Each subagent receives owned paths, frozen interface inputs, explicit acceptance tests, and forbidden scope. Each must write HANDOFF.md with changed files, commands, raw result artifacts, assumptions, failures, and unresolved risks. The Integrator alone may change shared schemas after an ADR review.

REQUIRED PHASE ORDER
P0 baseline freeze
P1 transformer internals laboratory
P2 local speculative baseline
P3 two-node speculative protocol
P4 multi-proposer token tree
P5 adaptive planner
P6 parallel quality/search mode
P7 adversarial resilience
P8 Windows experimental integration

Do not skip gates. Do not begin a later phase merely because code compiles.

INITIAL TASK: P0 ONLY
1. Audit the current repository against the prerequisite gate.
2. Create an ADR for cooperative inference and its experimental status.
3. Specify metric definitions and machine-readable schemas.
4. Build a deterministic benchmark runner for the existing single mode.
5. Add controllable network impairment in the test environment.
6. Define public/redistributable benchmark prompts and privacy-safe identifiers.
7. Pin model/runtime/hardware/network metadata in every result.
8. Write the statistical protocol before running comparisons.
9. Run the baseline matrix on available test nodes.
10. Produce experiments/reports/p0-baseline.md with raw artifact links, exact commands, variance, limitations, and prerequisite failures.

STOP CONDITIONS
- Stop and request input if the existing prototype has not reached the prerequisite gate.
- Stop before downloading large models, creating paid infrastructure, changing DNS, publishing binaries, or accepting a new model license.
- Stop if a requested runtime hook requires maintaining a large unsafe fork; present alternatives first.
- Stop if correctness cannot be demonstrated.

P0 DEFINITION OF DONE
- Existing single mode is regression-tested.
- Benchmark output is reproducible and machine-readable.
- Fastest-host baseline selection is independently verified.
- Network impairment is calibrated.
- Metric/statistical schemas are frozen.
- Subagent handoffs and risk register are complete.
- No cooperative speed claim has been made.

Begin by inspecting the current state. Present the prerequisite audit, proposed file changes, subagent assignments, experiment schema, and acceptance tests before editing. After approval, execute P0 only.
```

## Phase prompts

### P1 prompt

```text
Execute ModelSwarm cooperative Phase P1 only after P0 passes.

Build a transformer-internals laboratory and correctness oracle. Implement or adapt the smallest practical decoder-only transformer with explicit tokenization, embeddings, causal attention, residual paths, logits, sampling, prefill, and KV cache. Add deterministic fixtures and exhaustive small-vocabulary tests.

Document the exact information needed for distributed proposal and verification. Measure serialized sizes for token IDs, full logits, top-k logits, and representative KV state without extrapolating unsupported claims. Produce protocol recommendations, but do not implement multi-node generation.

Gate: baseline decoding and the reference verifier must match exactly on deterministic cases and pass randomized property tests. End with raw tests, commands, and HANDOFF files.
```

### P2 prompt

```text
Execute Phase P2 only after the P1 correctness gate passes.

Implement single-machine proposer/verifier speculative decoding behind an experimental feature flag. Start with greedy decoding. Then add sampled decoding only after implementing and testing mathematically valid acceptance and recovery.

Instrument draft tokens, accepted tokens, acceptance histogram, mean acceptance length, proposal time, verification time, ordinary decode time, network-independent completion time, and aggregate compute. Compare against ordinary decoding on the same machine, model profile, prompts, seeds, and output limits.

Do not optimize before correctness. Preserve negative results. Gate: greedy equality, sampled distribution tests, and at least one genuinely faster tested workload before moving to P3.
```

### P3 prompt

```text
Execute Phase P3 only after P2 passes.

Move proposer and verifier onto two real nodes using /msp/cooperative/1.0.0. Implement round state, prefix hashes, bounded candidate blocks, deadlines, cancellation, signed commits, disconnect recovery, and fallback to single mode.

Sweep proposal lengths and controlled RTT/jitter/bandwidth conditions. Compare every run to the fastest eligible single host. Do not use simulated acceptance for user output. Gate: one reproducible real-network configuration improves median completion time while preserving correctness; otherwise document the boundary and stop before P4.
```

### P4 prompt

```text
Execute Phase P4 only after P3 demonstrates a real benefit.

Add two to four proposers. Assign non-overlapping branches/seeds, build a bounded candidate tree, batch-verify candidates, and commit the valid prefix. Drop stragglers at the round deadline. Measure duplicate work, candidate diversity, acceptance, verifier utilization, synchronization, bytes per accepted token, total compute, and wall time.

Compare against fastest-single and best two-node speculative modes. Gate: measurable improvement on a predeclared workload without correctness regression. If no improvement exists, retain P3 and archive P4 as a negative experiment.
```

### P5 prompt

```text
Execute Phase P5 using only real P0-P4 measurements.

Implement an explainable execution planner choosing single, hedged, or speculative mode. Inputs include prompt/output lengths, peer speed, queues, RTT, jitter, loss, bandwidth, historical acceptance, proposal length, and verifier cost. Add online fallback when observed acceptance or synchronization violates the predicted budget.

Evaluate on held-out runs. Freeze success criteria before evaluation. Gate: better median completion time than always-single without exceeding the predeclared tail-latency and compute-overhead limits.
```

### P6 prompt

```text
Execute Phase P6 as a separate quality mode, not a lossless speed feature.

Implement parallel candidate/search workflows only for objectively verifiable tasks. Begin with code tasks using sandboxed tests and structured-output tasks using schema validation. Use controlled seed/role/branch diversity and a separately evaluated aggregator.

Measure verified success, pass@N, wall time, total compute, verifier errors, and marginal value per added peer. Never claim that more replicas necessarily improve quality. Gate: statistically supported improvement on a held-out benchmark with a reliable verifier.
```

### P7 prompt

```text
Execute Phase P7 as adversarial testing.

Attack candidate blocks, probabilities, round ordering, prefix hashes, signatures, capability tokens, coordinator decisions, verifier decisions, resource limits, and cancellation. Test Byzantine proposers, equivocation, collusion assumptions, replay, flooding, and privacy leakage.

Add the least expensive mitigation that meets the documented threat model. Gate: malicious proposers cannot change a lossless verified result, and remaining verifier/coordinator trust assumptions are explicit.
```

### P8 prompt

```text
Execute Phase P8 only after P3-P7 gates applicable to the selected release mode pass.

Integrate cooperative inference into the Windows Tauri application as an opt-in Experimental Performance feature. Show mode, participating peer count, privacy warning, measured speedup or slowdown, acceptance length, fallback events, and compute overhead. Preserve stable single mode and all previous installer/E2E tests.

Do not advertise a universal speedup. Release only measured support envelopes such as model profile, hardware class, RTT range, prompt/output range, and algorithm configuration.
```

## Acceptance tests

| Test | Required result |
|---|---|
| Greedy baseline versus verifier | Exact accepted-token equality |
| Sampled lossless mode | Passes predeclared distribution tests |
| Wrong prefix hash | Candidate rejected |
| Duplicate/stale round | Candidate rejected |
| Different cooperative profile | Peer excluded |
| Proposer lies about logprobs | Verification prevents output corruption |
| Slow proposer | Deadline removes it without stalling session |
| Coordinator disconnect | Explicit cancellation/fallback; no silent corruption |
| RTT rises mid-session | Planner reduces speculation or falls back |
| Acceptance collapses | Cooperative mode terminates or reduces depth |
| Cooperative mode slower | Slowdown recorded; no speedup claim |
| Synthetic acceptance enabled | Blocked from user-serving path |
| Prompt privacy | User warned that selected peers receive context |
| One million available peers | Only bounded micro-swarm selected |
| Fastest single host changes | Baseline and route are recomputed |

## Success criteria

The research program succeeds if it produces any of these outcomes with reproducible evidence:

- A bounded model/network envelope where speculative micro-swarms reduce one user’s completion time.
- A planner that reliably avoids cooperative modes when they would be slower.
- A parallel search mode that improves objectively verified task success.
- A rigorous negative result defining why public-internet replica cooperation does not improve token speed under tested conditions.

A negative result is valuable because it prevents ModelSwarm from spending engineering effort on a mechanism that only increases compute and privacy exposure. The original fastest-peer swarm remains useful regardless of the cooperative research outcome.

---

## References

1. [Per-Request Acceptance Metrics - vLLM Documentation](https://docs.vllm.ai/en/latest/features/speculative_decoding/acceptance_metrics/)

2. [vllm.v1.spec_decode.metrics](https://docs.vllm.ai/en/stable/api/vllm/v1/spec_decode/metrics/)

3. [Speculative Decoding Guide - vLLM Ascend](https://docs.vllm.ai/projects/ascend/en/main/user_guide/feature_guide/speculative_decoding.html) - vLLM Ascend plugin - a community-maintained hardware plugin for running vLLM on the Ascend NPU.

4. [Speculative Decoding - vLLM](https://docs.vllm.ai/en/stable/features/speculative_decoding/)

5. [Metrics - vLLM Documentation](https://docs.vllm.ai/en/stable/design/metrics/)

6. [每请求接受指标 · vllm](https://vllm.atomgit.com/features/speculative_decoding/acceptance_metrics.html) - none

7. [Mixture-of-Agents Enhances Large Language Model ...](https://arxiv.org/html/2406.04692v1) - Overall, using multiple different LLMs consistently yielded better results. Both results suggest tha...

