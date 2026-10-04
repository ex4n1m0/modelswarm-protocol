# ADR-002: Bundled, pinned llama.cpp server as an inference sidecar

Status: Accepted (Phase 0)

## Context

MSP needs local GGUF inference with prefill/decode metrics and an
OpenAI-compatible surface. Writing inference kernels in Rust duplicates a
mature project; binding llama.cpp statically complicates the Windows build.

## Decision

The node spawns a **bundled, version-pinned `llama.cpp` server** release as a
child process: bound to `127.0.0.1` on a random port, authenticated with a
random per-launch bearer secret, health-checked, gracefully stopped, restarted
with exponential backoff. The pinned build is acquired/bundled with checksum
verification (Phase 2). Runtime compatibility is expressed per profile as
`runtime {name: "llama.cpp", minBuild: <pinned>}`; a node may host a profile
only with a runtime meeting the pinned build.

## Consequences

+ Battle-tested inference; metrics endpoint for prefill/decode rates.
+ Clean process boundary; crash of the runtime cannot corrupt the node.
+ GGUF ecosystem compatibility for free.
− Two-process lifecycle to supervise (handled in `ms-runtime`).
− Version coupling: runtime build is part of profile identity.

## Alternatives rejected

- **Rust-native inference (mistral.rs / candle)**: too much build risk for v0.1.
- **Static linking**: build complexity; sidecar isolates crashes anyway.
- **Ollama as runtime**: adds its own service/model management layer and
  weaker control over pinning.
