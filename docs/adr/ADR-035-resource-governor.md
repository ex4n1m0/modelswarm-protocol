# ADR-035: The M2 resource governor — the enforced 70% ceiling and the winjob enforcement seam

Status: **Accepted (2026-10-13) — implemented with this ADR in
`crates/modelswarm-resource` (M2 opens with its governor ADR per the
master roadmap; gate D11 is the deciding record).** Owner: Resource
Engineer. Required reviews: Runtime Engineer, Windows Product Engineer,
Test and Release Engineer (ownership map; the winjob additions are a
Windows-Product-owned crate changed minimally under the D19
seam-integration authority — flagged for that review explicitly).

## Context

Gate D11 (architecture-gate-2026-10-09.md): "70% ceiling reconciled at
M2 — the recommendation engine may advise up to its own bar; the runtime
governor enforces 70%." Nothing enforced that number until now:

- v0.2.23 turned the family tier ON by default, so the verify engine's
  copy of the weights coexists with the serving sidecar. The desktop
  gained an ADVISORY verdict (`family_resource_verdict`,
  `crates/modelswarm-desktop/src/app.rs`): one platform sample at hosting
  start, weights-only projection against 70% of system RAM (and VRAM
  when the serving backend is vulkan), unobservable inputs PASS with a
  logged gap. The Windows Product handoff
  (`docs/reviews/handoff-windows-product-engineer-2026-10-13-v0223.md`
  §5.2) routed the follow-up here: the governor should OWN enforcement,
  and the projection models weights bytes only.
- `crates/modelswarm-resource` (the M1/M2 crate, gate D19) already ships
  the read-only `ResourceSampler` (CPU/RAM/working-set/GPU/VRAM at ≤1 Hz,
  mockable `SampleSource`) and the static `HardwareProfile`. The sampler
  handoff explicitly forbade using it as an enforcement mechanism until
  the governor existed — this ADR is that governor.
- `crates/modelswarm-winjob` already supervises engine children with
  kill-on-close job objects (crash-orphan protection) and exposes
  job-object membership; it is the natural Windows enforcement seam
  (gate D19: the Resource Engineer integrates it).

## Decision

### 1. The 70%-vs-90% reconciliation (gate D11)

Two bars, two authorities, no crossovers:

| Bar | Owner | Mechanism | Effect |
|---|---|---|---|
| 90% (0.90 headroom) | the RECOMMENDATION side — today the desktop model picker's `fit_and_score` gate | advisory fit scoring (`Fit::GpuFull`/`GpuPartial`/…) | may advise hosting/downloading models that fit up to 90% |
| 70% | the RUNTIME governor (this ADR) | binding verdicts + the OS-level backstop | ENFORCED for memory-coexistence decisions — the family tier's second resident copy today, contribution admission later |

The governor exposes both constants
(`ENFORCED_CEILING_PERCENT = 70`, `RECOMMENDATION_BAR_CEILING_PERCENT =
90`) but only the 70 number appears in any decision path; the 90
constant exists as the documented reconciliation anchor so no future
caller "harmonizes" the bars by accident. Advisory output can never
relax the enforced ceiling, and the governor never acts as a fit
recommender (it judges a specific ADDITION against a live sample, not a
model against static geometry).

### 2. Verdict semantics — enforcement-grade, fail closed

`ResourceGovernor::evaluate(&AdmissionRequest) -> GovernorVerdict` is
BINDING for the consumption it governs: `Decision::Admit` permits the
added copy; `Decision::Degrade` must turn the optional tier off at the
consumer (the desktop swap maps it to `FamilyStatus::off_detail(
"resource_ceiling", verdict.detail_line())`). Degrading the family tier
never touches primary serving — the decision governs the ADDED
consumption only.

Checks run in a fixed order (request validity → system RAM → VRAM when
the added copy lands there); the first failure wins and later domains'
numbers stay absent (a verdict never carries numbers for a check that
did not run). The semantic upgrade over the desktop's advisory check:

