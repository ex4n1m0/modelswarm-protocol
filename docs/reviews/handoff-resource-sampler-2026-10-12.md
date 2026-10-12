# Handoff — Resource Engineer: the live `ResourceSampler` (owner request 2026-10-12, desktop monitor panel)

New crate `crates/modelswarm-resource` — the M1/M2 resource crate's first
module: a cheap, pollable, mockable sampler of CPU / RAM / working-set /
GPU / VRAM for the task-manager-style desktop panel. The panel UI itself
is a separate Windows-Product task; this is the sampling layer it
consumes, and NOTHING is wired into the desktop yet. The M2 governor
(70%-ceiling enforcement, gate D11) is untouched and still opens with its
own ADR; this sampler is advisory/read-only and must not be used as an
enforcement mechanism.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-resource/Cargo.toml` | New crate manifest. `unsafe_code = "allow"` with the winjob-style justification (raw Win32 FFI isolated behind the `SampleSource` trait); crate lints re-assert `clippy::all = deny` since crate lints replace workspace lints. |
| `crates/modelswarm-resource/src/lib.rs` | `ResourceSample` (all-`Option` fields, per-field fail-soft), `ResourceSampler` (baseline-at-construction, pure delta math), `cpu_percent`/`used_percent` pure helpers, `ResourceSampler::platform(scope)` constructor, crate docs with the availability matrix and UI-cadence contract. 10 unit tests pin the math. |
| `crates/modelswarm-resource/src/source.rs` | The platform seam: `SampleSource` trait (injectable clock + sources; `Box<S>` forwarding impl), `ProcessScope` (CurrentTree / Roots), `TreeSnapshot`/`ProcSample`/`SystemMemory`/`GpuLive`, pure helpers `descendant_pids` (cycle-safe) and `luid_of_instance` (PDH instance-name parsing), and the public `MockSource` for tests (ours and the desktop's). 6 unit tests. |
| `crates/modelswarm-resource/src/windows.rs` | Full Windows path: Toolhelp32 snapshot -> `GetProcessTimes`/`GetProcessMemoryInfo` per tree member; `GlobalMemoryStatusEx`; `GetSystemInfo` logical cores; PDH `\GPU Engine(*)\Utilization Percentage` + `\GPU Process Memory(*)\Local Usage` via `PdhAddEnglishCounterW` (English counter names, vendor-neutral), one persistent query with baseline collect at open. All raw FFI lives here only. |
| `crates/modelswarm-resource/src/linux.rs` | The trivial `/proc` path: `stat` utime+stime deltas, `statm` RSS, `meminfo`. GPU honestly `None`. |
| `crates/modelswarm-resource/src/fallback.rs` | Other OSes: every field an honest `None`/error (see matrix). |
| `crates/modelswarm-resource/src/parsers.rs` | Pure `/proc` parsers (comm-with-parens-safe `stat` parsing, `statm`, `meminfo`), ungated so their unit tests run on every OS. 5 unit tests. |
| `crates/modelswarm-resource/src/profile.rs` | `HardwareProfile` (M1 static-geometry home; seed): cores + total RAM (RAM via the sanctioned `modelswarm_winjob::total_ram_bytes()` seam on Windows); GPU geometry injected via `with_gpu` from the caller's engine `--list-devices` detection — the sampler never re-detects per sample. |
| `crates/modelswarm-resource/tests/windows_smoke.rs` | Real-system smoke: range/sanity assertions only (cannot flake on load); asserts the live read works on Windows (CPU/RAM/WS present, GPU present-or-None, in range). Plus a tree-scope test proving a spawned child is included. |
| `crates/modelswarm-resource/examples/panel_probe.rs` | 5-sample 1 Hz usage sketch for the desktop team; doubles as the live-evidence probe (output below). |
| `Cargo.toml` (root) | Workspace member `crates/modelswarm-resource` added. No shared-schema, protocol, or dependency changes elsewhere; `Cargo.lock` gains only the new crate node. |

## 2. Public API (what the desktop panel consumes)

```rust
use modelswarm_resource::{ProcessScope, ResourceSampler, HardwareProfile};

