# ZCode Follow-Up: Competitive Learnings and Architecture Revision

## Purpose

This document is a continuation of the original ModelSwarm implementation plan and the later cooperative-performance plan. It instructs ZCode, using GLM-5.3 mode, to revise the project after reviewing comparable systems and research.

Do not restart the repository or discard completed work. Audit the current implementation, preserve compatible components, and introduce the changes below through small, testable commits.

## Core conclusion

The original marketplace and tracker concept is not sufficiently differentiated by itself. p2ptokens already describes a Rust, libp2p, Tokio, Axum, and Tauri application in which peers both serve and consume inference, a coordinator matches models, data travels directly between peers, and access is governed by a BitTorrent-style upload/download ratio with signed co-receipts.[^1]

ModelSwarm must therefore focus its technical identity on this narrower objective:

> Create network-aware micro-swarms of machines hosting the same immutable model profile, and use them to cooperatively accelerate or improve one request through exact speculative decoding, verified token-tree exploration, parallel solution search, and automatic fastest-host fallback.

The protocol must never assume that more peers automatically means better performance. It must measure the fastest eligible single host and use cooperative execution only when the predicted and measured outcome is superior.

## Mandatory differentiation

The implementation must make these properties first-class rather than optional marketing claims:

- **Exact-profile eligibility:** A peer may consume a model swarm only while it proves it is actively serving the same immutable `ModelProfileId`.
- **Micro-swarm execution:** Global membership can reach millions, but a request uses a small dynamically selected cohort.
- **Lossless default:** Cooperative acceleration must preserve the target model’s output distribution unless the user explicitly selects an approximate mode.
- **Adversarial verification:** Treat every Internet peer as potentially slow, stale, dishonest, or malicious.
- **Performance honesty:** Compare every cooperative mode with the fastest eligible individual host.
- **Automatic fallback:** Abort or avoid collaboration whenever coordination is predicted to be slower.
- **Content-blind control plane:** The Vercel hub handles metadata, eligibility, matchmaking, and signaling—not prompts, model activations, KV caches, or generated text.

## Prior-art decisions

| Existing system | Adopt or study | Do not copy blindly |
|---|---|---|
| p2ptokens | Rust/libp2p workspace design, signed metering receipts, authenticated heartbeats, direct data path, NAT traversal, retries, capacity reservation | Generic ratio economy as ModelSwarm’s main differentiator; full-completion fan-out presented as cooperative decoding[^1] |
| LocalAI | Separate one-worker federation from multi-worker model sharding; capability negotiation | Static clusters that cannot accept workers after inference begins; one-model limitations[^2][^3] |
| Petals | DHT discovery, layer coverage, session routing, churn-aware replacement | Wide-area per-token pipelines as the default architecture |
| Hyperspace | Capability registry, liveness challenges, DHT/gossip fallback, differentiated node roles | Unverified scale or performance claims as engineering assumptions |
| KwaaiNet | Rust-native P2P patterns, Hugging Face block management, whole-model and shard capabilities | Requiring experimental sharding for the first release |
| exo | Topology measurement, network-aware partitioning, high-speed local micro-clusters | Assuming LAN results apply to the public Internet |
| Pooled | Binary protocol frames, compact activations, speculative rollback, deterministic golden tests | Browser-specific transport as the Windows native foundation |
| PARALLAX | Live latency map, region-aware placement, dynamic pipeline selection | Global model redistribution on every membership change |
| DSD | Multi-token batch settlement and adaptive speculation windows | Approximate semantic relaxation in the lossless default mode[^4] |
| DSD-Sim | Simulator-first scheduling, RTT/jitter modeling, acceptance traces | Learned scheduling before sufficient real telemetry exists |
| FlowSpec | Continuous candidate-tree expansion, pruning, and verification | Complex tree scheduling before linear speculative decoding works |

## Revised architecture

### Global control plane

Use `ModelSwarm.deepflux.space` as a tracker and signaling service with the following responsibilities:

