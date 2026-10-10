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

---

## Addendum — revision 2 (2026-10-11): both reviews folded into ADR-032

Reviews: `docs/reviews/review-adr-032-testrelease-2026-10-11.md` (commit
`10de665`: B1–B4, R1–R3, A1–A4) and
`docs/reviews/review-adr-032-security-2026-10-11.md` (commit `5ad5215`:
RC-1..RC-8, A-1..A-5). All blockers, required changes, and advisories are
folded into ADR-032 revision 2 (still Proposed; the owner decision follows).

Reconciliation decisions where the reviews diverged or text was not adopted
literally:

1. **B2 mechanism (the one substantive divergence).** T/R offered (a) map to
   existing §6.5 codes or (b) a new code via the ADR-028 §5 amendment
   process; Security A-5 recommended neither — cooperative-namespace
   `CancelRound` reasons (`parent_mismatch | round_mismatch |
   commit_mismatch`) with NO registry change. **Adopted Security's**: fewer
   frozen surfaces touched, and T/R's actual requirement (exact, decided,
   testable strings) is fully satisfied — the strings are decided, as
   CancelRound reasons. Session-stage failure branches (unknown session →
   `bad_frame`; torn-down → `cancelled_by_peer`; lease-expired →
   `expired_token`; budget → `overloaded`) are pinned in the §2 partition
   table.
2. **Field name for the integer verify duration.** Security's literal delta
   said `verify_ms_ms`; the task delegated the name. **Chose
   `verify_pass_ms`** (u32 whole ms): a distinct name from revision 1's
   float `verify_ms` so a stale float-bearing implementation fails
   deserialization instead of silently coexisting. All other RC-2 content
   (integer-only rationale via canonical-JSON float rejection, measured
   cross-check + 25 ms, `verify_ms_lie` telemetry, per-verifier EWMA,
   under-report bound) adopted as written.
3. **T/R A4 vs Security RC-2 on value assertions — complementary, not
   conflicting:** fixture/gate pins assert `verify_pass_ms ≥ 0` only; the
   cross-check bound is coordinator behavior, asserted via telemetry in the
   default-feature test row, never against measured wall times.
4. **Internal fix surfaced by RC-5(c):** revision 1's I3 said "gap-free"
   while allowing abandoned rounds; revision 2 splits hash-chain
   gap-freeness (required) from round-number contiguity (NOT required; the
   shipped `round == state.round()+1` check is explicitly not inherited).

Housekeeping for the Integrator: the §1 cross-reference paragraph in
`protocol/msp-cooperative-v1.md` needs a one-line refresh to match revision
2 — replace "additive `verifier_backend` (SessionOffer) and
`engine {backend, batch_verify}` (SessionAccept) disclosures" with
"additive `verifier_backend` (SessionOffer), `engine {backend,
batch_verify, batch_window_max}` (SessionAccept), and the
`SessionReject {session_id, protocol_version, reason}` shape". Status
unchanged: Proposed; no code, no wire bytes, no gates run (docs-only; this
session is read-only).
