# ADR-012: EligibilityLease (renames and extends capability tokens)

Status: Accepted (Phase A, 2026-10-04) · Amends ADR-006; renames the
artifact msp-v1 §5 calls "capability token" to "eligibility lease". Field
changes are frozen here; msp-v1 §5 is amended in Phase B with the v2
contract document.

## Context

The revision requires: periodic hosting verification, at least one real
serving slot, degradation/suspension on refusal or bad output, an optional
bootstrap allowance for new peers, and audit epochs for revocation sweeps.

## Decision

```text
EligibilityLease {
  peer_id, installation_id, model_profile_id (msp1:… derived, ADR-011),
  issued_at, expires_at,          // still capped: lease TTL + 60 s grace
  can_host, can_consume, slots,
  verified_capacity: CapacityClass,   // new: measured, not self-reported
  audit_epoch: u64,                   // new: revocation/audit sweep counter
  challenge_id, nonce,
  issuer_signature
}
CapacityClass ∈ { cpu, gpu_entry (≤8 GB), gpu_mid (8–24 GB), gpu_high (>24 GB) }
```

- Multi-use within lifetime; per-request replay protection stays the
  single-use `requestId` (unchanged from the Phase 0 reconciliation).
- `verified_capacity` is assigned from challenge timings + hardware
  evidence the node already submits; self-reported hardware never raises it.
- `audit_epoch` increments on each audit sweep; leases referencing an epoch
  older than the tracker's current one for that peer are rejected without a
  per-token revocation lookup (cheap revocation propagation alongside the
  existing heartbeat `notices`).
- **Bootstrap allowance: OFF by default.** New peers get no consumption
  grace in the product; an allowance exists only as an experiments-scoped
  policy flag until Phase F evidence justifies promoting it (conflict with
  the original plan's no-economy non-goal is resolved in favor of the
  non-goal until then).
- Suspension: repeated challenge refusal/timeout/invalid output reduces
  slots to zero for a backoff window (deterministic policy table, Phase B
  test vectors).

## Consequences

+ Renaming aligns docs/code with the governing plan; semantics stay the ones
  already reviewed (lease-capped TTL, peer/profile binding).
+ Audit epochs make revocation O(1) for serving peers.
− One more freeze-breaking rename before any code depends on the old name
  (the right time for it).
