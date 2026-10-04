//! Benchmark harness (spec: `docs/research/bench-harness-spec.md`): runs
//! the fastest-single baseline and cooperative modes under the frozen
//! network matrix, emits machine-readable records conforming to
//! `experiments/schemas/`, and never computes claims by hand. Negative
//! results are recorded with the same fidelity as wins (ADR-013).
//!
//! Phase A: interface freeze only; the harness can only *run* from Phase C,
//! when transport + runtime exist. Phase A ships the spec + schemas.
