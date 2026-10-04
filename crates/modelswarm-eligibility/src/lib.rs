//! Eligibility: the host-to-consume rule's artifacts (ADR-012).
//!
//! `EligibilityLease` (lease-capped TTL, peer/profile binding, multi-use
//! with single-use request ids), `CapacityClass` assignment from challenge
//! evidence, audit-epoch cheap revocation, and the deterministic suspension
//! policy table. Bootstrap allowance is OFF by default and exists only as an
//! experiments-scoped flag until Phase F evidence.
//!
//! Phase A: interface freeze only. Implemented in Phase B with test vectors.
