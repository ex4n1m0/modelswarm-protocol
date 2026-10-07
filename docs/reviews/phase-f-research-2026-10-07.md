# Phase F research round 2 — prior art × internal seams → remaining actions

2026-10-07. Inputs: (1) external prior-art audit (Petals, exo, Pooled,
libp2p/ProbeLab, Tail-at-Scale, vLLM router, BitTorrent/BOINC/Helium/LES,
TopLoc/VeriLLM/SVIP, DERP/TURN economics — full citations in the auditor's
report below); (2) internal seam audit of the repo. Owner direction
confirmed: fastest-eligible-peer-per-request micro-swarms,
contribute-to-consume, thin clients deferred.

## Internal seam audit — more exists than the scoping doc assumed

| Piece | Status |
|---|---|
| Wire vocabulary | ✅ complete: Handshake/Ack, InferenceRequest, TokenDelta, Usage, StreamError, Cancelled, Completed, Cancel, Control (`modelswarm-transport/src/message.rs`) |
| Framed sessions + QUIC round-trip | ✅ exists (F11 driver-task fix in) |
| Loopback guard | ⚠️ one `if !addr.ip().is_loopback()` at `libp2p_backend.rs:206` — F0 is a flag, not a rebuild |
| Hub-signed leases | ✅ `EligibilityLease` issue/verify + tracker returns `lease: base64url(json).base64url(sig)` |
| Suspension/reputation | ✅ `policy.rs` violations, `suspension()`, `backoff_for_streak()` — unwired |
| Fastest-single comparator | ✅ `scheduler::select_microswarm` + `predicted_single_ms` (staleness penalties in) |
| Gateway retry semantics | ✅ `run_request`: retry only before first token, `interrupted` after (ADR-007) |
| **Serving-peer bridge** | ❌ **THE gap**: nothing maps inbound `InferenceRequest` frames → the local `InferenceExecutor` → outbound `TokenDelta` frames |
| RemoteExecutor (requester side) | ❌ gateway only has `SingleLocalExecutor` |
| Handshake auth binding | ◐ Handshake exists; ADR-020 peerId ↔ libp2p PeerId binding undecided |
| Real address advertisement | ❌ heartbeat still carries the honest `/ip4/0.0.0.0/tcp/0` placeholder |

## What prior art proved (short form — full audit appended)

1. **NAT reality**: hole punching succeeds ~60-70% at IPFS scale and worse
   for residential pairs → plan for 30-50% of internet pairs relay-dependent.
   Petals needed Circuit Relay for ~29% of servers even in a curated swarm.
2. **Relay v2 defaults are signaling-grade**: 128 reservations, 16
   circuits/peer, 2-min/128-KB per-circuit caps — a stock relay RESETS a
   token stream mid-flight. Our relay must raise limits; mid-stream circuit
   resets need explicit semantics vs ADR-007's no-retry-after-first-token.
3. **Relay economics are a non-problem on self-hosted hardware**: a token
   stream is ~0.3 KB/s; 1,000 concurrent streams ≈ 2 TB/month → ~zero on a
   €40-60/mo unmetered 1 Gbps box. Connection count, not GB, is the cost.
   Managed TURN would cost $75-800/mo — don't.
4. **Selection mechanism is proven, payoff is not**: hedged requests
   (delayed duplicate at ~95th percentile, ~5% extra load), EWMA/P2C,
   requester-measured values beating self-reports — all standard. But
   nobody has published remote-vs-fastest-single comparisons; MSP produces
   the first numbers and must publish them negative-or-not.
5. **Incentives**: tit-for-tat is exploitable (BitTyrant), unbacked credits
   get gamed (BOINC), money attracts pros (Helium), zero-incentive serving
   collapses (LES). MSP's contribute-to-consume with verifiable exact-
   profile hosting avoids all four preconditions. Open hole:
   **proof-of-hosting ≠ proof-of-service** (availability gaming) → F3
   adversarial tests + signed co-receipts. Thin clients stay gated.
