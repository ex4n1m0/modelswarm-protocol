//! Swarm scheduling: filtering ineligible candidates, probing a bounded
//! shortlist, and scoring peers by predicted completion time using locally
//! measured RTT, queue, prefill/decode rates, reliability, and staleness
//! (`docs/architecture.md` §Scheduler).
//!
//! Phase A: interface freeze only. Implemented in Phase C.
