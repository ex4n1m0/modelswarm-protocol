# ADR-021: Research runtime for real speculative-decoding measurements (Phase D entry)

Status: **Accepted for the beta engine (Phase H, 2026-10-05); research path
partially decided** (in-process C-API research adapter: ADR-031). The owner's Phase H direction (a real small model shipped
to test machines) closes the immediate-runtime question: the pinned,
**unforked** llama.cpp `llama-server` (CPU x64, `runtime-pins.json`) behind
the existing HTTP `LlamaCppAdapter` is the beta/production engine — this is
Option B's salvageable subset plus the stock server, with **no fork**:
`propose()` is implemented as greedy self-continuation through the standard
completion API, and the first real profile (Qwen2.5-0.5B-Instruct Q4_K_M,
`msp1:eb0a0d21…d8120c`) is registered and active. Option A (vLLM, CUDA)
remains the research-grade path for performance-envelope experiments once a
CUDA box exists; Option C is no longer the immediate next step (the beta uses
a real model instead). Option B as a *fork* stays rejected. **No performance
claims exist yet**: every number stays TEST-ONLY/labeled until Option A (or
measured Option-B-subset records) lands with reproducible experiment records
(ADR-013). Historical proposal below.

Context: ADR-019 §3 (research adapter needs its own ADR),
`docs/research/runtime-trait-spec.md` gap map, D8/E9 open posture.

## The decision to make

Every correctness property is proven on MockRuntime (greedy token-equality,
distribution preservation, adversarial immunity). The remaining claim —
"cooperative speculation beats the fastest eligible single host on real
hardware in some envelope" — requires a runtime that exposes real logits
plus proposal/verification hooks. Options with their true costs:

### Option A — pinned vLLM research adapter (recommended for measurement quality, requires Linux+CUDA)

- vLLM exposes speculative decoding internals and acceptance metrics
  (the prior-art matrix's metrics oracle); a pinned container on a Linux
  GPU box (or WSL2 with GPU passthrough) speaks to the existing
  `InferenceRuntime` trait via an HTTP/Python shim.
- Pros: real distributions, real batch verification, directly comparable
  acceptance telemetry (vLLM convention already adopted).
- Cons: not Windows-native (the product platform); adds a Python service
  to the research path; requires a CUDA machine the project does not yet
  have identified.

### Option B — llama.cpp fork with proposal/verify endpoints (rejected as primary)

- Meets the "large unsafe fork" stop condition head-on: llama.cpp has no
  external proposer/verifier hooks; maintaining a fork across upstream
  churn is exactly the maintenance trap the plans forbid.
- Salvageable subset (fallback if A is impossible): llama.cpp already
  supports *self*-speculative decoding (`--model-draft`) server-side; an
  adapter can measure **single-host** speculation baselines (useful
  comparator data for the cost model) without any fork — but it cannot
  exercise cross-peer proposals.

### Option C — deterministic tiny-transformer research engine (recommended as the immediate next step, no hardware needed)

- Implement a ~1–5M-parameter decoder-only transformer (pure Rust or
  ONNX/ggml-pinned) per the cooperative plan's P1 lab: real attention, KV
  cache, real distributions, small enough to run on CPU everywhere.
- Feeds every existing test seam: real logits for `verify_sampled_full_q`
  E2E, real prefill/decode costs for cost-model calibration, real draft
  divergence — while remaining fully offline, license-clean (train on
  synthetic data), and reproducible in CI.
- Cons: performance numbers stay **TEST-ONLY-labeled "tiny-model"** —
  valid for algorithm calibration, NOT product-speed claims.

## Recommendation (staged)

1. **Now (no owner resources needed): Option C** — build the
   tiny-transformer engine behind the existing trait; upgrade D/E suites
   from MockRuntime to it (real distributions everywhere, still honest
   labels). Also add Option B's *unforked* self-speculative adapter for
   baseline comparators once a GGUF profile is approved.
2. **When the owner identifies a Linux+CUDA box (or approves WSL2+GPU):
   Option A** for the actual performance-envelope experiments that can
   support product claims.
3. Option B as primary is rejected per the stop condition.

## What this ADR needs from the owner

- Is a CUDA-capable Linux machine/VM available for Option A? (Or should
  WSL2 GPU passthrough on the current machine be investigated?)
- Approval to spend effort on Option C now (estimated: one bounded
  engineering phase, analogous to a Phase D-extension).

Until decided: no real-model performance claim path exists (current
honest posture continues).