- Register signed peer heartbeats.
- Maintain an index keyed by exact `ModelProfileId`.
- Record peer capability classes and current capacity.
- Enforce host-to-consume eligibility.
- Return bounded candidate sets for direct connection.
- Exchange WebRTC or libp2p signaling metadata where necessary.
- Record signed usage receipts and audit outcomes.
- Issue short-lived session authorizations.
- Never proxy inference payloads.

Keep the hub interface stateless where possible. Place durable registry, accounting, and reputation state in an external store rather than process memory.

### Direct data plane

Use Rust libp2p for direct peer communication:

- Noise-authenticated transport.
- QUIC where supported; TCP plus Yamux as fallback.
- AutoNAT, identify, relay, DCUtR hole punching, and optional UPnP.
- Signed protocol envelopes with replay protection.
- Bounded frames, deadlines, cancellation, backpressure, and per-peer resource limits.
- Direct inference streams that bypass Vercel.

p2ptokens already documents Noise-authenticated TCP/Yamux communication and full NAT traversal, so these features should be treated as baseline engineering rather than product novelty.[^1]

### Execution plane

Expose five execution modes:

| Mode | Peers | Purpose | Correctness contract |
|---|---:|---|---|
| `single` | 1 | Fastest eligible host | Ordinary target-model generation |
| `hedged` | 2–3 | Reduce tail latency | Keep one complete stream; cancel others |
| `speculative_exact` | 2–8 | Improve single-request token latency | Exact target distribution |
| `search_verified` | 4–64 | Improve answer quality | Multiple candidates plus explicit verifier |
| `map_reduce` | Variable | Parallelizable documents, code, or data | Application-specific aggregation |

Do not expose approximate semantic acceptance until exact speculative execution is complete, benchmarked, and independently selectable.

## Exact model identity

Replace model-name matching with an immutable profile manifest:

```rust
pub struct ModelProfileManifest {
    pub schema_version: u16,
    pub hf_repo: String,
    pub hf_revision: String,
    pub artifact_hashes: Vec<ArtifactHash>,
    pub tokenizer_hash: Hash32,
    pub chat_template_hash: Hash32,
    pub architecture_hash: Hash32,
    pub quantization: QuantizationDescriptor,
    pub runtime_name: String,
    pub runtime_version: String,
    pub runtime_build_hash: Hash32,
    pub decoding_abi_version: u16,
    pub speculative_capabilities: Vec<SpeculativeCapability>,
}
```

Derive `ModelProfileId` from canonical serialization of the complete manifest. A Hugging Face update creates a new profile and swarm; it must never silently mutate an existing profile.

Peers must prove artifact possession through randomized chunk challenges plus successful local inference challenges. Hash possession alone does not prove that a peer can execute the model correctly.

## Micro-swarm scheduler

The global tracker returns candidate peers, but the requesting Windows client selects the execution cohort locally. The tracker must not become a latency-critical centralized scheduler.

Score candidates using:

- Exact profile and decoding ABI match.
- Measured direct RTT, jitter, loss, and bandwidth.
- Prefill and decode throughput.
- Current queue depth.
- Recent speculative acceptance rate with this profile.
- Receipt and audit reputation.
- Hardware class and available memory.
- NAT path type: direct, hole-punched, or relayed.
- Session failure history.

Start with deterministic heuristics. Do not introduce a learned scheduler until production telemetry shows that an explainable cost model is inadequate.

Suggested decision rule:

```text
predicted_swarm_completion
  = prefill_cost
  + proposal_cost
  + verification_cost
  + synchronization_cost
  + expected_rollback_cost
  + failure_risk_penalty

select cooperative mode only when:
predicted_swarm_completion + confidence_margin
  < predicted_fastest_single_completion
```

The confidence margin must initially be conservative.

## Cooperative protocol

### Roles

A cooperative session may assign these temporary roles:

- **Coordinator:** Owns the user connection and accepted-prefix state.
- **Proposer:** Produces speculative token blocks or candidate branches.
- **Verifier:** Applies the target model and exact acceptance algorithm.
- **Auditor:** Recomputes randomly selected rounds.
- **Standby:** Maintains warm state for failover.

