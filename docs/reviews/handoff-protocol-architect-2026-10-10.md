# Handoff — Protocol Architect — 2026-10-10 (ADR-032 draft)

## Changed files and why

- `docs/adr/ADR-032-batched-speculative-verification.md` (new, status
  **Proposed — NOT accepted**): the batched speculative-verification
  decision record the Integrator routed after the 9.6 pass-2 published
  negative. Specifies the `VerifyDrafts`/`VerifyDraftsResult` family
  (one engine pass per draft window) for the msp-cooperative-v1
  namespace, binding/replay/admission rules mapped onto the effective
  §6.5 registry, E0/ADR-024 backend pairing via additive
  SessionOffer/SessionAccept fields, the engine requirement boundary
  (adapter peers can batch-verify; HTTP peers are first-class singles,
  never verifiers), options A/B/C with **B recommended** (A's wire-true
  scheduler term landing first as the standing guard), non-goals held,
  enumerated Security attack surfaces and Test/Release gate tests, and
  the reviewer-facing verifier round state machine.
- `protocol/msp-cooperative-v1.md`: one cross-reference paragraph at the
  end of §1 recording the Proposed dependency; the frozen §2 field set
  is untouched.

## Delivery note

The drafting session ran in the architect's read-only ADR-proposal
posture (no file-write/shell tooling), so the ADR text, cross-reference,
and this handoff were delivered paste-ready and committed verbatim by
the Integrator. ADR-032 was verified as the next free number (highest
issued is ADR-031; ADR-016 is the historical never-issued skip; no
"ADR-032" existed under docs/).

## Commands run (by the committing Integrator session)

Documentation-only change (no code paths touched):
- `cargo fmt --check` — clean
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo test --workspace` — green (counts in the commit message)

## Test evidence

No new tests: the ADR is Proposed documentation; the gate tests it
specifies (§7) become obligations only upon acceptance. The workspace
suite above proves nothing regressed.

## Assumptions

1. The cooperative prerequisite gate is treated as met per the
   owner-approved post-gate ordering (ADR-027 dependency graph; the 9.6
   passes were sanctioned); this ADR adds no implementation.
2. SessionOffer/SessionAccept additive evolution via ADR is permitted by
   the namespace's own freeze terms; msp-v1 and protocol/messages.proto
   stay byte-identical (verified: nothing in this change touches them).
3. No second implementation currently reads the namespace — golden
   vectors land before one does (gate test §7.1).

## Unresolved risks

- Hard dependency on ADR-031's productionization gate (shape-stable
  batching, spawn_blocking, KV RAM accounting, Vulkan in-process test,
  CI gate) — until it passes, ADR-032 authorizes nothing runnable.
- Cross-machine CPU determinism (E0 open item) still gates any
  real-draft pass 3.
- The companion scheduler term correction (wire-true verification term;
  1.5-step batch term only for `batch_verify`-declared peers) crosses
  into Scheduler Scientist ownership and needs joint Scheduler +
  Protocol sign-off; it is option A and should land regardless of B.
- Linear verify only — multi-proposer upside is bounded until a future
  tree ADR.

## Suggested next task for the integrator

Route review: Security Engineer and Test and Release Engineer as
blocking reviewers; Runtime Engineer and Scheduler Scientist as
consultees (§4 engine boundary and cost-model correction respectively).
After reviews, bring the accept/reject decision to the owner with the
three-line recommendation: adopt B (VerifyDrafts family in the additive
cooperative namespace), land A's engine-true scheduler term first as the
standing guard, keep msp-v1 byte-identical.