6. **Verification**: a practical tier exists — temperature-0 re-execution
   spot-checks by a second same-profile peer (~1% cost, VeriLLM) +
   activation-hash identity proofs (TopLoc, deployed in INTELLECT-2).
   Open experiment: llama.cpp/Vulkan quantized greedy determinism across
   vendors — must be tested before text-equality spot-checks are sound.

## Remaining actions (ordered, sized)

- **E0 — determinism experiment (S, can run TODAY, no transport needed):**
  greedy temperature-0 completions, same profile, CPU vs CPU cross-machine
  and CPU vs Vulkan — logprob/text equality matrix. Decides F3's
  verification tier. Uses existing artifacts + the real-model-e2e harness.
- **F0 — dialable listener + serving bridge (M):** flag the loopback guard
  open (`MSP_LISTENER=1`); advertise the real multiaddr in heartbeats;
  build the frame↔executor bridge; bind ADR-020 peerId at handshake; lease
  check at session open. Exit: two machines on one LAN, cross-machine
  completion, census-verified.
- **F1 — RemoteExecutor + selection (M):** gateway executor over transport
  sessions; `select_microswarm` fed by requester-measured EWMA (TTFT
  percentile signals); relayed path = scheduler cost class; fastest-single
  honesty report as a built-in benchmark. Exit: A consumes B over LAN with
  honest fallback; negative results published.
- **F2 — NAT/relay (M + one owner decision):** rust-libp2p relay with
  custom limits (duration/data caps raised, reservations ≥1024), DCUtR
  hole-punch, measure + publish the direct-vs-relay split. Owner decision:
  one €40-60/mo unmetered VM (relay + future bootstrap).
- **F3 — hostile hardening (M-L):** availability-gaming and receipt-
  forgery adversarial tests, malformed-frame fuzzing, replay nonces,
  wire suspension hooks; verification tier per E0's outcome. Security
  Engineer leads; release gate.
- **Deferred by evidence:** thin-client mode (attack magnet per every
  audited system); cooperative track (unlocks after F1 proven).

## Corrections from this round

- `docs/research/prior-art-matrix-notes.md` §8: VeriLLM is arXiv
  2509.24257, not 2511.11733 (fixed 2026-10-07).

---

## Appendix — Prior-art audit (full report)

Five areas, each with systems + links, proven/disproved numbers, the
single most important lesson, and MSP's differentiation gap.