Roles are per session and must not imply permanent network authority.

### Session messages

Implement versioned binary messages:

```rust
pub enum SwarmMessage {
    SessionOffer(SessionOffer),
    SessionAccept(SessionAccept),
    PrefillRequest(PrefillRequest),
    PrefillReady(PrefillReady),
    DraftRequest(DraftRequest),
    DraftProposal(DraftProposal),
    VerifyRequest(VerifyRequest),
    VerifyResult(VerifyResult),
    CommitPrefix(CommitPrefix),
    CommitAck(CommitAck),
    AuditChallenge(AuditChallenge),
    AuditResult(AuditResult),
    CancelRound(CancelRound),
    PeerReplace(PeerReplace),
    SessionClose(SessionClose),
}
```

Every state-changing message must contain:

- Protocol version.
- Session ID.
- Round and sequence number.
- `ModelProfileId`.
- Accepted-prefix hash.
- Generation-parameter hash.
- Sender `PeerId`.
- Deadline.
- Nonce.
- Signature.

All handlers must be idempotent. Duplicate commits cannot advance state twice, and stale-prefix proposals must be rejected.

### Initial exact algorithm

Implement the first cooperative mode with one coordinator, one proposer, and one verifier:

1. Coordinator selects two low-RTT peers with the exact profile and decoding ABI.
2. All participants prefill the same prompt and report a KV-state commitment.
3. Proposer generates a speculative window of token IDs and required proposal probabilities.
4. Verifier evaluates the entire window in one target-model pass.
5. Verifier applies the standard lossless acceptance/rejection rule.
6. Coordinator commits the accepted prefix plus the verifier’s correction token where required.
7. Participants acknowledge the exact committed prefix hash.
8. Window size increases or decreases according to measured acceptance, RTT, and processing cost.
9. Any inconsistency causes rollback to the last committed prefix and replacement or single-host fallback.

Distributed speculative systems reduce synchronization by settling multiple accepted tokens in one round; Gradient’s DSD implementation describes draft, verification, and batch-settlement stages and reports project-measured acceleration under its tested conditions. Treat those figures as research evidence, not as ModelSwarm performance promises.[^4]

### Multi-proposer extension

Only after two-peer exact mode passes all gates, add 2–7 proposers:

- Assign distinct deterministic branches or seeds.
- Deduplicate identical token prefixes.
- Build a bounded candidate trie.
- Prune branches using target-compatible scores.
- Batch-verify candidates where the runtime supports tree attention.
- Commit only a prefix validated by the exact acceptance contract.
- Cancel losing branches immediately.

Do not transfer complete KV caches over the public Internet by default. Prefer recomputation, compact token sequences, or paged checkpoint deltas after benchmarks prove a benefit.

## Reciprocal eligibility

The user’s rule remains:

> A peer may consume only the swarm for a model profile it actively hosts.

Implement it as a short-lived eligibility lease:

```rust
pub struct EligibilityLease {
    pub peer_id: PeerId,
    pub model_profile_id: ModelProfileId,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    pub verified_capacity: CapacityClass,
    pub audit_epoch: u64,
    pub issuer_signature: Signature,
}
```

Requirements:

- Hosting must be verified periodically.
- The peer must advertise at least one real serving slot.
- Repeated refusal, timeout, or invalid output reduces or suspends eligibility.
- New peers receive a small bootstrap allowance so the network can grow.
- Credits should remain profile-specific in the first version.
- Capacity-weighted fairness may be added later, but a weak host must not receive unlimited use solely by staying online.

p2ptokens uses a more general upload/download ratio with newcomer grace and signed chunk receipts. Study that mechanism, but preserve ModelSwarm’s exact-profile lease as the defining access rule.[^1]

## Security requirements

### Threat model

Assume peers can:

- Advertise models they do not hold.
- Return plausible but fabricated proposals.
- Replay old messages or receipts.
- Withhold results after consuming coordinator resources.
- Manipulate latency measurements.
- Collude during audits.
- Flood registration and matching endpoints.
- Attempt prompt extraction or retention.
- Send malformed protocol frames.

