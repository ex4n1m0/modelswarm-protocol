//! Cooperative session state machine (Phase D, `/msp/cooperative/1.0.0`):
//! session open/close, round sequencing, prefix-hash-linked commits,
//! rollback to the last committed prefix, idempotent handlers (duplicate
//! commits never advance state twice), straggler drops, and peer
//! replacement mid-session without corrupting the accepted sequence.
//!
//! Phase A: interface freeze only. Property gates (stale/duplicated/
//! reordered/cross-profile messages cannot advance state) are the Phase B
//! exit gate per ADR-015.
