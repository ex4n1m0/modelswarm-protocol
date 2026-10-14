# Handoff — Windows Product Engineer — 2026-10-14 — ADR-037 §7 steps 1–2 (advisory client core + chat-wall suite)

Scope: ADR-037 §7 steps 1–2 ONLY. Digest plane; the assistant surface
is a later leg and NOTHING in this change serves it. No version bump,
no UI, no protocol wire surface change (`protocol/msp-v1.md`,
`protocol/messages.proto`, `catalog/schema.json`, `protocol/vectors/`
byte-untouched; the two new `protocol/*.json` files are the ADR-033
§9.4-sanctioned advisory contract artifacts, not wire surface).

## 1. Changed files and why

**New — the advisory client core (ADR-033 rev 2, digest plane):**

- `crates/modelswarm-node/src/advisory/mod.rs` — module root: the
  arm decision (`arm()`: enabled + key + https endpoint + model;
  keyless = inert by construction), the single production wiring point
  (`wire()`: silent when disabled, ONE `advisory.cycle.gap` row when
  enabled-but-not-armable, the timer runner when armed), and the
  counter-sensitivity control + chat-wall success-path tests.
- `crates/modelswarm-node/src/advisory/config.rs` — `advisory.json`
  reading (TR-RC-8: dedicated file, never `config.json`; flat keys;
  unknown keys tolerated; malformed = inert with `no_config`, never a
  guess), the eleven ADR-033 §4 digest defaults + the nine ADR-037 §3.4
  `assistant.*` rows (READ-BUT-INERT at this leg, golden-pinned), the
  https-only endpoint gate (§3.3), and env-first key-file resolution
  (§3.4: `MSP_ADVISORY_KEY_FILE` → config → platform default
  `%USERPROFILE%\ModelSwarm-advisory-key.txt` / `~/.modelswarm/
  advisory-key.txt`; single line, trimmed; NEVER in the repo; NO
  rotation anywhere — ADR-037 §3.6 standing owner directive).
- `crates/modelswarm-node/src/advisory/egress.rs` — the ONE
  value-typed allowlist (integers, booleans, strict RFC-3339-UTC
  timestamps, artifact-frozen enum values, 16-hex peer/profile
  references per RC-1) mechanically driven by the frozen artifacts via
  `include_str!` (the compiled gate reads exactly the frozen files),
  the RC-3 choke-point re-scan (re-parse + value re-match +
  `scrub_content`-unchanged + byte cap ⇒ `empty_after_redaction`,
  nothing sent), and the clamp application over
  `advisory-clamp-table-v1.json` (RC-7: in-range / clamped /
  `requires_adr`; no improvised ranges).
- `crates/modelswarm-node/src/advisory/accum.rs` — the typed
  in-process accumulators (RC-4): frozen event-type aggregation
  allowlist (EXCLUDES `advisory.*`), frozen timing-metric names,
  integer min/median/max ms, frozen-vocabulary tallies; no stringly
  entry point exists (metric/event names and tally values are
  allowlisted — a forged JSONL line has no parse path, proven by test).
- `crates/modelswarm-node/src/advisory/state.rs` — the PURE state
  machine `step(state, input) -> (state, effects)` (TR-RC-4/TR-AD-2):
  injected clock (virtual `now_ms` on every time-bearing input),
  injected transport (a CALL is an Effect; the runner executes it),
  runner-injected cycle nonces (RC-8's `identity::new_nonce()` pin
  lives in the runner). The 12-value gap vocabulary (golden enum +
  artifact tie), the 12-value malformed-response table
  (first-failure-wins, whole-body vs per-suggestion discard), cap
  accounting (calls INITIATED consume budget; UTC-day window with
  midnight rollover pinned as intended), min_interval cooldown,
  events_threshold gate, `late` permitted only after `timeout` for the
  same cycle (R-6), foreign `cycle_id` discarded whole. The recorded
  suggestion is the §5 ledger type: already-clamped, already-typed
  values + a Display summary carrying the RC-5
  `[advisory — untrusted, metadata-derived]` label; there is NO API
  through which production code could apply a hint.
- `crates/modelswarm-node/src/advisory/transport.rs` — the injectable
  `AdvisoryTransport` trait, the pinned reqwest adapter (https-only
  constructor refusal, `redirect::Policy::none()` via one shared
  `pinned_client()` — a 3xx returns as `HttpStatus(3xx)` ⇒ gap
  `http_error`; the request incl. Authorization is never re-sent
  elsewhere; endpoint+model pinned at construction, not per-call), the
  fixed repo-versioned digest system prompt (§3.2), and the public
  counting double the suites ride.