### Required controls

- Ed25519 peer identity bound to libp2p `PeerId`.
- Signed heartbeats, leases, proposals, commits, and receipts.
- Prefix-hash and profile-hash binding on every inference message.
- Random redundant verification for sampled rounds.
- Rate limits and proof-of-work or deposits only if Sybil abuse is demonstrated.
- Reputation separated by model profile and capability.
- Maximum prompt bytes, output tokens, concurrent sessions, frame size, and speculative window.
- Sandboxed runtime boundary for downloaded model metadata and templates.
- No execution of arbitrary Hugging Face repository code.
- Clear warning that serving peers see plaintext prompts unless a later confidential-compute tier is implemented.

## Telemetry and benchmarks

Collect per session:

- Time to first token.
- Time per output token.
- End-to-end completion time.
- Prompt and output token counts.
- Fastest-single prediction and actual result.
- Cooperative prediction and actual result.
- Proposal window size.
- Accepted tokens per round.
- Acceptance rate.
- Rollback count.
- Bytes sent per accepted token.
- Coordinator, proposer, and verifier compute time.
- RTT, jitter, packet loss, and relay status.
- Failure and fallback reason.
- Quality or exactness result.

The benchmark harness must compare:

- Local fastest host.
- Remote fastest host.
- Two-host hedge.
- Two-peer exact speculation.
- Four-peer exact speculation.
- Multi-proposer tree mode when available.

Test network matrices at minimum:

- LAN or loopback.
- 5, 10, 20, 40, 80, and 150 ms RTT.
- Controlled jitter.
- 0%, 0.1%, 1%, and 3% packet loss.
- Direct and relayed paths.
- Symmetric and asymmetric peer hardware.

LocalAI documents that its P2P worker mode is limited to llama.cpp-compatible models, one distributed model, and workers known before inference starts. ModelSwarm tests must therefore include joining, leaving, and replacing peers between speculative rounds without corrupting the accepted sequence.[^2][^3]

## Repository changes

Create or revise these Rust crates:

```text
crates/
  modelswarm-types/         canonical IDs, manifests, protocol types
  modelswarm-identity/      keys, signatures, peer identity
  modelswarm-transport/     libp2p behavior and NAT traversal
  modelswarm-tracker-api/   client for modelswarm.deepflux.space
  modelswarm-eligibility/   leases, profile-specific accounting, audits
  modelswarm-runtime/       llama.cpp or selected inference backend adapter
  modelswarm-scheduler/     peer scoring and execution-mode decision
  modelswarm-session/       state machine, commits, rollback, replacement
  modelswarm-speculation/   exact proposer/verifier logic
  modelswarm-bench/         repeatable benchmark and network emulation
  modelswarm-node/          daemon composition
  modelswarm-desktop/       Tauri Windows application
```

Create tracker packages:

```text
apps/tracker/
  api/register
  api/heartbeat
  api/candidates
  api/session-authorize
  api/lease
  api/receipt
  api/audit
  api/model-catalog
```

The tracker API must use lowercase canonical hostname configuration even if the display name is `ModelSwarm.deepflux.space`.

## Subagents

ZCode must create these subagents and restrict each to its domain:

### Prior-Art Auditor

- Map current repository features against p2ptokens, LocalAI, Petals, Hyperspace, KwaaiNet, exo, Pooled, PARALLAX, DSD, and FlowSpec.
- Identify duplicated work, reusable ideas, protocol conflicts, and differentiation gaps.
- Produce architecture decision records, not code.

### Protocol Architect

- Define canonical model identity, leases, session states, binary messages, replay rules, and compatibility negotiation.
- Produce state diagrams and protocol test vectors.
- Must not edit UI code.

### Runtime Engineer

- Expose deterministic tokenization, prefill, decode, logits, speculative verification, KV commitments, rollback, and cancellation.
- Build behind a runtime trait.
- Must not depend directly on tracker APIs.

### Network Engineer

- Implement libp2p transport, discovery hints, Noise identity, QUIC/TCP, relay, DCUtR, AutoNAT, deadlines, and resource limits.
- Must not implement economic policy.

