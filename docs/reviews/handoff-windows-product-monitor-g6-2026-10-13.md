# Handoff — Windows Product Engineer: monitor panel, ADR-034 tier 1, TR-RC-8 config fix

Date: 2026-10-13 · Scope: `crates/modelswarm-desktop`, `crates/modelswarm-node`
(the new ADR-034 engine seams only), `crates/modelswarm-winjob`, plus this
handoff. Three parts, one commit each, all pushed to `origin/main`:

| Part | Commit | One line |
|---|---|---|
| 1 — task-manager resource monitor panel | `10d8ff1` | CPU/RAM/GPU meters below Privacy, winjob-scoped, 1 Hz, measured geometry |
| 2 — ADR-034 tier 1 family auto-pin | `905b362` | rules 1–7: deterministic serving-side pin + probe pairing confirmation |
| 3 — TR-RC-8 config-writer fix | `6c249be` | config.json read-modify-write preserves unknown keys |

The resource crate (`crates/modelswarm-resource`) was consumed READ-ONLY
(no bytes changed). No `protocol/`, `catalog/`, or `modelswarm-session`
bytes changed anywhere (ADR-034 tier 1 lives in the desktop plus the
node's existing seams, exactly as the ADR scopes it).

## Part 1 — the resource monitor panel (owner request 2026-10-12)

### Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-winjob/src/lib.rs` | New `supervised_process_ids()` — job-object MEMBERSHIP via `QueryInformationJobObject(JobObjectBasicProcessIdList)` over every retained kill-on-close job (64-slot buffer, one doubling on `ERROR_MORE_DATA` to 1024, honest empty on any failure; deduped sorted union). This is the sampler handoff §4's PREFERRED option (a): the panel's process tree equals the supervised set, immune to PID-reuse orphans. +1 test (`supervised_process_ids_reports_assigned_children`); both job tests now serialize on a shared lock — the retained-jobs stack is LIFO and `close_one_retained_job_for_test` popping another test's job was a real cross-test kill (found live on the first run). |
| `crates/modelswarm-desktop/Cargo.toml` | Optional `modelswarm-resource` dep wired under `tauri-shell` (plus Part 2's optional `sha2`/`hex`). |
| `crates/modelswarm-desktop/src/app.rs` | One `resource_sample` IPC command (doc-table row added): lazily creates ONE `ResourceSampler` behind a sync mutex, samples on the blocking pool (sync Toolhelp/PDH FFI), returns the `ResourceSample` field names verbatim with per-field `null`. Scope roots = own PID ∪ `winjob::supervised_process_ids()` (Windows) — `ProcessScope::Roots`, so the sampler rebuilds when the supervised set changes (hosting start/stop; one honest CPU "n/a" tick on rebuild, the fresh baseline). Pure helpers `monitor_roots` / `ensure_panel_monitor` / `resource_sample_json` + 3 unit tests. |
| `crates/modelswarm-desktop/ui/index.html` | `Resources` section below Privacy: collapsed by default (one summary line "CPU x% · RAM y% · GPU z%"), `details`/`hide` toggle (aria-expanded) expanding three labeled meters — CPU = process-tree %, RAM = system % + "ours <working set>", GPU = util % + "<vram> VRAM"; every `null` field renders "n/a" with the bar at 0% (the degraded-chips pattern); ONE `setInterval(refreshResources, 1000)` task. |
| `crates/modelswarm-desktop/ui-harness/monitor.html` (new) | Committed stub harness (census-harness pattern): loads the REAL `ui/index.html` into a same-origin 1152×648 iframe, stubs the bridge, and asserts values-verbatim, n/a degradation, the toggle, and MEASURES rendered geometry (details below). |

### Rendered-geometry evidence (the owner's standing rule, measured not assumed)

Headless Edge (`--headless=new --virtual-time-budget`) against the
committed harness served over http, results beaconed to a local recorder
(log below re-run against the FINAL UI after Parts 2–3; 9/9 PASS):

- Baseline (panel hidden): right column scrollHeight **732** vs clientHeight
  **552** — the column ALREADY scrolled before this change, so the
  collapsed/expanded affordance was required, and the panel ships
  COLLAPSED by default.
- Collapsed panel cost: section height **72 px**; column 732 → **815**
  (= section + one 10 px gap — one summary row, nothing more).
- Expanded: section height **175 px** (meters + note), column **918**;
  bar widths track the percentages exactly (12.5% / 51.3% / 3.25% widths
  from a 12.5/51.3/3.25 stub sample).
- All-null tick (the macOS-fallback shape): every meter reads "n/a",
  every bar 0%, summary "CPU n/a · RAM n/a · GPU n/a" — never a zero or
  a guess.
- Value formatting pinned: ≥10% renders zero decimals (12.5 → "13%"),
  <10% one decimal; bytes humanize MB/GB.

Screenshots kept outside the repo at
`%TEMP%\msp-verify\shots\{monitor-beacon,monitor-final,ui-collapsed}.png`
(78–81 KB each; the census handoff's pattern — committed artifact is the
harness, the shots are run evidence).

### Commands and outcomes (Part 1)

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` | CLEAN |
| `cargo clippy --workspace --all-targets -- -D warnings` | CLEAN |
| `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` | CLEAN |
| `cargo test --workspace` | **493**/0/6 (baseline 492/0/6; +1 winjob membership test) |
| `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --features tauri-shell` | 21/0/0 (was 18; +3 monitor) |
| Beacon harness run | 9/9 PASS (table above) |

## Part 2 — ADR-034 tier 1: the family auto-pin (rules 1–7)

### Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-node/src/lib.rs` | `NodeHandle` gains `pub runtime: Option<Arc<dyn InferenceRuntime>>` (the SAME instance the gateway executor serves through — no new secret copies; the sidecar bearer stays in-process, ADR-002) + two seams: `engine_tokenize` (the sidecar's `/tokenize` surface — ADR-034 rule 3's "any surface yielding the pinned engine's ids") and `engine_propose_window` (the coordinator's greedy continuation via the runtime's `propose`, rule 6b: the probe's window is drafted by the coordinator's OWN hosting engine, no second engine). Public free-function cores `runtime_tokenize` / `runtime_propose_window` + 1 test with the mock runtime (mapping exact; honest "no supervised engine" fail-closed on None). |
| `crates/modelswarm-desktop/src/app.rs` | The tier-1 implementation (below). 7 new tests. |
| `crates/modelswarm-desktop/ui/index.html` | Family status line renders the pin (`— pin auto family-prompt-v1, N tok, abcd1234…` / `file`); the probe result line renders the pairing verdict with counts (`— pairing: pairing_confirmed (8/8)` / `pin_mismatch (0/8, divergence at 0) [detail]`) and the E0 caveat is carried in the comment copy. |
| `crates/modelswarm-desktop/Cargo.toml` | Optional `sha2`/`hex` (the pin hash) under `tauri-shell`. |

Implementation shape: `FAMILY_PROMPT_V1` frozen verbatim from the ADR
(byte-identical, test-pinned); `family_prompt_render_system_only` (a
SYSTEM-ONLY ChatML turn — ADR-025's catalog template family — with NO
assistant opener: the pin is the root context the verifier prefills and
the coordinator continues); `derive_family_prompt_ids(text, tokenizer)`
generic over an async tokenizer closure so tests inject a deterministic
byte tokenizer and production passes the node's engine seams — ONE
derivation path everywhere; `family_pin_hash` = bare 64-hex sha256 over
LE-u32 ids with NO parent (preimage pinned by test against an accidental
parent-prefix form); `family_pin_plan` = the pure rule-4 decision
(WriteAuto / RewriteAuto{previous_version} / KeepFile / Junk) with a
LAZY current-version derivation (an expert file needs no engine);
`write_family_pin_file` = `.part` + rename (rewrite path removes the
old file first — std rename does not replace on Windows; sub-ms window
at hosting start, documented); `ensure_family_pin` runs rules 1–4 inside
`compose_family_serving` BEFORE the pin is read and before
`capi_family_source` constructs, telemetering `family.pin`
(provenance/version/tokens/hash — counts and a hash only, rule 7) or
the unchanged `family_prompt_unpinned` off-reason with a `detail`.

The probe (rule 5+6b): after a clean open it binds to the same pin —
the local FILE wins (the expert seam is identical-files-on-both-ends);
absent file ⇒ the SAME derivation on demand (stock installs pair with
zero files); a present junk file ⇒ `pin_unpinned` (fail-closed, never
derives over a file the user may own). w = `min(8, accepted
batch_window_max)`; the window is drafted through
`runtime_propose_window`; ONE `client.round` ⇒ `pairing_confirmed` on
full acceptance, `pin_mismatch` on any rejection (divergence point 0
canonical; round-failure detail labels `rejected`/`timed_out`/… stable
and counts-only). Result + telemetry go through
`pairing_result_fields` — a shape with NO array field, so token ids
cannot ride it (structurally pinned by test).

### ADR-034 rule-to-test traceability (the review section, 1:1)

| ADR requirement | Test (`app::tests`, tauri-shell) |
|---|---|
| Determinism: same text + pinned tokenizer ⇒ identical ids and pin hash across runs; system-only render; LE-u32 no-parent hash | `family_pin_derivation_is_deterministic_and_system_only` (also pins the v1 text byte-for-byte and the exact sha256 preimage) |
| Auto-write-on-absent | `family_pin_auto_writes_when_absent` (plan WriteAuto; write lands parsable, in order; no `.part` litter) |
| Junk-file stays fail-closed unpinned | `family_pin_junk_file_stays_fail_closed_and_untouched` (junk, hex, whitespace, empty ⇒ Junk; file bytes never touched) |
| Present-file precedence (expert override wins, incl. equal-to-current) | `family_pin_present_file_wins` |
| Version-advance rewrite (unmodified auto pin rewritten; expert untouched) | `family_pin_version_advance_rewrites_only_unmodified_auto` (two-version table: v0-equal ⇒ RewriteAuto flagged v0, applied ⇒ v1 ids; `1 2 3` ⇒ KeepFile, bytes unchanged) |
| Probe label stability incl. NO token ids in results or telemetry | `pairing_labels_are_stable_and_results_carry_counts_only` (verdict matrix incl. divergence-0; failure labels; serialized fields contain no `[` — no array can exist) |
| Status provenance fields | `family_status_pin_provenance_fields` (exactly 4 pin keys: provenance/version/token_count/pin_hash; `family_status_off_carries_the_reason` extended: pin null while unpinned) |

Rules 1–3 and 5–7 are carried by the implementation sites quoted above;
rule 8 (reserved wire vocabulary) intentionally NOT implemented.

### Commands and outcomes (Part 2)

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` | CLEAN |
| `cargo clippy --workspace --all-targets -- -D warnings` | CLEAN |
| `TAURI_CONFIG=… clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` | CLEAN |
| `cargo test --workspace` | **494**/0/6 (+1 node engine-seam test) |
| `cargo test -p modelswarm-node --lib` | 23/0/3 |
| `cargo test -p modelswarm-desktop --features tauri-shell` (TAURI_CONFIG) | 28/0/0 (+7) |

## Part 3 — TR-RC-8: the config-witer fix

`persist_config` serialized a two-field `PersistedConfig`, silently
deleting every unknown key on each rewrite; `set_tracker` had a second
inline copy of the same write. Both sites now share ONE
`persist_config` that reads the file into a `serde_json::Map`, updates
exactly `tracker` and `profile_id`, and writes back preserving
everything else (missing/corrupt file starts from `{}` — the
fresh-install shape, same as before). `read_config` is unchanged (it
already reads the two keys tolerantly).

Pinned by `config_rewrite_preserves_unknown_keys`: an `advisory` subtree
and a `window_state` object survive a tracker change AND a profile
change verbatim while both daemon keys update; and
`config_write_fresh_file_and_null_profile` pins the fresh-file shape
(exactly two keys; `profile_id: null` round-trips; `read_config` agrees).

| Command | Outcome |
|---|---|
| gates as above | fmt CLEAN; both clippy CLEAN; `cargo test --workspace` **494**/0/6; desktop tauri-shell **30**/0/0 (+2) |

## Assumptions

1. The panel's CPU/working-set scope includes the desktop process
   itself plus the job membership (the owner's "task-manager-like" ask
   reads as OUR footprint); the webview children fall under the own-PID
   tree walk. `ProcessScope::Roots` = roots + descendants, so with no
   jobs this is exactly the old `CurrentTree` set.
2. One honest CPU "n/a" tick per hosting start/stop (sampler rebuild
   for the changed root set) at 1 Hz cadence — visible, bounded,
   documented; preferred over a stale-scoped sampler.
3. ADR-034 rule 3's system-only render deliberately omits the assistant
   generation prompt (the pin is the root the engine continues from);
   frozen here, byte-pinned by test — both ends derive identically, and
   the ADR's "SYSTEM-ONLY turn" wording is the anchor. ChatML is the
   catalog invariant (ADR-025 rule 2); a non-ChatML profile would need
   the ADR-025 rule-3 gate before joining, so no per-profile template
   switch exists yet.
4. The probe derives (does not WRITE) when its pin file is absent — the
   rule-4 write discipline is anchored to hosting start (the family
   composition path); a listener-off coordinator still pairs because
   the verifier holds its own file.
5. `pairing_verdict` treats a zero-length window as `pin_mismatch`
   (guard; the draft path labels empty drafts `draft_unavailable`
   before a round is ever sent).
6. Node seam naming (`engine_tokenize`/`engine_propose_window`) is
   desktop-facing product surface in my crate; the underlying trait and
   adapters are untouched.

## Unresolved risks

1. Cross-machine E0 determinism: on real two-machine venues a greedy
   draft can legitimately diverge from the verifier's engine, so
   `pin_mismatch` means the context-or-backend DIVERGENCE CLASS — the
   UI copy and the ADR's caveat say so; the RC-1 audit path
   (`verifier_divergence`) remains the independent signal. The live
   two-desktop probe run is still pending a second venue (this machine
   verified open/refusal paths only; the round logic is unit-pinned).
2. The Vulkan sidecar drafting against a CPU family engine (or vice
   versa) can present as `pin_mismatch` — same divergence-class reading;
   a future backend-aware probe could compare disclosures first.
3. The version-advance rewrite is dormant while v1 is the only version
   (table empty in production); the day v2 ships, the rewrite fires for
   unmodified auto pins and the ADR's "pairing-visible event" note
   applies (telemetry already carries version + rewrote_from).
4. `resource_sample` polls only while the UI timer runs; a backgrounded
   webview may throttle the interval (browsers clamp hidden-page timers)
   — the panel then freezes on its last values, which is honest for an
   advisory display but worth knowing.
5. The winjob membership query reads the retained per-child jobs; if a
   future refactor moves to ONE shared job object, the union query keeps
   working (it iterates all retained handles) but the test's `contains`
   assertion style should be revisited.

## Suggested next task for the integrator

Route to Security + Test and Release for the ADR-034 §review sign-off on
this handoff (both are the ADR's required reviewers). Natural follow-ups:
(a) a live two-desktop probe run on the swarm test pair (peersOnline=2
venue) to record the first REAL `pairing_confirmed`; (b) the ADR-034
resource note's family-hosting toggle decision with the Resource
Engineer's governor coordination (the family engine now inits on stock
installs — the RAM/VRAM-doubling posture vs the 70% ceiling, gate D11);
(c) fold `family_probe`'s pin derivation behind the same caching as
`earn_lease` if probe cadence ever becomes automatic (rate-limit note).