// once, at panel open — opens the PDH query and takes the CPU baseline:
let mut sampler = ResourceSampler::platform(ProcessScope::CurrentTree);
// then, on a 1 s UI timer (NEVER per frame):
let s = sampler.sample(); // -> ResourceSample
```

`ResourceSample` fields (all `Option<f64/u64>`; `None` renders as "n/a"):

- `process_tree_cpu_percent` — tree CPU as % of TOTAL system capacity
  (normalized by all logical cores, Task-Manager style; clamped 0–100)
- `system_ram_used_percent`, `system_ram_used_bytes`, `system_ram_total_bytes`
- `process_tree_working_set_bytes` — sum of per-process working sets
- `gpu_utilization_percent` — busiest adapter; sum over its engine
  instances, clamped (concurrent 3D+decode exceeds 100 pre-clamp)
- `gpu_vram_used_bytes` — dedicated VRAM in use on that same adapter
  (WDDM-tracked, the Task-Manager approximation)

Static labels (total RAM/VRAM for the panel header) come from
`HardwareProfile::detect()` + `.with_gpu(name, vram_bytes)` fed by the
desktop's EXISTING `--list-devices` detection — do not re-detect.

## 3. Field availability matrix (cross-platform honesty)

| Field | Windows (primary) | Linux | other OS |
|---|---|---|---|
| `process_tree_cpu_percent` | Toolhelp + `GetProcessTimes` deltas | `/proc/<pid>/stat` utime+stime deltas | `None` |
| `system_ram_used_*` / `total` | `GlobalMemoryStatusEx` | `/proc/meminfo` | `None` |
| `process_tree_working_set_bytes` | `GetProcessMemoryInfo` | `statm` RSS (4 KiB pages assumed) | `None` |
| `gpu_utilization_percent` | PDH `GPU Engine` (Task-Manager source, Win8+); `None` if PDH/GPU absent | `None` | `None` |
| `gpu_vram_used_bytes` | PDH `GPU Process Memory\Local Usage` summed per adapter; `None` if absent | `None` | `None` |

GPU honesty rule honored: same source family as the M1 static detection's
backend (system GPU services, vendor-neutral), live counters only where
reliably obtainable without heavy deps; every unavailable field is
`None` ("n/a"), never a guess. Linux path is deliberately trivial and
documents its two assumptions (`USER_HZ == 100`, 4 KiB pages).

## 4. Process-tree scoping note for Windows Product

The natural set for "our processes" is the **winjob job-object
membership**: every engine child accepted by
`modelswarm_winjob::assign_child` is in the kill-on-close job, so job
membership == "processes we supervise". The winjob crate does not expose
a membership query today and it is Windows-Product-owned — this handoff
requests (via review) either (a) a `QueryInformationJobObject`-backed
membership/job-accounting accessor, or (b) confirmation to keep using
`ProcessScope::CurrentTree`, which covers the same set in practice
because the desktop spawns the node/engine itself (and in ADR-031
composed mode the engine is in-process, i.e. already inside our own CPU
time). `ProcessScope::Roots(vec![engine_pid])` is available now if the
panel wants to scope explicitly to PIDs the node already knows. PID-reuse
orphans (a foreign process reparented onto a dead tree PID) are a known
1 Hz-monitor-grade approximation; the winjob-membership query would
eliminate it.

## 5. Exact commands and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all` then `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (0 errors/warnings); `cargo clippy -p modelswarm-resource --all-targets -- -D warnings` re-forced after touch |
| `cargo test --workspace` | 492 passed / 0 failed / 6 ignored (baseline 469/0/6; +23 = 21 unit + 2 Windows smoke, all new) |
| `cargo test -p modelswarm-resource` | 21 lib tests + 2 smoke tests green |
| `cargo run -p modelswarm-resource --example panel_probe` | 5 ticks at 1 Hz on the dev machine (24 cores / 32 GiB): cpu 0.0–0.1%, ram 50.3–50.4% (16.1 GiB), ws 9–10 MiB, gpu util 2.7–15.5%, vram 6.6 GiB — every field live and plausible |

## 6. Test evidence summary

- Math is pinned by unit tests that run on every OS (normalization by
  cores, clamping at 100, zero-wall/zero-cores guards, per-PID churn:
  new/dead/reused PIDs, baseline-failed, transient-error re-baseline,
  zero-interval, memory/GPU passthrough, `None` honesty, /proc parsers).
- The real-system read is smoked (Windows only, range assertions that
  cannot flake on machine load, plus child-in-tree proof with a spawned
  process). No env-gating, no ignored tests; the 6 pre-existing workspace
  ignores are untouched.

## 7. Assumptions

1. The panel polls at <= 1 Hz from a single task (PDH query handles are
   not documented thread-safe; `sample()` takes `&mut self` so the Rust
   API enforces serialization anyway).
2. First `sample()` happens >= ~100 ms after construction (PDH rate
   counters need two collects separated in time; the constructor takes
   the baseline collect so tick 1 already reports GPU).
3. Linux: `USER_HZ == 100`, 4 KiB pages (documented in-module).
4. "Working set" on Linux means RSS (statm) — close enough for a panel,
   documented as such.
5. GPU utilization semantics = busiest adapter, engine-instance sum
   clamped to 100 (documented on the field); VRAM = WDDM-tracked local
   usage, matching Task Manager's dedicated-memory column in practice.
6. No ADR filed: no protocol/shared-schema field, no cross-crate
   contract change; root `Cargo.toml` membership addition follows the
   standing practice for new crates.

## 8. Unresolved risks

1. `GPU Process Memory` counters may be absent on some Windows SKUs
   (older Server cores) — handled: VRAM degrades to `None`, utilization
   still live.
2. Elevated engine children cannot be `OpenProcess`-ed from a standard
   desktop — skipped (honest undercount for that tick); not expected in
   our topology (we spawn the engine ourselves).
3. PDH `PdhGetFormattedCounterArrayW` needs the two-collect cadence; a
   poll interval < ~50 ms could yield `None` GPU ticks — acceptable at
   the documented 1 Hz cadence, and the fields simply read "n/a".
4. macOS is all-`None` by design (fallback source); if the panel ships
   there, "n/a" display strings must exist for every field.
5. The winjob membership query (section 4) is a cross-crate request into
   Windows-Product-owned code — not started unilaterally per ownership
   rules.

## 9. Suggested next task for the integrator

Route this handoff to the Windows Product Engineer for the panel wiring
(API in section 2) and the section-4 scoping decision; separately, the
Resource Engineer's next owned milestone is the M2 governor ADR (70%
ceiling vs the 90% advisory bar reconciliation, fail-closed semantics),
which should precede any sampler-derived throttling logic.

Required reviewers: Runtime Engineer, Windows Product Engineer, Test and
Release Engineer.