### 1. P2P serving + NAT
Petals ([arXiv:2209.01188](https://arxiv.org/abs/2209.01188)): 14-server
internet swarm, BLOOM-176B 0.83 steps/s @seq128; 1.24→0.57 steps/s as RTT
hit 100 ms; 4/14 servers needed Circuit Relay; slow-peer handling =
client re-route + KV-state replay; incentives proposed, never shipped.
exo: gRPC, no NAT story — LAN-only by design. Pooled: WebRTC data
channels + TURN/TLS-443 works for browser inference. ProbeLab (4.4M
measurements): DCUtR+QUIC punch ~60% at IPFS scale, 70%±7.1% academically;
relayed paths ≈ 1.4×+ direct RTT for many peers. Circuit Relay v2
defaults (go-libp2p docs): 128 reservations, 16 circuits/peer, 1 h TTL,
2 min/128 KB per circuit — signaling-grade. rust-libp2p implements relay
server + client + DCUtR. Lesson: relay-first is a deployment pattern;
ADR-014's relayed-as-cost-class design is validated. Differentiation: MSP
relays request+token stream (RTT once per token), not per-layer tensors;
nobody ships exact-profile micro-swarms with relay-vs-fastest-single
honest benchmarks.

### 2. Fastest-peer selection at request time
Tail at Scale (Dean/Barroso CACM 2013): hedged requests — delayed second
copy at ~95th-percentile expected latency, big p99.9 wins at ~5% extra
load; the delay threshold is the trick. vLLM production-stack: TTFT/TBT
exported as observability, not yet a scheduling signal. P2C/EWMA:
production standard (HAProxy/Envoy class). Petals: RTT beam search,
mid-session re-routing with state replay. Lesson: local measurement beats
advertisement; prioritize TTFT-percentile and queue-misprediction
signals. Differentiation: identical-profile replicas make hedged
requests safe to compare and spot-check; fastest-single-fallback honesty
(including when local wins) is unclaimed by anyone.

### 3. Contribute-to-consume / thin clients
BitTyrant (NSDI 2007): tit-for-tat exploitable. BOINC: credits stable
only when backed by replicated validated work; benchmark credits gamed.
Helium: proof-of-coverage spoofed at scale; money attracts professional
attackers. Ethereum LES: zero-incentive serving → supply collapse.
p2ptokens: ratio economy + grace + optimistic unchoke + signed
co-receipts — closest working accounting. Lesson: minimum viable anti-
freeloading = verifiable work + signed bilateral receipts + grace; MSP's
eligibility-as-the-rationed-good with verifiable hosting raises Sybil
cost to "hosting the model per fake identity". Open hole:
proof-of-hosting ≠ proof-of-service → F3 tests. Thin clients: every
system that allowed consume-without-contribute collapsed, got gamed, or
needed payments (ruled out v0.x) — correctly deferred.

### 4. Adversarial verification of remote inference
TopLoc (ICML 2025, deployed in INTELLECT-2): activation LSH proofs, 258
B/32 tokens, robust to GPU nondeterminism, sensitive to quantization
changes — exactly MSP's pinning property. VeriLLM
([arXiv:2509.24257](https://arxiv.org/abs/2509.24257)): partial
re-execution ≈ 1% of inference cost. SVIP
([arXiv:2410.22307](https://arxiv.org/abs/2410.22307)): <3% FP/<5% FN
per-prompt detectors, needs per-model training. zkML: impractical at
LLM scale everywhere surveyed. Lesson: practical tier = greedy/temperature-0
re-execution spot-checks by a second same-profile peer + activation-hash
identity samples; sampling-correctness verification stays open.
Differentiation: same-quantization replica swarms remove the
model-mismatch confound. Unverified hypothesis: llama.cpp/Vulkan
cross-vendor greedy determinism → E0 experiment before any correctness
claim.

### 5. Relay economics + deployment
Tailscale DERP: company-run free relays; `derper` self-hostable.
Managed TURN: Cloudflare $0.05/GB past 1 TB free; Twilio $0.40-0.80/GB.
Self-host: unmetered 1 Gbps dedicated ~€39-59/mo (verify at order).
Arithmetic (ours): 0.3 KB/s per stream; 1,000 concurrent streams ≈
1.5-2 TB/mo → ~zero on unmetered hardware; $75-100/mo Cloudflare;
$600-800/mo Twilio. Cost driver is connection count (reservations,
per-circuit memory, handshake CPU), not bytes. Lesson: own cheap VM with
custom limits is economically correct (as ADR-014 forces);
budget circuits, not gigabytes.

### Ranked riskiest Phase F assumptions
1. Direct-connect suffices — FALSE at scale; plan 30-50% relay-dependent;
   F2 exit must measure and publish the direct-vs-relay split.
2. Stock relay carries streams — FALSE; defaults reset streams at
   2 min/128 KB; custom limits + mid-stream reset semantics are real work
   (and collide with ADR-007 rule 6).
3. Fastest-peer beats local often enough — mechanism proven, payoff
   unproven anywhere; MSP publishes the first numbers, possibly negative.
4. Contribute-to-consume is game-resistant — strong by prior art, but
   availability-gaming + receipt forgery need explicit F3 adversarial
   tests; thin-client stays gated.
5. Cheap verification — practical tier exists, gated on the llama.cpp
   determinism experiment (E0).
