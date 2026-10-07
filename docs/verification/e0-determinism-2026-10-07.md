# E0 — llama.cpp greedy determinism across backends (2026-10-07)

Question (from Phase F research round 2): can a second same-profile peer
spot-check a remote peer's output by re-executing at temperature 0 and
comparing tokens? Valid only if greedy decoding is deterministic across
the engines peers actually run.

## Setup

Pinned b11407 engines (canonical CPU + Vulkan variant), RTX 5080 Laptop
machine, fixed prompt, `temperature: 0, seed: 7, max_tokens: 48`,
`/v1/completions`. Compared sha256 of the generated text.

## Results

| Config | sha256 (16) | Verdict |
|---|---|---|
| SmolLM2-135M · CPU · default threads | `10becdcaf86447ae` | |
| SmolLM2-135M · CPU · `-t 1` | `10becdcaf86447ae` | = CPU default |
| SmolLM2-135M · CPU · `-t 8` | `10becdcaf86447ae` | = CPU default |
| SmolLM2-135M · Vulkan · RTX 5080 | `1d61647790480260` | ≠ CPU |
| SmolLM2-135M · Vulkan · Intel iGPU (`--device Vulkan1`) | `1d61647790480260` | = Vulkan/5080 |
| Qwen3.8-27B · CPU | `78c4aca35e7f0130` | |
| Qwen3.8-27B · Vulkan | `4b3ba25bc8f65523` | ≠ CPU (same opening, diverges mid-stream) |

## Findings

1. **CPU is deterministic across thread counts** (1/8/default identical).
2. **Vulkan is deterministic ACROSS GPU VENDORS** — NVIDIA 5080 and Intel
   iGPU produce byte-identical greedy streams. The Vulkan kernels' math
   order is hardware-independent.
3. **CPU ≠ Vulkan** — quantized kernel arithmetic differs per backend;
   divergence starts early (135M) or mid-stream (27B).
4. **Untested:** cross-machine CPU with different microarch kernels
   (zen4 vs alderlake dispatch) — needs a second machine; plausible it
   also diverges. Logged as the open sub-question.

## Consequence for F3 verification design

Re-execution spot-checks are sound **within a backend family**, not
across. The verifier must run the same backend as the prover — and the
backend is already disclosed: the engine handshake/lease must carry it
(`backend: cpu|vulkan`, exists in engine telemetry since ADR-024).
Concretely: pair CPU-verifiers with CPU-provers and Vulkan-verifiers with
Vulkan-provers; either family spans GPU vendors (finding 2) and thread
counts (finding 1). Cross-backend spot-checks must use logprob-distance
or activation-hash comparison instead (TopLoc tier) — a follow-up
experiment, not needed for the v1 verification tier.

This also REINFORCES ADR-024's forward constraint: backend must be part
of eligibility when cooperative modes begin.
