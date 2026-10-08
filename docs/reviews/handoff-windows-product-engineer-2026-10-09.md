# Handoff — Windows Product Engineer (domain audit, master prompt §1/§2/M-phases)

Date: 2026-10-09 · Branch: `audit/master-prompt-2026-10-09` (baseline `730cebf`; main at
`63b98e1` with the CI-only tauri-shell test fix) · Product v0.2.20.
READ-ONLY audit: the only file created is this handoff. No cargo/npm/installer builds
were run; suite evidence is the freeze log set + live CI queries (read-only).

## 1. Changed files and why

| File | Change |
|---|---|
| `docs/reviews/handoff-windows-product-engineer-2026-10-09.md` | this audit handoff (only artifact) |

## 2. Scope reviewed and files inspected

`crates/modelswarm-desktop/` (src/app.rs 2301 ln, src/main.rs, ui/index.html 486 ln,
tauri.conf.json, Cargo.toml, build.rs), `crates/modelswarm-node/` (lib.rs, engine.rs,
executor.rs, serving.rs, remote.rs, artifact.rs, main.rs, catalog.rs, selftest.rs),
`crates/modelswarm-winjob/src/lib.rs`, `installer/` (README.md, stage-engine-windows.mjs,
tauri/nsis-notes.md), `.github/workflows/{rust,desktop-installer,windows-package-dry-run}.yml`,
`target/audit-logs/*` (freeze suite), mandatory reads (AGENTS.md, zcode-master-prompt §1/§2/§10,
UX study 2026-10-06, audit-freeze 2026-10-09, ADR-008/024/025, docs/privacy.md).

## 3. §1 FINAL PRODUCT compliance table (v0.2.20 verified)

| §1 element | State | Evidence |
|---|---|---|
| Current state indicator | **PASS** | header `conn-note` (`connecting…/live/ipc error`) `ui/index.html:86,362-363`; hosting state word `index.html:130,392-393`; per-card `● hosting` badge `index.html:253`; listener chip is truth from `get_status` `index.html:405-408` |
| ONE Start/Stop | **PASS (two visible affordances)** | hosting toggle `<button role=switch>` `index.html:129,453-457`; per-card Start/Stop `index.html:256-257,273-278` — justified-for-beta while a model picker exists; collapses to one button when M4 lands |
| Chat interface | **PASS** | `index.html:142-151,459-481`; disabled with honest placeholder when off (`:409-411`) |
| Local API address **+ copy button** | **PARTIAL — copy button MISSING** | API section + curl snippet `index.html:175-180`; `grep -i copy\|clipboard` over the UI: zero hits. §1 names the copy button explicitly. 30-min fix |
| Download/preparation progress | **PASS** | progress bar + MB/% label `index.html:110-113,447-452`; `download` events `app.rs:810-815,1214-1219`; "switching…" transition label `index.html:293-295` |
| Short actionable error | **PASS** | error banner `index.html:100,212`; real catalog error text incl. retry `app.rs:653-673`; gateway errors surface as ⚠ chat turns with code + remedy `index.html:476`, `app.rs:1775-1794` |
| Advanced diagnostics must not complicate normal interface | **PARTIAL** | HF request picker collapsed `<details>` `index.html:114-123` (ok); but Peers & NAT table, Setup (tracker URL/engine/approval), Degraded chips all sit on the primary surface (right column). Acceptable beta; classify below |
| Local API default `http://127.0.0.1:11435/v1` | **PASS** | `modelswarm-gateway` `DEFAULT_PORT = 11_435` (`gateway/src/lib.rs:52`); UI advertises the real bound addr when on (`index.html:419-423`) and the 11435 default when off (`:416`); stable-port-first + ephemeral fallback `app.rs:890-909` |
| Steps 4–5 (hardware analysis, auto strongest-safe selection) | **NOT AUTOMATIC** | `analyze_hardware` + `fit_and_score` + top-6 "recommended for your PC" exist (`app.rs:380-550,739-784`, hw line `index.html:233-240`) but the user still picks. This is the genuinely-new M4 capability; the UI is ready for it |

