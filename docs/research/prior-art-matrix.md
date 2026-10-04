# Prior-Art Matrix — verified 2026-10-04

Owners: Prior-Art Auditor (research) + Integrator (assembly). Sources checked
live on the audit date; stars/commit counts are point-in-time. Confidence
per row: **verified** = read from primary source (repo/docs/paper);
**secondary** = reputable secondary only.

## Matrix

| System | License | Maturity | Discovery | Access/economy | Execution | Single-request cooperative accel? | Output verification | Transport |
|---|---|---|---|---|---|---|---|---|
| **p2ptokens** | MIT | Early (28 commits; v1 coordinator in-memory) | Tracker/coordinator | BitTorrent-style upload/download ratio; newcomer grace; optimistic unchoke; signed co-receipts | Whole-request fan-out (single/racing/quorum/ensemble) to independent peers | **No** — fan-out/redundancy, not cooperative decoding; sharding deferred v2+ | Signed co-receipts ("neither side can lie by more than one chunk") | rust-libp2p (Noise TCP/Yamux), full NAT traversal |
| **LocalAI (P2P mode)** | MIT | Very active (49.4k★); P2P mode "tech preview" | go-libp2p DHT + token; mDNS fallback | Shared token; no economy | Federated (load-balance whole requests) or worker mode (llama.cpp RPC weight sharding; one model; static workers) | **No** | None described | go-libp2p tunnels; gRPC to workers |
| **Petals** | MIT | **Dormant** (last push 2024-09) | Hivemind DHT | Open volunteer swarm; no economy | Layer-pipeline sharding + fine-tuning | **No** | Redundancy-based trust; explicit privacy caveat | Hivemind RPC; WS/HTTP clients |
| **Hyperspace** | **None detected** (README-only repo, no code) | 330★; claims 2M+ nodes unverifiable | Kademlia DHT + GossipSub | Points economy + USDC settlement | Single-node serving; Pods = pipeline sharding (2–10 machines) | **No** | Pulse = Merkle-proof *liveness* attestation (not output correctness) | libp2p v3, relay v2, WS |
| **KwaaiNet** | MIT | Early (26★, active, v0.6.x) | rust-libp2p + Kademlia | Open; W3C VC trust tiers; no settlement | Whole-model (Ollama proxy) or experimental block sharding | **No** | Node trust scores; none for outputs | rust-libp2p: Noise, AutoNAT, relay, DCUtR, UPnP |
| **exo** | Apache-2.0 | Very active (47.7k★) | Zero-config LAN | None (personal cluster) | Tensor+pipeline parallelism across devices | **Qualified** — sharding cooperates on one generation, but by splitting weights, not by speculation across replicas | None described | MLX distributed, TB5 RDMA; Linux CPU today |
| **Pooled** | MIT | Brand-new (Sep 2026; 567★/month) | PeerJS rooms by link | Free; optional local API token | Layer sharding across browser tabs; hidden state hops devices; in-engine speculative decoding | **Qualified yes** — one generation cooperatively computed, but draft+verify live inside the *same sharded pipeline* | **Golden tests assert speculative == plain (bit parity)** — the best correctness-contract precedent found | WebRTC data channels; TURN/TLS-443 (weights never relayed) |
| **PARALLAX** | Apache-2.0 | Young + real (1.4k★; live public chatbot) | Hivemind DHT mesh | Open self-hosted | Layer-sharded pipeline parallelism; SGLang/MLX runtimes; paged KV | **Yes with DSD** (DSD is an engine feature inside Parallax) | "Verified across the mesh" via separate VeriLLM layer | gRPC tensor streaming; works without public IP |
| **DSD (Gradient)** | Blog/engine feature | Vendor-reported (2.6× E2E claims, A800 clusters) | (inside Parallax) | — | Local draft + sharded target; γ-window batch settlement; adaptive key-token verification | **Yes — closest prior art** for turning latency into throughput | **Not strictly lossless**: strict on "key tokens", *relaxed* on non-key tokens | gRPC tensor streaming; InfiniBand testbed |
| **FlowSpec** | Paper (arXiv:2507.02620); code repo no license | Paper v3 2026-01; code 22★ | Testbed (not a discovery system) | Academic | Pipeline-parallel **tree-based** speculative decoding; step-wise score-based verification; causal pruning; dynamic expansion | **Yes** (edge testbed, 1.37–1.73×) | Step-wise verification; distribution-losslessness not explicitly claimed | Testbed; reference code exists |

Confidence: all rows **verified** from primary sources on 2026-10-04 except:
Petals DHT detail (known-architecture/secondary), Hyperspace network claims
(README-only, no code to verify), DSD figures (vendor blog, not peer review).

## What this changes/confirms for ModelSwarm (ADR-009 evidence)

1. **The thesis holds**: no shipped system does *same-profile replicas as
   parallel speculators + strictly lossless cross-peer verification +
   product-grade fastest-single fallback*. Every system either (a) shards
   one copy across peers, (b) replicates/fan-outs whole requests, or (c)
   speculates over a *sharded* target with relaxed acceptance (DSD) or in a
   lab testbed (FlowSpec).
2. **Strict losslessness is our differentiator and a real design constraint**:
   DSD explicitly relaxes non-key-token acceptance. Our `speculative_exact`
   must NOT borrow that shortcut (ADR-013); Pooled's "speculative == plain"
   bit-parity golden tests are the correctness-contract precedent to copy.
3. Adopt/study shortlist: DSD batch settlement + key-token ranking; FlowSpec
   tree-verification scheduling and pruning; Pooled golden-test contract;
   PARALLAX/KwaaiNet DHT + trust-tier patterns; p2ptokens co-receipts and
   newcomer grace (economy itself rejected — ADR-009).
4. Warnings absorbed: Petals' dormancy (volunteer-only economics fade);
   Hyperspace's unverifiable scale claims (we will not assume unproven
   numbers as engineering assumptions); LocalAI's static-worker limitation
   (our sessions must survive peer join/leave/replace mid-round).

Full per-system notes with source URLs preserved in
`docs/research/prior-art-matrix-notes.md` (appendix).