### Scheduler Scientist

- Implement the fastest-single baseline, explainable cost model, micro-swarm selection, adaptive windows, and simulator.
- Must publish negative benchmark results.
- Must not substitute simulated improvements for hardware measurements.

### Security Engineer

- Threat-model hostile peers, define challenge/audit flows, fuzz protocol decoders, verify signatures, and test replay/Sybil/resource-exhaustion defenses.
- Block release on critical findings.

### Tracker Engineer

- Implement the Vercel control plane and durable storage.
- Enforce content blindness and strict request limits.
- Must not proxy prompts or generated tokens.

### Windows Product Engineer

- Build installer, tray/background service, model setup, hosting controls, network diagnostics, benchmark view, privacy warnings, and updates.
- Must not hide fallback or performance results.

### Test and Release Engineer

- Own CI, deterministic golden tests, integration environments, network emulation, compatibility matrix, signed Windows artifacts, and release gates.
- Must reproduce every performance claim.

## Development phases

### Phase A — Audit and rebaseline

Deliverables:

- Feature comparison and ADRs.
- Current repository dependency graph.
- Fastest-single-host benchmark.
- Exact model-profile schema.
- Decision on whether to reuse, fork, interoperate with, or merely study p2ptokens.

Exit gate:

- No implementation proceeds until the project’s differentiation is written in one testable paragraph.

### Phase B — Protocol foundation

Deliverables:

- Canonical serialization and `ModelProfileId`.
- Signed protocol envelope.
- Session state machine.
- Eligibility lease.
- Receipt and audit types.
- Golden test vectors shared by tracker and Rust clients.

Exit gate:

- Property tests prove that stale, duplicated, reordered, and cross-profile messages cannot advance session state.

### Phase C — Single and hedged modes

Deliverables:

- Direct single-provider inference.
- Fastest-candidate probing.
- Two- and three-peer racing.
- Cancellation and capacity release.
- Honest telemetry.

Exit gate:

- Hedging improves selected tail-latency workloads without duplicate output or leaked capacity.

### Phase D — Exact two-peer speculation

Deliverables:

- Proposer/verifier protocol.
- Lossless acceptance implementation.
- Adaptive but bounded window controller.
- Replay-safe commit protocol.
- Single-host fallback.

Exit gate:

- For deterministic decoding, outputs match ordinary decoding exactly across the golden corpus.
- For sampling, statistical tests show the required target distribution contract.
- At least one real low-RTT configuration beats the fastest single host reproducibly.

### Phase E — Multi-proposer trees

Deliverables:

- Candidate trie.
- Branch assignment and deduplication.
- Batched/tree verification where available.
- Straggler cancellation.
- Auditor sampling.

Exit gate:

- Four-peer mode beats two-peer mode on at least one declared workload without violating correctness.

### Phase F — Public hostile swarm

Deliverables:

- NAT traversal and relays.
- Eligibility audits.
- Reputation and suspension.
- Malformed-frame and load testing.
- Prompt privacy warnings.

Exit gate:

- Malicious test peers cannot corrupt accepted output, forge usage, replay commits, or wedge sessions indefinitely.

### Phase G — Windows beta

Deliverables:

- Signed `.msi` and `.exe`.
- Automatic compatible model download from pinned Hugging Face revisions.
- Hosting status, exact profile ID, serving health, earned eligibility, peer path, and mode visibility.
- Opt-in diagnostics export with prompt contents excluded.

Exit gate:

- Clean install, upgrade, model migration, offline launch, firewall recovery, crash recovery, and uninstall pass on supported Windows versions.

## Non-negotiable release gates

Do not claim cooperative acceleration unless all are true:

- Baseline is the fastest eligible single host.
- Tests use real hardware and controlled network conditions.
- Results include failures and regressions.
- Exact mode satisfies its correctness contract.
- The scheduler falls back when conditions cross the break-even point.
- No prompt or generated token passes through the tracker.
- Every model and runtime artifact is cryptographically identified.
- Every performance chart can be reproduced from committed benchmark metadata.