**Element classifications (§1):** model picker cards — justified-for-beta (hide post-M4);
HF "more models" picker — justified-for-beta, move behind a support/diagnostics command post-M4;
hosting panel — §1-compliant (it IS the state surface); Compute row — justified-for-beta
(ADR-024 honest backend disclosure; required while CPU/GPU fallback exists; hide post-graduation);
Degraded-states chips — §1-compliant after the state-not-fault rework (`index.html:398-408`);
Peers & NAT table — must-hide post-beta (diagnostics); Setup section — must-hide post-beta
(tracker URL editing is not a final-product action; device approval remains until auto-approval);
Privacy section — keep (contribution disclosure is mandated). Retitle "Test box — chat with the
hosted model" → "Chat".

**Error-surface audit:** `send_chat` ⚠ turns render gateway error codes with a remedy
(`index.html:476`, `app.rs:1775-1794`) — good. Honest mode labels verified:
`"swarm — remote peer <12hex>…"`, `"local (remote failed: <code>)"`, `"local (remote dial
failed)"`, `"local single"`, `"local error"` (`app.rs:1540-1616,1821-1827`); served_by
local/swarm in every reply; swarm telemetry carries served_by remote/local_fallback
(`app.rs:1628-1642`). Privacy warning that serving peers observe plaintext prompts is in the
first-run gate (`index.html:90-99`) and `docs/privacy.md`; per-reply recipient is in the mode
label. Honest, per ADR-025 the desktop sends plain `{role, content}` (`app.rs:1743-1748`) —
verified, no client-side ChatML remains.

## 4. §2 / M2 governor seam analysis

**What exists today (enforcement hooks):**

| §2 dimension | Current state | Where |
|---|---|---|
| Engine CPU threads | `-t` only if `Some` — desktop passes `None` → llama-server default = all cores | `app.rs:899`, `engine.rs:400-403` |
| Process priority | none (no BELOW_NORMAL / nice) | `engine.rs:411-450` |
| Context/KV | fixed `-c 4096` at every start | `engine.rs:396-397,148` |
| GPU offload | no `-ngl` (ADR-024 auto-fit amendment); `gpu_layers` field exists but is never set by the desktop | `engine.rs:298,404-407`, `lib.rs:285-315` |
| RAM | TOTAL detection only (GlobalMemoryStatusEx; `dwMemoryLoad`/`ullAvailPhys` parsed but unused) | `winjob/src/lib.rs:35-58` |
| Job object | KILL_ON_JOB_CLOSE only — no CPU-rate, no process/job memory limit (the JOBOBJECT structures already support all three) | `winjob/src/lib.rs:96-135` |
| VRAM | one-shot probe: largest Vulkan device via the bundled engine's `--list-devices` (total+free at probe time); no runtime query, no reservation | `app.rs:449-478,521-539` |
| Concurrency/batch | serving Admission 8 sessions / 2 per peer; wire clamps (deadline, max_tokens, prompt bytes) | `serving.rs:103-114,174-207` |
| Thermal / battery / bandwidth / foreground probes | none | — |
| Emergency rejection | `over_limit` session refusal + gateway clamps; nothing resource-triggered | `serving.rs:70-78` |

**Deviation:** `fit_and_score` gates at 90% of RAM (`ram_est > hw.ram_mb * 9/10`, `app.rs:427`)
and 90% of VRAM (`app.rs:431`) — §2 mandates a 70% ceiling. This ships today in the
recommender; must be aligned (with the §2 margins) when M4 adopts it.

**Minimal platform-neutral governor crate shape (`modelswarm-governor`, needs an ADR —
new crate, touches Runtime engine args and Scheduler work ordering):**

- `ResourcePolicy` — derived budgets from a `HardwareProfile`; 70% as ceiling-not-target;
  priority ladder OS > local chat > committed remote work (in-flight serving sessions) >
  speculative > deliberation > background mapped to admission classes, not thread priorities alone.
- `PlatformBudget` trait with per-OS impls: `cpu` (rate + priority class), `memory`
  (ceiling + pressure query + RSS), `gpu` (VRAM reservation + utilization where supported),
  `power` (battery/AC, thermal where exposed), `disk` (capacity + temp install space),
  `net` (bandwidth cap). Windows enforcement extends `modelswarm-winjob` (job-object CPU-rate
  `JOB_OBJECT_CPU_RATE_CONTROL_INFORMATION`, `JOB_OBJECT_LIMIT_PROCESS_MEMORY`/`JobMemoryLimit`)
  — it stays the single unsafe-sanctioned crate; macOS/Linux start advisory (nice, cgroup v2
  where writable) and are honest about what they cannot enforce.
