# Phase C Acceptance Tests — Single & Hedged Modes

Gate (revision): "Hedging improves selected tail-latency workloads without
duplicate output or leaked capacity." Plus the honest-telemetry deliverable.
Evidence → `docs/verification/phase-c.md`.

## Runtime

- **C1** `InferenceRuntime` trait compiles against two adapters
  (LlamaCppAdapter; MockRuntime under the off-by-default `mock-runtime`
  feature — plain `cargo check` must not see it).
- **C2** MockRuntime determinism: same seed ⇒ identical token streams
  (100 cases); draft_accuracy 0/1.0 knob behavior (200 rounds).
- **C3** LlamaCppAdapter maps a mock llama.cpp server's responses/latencies/
  errors correctly, including timeout paths.

## Scheduler

- **C4** Cost model v2 pure function over documented inputs; engage rule
  flips at break-even; margin floor 0.05 enforced (`MarginTooLow`).
- **C5** Measured beats advertised: a fast-advertising/slow-measured peer
  ranks below the reverse.
- **C6** Micro-swarm selection: exact-profile filter, bounded ≤ 8,
  deterministic tie-breaks.

## Gateway

- **C7** OpenAI compatibility: `/v1/models`, `/v1/chat/completions` stream
  (exact SSE chunk shapes + `[DONE]`) and non-stream; clamps applied not
  rejected, echoed via `x-msp-clamped`.
- **C8** ADR-007 semantics: retry before first token (bounded 2), explicit
  `interrupted` after first token, no fabricated continuation.
- **C9** Prompt-size cap (32 KiB) → 400; no message content in any log line
  (captured-log assertion).

## Transport (ADR-018 backend) & simulator

- **C10** Loopback-only guard: binding non-loopback refused (tested).
- **C11** Handshake signature verified; tampered handshake rejected; frames
  > 256 KiB rejected pre-allocation; deadlines produce Timeout + close.
- **C12** RTT measurement over loopback yields sane stats; 4 concurrent
  sessions on one listener; cancel delivered mid-stream.
- **C13** Sim scenarios (`pair`, `mesh n`, `kill at_ms`) emit one-JSON-per-
  line machine output; `kill` produces an explicit failure event (no hang).

## Bench harness (frozen → first executable runs)

- **C14** Records validate against `experiments/schemas/` (structural
  validation in-test); determinism same-seed byte-identical; bootstrap CI
  contains the median; every mock-backend record labeled TEST-ONLY.
- **C15** `single` comparator machinery present: fastest-of-N recorded as
  prediction + actual.

## Honest boundary

Everything here runs on loopback with the MockRuntime or a mock llama.cpp
server. **No real-model performance claim is made or implied.** Real-GGUF
execution + the three-machine matrix remain gated on the approved 4B
profile (stop-condition honored). Hedging tail-latency evidence at this
phase is structural (event ordering, single-winner, capacity release), not
a wall-clock claim.
