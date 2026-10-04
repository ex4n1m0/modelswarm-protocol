# Runtime Trait Specification (Phase A design; implemented Phase C)

Owner: Runtime Engineer · Crate: `modelswarm-runtime` · Status: draft for
Phase B/C implementation, ADR-002 (sidecar) and ADR-011 (identity) context.

## Purpose

One trait through which the scheduler, gateway, and speculation crates use
any inference backend (pinned llama.cpp first; a research adapter for
cooperative hooks later). The trait must never depend on tracker APIs.

## Trait sketch (semantics frozen; exact Rust signatures settled in Phase C)

| Operation | Contract |
|---|---|
| `load(profile) -> Handle` | Verify artifact digests (ADR-011) before load; refuse on mismatch |
| `tokenize(text) / detokenize(ids)` | Deterministic, idempotent; tokenizer hash is part of profile identity |
| `prefill(handle, token_ids) -> KVCommitment` | Commitment = hash of KV-state digest + token span; used by cooperative prefill checks |
| `decode_step(handle, ...) -> Token` | Ordinary autoregressive step (single-mode baseline) |
| `apply_chat_template(messages)` | Template hash is part of profile identity; mismatch = different profile |
| `propose(handle, prefix, window, branch) -> CandidateBlock` | Speculative proposals (research runtime; gated by `speculative_capabilities`) |
| `verify_batch(handle, prefix, candidates) -> Acceptance` | One target pass over a window/tree; exact lossless acceptance rule (ADR-013) |
| `rollback(handle, committed_prefix)` | Discard speculative KV beyond committed prefix; verify via KVCommitment |
| `metrics(handle) -> RuntimeMetrics` | Prefill/decode rates, queue depth, KV usage |
| `cancel(task_id)` | Cancellation must free the serving slot within a bounded deadline |

## llama.cpp capability map (baseline adapter)

| Trait capability | llama.cpp server (pinned build) | Gap |
|---|---|---|
| load/tokenize/prefill/decode/metrics/cancel | Native endpoints | None |
| apply_chat_template | Client-side template application; template hash pinned in manifest | Discipline, not code |
| proposal/verification hooks | **Not exposed** | Research adapter required (Phase D entry gate; ADR on vLLM-pinned vs fork; stop condition: large unsafe fork) |
| KV commitments | Approximate via token-span digests; exact KV introspection not exposed | Documented approximation for v0.1; exactness needed only for cooperative mode |

## Non-goals

No GPU management, no model redistribution, no arbitrary HF repo execution
(security requirement), no native pointers over the wire (ADR-013/revision).
