//! Typed client for the tracker (`modelswarm.deepflux.space`): signed
//! request envelopes, catalog fetching + verification, peer registration and
//! heartbeats, rendezvous signaling, eligibility-lease handling, and job
//! receipts — all per `protocol/msp-v1.md` §2–§5.
//!
//! Phase A: interface freeze only. Implemented in Phase B alongside the
//! tracker endpoints. The hub remains content-blind: this client must never
//! send prompt or completion payloads anywhere (ADR-001).
