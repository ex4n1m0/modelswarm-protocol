# Handoff — Resource Engineer: the M2 resource governor (ADR-035, gate D11 enforcement authority)

The M2 milestone opens with its governor ADR as required:
`docs/adr/ADR-035-resource-governor.md`. This landing implements it in
my owned crate `crates/modelswarm-resource` (new `governor` + `enforce`
modules) and integrates the Windows enforcement seam with a MINIMAL,
additive change to `crates/modelswarm-winjob` (Windows-Product-owned;
flagged for their review per D19's seam-integration note). The desktop
crate is NOT touched — the advisory-check swap is the Integrator's
call-site follow-up, specified exactly in §2.

Baselines stated first (task requirement): `cargo test -p
modelswarm-resource` was 23/0 (21 lib + 2 smoke); `cargo test -p
modelswarm-winjob` was 3/0; workspace was 496/0/6.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-resource/src/governor.rs` (NEW) | The governor policy: `AdmissionRequest`, `GovernorVerdict` (`Decision::Admit/Degrade`, `DegradeReason` with exact `DomainNumbers`, `KnowledgeGap::KvUnmodeled`, `VerdictNumbers`, desktop-parity `detail_line()`), `ResourceGovernor::evaluate` (fixed check order: validity → RAM → VRAM-when-GPU-copy; first failure wins; fail-closed on unobservables), cap arithmetic (`ceiling − outside-tree usage`, provably > added on Admit), `ENFORCED_CEILING_PERCENT = 70` + the D11 reconciliation anchor `RECOMMENDATION_BAR_CEILING_PERCENT = 90` (document-only, never in a decision path). 4 unit tests (math saturating/boundary, constants, disjoint labels). |
| `crates/modelswarm-resource/src/enforce.rs` (NEW) | The platform-neutral `CeilingEnforcer` trait + `EnforcementRecord` (`Armed{mechanism,cap_bytes}` / `NotEnforced{label}` with the frozen label vocabulary), `WinJobEnforcer` (Windows; calls `modelswarm_winjob::apply_tree_memory_cap`), `UnsupportedEnforcer` (honest `not_enforced_unsupported_platform`, defined on every OS so the label is pinnable everywhere), public `MockEnforcer` (mirrors `MockSource` precedent), `platform_enforcer()`. 4 unit tests. |
| `crates/modelswarm-resource/src/lib.rs` | Module wiring + re-exports; header updated (the "governor lands later behind its own ADR" line is now false — it landed). |
| `crates/modelswarm-resource/Cargo.toml` | Description update only (no dependency changes). |
| `crates/modelswarm-resource/tests/governor_pressure.rs` (NEW) | The ownership-map memory-exhaustion/responsiveness suite, all mocked (no real OOM): 13 tests — admit/breach/boundary (desktop `>` parity: at-ceiling admits, one byte over degrades), KV gap semantics (same machine admits weights-only WITH gap, degrades with modeled KV), fail-closed unobservables with distinct labels, pressure-flip Admit→Degrade on the next tick, cap arithmetic (WS accounting + saturation), enforcement-label honesty (opt-out / unsupported / arm-failed), verbatim detail-string parity. |
| `crates/modelswarm-resource/tests/winjob_enforcement.rs` (NEW, Windows) | The enforcement path EXERCISED for real: (a) governor Admit arms the cap on actual kernel job objects, read back via `QueryInformationJobObject`; (b) the honest bite test — a 450 MB cap armed before spawn makes an assigned PowerShell child's 768 MiB NATIVE commit (`AllocHGlobal`) fail (`CAP_BIT`) while the uncapped control succeeds (`ALLOCATED`). Bounded by construction; nothing can freeze the machine — the cap is the anti-freeze mechanism. |
| `crates/modelswarm-winjob/src/lib.rs` | MINIMAL additive seam (Windows Product review requested): `set_job_limits` helper (kill-on-close + optional `JOB_OBJECT_LIMIT_PROCESS_MEMORY` in ONE `SetInformationJobObject` call — flags can never be observed half-applied), sticky `TREE_COMMIT_CAP` (children assigned after arming are BORN capped; read at assign so no lock nesting), `apply_tree_memory_cap(bytes) -> Result<usize, String>` (refuses 0; applies to every retained job; honest per-job undercount on failure), `tree_memory_cap_bytes()` (readback), `retained_job_commit_caps()` (kernel-truth diagnostic readback), `reset_tree_memory_cap_for_test()`. `assign_kill_on_close_job` refactored onto the helper — byte-identical behavior when no cap is remembered (all 3 pre-existing tests unchanged and green). +1 test (cap lands/sticks/refuses-zero, order-independent assertions). |
| `docs/adr/ADR-035-resource-governor.md` (NEW) | The M2 governor ADR: 70-vs-90 reconciliation, fail-closed verdict semantics, the KV honesty position with the runtime-surface inventory, the winjob enforcement-in-depth design (commit-vs-working-set directionality, the .NET lazy-commit empirical note), options considered, consequences, test obligations, routed follow-ups. |

## 2. The exact API the desktop should consume (Integrator's swap)

Replace `family_resource_verdict(weights_bytes, hardware, backend)` in
`crates/modelswarm-desktop/src/app.rs` (`compose_family_serving`) with:

```rust
// in the spawn_blocking, exactly where ResourceSampler::platform is built today:
let mut governor = modelswarm_resource::ResourceGovernor::platform(
    modelswarm_resource::ProcessScope::Roots(vec![std::process::id()]),
);
let verdict = governor.evaluate(&modelswarm_resource::AdmissionRequest {
    // the second copy's weights (today's `weights_bytes` argument)
    added_weights_bytes: weights_bytes,
    // KV/context overhead of the added copy. HONEST MAPPING TODAY: None —
    // no public runtime API can size the not-yet-constructed verify
    // engine's KV (see ADR-035 §3). The verdict then carries
    // KnowledgeGap::KvUnmodeled explicitly; pass Some only once the
    // runtime exposes a pre-construction estimator.
    added_kv_bytes: None,
    // true iff handle.engine_backend == Some("vulkan") (the VRAM doubling)
    added_copy_lands_in_vram: handle.engine_backend.as_deref() == Some("vulkan"),
    // hardware.gpu.map(|g| g.vram_mb as u64 * 1_000_000)
    vram_total_bytes: hardware.gpu.as_ref().map(|g| g.vram_mb as u64 * 1_000_000),
    // arm the winjob commit-cap backstop on Admit (enforcement in depth)
    arm_tree_commit_cap: true,
});
match verdict.decision {
    modelswarm_resource::Decision::Admit => { /* family tier proceeds */ }
    modelswarm_resource::Decision::Degrade => {
        // same handling as today's Err(detail):
        // FamilyStatus::off_detail("resource_ceiling", verdict.detail_line())
        // telemetry: verdict.reason.label() (ram_ceiling | vram_ceiling |
        //   ram_unobservable | vram_unobservable | vram_total_unknown |
        //   weights_not_positive) + verdict.gaps (kv_unmodeled) +
        //   verdict.enforcement.label()
    }
}
```

Behavioral notes for the swap (both are INTENTIONAL upgrades, pinned by
tests): (1) unobservable inputs now DEGRADE (fail closed) instead of
passing with a gap — on Windows unreachable in practice
(GlobalMemoryStatusEx/PDH), but the UI copy should say "cannot enforce"
not "unknown"; (2) `detail_line()` reproduces today's breach strings
verbatim ("system RAM: projected X/Y MB would pass the 70% ceiling
(...)"), appending the KV-gap and enforcement annotations, so the
FamilyStatus text survives the swap unchanged on the breach paths.
Monitor-tick re-evaluation (flip to Degrade under arriving pressure and
drop the tier) is supported by calling `evaluate` on the panel cadence
with a long-lived governor — recommended as the swap's second step.

## 3. Exact commands and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all` then `cargo fmt --all --check` | clean |
| `cargo clippy -p modelswarm-resource --all-targets -- -D warnings` | clean |
| `cargo clippy -p modelswarm-winjob --all-targets -- -D warnings` | clean |
| `cargo test -p modelswarm-resource` | **45 passed / 0 failed** (28 lib + 13 governor_pressure + 2 windows_smoke + 2 winjob_enforcement; baseline 23/0) — stable across 3 consecutive runs |
| `cargo test -p modelswarm-winjob` | **4 passed / 0 failed** (baseline 3/0; +1 cap test) — stable across 3 runs |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | **519 passed / 0 failed / 6 ignored** (baseline 496/0/6; +23 = exactly the new tests, zero regressions) |
| `cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` | clean (the desktop consumes this crate; additive change verified) |
| `cargo test -p modelswarm-desktop --features tauri-shell` | 50/0 (baseline exact) |

## 4. Test evidence summary

- Policy honesty (every OS): the pressure suite pins the verdict
  math, the KV gap TYPE (weights-only admit carries the gap; the same
  machine with modeled KV degrades), fail-closed unobservables, the
  pressure flip, cap arithmetic, label vocabulary, and the verbatim
  desktop parity strings.
- Enforcement reality (Windows): the cap lands on kernel job objects
  (independent `QueryInformationJobObject` readback; the kernel may
  round the limit DOWN by up to a page — the assert allows one page of
  downward slack, never an increase), sticks for children assigned
  after arming, and BITES: a real 768 MiB native commit fails under a
  450 MB cap while the uncapped control succeeds.
- Empirical finding recorded in ADR-035 §4: freshly-created .NET
  managed arrays commit lazily and can slip under the cap until
  written; native allocations (the llama.cpp engine's class) commit
  eagerly and are what the cap restrains.

## 5. Assumptions

1. The governor judges ONE added resident copy (family tier shape);
   contribution admission later uses the same request shape.
2. `evaluate`'s sample is taken once per call; construction-per-verdict
   matches the desktop's current sampler pattern (acceptable at
   hosting-start cadence).
3. The VRAM check uses the busiest-adapter usage from the sample vs the
   M1 total (same pairing the desktop's advisory check used).
4. A failed backstop arm does not unmake the admission (the verdict
   binds); consumers wanting it mandatory can degrade on any
   `NotEnforced` label — surfaced, not forced.
5. The winjob change is additive; when no cap is remembered,
   `assign_kill_on_close_job` behaves byte-identically to before (the
   pre-existing tests are untouched and green).

## 6. Unresolved risks / seams flagged for routing

1. **Integrator (primary follow-up)**: the desktop call-site swap (§2).
   Until it lands, the DESKTOP advisory check remains the live gate —
   two implementations of the 70% number coexist (same value, same
   breach math; the governor adds fail-closed + KV gap + the cap).
2. **Runtime Engineer**: no public pre-construction KV estimator exists
   (`estimate_kv_bytes` is private to the capi module; `kv_usage()` is
   live-handles-only; `RuntimeMetrics::kv_bytes_estimate` cannot
   distinguish zero from unaccounted) — so the honest mapping today is
   `added_kv_bytes: None` with the explicit gap. Exposing the capacity
   estimator would retire the gap without touching the governor.
3. **Windows Product Engineer**: review the winjob additions (their
   crate): sticky `TREE_COMMIT_CAP`, `apply_tree_memory_cap`,
   readbacks, test hook — additive and kill-on-close-preserving.
4. Linux enforcement backstop is cgroup-v2 future work (honestly
   labeled `not_enforced_unsupported_platform` today); M1 profile gaps
   (thermal, battery, foreground-state) remain open — the governor has
   no thermal/battery inputs and degrades on memory only.
5. The cap is commit-based while the projection is working-set-based;
   the directionality argument (section-backed weights under-count
   commit, so the cap bites at-or-before the projected point) is
   documented in ADR-035 §4, not a hard guarantee — the binding
   decision remains the verdict + re-evaluation.
6. `retained_job_commit_caps()`/`tree_memory_cap_bytes()` are
   diagnostics; a caller must not treat a stale remembered cap as proof
   of a currently-armed limit on newly-replaced jobs.

## 7. Suggested next task for the Integrator

Route the desktop swap (§2) as the next cross-crate step — it retires
the advisory check, makes the governor the live enforcement point for
the family tier, and turns on the winjob backstop on every Windows
hosting start. Second: the Runtime Engineer's public KV estimator
(§6.2). Both are small, independently shippable, and unblock retiring
the `KvUnmodeled` gap.

Required reviewers: Runtime Engineer, Windows Product Engineer, Test
and Release Engineer.