- `crates/modelswarm-node/src/advisory/runner.rs` — the timer-driven
  digest cycle: the ONLY production call site (tick = `min_interval_ms`
  — no new config key); snapshots accumulators, drives `step`, executes
  the single CALL with its deadline, feeds the outcome back, resets the
  aggregation window per initiated call, and emits ONLY the frozen
  `advisory.*` telemetry rows (counts/labels; the A3 filtered-stream
  discipline).

**New — the frozen contract artifacts (ADR-033 §9.4; created by this
first implementation session under revision 2's authority, then frozen
— changes only by ADR-033 amendment):**

- `protocol/advisory-egress-v1.json` — the digest-plane vocabulary of
  record (R-1): the §6.5 effective registry (16, ADR-028 §5-amended),
  `finishReason`, `served_by`, divergence verdicts, `CancelRound`
  reasons, the 12 gap values, the 4 suggestion types, confidence, the
  `serving.session_ended` prefix code set, the mechanical §3.2
  stringly-field reduction table, the aggregation event allowlist, and
  the plane's three telemetry events. COPIED, not referenced. The
  ADR-037 §4.2 rows (assistant context-block fields, `plane` enum,
  `assistant.turn.*`, `peer_pattern_note`/`catalog_presentation_hint`,
  `rate_limited`/`request_oversized`) are deliberately ABSENT — they
  land with the assistant leg.
- `protocol/advisory-clamp-table-v1.json` — the three ADR-033 §4 rows
  (RC-7/R-2), unchanged by ADR-037.

**New — the chat-wall suite (ADR-037 §4.4 item 10):**

- `crates/modelswarm-node/tests/advisory_chat_wall.rs` — Layer 1
  (`cargo metadata --no-deps`: the frozen nine-crate decision set
  declares no dependency on `modelswarm-node` (the advisory host), no
  dev-dep for gateway/session/types/eligibility/scheduler, no
  `modelswarm-advisory` crate may exist; `modelswarm-node`'s normal
  dependents are exactly {desktop, bench}, bench being the §9.2(6)
  shadow-ledger exemption — layer 2 proves bench touches no advisory
  name). Layer 2 (source scope, directional: advisory type names exist
  only in the advisory module + node `lib.rs` (the composition host) +
  `*/tests/`; zero in the gateway crate, `serving.rs`,
  `verifydrafts/**`, or anywhere else; the `send_chat`/
  `try_swarm_chat` scopes are additionally checked against the full
  advisory vocabulary AND the bare word "advisory"; the advisory
  modules reference no `chat_log`/`ChatTurn`/`chat-delta`/serving-frame/
  carrier types (W3); the enumerated-call-sites pin: `transport.send`
  executes only in `runner.rs`, `advisory::wire(` only in `Node::start`).
  Layer 3 node leg: armed plane + counting double, chat turns through
  the loopback gateway (runtime-less no_peer 503 + unknown-profile
  failure) record ZERO calls).
- `crates/modelswarm-node/tests/advisory_redaction_suite.rs` — the
  §9.2(1) frozen adversarial value set through the gate + choke point
  (prompt-shaped text, control chars, base58 PeerIds, serde fragments,
  "X->Y" composites, float timings, sk-/JWT/Bearer/hex blobs, overlong,
  unicode/RTL): raw adversarial bytes never appear in any serialized
  digest; peer refs are exactly `^[0-9a-f]{16}$` SHA-256 prefixes; RC-3
  post-gate corruption stops the request; RC-4 forged JSONL changes
  nothing; NaN/Inf have no numeric seam; the RC-5 Display rendering is
  labeled + capped + plain.

**Modified:**

- `crates/modelswarm-node/src/lib.rs` — `pub mod advisory`; the
  `NodeConfig.advisory_transport` TEST seam (documented; production
  `None` ⇒ pinned reqwest adapter); `Node::start` wires `advisory::wire`
  (default installs are byte-silent); `NodeHandle.advisory` exposes only
  the typed accumulator feed handle.
- `crates/modelswarm-node/src/selftest.rs` — the new NodeConfig field
  (`None`).
- `crates/modelswarm-desktop/src/app.rs` — the NodeConfig initializer
  gains the seam field (`None`, production); the desktop chat-wall test
  module (`mod tests::chat_wall`): armed plane in-process over a
  counting double (fed past threshold) + `try_swarm_chat`'s
  tracker-unreachable and no-eligible-peer failure paths (local axum
  mock tracker) record ZERO advisory calls.