- `EngineBudget` — computed per start: `threads ≤ floor(cores·0.7)`, below-normal priority,
  ctx from policy + KV estimate, `-ngl` left auto-fit but bounded by reserved VRAM.
  Note: llama-server cannot retune live; mid-session pressure acts on admission/concurrency,
  engine-arg changes apply at the next supervised restart.
- Pressure loop + action ladder: reject-new-remote > shed speculative/pause contribution >
  shrink serving concurrency > pause hosting (battery default-on) > emergency stop + telemetry.
  Every probe failure yields a conservative documented fallback; RAM cross-checked against a
  second source; no single detection source trusted (§2, M1).
- Foreground responsiveness: input-idle probe (Windows `GetLastInputInfo` via winjob-adjacent
  FFI) + local-gateway activity = priority-2 signal; host-chat-yields rules ride the ladder.

## 5. M1 seed distance (v0.2.19 hardware analyzer)

Have: total RAM (one source), CPU cores (`available_parallelism`), largest Vulkan device by
asking the shipping engine itself (`--list-devices` parse — right source-of-truth choice),
`hardware_requirements` conservative RAM model, `fit_and_score` explainable ranking, top-6
shortlist, honest "recommendations unavailable" on unknown size (`app.rs:739-784,2287-2300`).
Missing for M1: available-RAM + memory-load signal, disk capacity/temp space, battery/thermal,
CPU ISA/topology, GPU driver info, multi-source cross-checks, a machine-matrix validation doc,
and failure-mode hardening — today a single unparsable stdout line silently disables GPU
detection (finding F6). The logic lives inside the desktop crate; M1/M4 require extracting it
to a platform-neutral crate so the headless node and the selector share one profile.

## 6. Findings (severity-ranked, with time-boxed fixes)

**S1 (close before/at M4 gate)**
- **F1 · §1 copy button missing.** `ui/index.html:175-180` has the API address + curl but no
  copy affordance. Fix: `navigator.clipboard` button beside `#api-base`. 30 min.
- **F2 · chat_log never cleared — cross-model thread bleed (UX-study leftover still open).**
  Backend log persists across stop and switch (`app.rs:69,1729,1784,1810` — only appended/assigned,
  never cleared in `set_hosting(false)` `app.rs:846-854` or `select_model` `app.rs:1226-1235`);
  the UI transcript is likewise never cleared. Old-model prompts (and the remote peer they went
  to) leak into the new model's context. Fix: clear `chat_log` + transcript on profile change
  and on stop; add test. 1 h.
- **F3 · app.rs is never clippy-linted.** No clippy run enables `tauri-shell` (rust.yml:33,41 =
  default + libp2p only; freeze matrix same), so 2301 lines of IPC orchestration escape
  `-D warnings` (at minimum `useless_format` at `app.rs:1596` would fire). Fix: add
  `cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` to
  rust.yml (with the `TAURI_CONFIG` overlay from `63b98e1`). 30 min + lint fixes.
- **F4 · uninstall vs data-dir split unverified (and the split may not protect anything).**
  `default_data_dir` = `%LOCALAPPDATA%\ModelSwarm\Data` (`lib.rs:144-162`) sits INSIDE the Tauri
  NSIS per-user install root (`%LOCALAPPDATA%\ModelSwarm`). The code comment claims the split
  keeps state "OUT of the … install root" — it is a subdirectory of it; safety depends entirely
  on the NSIS uninstaller deleting only its registered files (unproven). `installer/tauri/
  nsis-notes.md:73-77` still documents the PRE-split expectations. Identity seed, state.sqlite,
  and downloaded models must demonstrably survive uninstall (default keep-weights semantics per
  nsis-notes) or be offered for explicit removal. Fix: one VM uninstall test (what is actually
  deleted?), then either accept + document, or move Data outside the install root. 0.5 d + decision.
