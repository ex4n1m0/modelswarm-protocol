# ADR-019: Runtime adapters (mock / llama.cpp / research) and benchmark honesty labels

Status: Accepted (Phase B, 2026-10-05)

## Context

Phase C needs an executable runtime against `docs/research/runtime-trait-spec.md`
before any real GGUF artifact is approved (catalog still empty; large-model
downloads are a stop-condition without owner approval). Phase D needs
proposal/verification hooks llama.cpp does not expose. Benchmarks run in
Phases C–E must therefore be explicit about what backend produced them.

## Decision

Three adapters behind one trait, selected by profile/environment:

1. **MockRuntime (test-only)**: deterministic toy decoder (integer token
   ids, small vocab, seeded next-token function) with a configurable draft
   accuracy knob for acceptance-rate experiments. **compile-gated by a
   `mock-runtime` feature; the node binary never enables it; any benchmark
   record produced against MockRuntime MUST carry
   `runtime.name = "mock"` and is excluded from any product claim** (the
   run-manifest schema already pins runtime identity).
2. **LlamaCppAdapter (production baseline)**: HTTP client to the pinned,
   loopback-bound llama.cpp sidecar (ADR-002). Download/install of the
   pinned build is implemented behind explicit user action; nothing
   auto-downloads.
3. **ResearchAdapter (Phase D entry)**: the vLLM-vs-fork decision from the
   runtime trait spec's gap map; requires its own ADR before
   implementation; the "large unsafe fork" stop condition applies.

Benchmark honesty: the harness emits records per `experiments/schemas/`;
records with `runtime.name = "mock"` are labeled TEST-ONLY everywhere they
appear (report headers included). Synthetic acceptance exists only inside
MockRuntime-driven tests, never in any user-serving path (revision rule).

## Consequences

+ Phases C–E are fully executable and testable today, honestly labeled.
+ No stop-condition is violated: no model weights, no unaudited runtimes.
− All mock-backend numbers are structural evidence only; the first real
  performance claims wait for the approved 4B profile on real hardware.
