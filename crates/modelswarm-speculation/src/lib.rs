//! Exact speculative decoding algorithms (Phases D–E): proposal windows,
//! the lossless acceptance/rejection rule (greedy token-equality contract;
//! sampled rejection-sampling contract with distribution tests), bounded
//! candidate tries, branch deduplication, and batch verification. Built
//! against the tiny-transformer correctness oracle from `research/`
//! (cooperative plan P1) before touching any distributed path.
//!
//! Approximate/semantic acceptance is out of scope here by ADR-013 — it
//! would require its own ADR after `speculative_exact` is complete.
//!
//! Phase A: interface freeze only.