## Master prompt for ZCode

Copy the following into ZCode GLM-5.3 mode:

```text
Continue the existing ModelSwarm project. Do not restart it and do not discard working code.

First, create and coordinate nine bounded subagents:
1. Prior-Art Auditor
2. Protocol Architect
3. Runtime Engineer
4. Network Engineer
5. Scheduler Scientist
6. Security Engineer
7. Tracker Engineer
8. Windows Product Engineer
9. Test and Release Engineer

Read the existing architecture, implementation plan, cooperative-performance plan, and this competitive-learnings addendum before editing code.

The architectural objective is now precise:
ModelSwarm is not primarily another P2P inference marketplace. It forms small, network-aware micro-swarms from a potentially massive population of peers hosting the exact same immutable model profile. Those peers cooperatively accelerate or improve one request using exact speculative decoding, verified candidate trees, parallel search, and automatic fallback to the fastest single eligible host.

Preserve these constraints:
- Primary implementation language: Rust.
- Product: installable Windows executable using Tauri where appropriate.
- Control-plane domain: modelswarm.deepflux.space.
- Hugging Face models are downloaded at pinned revisions and cryptographically verified.
- A user may consume only a swarm for the exact model profile that user actively hosts.
- The tracker is content-blind and never proxies inference payloads.
- Direct peer transport uses authenticated libp2p.
- Global swarm size and per-request micro-swarm size are separate concepts.
- Exact speculative mode must preserve the target model’s output contract.
- Cooperative execution must be disabled whenever it is predicted to lose against the fastest eligible single host.

Treat p2ptokens as the closest architectural prior art. Audit it before rebuilding equivalent tracker, receipt, NAT, retry, and Rust/Tauri patterns. Document whether to reuse, fork, interoperate, or differentiate. Do not copy source unless licensing, attribution, and project policy permit it.

Implement phases in strict order:
A. Audit and fastest-single baseline
B. Protocol foundation and exact model identity
C. Single-provider and hedged modes
D. Two-peer exact speculative decoding
E. Multi-proposer candidate trees
F. Public hostile-swarm hardening
G. Signed Windows beta

For every phase:
- Write an ADR before major architectural changes.
- Define measurable acceptance criteria.
- Add unit, property, integration, adversarial, and benchmark tests as applicable.
- Commit in small reviewable changes.
- Report uncertainties and negative results.
- Never report a speedup using an average or deliberately slow single-host baseline.
- Never replace real benchmarks with simulation.

Begin only with Phase A. Inspect the repository, produce the prior-art matrix, identify overlapping components, freeze a fastest-single-host benchmark harness, define the immutable ModelProfileManifest, and present the proposed ADRs. Do not implement multi-peer speculation until Phase A receives approval.
```

## First ZCode response required

ZCode’s first response must contain only:

- Repository audit summary.
- Components that can remain unchanged.
- Components requiring redesign.
- p2ptokens reuse/fork/interoperate/differentiate recommendation.
- Proposed ADR list.
- Phase A task graph by subagent.
- Risks and unknowns.
- Exact commands and tests it intends to run.

It must not claim that implementation is complete, and it must not begin the later phases automatically.

---

## References

1. [github.com · pur4v · p2ptokensGitHub - pur4v/p2ptokens: Distributed, peer-to-peer ...](https://github.com/pur4v/p2ptokens) - Distributed, peer-to-peer, decentralized network for LLM inference — peers seed & leech completions,...

2. [P2P / Federated Inference - LocalAI](https://localai.io/docs/features/distribute/index.print.html) - Tip Looking for production-grade horizontal scaling with PostgreSQL and NATS? See Distributed Mode. ...

3. [Getting started](https://localai.io/docs/getting-started/index.print.html) - Welcome to LocalAI! This section takes you from a fresh install to a working chat, a working API cal...

4. [Speculative Decoding for the Decentralized Inference](https://gradient.network/blog/turning-latency-into-throughput-speculative-decoding-for-the-decentralized-inference) - Decentralized Speculative Decoding (DSD), which we have implemented inside the Parallax distributed ...