- **F5 · M7 Stop sequence: no deregister/drain.** `set_hosting(false)` aborts the heartbeat and
  kills the node; `TrackerClient::drain` (exists, `tracker-api/src/lib.rs:243`) is never called —
  the roster lease just expires server-side; the stale `swarm_executor` QUIC pool is kept in
  `Inner` across stop. Fix: best-effort `drain(lease_id)` before abort; drop the executor pool
  on stop. 2 h.

**S2 (fix batch before M2/M4 land)**
- **F6 · GPU detection aborts on one bad line.** `parse_largest_device` uses `?`/`let-else`
  inside the loop (`app.rs:456-467`) — any `": "`-containing line without `" ("` returns None
  for the WHOLE parse → silently CPU-only recommendations. Change both early-outs to `continue`
  + test. 30 min.
- **F7 · 90% fit ceilings vs §2's 70%.** `app.rs:427,431`. Align with §2 (70% ceiling, margins)
  when extracting the selector; changing the visible recommender is a product decision — do it
  in the M4 ADR, not silently. 1 h + tests.
- **F8 · 15 s innerHTML card re-render (UX-study A5 still open).** `refreshModels` replaces all
  cards via `innerHTML` (`index.html:250`) every 15 s (`index.html:483`) — can swallow an
  in-flight click and drops focus. Patch card state in place or diff by `data-id`. 2 h.
- **F9 · blocking-in-async.** `std::process::Command::output()` engine probe inside async
  (`app.rs:525-533`), blocking `sysctl` (`app.rs:505-513`), synchronous rusqlite `Store::open`
  inside async IPC handlers (`app.rs:674,807,875,1211`), `std::fs` writes (`app.rs:337-347`).
  One-shot and bounded today; convert the engine probe to `tokio::process` and wrap store opens
  in `spawn_blocking` as part of the app.rs refactor. 2 h.
- **F10 · engine defaults violate §2 spirit.** `engine_threads: None` (`app.rs:899`) → all
  cores, normal priority. M2 must pass `threads ≤ 0.7·cores` + below-normal priority (see §4).
- **F11 · remote.rs diagnostics invisible in the GUI.** `eprintln!` at `remote.rs:143,171-173,
  212,228` — `windows_subsystem="windows"` makes stderr a no-op; app.rs itself documents this
  ("a windowed app's stderr is invisible", `app.rs:1117-1119`). Route through telemetry. 1 h.
- **F12 · a11y residue (UX-study C4/C5 partially open).** `aria-selected` on `role="button"`
  cards is invalid (`index.html:260`); download bar is `aria-hidden` with no `role=progressbar`
  (`index.html:111`). 1 h.

**S3 (nits / hygiene)**
- F13 · `installer/tauri/nsis-notes.md` stale: "STAGED, NO ARTIFACT PRODUCED" header and
  pre-split data-dir claims vs shipped v0.2.20 + `Data/` split. Refresh with F4's results.
- F14 · duplicated `#[cfg(feature = "libp2p-backend")]` attributes (`lib.rs:54-55,57-58`).
- F15 · `download_model` IPC is dead from the UI (no `invoke("download_model"` in index.html)
  and resolves the FIRST active profile ignoring selection (`app.rs:804`) — delete or repoint.
- F16 · no rendered-geometry gate. The B6 overflow is structurally fixed by CSS (constrained
  grid, per-column `overflow-y:auto`, `min-height:0`, `index.html:22-26`) but this is another
  unverified layout claim; the study's 1152×648 screenshot/scrollHeight check never landed.
  Add the stub-bridge harness + geometry assertion the study proposed (under `tests/`).
- F17 · engine restart backoff is fixed 500 ms ×3 (`engine.rs:277-279,377`); no exponential
  backoff, no node-level watchdog if the whole node dies (crash recovery = process exit).
- F18 · UX-study P1 `node_state` Tauri event still missing — the UI polls every 2 s
  (`index.html:483`); transitions render only on the next poll.

**UX-study cross-check (which items landed):** A1/A2/A3 fixed (one-action swap `app.rs:1183-
1238` + switching label); A4 partially (busy guard still silently no-ops in JS but cards carry
`aria-disabled` and explicit buttons); A5 open (F8); B1/B2/B3/B4/B5 fixed (single lifecycle
surface, model name shown, stale labels cleared `index.html:412-418`, chat disabled `:409-411`);
B6 structurally fixed but unverified (F16); B7 fixed (states not faults `:398-408`); C1/C2/C3
fixed (real `<button role=switch>` `:129`, `:focus-visible` `:42`, min-height 32); C4/C5
residue (F12); C6 open by design (no tray/title state — P2, daemon-composition decision).
The study's scripted UI-journey test was never checked in.