> **Unobservable inputs fail closed.** The advisory check passed with a
> logged gap; the governor degrades instead, because a governor that
> cannot observe an input a required check depends on cannot enforce the
> ceiling, and silently passing is exactly the failure mode D11 forbids.
> Distinct stable labels make the failures diagnosable:
> `ram_unobservable`, `vram_unobservable` (no live counters),
> `vram_total_unknown` (no static geometry), plus the projection labels
> `ram_ceiling` / `vram_ceiling`, `weights_not_positive`, and the gap
> label `kv_unmodeled`.

Boundary parity with the desktop is preserved exactly: the breach test
is strictly-greater (`projected > total * 70 / 100`), so
exactly-at-ceiling admits and one byte over degrades; integer math
truncates, never rounds into leniency.

### 3. KV/context honesty — explicit gap, never a guessed number

The projection is `used + weights + kv` when the caller can model the
added copy's KV/context overhead, and `used + weights` otherwise — with
the missing term recorded as an explicit `KnowledgeGap::KvUnmodeled` in
the verdict (and appended to every rendered detail line, so a
weights-only projection can never present as complete).

The runtime surface inventory behind a caller's choice of `Some(bytes)`:

- `RuntimeMetrics::kv_bytes_estimate` (runtime trait): adapter-wide,
  capacity-based, per live handle — but a bare `u64` where 0 means both
  "zero" and "not accounted" (the HTTP sidecar's memory is
  process-level, not per-handle).
