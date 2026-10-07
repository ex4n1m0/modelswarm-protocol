# Prior-Art Audit — Per-System Notes and Sources (appendix to prior-art-matrix.md)

Verbatim research record from the Prior-Art Auditor agent, 2026-10-04
(GitHub data pulled live via the GitHub API on the audit date).

## 1. LocalAI

- Two distinct modes: federated (load-balancer routes each whole request to
  one node, every node holds the full model) vs worker mode ("Requests are
  processed by all the workers which contributes to the final inference
  result (by sharing the model weights)", llama.cpp RPC only, single model,
  workers must join before inference starts).
- Discovery via go-libp2p DHT under a shared token (EdgeVPN library);
  `LOCALAI_P2P_DISABLE_DHT` falls back to mDNS; docs steer production users
  to PostgreSQL+NATS distributed mode instead.
- Adopt/study: the token-gated libp2p overlay (DHT + mDNS fallback) is a
  proven, simple swarm-membership pattern for micro-swarm sessions.
- Sources: https://localai.io/docs/features/distribute ·
  https://localai.io/docs/features/p2p ·
  https://api.github.com/repos/mudler/LocalAI (MIT, 49,385★, pushed 2026-10-04).
- Confidence: verified-from-source.

## 2. Petals

- The original "BitTorrent-style" layer-sharded swarm (10.6k★), effectively
  unmaintained (last push 2024-09-07). Trust via redundancy, not
  verification; public swarm has explicit privacy caveats.
- Adopt/study: layer-fraction allocation; the public swarm monitor
  (health.petals.dev) as UX precedent; dormancy as a warning about
  volunteer-only economies.
- Sources: https://github.com/bigscience-workshop/petals ·
  https://api.github.com/repos/bigscience-workshop/petals ·
  PETALS paper (OpenReview).
- Confidence: verified-from-source (DHT detail: known-architecture).

## 3. Hyperspace

- hyperspaceai/hyperspace-node is the unambiguous AI-inference P2P network
  (name collisions exist elsewhere). **Caution**: main repo is README-only
  (~11 commits, no source, no license file) — network claims (2M+ nodes,
  libp2p v3, Pulse protocol) are self-reported. Pods docs
  (pods.hyper.space) do document pipeline-parallel sharding (Qwen 27B
  layers split across H100 + M3 Max) with Raft consensus and NAT traversal.
- Adopt/study: Pulse commit/challenge/verify heartbeat (Merkle liveness
  proofs + strikes) as a cheap peer-attestation primitive; the points/USDC
  economy is a counter-model to open swarms.
- Sources: https://github.com/hyperspaceai/hyperspace-node ·
  https://pods.hyper.space · https://github.com/hyperspaceai .
- Confidence: verified-from-source for docs/README claims;
  claims-not-code-verifiable.

## 4. KwaaiNet

- Rust libp2p node from the Kwaai 501(c)(3): whole-model serving via Ollama
  proxy plus experimental Petals-style block sharding (CPU/CUDA; "Metal
  decode is slower than the CPU path"), intent-based routing planned but
  unimplemented, W3C Verifiable Credential trust tiers, no settlement yet.
- Adopt/study: verifiable-credential trust tiers map onto eligible-host
  ranking.
- Sources: https://github.com/Kwaai-AI-Lab/KwaaiNet · https://www.kwaai.ai .
- Confidence: verified-from-source.

## 5. exo

- `exo-labs/exo` → `exo-explore/exo` (org rename). Apache-2.0, 47.7k★,
  active. Auto discovery, tensor+pipeline parallelism, topology-aware
  auto-parallel, Thunderbolt-5 RDMA; no speculative decoding, no
  verification contract. Linux CPU-only currently.
- Adopt/study: topology-aware partition planning; ModelSwarm is its
  mirror-inverse — we keep whole replicas per peer and parallelize the token
  tree, not the weights.
- Sources: https://github.com/exo-explore/exo · https://exolabs.net .
- Confidence: verified-from-source.

## 6. Pooled

- 1 month old, 567★: layer sharding of one model across browser
  tabs/devices via a custom WGSL/WebGPU engine; hidden state (4–10 KB) hops
  device-to-device per token; host samples; speculative decoding +
  batched prefill in-engine; **golden tests assert the speculative stream
  is bit-identical to plain decoding**.
- Key distinction: peers hold *different* layers (one model copy across the
  swarm); same-profile replicas racing/verifying token trees has no shipped
  equivalent.
- Adopt/study: the lossless "speculative == plain" golden-test contract is
  precisely our correctness harness; WebRTC + TURN-443-without-relaying-
  weights is the right browser transport pattern (not our foundation).
- Sources: https://github.com/Nehanth/pooled · https://pooled.run .
- Confidence: verified-from-source.

## 7. PARALLAX

- Gradient Network's serving engine (github.com/GradientHQ/parallax,
  Apache-2.0, 1.4k★, created 2025-09-22, last push 2026-07-01): Hivemind-DHT
  mesh, layer-sharded pipeline parallelism, modified SGLang (GPU) + MLX
  (Apple), paged KV cache, live public chatbot (qwen3-235b-a22b,
  gpt-oss-120b). Part of an "Open Intelligence Stack" with Lattica.