- `crates/modelswarm-desktop/Cargo.toml` — tokio gains `net`; axum
  dev-dependency (the mock tracker; test-only).
- `Cargo.lock` — regenerated.

## 2. Exact commands run and outcomes

- `cargo fmt --all && cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors.
- `cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings`
  (env `TAURI_CONFIG={"bundle":{"resources":[]}}`) — 0 errors.
- Feature-combo clippy (the CI check-job matrix, re-run because
  `lib.rs`/`Cargo.toml` changed):
  `--features modelswarm-node/libp2p-backend`,
  `--features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`,
  `--features modelswarm-runtime/capi-adapter`,
  `--features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend`
  — all 0 errors.
- Guards key-scan replica: `git ls-files | xargs grep -rlE 'sk-[0-9a-f]{32}'`
  (minus rust.yml) — clean; no test key string matches the pattern
  (dummy keys are `test-key-local-double`).

## 3. Test evidence

Baseline stated first: `cargo test --workspace` at HEAD `dba2a58` =
**528 passed / 0 failed / 6 ignored** (verified before any change).

- `cargo test --workspace` — **578 / 0 / 6** (+50: 38 node-lib advisory
  unit tests, 6 `advisory_chat_wall.rs`, 6 `advisory_redaction_suite.rs`).
- `cargo test -p modelswarm-desktop --features tauri-shell` — **57 / 0
  / 0** (56 baseline + the desktop chat-wall test).
- `cargo test --workspace --features modelswarm-node/libp2p-backend` —
  **601 / 0 / 7**; `--features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`
  — **644 / 0 / 7** (each exactly +50 over its own pre-change count;
  the ignored 7 include the standing env-gated live tests).
- Key named tests (all ok): `golden_defaults_pin_adr033_table_and_adr037_assistant_rows`,
  `golden_gap_enum_is_the_twelve_frozen_values`,
  `artifacts_parse_and_carry_the_frozen_digest_set`,
  `clamp_property_over_the_frozen_table`,
  `adversarial_flood_never_beats_caps_on_the_virtual_clock`,
  `utc_midnight_rollover_resets_the_cap_window`,
  `late_fires_only_after_timeout_for_the_same_cycle`,
  `malformed_suggestions_discard_and_count_siblings_parse`,
  `pinned_client_never_follows_redirects`,
  `armed_plane_runs_a_cycle_and_the_double_records_it` (the
  counter-sensitivity control — the wall's zero-call assertions are
  provably non-vacuous),
  `chat_wall_success_path_records_zero_advisory_calls` (mock runtime,
  full gateway turn, armed plane, zero calls),
  `chat_wall_runtimeless_failure_paths_record_zero_advisory_calls`,
  `layer1_decision_crates_do_not_depend_on_the_advisory_host`,
  `layer2_advisory_names_live_only_in_the_advisory_module_and_tests`,
  `layer2_chat_scopes_reference_no_advisory_surface`,
  `layer2_request_builder_modules_reference_no_chat_or_carrier_types`,
  `layer2_the_provider_call_executes_from_exactly_one_site`,
  `chat_wall_try_swarm_chat_failure_paths_record_zero_advisory_calls`
  (desktop), the six `advisory_redaction_suite.rs` properties.

Chat-wall evidence summary (the suite that must be green before any
assistant code exists — it is): layer 1 pins the dependency graph (no
decision crate reaches the advisory host; the host is consumed only by
desktop + the bench exemption, and layer 2 proves bench carries no
advisory name); layer 2 pins the source scope (advisory names exist
only in the advisory module/composition host/tests; the chat scopes
contain no advisory vocabulary at all; the advisory modules cannot name
chat/serving/carrier types; the provider call executes from exactly one
site — the digest timer); layer 3 pins behavior (armed + keyed + fed
plane, counting double: gateway success turn, runtime-less and
unknown-profile failures, tracker-unreachable and no-eligible-peer
swarm failures — ZERO calls, zero bodies, zero key presentations; with
the counter-sensitivity control proving the double records when a
cycle legitimately fires).

## 4. Assumptions made

1. **Module placement** (`modelswarm-node::advisory`, not a dedicated
   crate) per ADR-033 §9.4 — the Integrator-sign-off alternative was
   not taken. Layer 1 therefore guards the host crate directly, per
   §9.2(6)'s "which layer 1 then guards directly" clause.
2. **`modelswarm-bench`'s pre-existing optional dependency** on
   `modelswarm-node` (measure/remote/serving) is the §9.2(6)
   shadow-ledger exemption, not a wall breach; layer 2 pins that bench
   references no advisory name. If the Integrator prefers the literal
   ADR-037 item-10 wording (desktop ALONE), the bench dep is the
   Scheduler seat's to unwind.
3. **Digest-plane request-body oversize** fails closed as
   `empty_after_redaction` (nothing sent): the frozen 12-value gap
   vocabulary at this leg has no `request_oversized` — that value
   arrives with the assistant leg (ADR-037 §4.3). Documented in
   `state.rs`.
4. **The runner's tick period is `min_interval_ms`** — no ADR config
   key exists for a tick period and none was invented.
5. **Transport returns reply CONTENT bytes** (provider envelope
   stripped by the adapter); the pure state machine parses only the
   strict `advisory-suggestion/v1` schema — provider shape stays out of
   the pure core. The provider envelope shape (chat-completions) is a
   provider detail, not a protocol surface.
6. **402 ⇒ `quota`**, other non-2xx (3xx included) ⇒ `http_error`,
   network ⇒ `http_error`, per the §4 table; live-endpoint behavior is
   owner-local-only evidence (§9.2 venue matrix) — CI covers the mock.
7. **Timing-metric names are frozen** to the §3.2 reduction row
   (`probe_p50_ms`, `ttft_ewma_ms`, `total_ewma_ms`,
   `pick_predicted_ms`), pinned against the artifact — the redaction
   suite's RC-4 test caught hostile metric names becoming field names;
   the fix is that allowlist.
8. **Timestamps admit only `YYYY-MM-DDTHH:MM:SS[.f]Z`** — a charset
   check would have admitted float-formatted timings ("823.7"); the
   strict shape is the §3.2 reduction rule's intent.
9. The assistant `history_turns_sent` etc. rows are parsed and pinned
   but no consumer exists — reading them prepares ADR-037 §7 step 3
   without enabling anything.

## 5. Unresolved risks / open items

1. **Accumulator feed sites are minimal at this leg** — the runner owns
   typed accumulators but producing components do not yet record into
   them (the skeleton is honest: a default install arms nothing, and an
   armed install with no feed never reaches `events_threshold`, so no
   cycle fires). Production emit-site wiring is the natural next leg of
   the digest plane (with the desktop exposing the feed handle).
2. **The A3 corpus double-run (§9.2(7))** is pinned at the node seam
   (the plane emits only `advisory.*` rows; default installs are
   byte-silent) but the full shipped-corpus sweep (scheduler/session/
   sim/bench corpora, plane-absent vs erroring-double) is T/R's venue
   item and has not been run here.
3. **The R-1 two-sided guards tie** (tests/fixtures/advisory-egress/
   inventory + the guards completeness-tie step diffing emit-site
   vocabularies against the artifact) is Test and Release's half; the
   artifact now exists for them to mirror. The in-crate suites here
   walk the same adversarial value set.
4. **bench's optional dep** (assumption 2) — flag for the Integrator.
5. **`modelswarm-bench`'s shadow-join extension (§9.2(5))** and the
   promotion checker remain Scheduler-owned; nothing here emits
   `sched.shadow.advisory` rows yet.
6. The desktop success-path chat turn rides the node-level mock test
   (constructing `tauri::State`/`Window` headlessly is not possible);
   the desktop leg covers `try_swarm_chat`'s drivable failure paths +
   the source/dependency walls. The deep QUIC dial-failure paths are
   structurally covered by layers 1–2 but not behaviorally driven.
7. The layer-2 scanner strips only a TRAILING `#[cfg(test)] mod` from
   production files; a mid-file test module referencing advisory names
   would (safely) red the build until moved.

## 6. Suggested next task for the integrator

1. Route to Protocol + T/R review: the two frozen artifacts
   (`protocol/advisory-egress-v1.json`,
   `protocol/advisory-clamp-table-v1.json`) are now frozen — Protocol
   confirms the vocabulary rows; T/R mirrors the fixture inventory and
   adds the guards completeness-tie step (R-1) and the layer-1 guards
   step (a CI-native `cargo metadata` step can replace or duplicate the
   test-level assertion).
2. ADR-037 §7 step 3 (the assistant surface) may proceed once this
   handoff's reviews land — the chat wall is green and pinned.
3. Decide the bench-dependency question (risk 4) with the Scheduler
   seat.