- The capi adapter's `estimate_kv_bytes` — `2 (K+V) × n_layer × n_ctx ×
  n_head_kv × head_dim × 2 B (f16)` — is the right PRE-CONSTRUCTION
  shape (the engine pre-allocates the full window), but it is private
  to that module today.
- `LlamaCppCapi::kv_usage()` reports live handles only — the verify
  engine does not exist yet at admission time.

Honest conclusion: at family-admission time the added copy's KV is NOT
yet queryable through any public API, so the desktop call-site swap
passes `added_kv_bytes: None` (gap flagged, weights-only projection —
numerically identical to today's advisory behavior, now with the gap
visible in the type). A Runtime-Engineer follow-up (making the capacity
estimator public or exposing a `projected_kv_bytes(n_ctx)` on the
runtime seam) would let the swap pass `Some` without touching the
governor; the protocol for that change is theirs, not guessed here.

### 4. Enforcement in depth — the winjob seam

The binding verdict is the ceiling's decision layer everywhere. The
backstop layer is the platform-neutral `CeilingEnforcer` trait
(`arm_tree_commit_cap(bytes) -> EnforcementRecord`); nothing
Windows-shaped leaks above it (gate D19):

- **Windows**: `WinJobEnforcer` → `modelswarm_winjob::apply_tree_memory_cap`
  — `JOB_OBJECT_LIMIT_PROCESS_MEMORY` on EVERY retained kill-on-close
  job (the supervised engine tree), remembered so children assigned
  later are BORN capped, with kill-on-close always preserved (both
  flags ride one `SetInformationJobObject`). A memory-exhaustion test
  in this repo proves the cap bites on a real native 768 MiB commit
  under a 450 MB cap while the uncapped control succeeds.
- **Elsewhere**: `UnsupportedEnforcer` returns the honest
  `not_enforced_unsupported_platform` record (Linux cgroup-v2 is the
  recorded follow-up; the admission verdict still binds there).

Cap arithmetic: `cap = ram_ceiling − (system_used − tree_working_set)`
— the supervised tree may grow exactly until the SYSTEM would reach the
enforced ceiling. On Admit the cap is provably `> added_bytes` (never
an instant-allocation-failure configuration); a belt guard records
`not_enforced_cap_underflow` if that invariant ever breaks.

Directionality (why the backstop cannot oversell): the OS enforces the
cap on COMMIT while the projection is working-set arithmetic. Mapped
GGUF weights are largely section-backed and do not count against
commit, so the cap bites at or before the point the working-set
projection would reach the system ceiling — never later. Empirical
note from the bite test: freshly-created .NET managed arrays commit
lazily on first touch and can slip under the cap until written; the
protected class (native engine allocations — llama.cpp is C/C++)
commits eagerly and is exactly what the cap restrains.

A failed backstop arm is an honest `not_enforced_arm_failed` record,
not a failed admission (the binding decision is the verdict). A
consumer that wants the backstop mandatory can degrade on any
`NotEnforced` label — the verdict type gives it that choice without
this crate forcing it.

### 5. Sampling and re-evaluation contract

The governor owns its `ResourceSampler` (verdict inputs are the
governor's own, not the caller's). `evaluate` samples once per call:
admission at hosting start, and re-evaluation on monitor ticks — a
later sample under pressure flips Admit→Degrade, which the consumer
applies by dropping the optional tier. Construction per verdict (the
desktop's current pattern) costs one tree read plus the PDH baseline
and is acceptable at hosting-start cadence; a long-lived governor in
desktop state is the cheaper monitor-tick form.

## Options considered

| Option | Content | Ruling |
|---|---|---|
| A — keep the desktop advisory check, add sampling only | No enforcement authority; unobservables pass | Rejected: D11's premise is a runtime governor that enforces |
| B — verdict-only governor (no OS cap) | Projection decides; nothing restrains post-admission growth | Rejected: the M2 exit criterion is that memory-exhaustion tests cannot freeze the machine — a verdict cannot stop a runaway engine after admission |
| C — verdict + platform enforcement seam (CHOSEN) | Binding verdicts everywhere; winjob commit-cap backstop on Windows; honest labels elsewhere | The desktop's advisory numbers move into the governor unchanged; enforcement becomes real where an OS seam exists |
| D — all-platform caps now (cgroup v2, rlimits) | Full parity day one | Deferred: Linux is not the shipped desktop OS; the seam records the gap honestly instead of shipping an untested cgroup path |

## Consequences

+ The 70% ceiling has an enforcement authority with explicit
  fail-closed semantics; the family tier can no longer pass silently on
  unobservable inputs, and a runaway supervised engine on Windows hits
  a commit cap instead of the pagefile.
+ The desktop's advisory check becomes a consumer of a tested,
  crate-owned verdict (the swap is the Integrator's call-site follow-up;
  exact API in the Resource Engineer handoff 2026-10-13).
+ The KV question stops being a silent approximation: the gap is a
  typed field pinned by tests.
− The fail-closed upgrade changes behavior on machines where sampling
  is unobservable (family tier degrades instead of passing-with-gap) —
  on Windows this is unreachable in practice (GlobalMemoryStatusEx and
  PDH are the sources); on the fallback platform family tier degrades
  by design. The desktop swap inherits this and must say so in its UI
  text.
− One more frozen vocabulary to maintain (reason/gap/enforcement
  labels — golden-tested; a new label is an ADR-035 amendment).
− The winjob crate gains four public functions and one static
  (minimal, additive; flagged for Windows Product review).

## Review and test obligations (landed with this ADR)

- `crates/modelswarm-resource/tests/governor_pressure.rs` — 13 tests:
  admit/breach/boundary math (desktop parity), KV gap semantics
  (weights-only admit with gap vs modeled-KV degrade on the same
  machine), fail-closed unobservables (RAM, VRAM-usage,
  VRAM-geometry), pressure-flip responsiveness (Admit→Degrade on the
  next tick), cap arithmetic (working-set accounting, saturation),
  enforcement-label honesty (opt-out, unsupported platform, arm
  failure), detail-line parity strings.
- `crates/modelswarm-resource/tests/winjob_enforcement.rs` (Windows) —
  the real seam: kernel-readback of the armed cap through the governor,
  and the native-commit bite test with an uncapped control.
- `crates/modelswarm-winjob/src/lib.rs` — the cap lands on retained
  jobs as the kernel sees it, sticks for children assigned later, and
  refuses a zero cap; all pre-existing kill-on-close behavior is
  unchanged (their tests are byte-identical and still green).

## Follow-ups routed

1. **Integrator**: the desktop call-site swap (replace
   `family_resource_verdict` with the governor's verdict; exact API in
   the handoff; the desktop crate is NOT touched by this ADR's landing).
2. **Runtime Engineer**: a public pre-construction KV estimator so the
   desktop can pass `added_kv_bytes: Some(…)` and retire the gap.
3. **Resource Engineer (next)**: Linux cgroup-v2 enforcer; M1 profile
   completion (thermal, battery, foreground-state awareness — the
   governor currently has no thermal/battery inputs and degrades on
   memory only).
4. **Windows Product Engineer**: review the winjob additions (their
   crate; the change is additive and kill-on-close-preserving).