- DSD and verification mechanics live in the DSD blog + separate VeriLLM
  layer, not in the parallax docs pages.
- Adopt/study: two-level scheduler + self-healing shard routing over DHT is
  the direct architectural cousin of the micro-swarm; the "fastest path:
  single host, LAN, or public internet" routing goal is shared.
- Sources: https://docs.gradient.network/the-open-intelligence-stack/parallax.md ·
  https://github.com/GradientHQ/parallax .
- Confidence: verified-from-source.

## 8. DSD (Gradient blog "Turning Latency into Throughput")

- "We convert network latency from a bottleneck into a throughput asset":
  local draft emits γ tokens; sharded target verifies the window in one
  forward pass; Batch Settlement amortizes sync to one round trip; up to
  2.6× end-to-end (Llama 3.1 8B / Qwen 3 8B, 4–8 nodes, A800/InfiniBand).
- **Critical nuance**: Adaptive Verification is *relaxed for non-key tokens*
  ("accuracy parity", not bit-exact losslessness) — MSP's verified token
  trees need strict verification everywhere to claim losslessness.
  Idempotent commit (DraftID+version) handles network retries; VeriLLM
  cited for cryptographic output trust.
- Adopt/study: batch settlement over the WAN round trip; key-token
  verification ranking. Treat all figures as research evidence, not
  promises.
- Sources: https://gradient.network/blog/turning-latency-into-throughput-speculative-decoding-for-the-decentralized-inference ·
  arXiv:2511.21669 · VeriLLM arXiv:2509.24257 (corrected 2026-10-07: 2511.11733 is a different paper — "Speculative Decoding in Decentralized LLM Inference"; found by the Phase F prior-art audit).
- Confidence: verified-from-source (vendor blog, not peer review).

## 9. FlowSpec

- arXiv:2507.02620 (v1 Jul 2025, v3 Jan 2026): pipeline-parallel
  tree-based speculative decoding for edge clusters — score-based
  step-wise verification, draft management with causal-correctness pruning,
  dynamic draft expansion; 1.37–1.73× on a real testbed. Code:
  github.com/Leosang-lx/FlowSpec (22★, no license file).
- Closest academic match to "verified token trees across peers".
  Losslessness not explicitly claimed in the abstract.
- Adopt/study: verification-order scoring + invalid-branch pruning are
  exactly the scheduling policies our token-tree verifier needs.
- Sources: https://arxiv.org/abs/2507.02620 ·
  https://github.com/Leosang-lx/FlowSpec .
- Confidence: verified-from-source (abstract + repo metadata).

## Bottom line (auditor's synthesis)

No shipped system does what MSP proposes — N replicas of the exact same
immutable model profile cooperatively accelerating ONE request via
cross-peer lossless speculative decoding with verified token trees, plus
fastest-single-host fallback. Systems either (a) shard one copy across
peers (Petals, PARALLAX, Pooled, KwaaiNet sharding, LocalAI workers, exo),
(b) load-balance/replicate whole requests (LocalAI federated, Hyperspace,
KwaaiNet proxy, p2ptokens), or (c) speculate but over a sharded target with
relaxed acceptance (DSD) or in a lab testbed (FlowSpec). Nearest neighbors
to study deeply: DSD, FlowSpec, Pooled (golden contract), PARALLAX/KwaaiNet
(DHT + trust models).