**M7 sequences.** Start: select/validate (`resolve_profile`, `app.rs:315-335`) → download with
progress (`ensure_artifact`, `app.rs:1213-1221`) → load/warm (`Node::start` binds gateway
FIRST then spawns + health-blocks the engine, `lib.rs:255-346`, `engine.rs:284-318`) → register
(heartbeat: device-enroll → register, `app.rs:950-1157`) → prove eligibility: **deferred** —
the hosting challenge runs lazily at first swarm chat (`earn_lease`, `app.rs:1281-1393`), not
during Start → P2P reachable: **dev-gated** (`MSP_LISTENER=1` only, `app.rs:248-294`; disclosed
in the UI chip) → serve bounded (serving bridge: lease gate, clamps, admission, replay window,
`serving.rs:39-411`) → chat/API enabled (chat gated on hosting; gateway always loopback).
Stop: reject new leases — implicit (listener dies with node via shared shutdown watch) but no
tracker `drain` (F5); in-flight remote chat is not cancelled (running executor fails visibly);
deregister missing (F5); close peers — pool lingers (F5); unload ✓ (`child.kill` + kill-on-close
job object, `engine.rs:355-379,433-448`, `winjob`); release ✓ (job handles retained until
process death by design). Engine supervisor: fail-fast health loop with a dead-flag
(`engine.rs:272-318`), bounded restarts (3) with same port/bearer so the adapter reconnects
seamlessly, graceful kill on shutdown, orphan prevention via job object (best-effort, warned).

**M13 gaps (mine):** signed updates — none (updater correctly disabled until keys, ADR-008);
beta channel — none; incident process — none owned by me; uninstall cleanup — unverified (F4);
crash recovery — engine-child only, no node watchdog; loopback-only API default ✓.

**Code quality (scope item 4).** app.rs is a 2301-line monolith mixing IPC, enrollment, chat
orchestration, hardware analysis, and catalog ranking — REFACTOR before M2/M4: extract
(a) hardware/selection into the future platform-neutral crate, (b) the heartbeat/enrollment
task into a module, (c) chat orchestration into a module; keep IPC shims thin. Secrets over
IPC: verified clean — `get_status` returns state only (paths, ids, ports); bearer, seed,
session token, lease tokens never enter responses (`app.rs:559-605`, doc table `app.rs:3-18`);
`open_approval_page` allowlists the tracker origin (`app.rs:1836-1846`). Feature-gate hygiene:
tauri-shell off by default with release-gate build.rs (staged-engine panic), single-instance
plugin present (`app.rs:2136-2140`), CI gap = F3. Executor/serving seams in modelswarm-node are
clean: single `InferenceExecutor` shared by gateway and P2P bridge (`lib.rs:389-400`,
`app.rs:915-931`), ChatML applied exactly once in the executor (`executor.rs:93-118`, pinned by
test `executor.rs:206-233`), closed error-code tables on every trust boundary (executor
`executor.rs:65-78`, serving `serving.rs:119-142`).

## 7. Salvage matrix (owned components)

| Component | Role | Decision | Reason / dependencies |
|---|---|---|---|
| desktop IPC surface (app.rs commands) | product API | KEEP WITH TESTS | secrets-clean; add clippy-feature gate (F3), extract modules |
| hardware detection + fit_and_score | M1/M4 seed | REFACTOR | extract to platform-neutral crate; fix F6/F7; multi-source |
| select_model one-action swap | UX core | KEEP WITH TESTS | project rule kept (sequential, one profile); add UI-journey test; clear chat_log (F2) |
| send_chat/try_swarm_chat/earn_lease | honest swarm chat | KEEP WITH TESTS | mode labels + lease chain honest; pool cleanup (F5) |
| download_model IPC | legacy | DELETE | unused by UI, wrong semantics (first active profile) |
| winjob | Windows enforcement seed | KEEP WITH TESTS → extend | add CPU-rate/memory-limit classes behind the governor ADR |
| node engine.rs supervisor | engine lifecycle | KEEP WITH TESTS | fail-fast + bounded restarts + orphan guard; backoff nit (F17) |
| serving.rs bridge | P2P serving | KEEP WITH TESTS | lease gate, clamps, admission; runtime/network own internals |
| remote.rs | requesting side | KEEP WITH TESTS | pooled reuse, ADR-007 semantics; telemetry not eprintln (F11) |
| executor.rs | local executor | KEEP | ADR-025 anchor, regression-pinned |
| artifact.rs ensure_artifact | M5 core | KEEP WITH TESTS | atomic rename + hash-while-stream; NOT resumable, no disk check — M5 follow-up |
| ui/index.html | the product screen | REFACTOR | add copy button, clear transcript, patch-in-place cards, geometry gate |
| installer workflows | release gates | KEEP | per-file hash verify + engine-inside-artifact checks are exemplary |
| installer/tauri/nsis-notes.md | packaging doc | REFACTOR | stale status + pre-split uninstall claims |

## 8. Commands run and outcomes (this audit, read-only)

- Repo/history: `git log/status/show` (baseline `730cebf` confirmed; `63b98e1` = CI-only
  `TAURI_CONFIG` overlay on the tauri-shell test step; engines remain staged in installer
  workflows only — verified by reading both workflows).
- CI (read-only `gh run list`): `desktop-installer.yml` last main run **success**
  (2026-10-08T14:29Z); `windows-package-dry-run.yml` green per freeze record; `rust.yml` run
  for `63b98e1` (id 37858753428) **in_progress** at audit time — local proof of that fix is in
  the freeze record (bare-runner repro); not counted as green here.
- Freeze suite logs (`target/audit-logs/summary.txt`): fmt PASS, clippy default PASS,
  test default PASS (301), clippy+test libp2p PASS (313), desktop tauri-shell PASS (7),
  tracker typecheck/test/build/vectors PASS. All at `730cebf`.
- Source greps for secrets-over-IPC, copy button, chat_log clearing, drain calls, geometry
  gates, clippy feature coverage (findings F1–F18 above).

## 9. Test evidence

No new tests were run (read-only audit; suite evidence above). Every proposed fix lists its
test: F2 (chat_log clear) needs an IPC-level test + UI-journey assertion; F3 adds a CI step;
F6/F7 unit tests beside `fit_and_score`'s existing vectors (`app.rs:2220-2285`); F16 is the
UX-study stub-bridge harness + `scrollHeight ≤ 648` assertion; F4 is a VM uninstall procedure
with before/after directory manifests.

## 10. Assumptions

1. Freeze suite logs + green installer CI runs are accepted as the evidence baseline (per task
   constraints; no builds re-run).
2. The 1152×648 geometry conclusion is CSS-structural only — no browser rendering was possible
   in this session; labeled as unverified (F16).
3. Uninstall behavior was not executed (would mutate this machine); F4 is explicitly an open
   verification item, not a confirmed defect.
4. `rust.yml` fix run still in flight at handoff time; treat main CI as pending, not green.
5. §2's 70% policy not yet ADR'd for this codebase; F7 flags the delta without changing
   shipped behavior.

## 11. Unresolved risks

- F4 (uninstall may delete identity/models) is the largest user-harm risk in my domain.
- The recommender ships a 90% RAM ceiling today; a 32 GB machine will happily recommend a
  ~28 GB-weights model that can make it unusable — §2 risk until M2/M4.
- app.rs monolith + missing clippy coverage compound every future change (M2 hooks, M4
  selection, preset UI).
- No rendered-geometry or UI-journey automation: layout regressions ship silently again.

## 12. Suggested next task for the integrator

Approve one batched PR from this audit (all read-only-verified, no protocol/ADR surface):
F1 copy button, F2 chat_log/transcript clear, F3 clippy-tauri-shell CI step, F5 stop drain +
pool drop, F6 parse hardening (+tests), F11 telemetry routing, F13/F14/F15 doc+dead-code
cleanups. Separately schedule the F4 uninstall VM test and the M1/M2 extraction ADR
(platform-neutral profiler/governor crate shape per §4/§5) — the gate decision for M4 depends
on both. Reviewers per AGENTS.md: Protocol Architect + Test and Release.
