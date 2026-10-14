# Handoff — Windows Product Engineer — 2026-10-14 — ADR-037 §7 step 3 (the assistant surface)

Scope: ADR-037 §7 step 3 ONLY — the assistant surface, on top of the
landed ADR-033 rev-2 client core and the (now sound, R-2-fixed) chat-wall
suite. Two commits: **`eba5ddf`** (the protocol review's Finding R-2, the
leg's required FIRST commit, before any assistant code) and the assistant
leg. ADR-037 §7 step 4 (the `peer_pattern_note`/`catalog_presentation_hint`
management additions) is NOT in this change. No version bump, no key
rotation (ADR-037 §3.6 standing owner directive), no protocol wire-surface
change (`protocol/msp-v1.md`, `protocol/messages.proto`, `catalog/*`,
`protocol/vectors/` byte-untouched; `protocol/advisory-egress-v1.json` is
extended exactly per ADR-037 §4.2's enumerated rows).

## 1. Changed files and why

**Commit 1 — R-2 (the scanner soundness fix):**

- `crates/modelswarm-node/tests/advisory_chat_wall.rs` — the layer-2
  `production_source` strip is now sound per the review's exact spec:
  strip ONLY a `#[cfg(test)] mod` that is the file's LAST item
  (Rust-lexically brace-matched: strings, raw strings, char/lifetime
  ticks, nested comments; only whitespace/comments may follow the
  close), stack-repeated for multiple trailing modules (`egress.rs`'s
  shape), FAIL LOUDLY (a panic naming byte + fix) on a non-trailing
  marker, `\r?\n`-tolerant so a CRLF checkout cannot disable the strip,
  pinned by the synthetic-input unit fixture
  `layer2_strip_fails_loudly_on_midfile_test_modules`.
- `crates/modelswarm-node/src/remote.rs` — the sound scanner caught a
  REAL drift instance: `mod tests` sat mid-file above the
  `FailoverExecutor`/pooled-session production code, which had silently
  left every layer-2 scan since the wall suite landed. Pure relocation
  of the module to the file's end.
- `crates/modelswarm-node/src/serving.rs` — same: `lease_helpers` +
  `mod tests` sat above the F3 lease-verification code. Both relocated
  to the file's end (pure move; `lease_helpers` stays `pub(crate)`,
  still referenced by `remote.rs` tests).

**Commit 2 — the assistant surface:**

- `protocol/advisory-egress-v1.json` — the §4.2 extension as ONE edit:
  `advisory_gap` 12 → 14 values (`rate_limited`, `request_oversized`);
  the new context vocabularies (`family_state`, `family_off_reason`,
  `family_engage_label`, `governor_verdict`, `governor_gap`,
  `family_probe_verdict`, `hosting_listener_state`, `engine_backend`,
  `capacity_class`, `os_platform`, `picker_fit` — all copied, not
  referenced); the `assistant_context_block` section (the frozen field
  set + typed shapes + bounds + the picker-row sub-schema, max 8 rows);
  the `plane_enum` (`digest | assistant`); `emit_site_events` gains
  `assistant.turn.sent`/`assistant.turn.gap` as `plane_own` per the
  review's extension rule. `suggestion_type` stays 4 — the §3.3 display-
  only additions land with §7 step 4 (documented in `frozen_by`).
- `crates/modelswarm-node/src/advisory/state.rs` — `Gap` extends to the
  14-value frozen vocabulary (golden test renamed/updated); `ProviderRequest`
  gains `system_prompt: Option<&'static str>` (repo-versioned constants
  only — the digest default or the assistant prompt; never caller text);
  `parse_one_suggestion` is now `pub(super)` (shared by the assistant
  reply's `hints` parse).
- `crates/modelswarm-node/src/advisory/egress.rs` — the ONE gate grows
  the assistant context discipline: `AssistantContextSection`/
  `PlaneEnumSection` artifact parsing; the typed `ContextValue`
  (Int/Bool/Enum/DisplayName/GpuName/Version/VocabCountMap/PickerRows —
  no free-text variant by construction); `admit_context` (name must be
  on the frozen field set, value must match its declared type, forbidden
  names drop); `choke_point_rescan_assistant` (the FINAL bytes: context
  name→type re-binding — advisory A-1's defense-in-depth — history/turn
  string shapes, `scrub_content` unchanged, byte cap). A poisoned count
  map drops WHOLE (fail closed, never a partial count set — pinned).
- `crates/modelswarm-node/src/advisory/assistant.rs` (NEW) — the
  assistant-plane pure client (§3.7): `ASSISTANT_SYSTEM_PROMPT`
  (§3.2c-extended, repo-versioned); the IDLE→BUILD→SCRUB→CALL→PARSE→
  RECORD machine over the injected clock/transport with the §3.4 caps
  (300/day, 60/hour, 40/conversation, 15-min idle reset, 12 history
  turns, 48 KiB, 1024 tokens, 45 s; UTC windows; failed calls consume;
  no in-turn retry); `assistant_arm` (assistant.enabled is INDEPENDENT
  of the digest `enabled`); the §3.7 malformed table (echo mismatch →
  whole-body `schema`; tuning_hint class discarded+counted BEFORE parse
  — a provider conversation cannot mint tuning hints; empty reply with
  no parseable hints → `schema`, no fabricated fallback; response cap
  65 536 shared; `late` diagnostic only after `timeout` for the same
  turn); `AssistantPlane::spawn`/`run_turn` (the SECOND and final
  transport execution site, W2) emitting only the frozen
  `assistant.turn.*` rows. Unit tests pin the §3.4 golden defaults, the
  malformed table, the caps property on the virtual clock (item 12),
  adversarial user-text scrub, and the independent arming.
- `crates/modelswarm-node/src/advisory/transport.rs` — send uses the
  request's pinned system prompt; the counting double gains
  `assistant_echo` (echoes the plane's internally-generated
  conversation_id — a canned body never could).
- `crates/modelswarm-node/src/advisory/mod.rs` — `pub mod assistant`;
  `env_lock()` — the A-4 serial guard for env-riding tests, now
  load-bearing (the assistant leg added env-riding tests and the race
  fired exactly as the review predicted; all advisory env tests take
  the lock).
- `crates/modelswarm-node/src/advisory/config.rs` — the env-lock fix
  (the whole `key_resolution_order…` test rides the lock — its
  LOCK-FREE assertions also read the process env).
- `crates/modelswarm-node/tests/advisory_assistant_suite.rs` (NEW) —
  §4.4 item (11), node half: canary discipline through every typed
  channel (over-bound strings, unknown names, foreign enums, poisoned
  count maps, over-bound picker rows all drop; canary bytes in NO
  serialized request; the context re-matches the allowlist
  post-serialization), `scrub_content` on sk-/JWT/Bearer/hex-32 user
  text, post-gate corruption of one context field stops the request,
  honest gap rows (labels only, no key), next-turn retry.
- `crates/modelswarm-node/tests/advisory_chat_wall.rs` — extended for
  the assistant leg: `ADVISORY_NAMES` gains the assistant type names;
  the layer-2 sanctioned homes gain the desktop assistant module
  (`crates/modelswarm-desktop/src/app/assistant.rs`, the §4.5 routing —
  `app.rs` itself stays unexempted so the chat scopes remain clean);
  the W2 enumerated pin now asserts `.send(&request` executes in
  exactly {assistant.rs, runner.rs} (two entry classes; a third needs
  an ADR-037 amendment).
- `crates/modelswarm-desktop/src/app/assistant.rs` (NEW) — the PANEL
  surface (§3.2/§3.6/§4.5): the §3.6 texts VERBATIM (disclosure,
  provenance chip, degraded line with the gap label); the typed
  context-block builders (§3.5, W3-typed — `ContextInputs` has no
  member that could carry a conversation/frame/carrier; picker rows
  from the `list_models` cache with structured `quant_method`/
  `quant_bits` twins of the rendered line); the static-help corpus
  (every FamilyStatus off-reason, governor label incl. enforcement
  + gap, ladder label, probe verdict, fit tier, and all 14 gap labels
  — coverage pinned against the FROZEN vocabularies via the node's
  parsed artifact); the deterministic word-boundary static lookup; the
  reply renderer (plain text, 4000-char cap with the honest marker);
  the `Plane` wrapper (app.rs stays advisory-name-free) and the
  `turn()` driver (disclosure gate → cloud/opt-out/degraded paths).
- `crates/modelswarm-desktop/src/app.rs` — `mod assistant`;
  PersistedConfig + writer + Default gain `assistant_acknowledged`
  (default FALSE — the first cloud call is blocked until the §3.6
  disclosure is acknowledged) and `assistant_enabled` (default TRUE —
  the Settings opt-out; the §3.4 master switch stays advisory.json's);
  Inner gains the plane handle, the degrade gap, the last-cards cache,
  and the four honest fact fields (`last_engage_reason` — stored at the
  engage checkpoint and `elected_fell_back`; `last_governor_verdict` —
  via `FamilyStatus.governor_label` from the hosting-start check;
  `last_probe_verdict` — the probe's pairing label;
  `capacity_class` — the challenge/complete response's ADR-012 label);
  `list_models` cards gain the typed quant twins + cache; get_status
  carries the `assistant` object (mode/gap/acknowledged/enabled/
  disclosure); four IPC commands (`assistant_turn`, `assistant_reset`,
  `assistant_acknowledge`, `set_assistant_enabled`) registered;
  `assistant-turn`/`assistant-reset` webview events (the W4 namespace);
  tests: the §4.4 item-13 disclosure/opt-out/degraded suite, the
  item-11 desktop canary (transcript + carrier-payload canaries in
  process state; absent from every captured body), the item-14 A3
  extension (census/fit byte-identical; telemetry byte-identical after
  filtering `advisory.*`/`assistant.*`; the plane's own rows all
  `assistant.*`), and the QUIC dial-failure wall leg.
- `crates/modelswarm-desktop/ui/index.html` — the assistant panel (its
  OWN section, OWN input, OWN `assistant-*` namespace): the mode line,
  the disclosure + acknowledge button, the AD-3 local-facts line (every
  cloud reply renders adjacent to the local status facts it may name),
  the transcript (textContent only — no markdown/HTML parsing, JS-side
  4000-char clip), the always-adjacent provenance chip, built-in help
  rendering, the Settings cloud-assistant toggle; the JS renders from
  BOTH the invoke return and the `assistant-turn` event and NEVER
  touches `#transcript` (chat-delta only).
- `crates/modelswarm-desktop/ui-harness/assistant.html` (NEW) — the
  rendered-UI harness (the standing 1152×648 rule): scenarios A–E
  (disclosure verbatim + blocking, plain-text/cap/chip/AD-3, degraded
  static help, the opt-out toggle, the namespace/W4 isolation).

## 2. Exact commands run and outcomes

- `cargo fmt --all` + `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors.
- `cargo clippy -p modelswarm-node --all-targets --features verifydrafts -- -D warnings` — 0 errors.
- `cargo clippy -p modelswarm-node --all-targets --features verifydrafts,libp2p-backend -- -D warnings` — 0 errors.
- `TAURI_CONFIG={"bundle":{"resources":[]}} cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` — 0 errors.
- Guards key-scan replica (`git ls-files | xargs grep -lE 'sk-[0-9a-f]{32}'`, minus rust.yml) — clean.

## 3. Test evidence

Baseline stated first (before any change of this leg): `cargo test
--workspace` at HEAD `e9db904` = **579 passed / 0 failed / 6 ignored**
(re-verified); desktop tauri-shell baseline **57 / 0 / 0**.

- `cargo test --workspace` — **602 / 0 / 6** (+23 over baseline: 18
  node-lib advisory-side additions [assistant machine + gate + config],
  4 `advisory_assistant_suite.rs`, 1 R-2 fixture already counted in
  commit 1's 7).
- `cargo test -p modelswarm-desktop --features tauri-shell` — **69 / 0 /
  0** (57 baseline + 5 assistant-module + 6 assistant-panel + 1 QUIC
  dial-failure).
- `cargo test -p modelswarm-node` (5 consecutive runs) — **97 / 0 / 3**
  every run (the A-4 env-race fix holds).
- `cargo test --workspace --features modelswarm-node/libp2p-backend` and
  `--features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`
  — see the completion report numbers (both green).
- Rendered-UI verification: `python -m http.server 4817` +
  `ui-harness/assistant.html` (the 1152×648 harness page, committed) and
  a jsdom behavioral run over the REAL `ui/index.html` (the T/R seat's
  ADR-036 venue; headless Edge remains dead on this host per the
  standing `10d8ff1` note, and the browser MCP tool is unavailable in
  this session): **21/21 PASS** — disclosure VERBATIM + first-call
  blocking, plain-text-only rendering of markup-hostile replies, the
  4000-char clip (4012 = 4000 + the 12-char marker), the chip VERBATIM,
  the AD-3 facts line, degraded static help with the gap label, the
  opt-out write-through, both `assistant-*` listeners, and W4 isolation
  (assistant output never reaches `#transcript`). The in-browser
  geometry leg at 1152×648 (real layout height) rides the harness page
  for the Integrator's standing in-browser run.
- Key named tests (all ok): `layer2_strip_fails_loudly_on_midfile_test_modules`
  (R-2), `golden_gap_enum_is_the_fourteen_frozen_values`,
  `golden_assistant_defaults_pin_adr037_section_3_4`,
  `adversarial_turn_flood_never_beats_caps_on_the_virtual_clock`,
  `utc_midnight_rolls_the_daily_window`, `hourly_burst_bound_is_rate_limited`,
  `idle_reset_opens_a_fresh_conversation`,
  `tuning_hint_class_is_discarded_and_counted_the_reply_renders`,
  `oversize_request_fails_as_request_oversized_even_at_zero_history`,
  `assistant_context_gate_admits_typed_and_drops_the_rest`,
  `static_help_covers_every_frozen_vocabulary_label`,
  `disclosure_blocks_the_first_cloud_call_until_acknowledged`,
  `opt_out_yields_zero_calls_and_static_help`,
  `desktop_canary_suite_transcript_and_carrier_absent`,
  `assistant_a3_extension_plane_absent_vs_erroring`,
  `chat_wall_quic_dial_failure_records_zero_advisory_calls`,
  `layer2_the_provider_call_executes_from_exactly_one_site` (now TWO
  pinned sites), `layer2_advisory_names_live_only_in_the_advisory_module_and_tests`
  (with the desktop assistant module exemption).

Chat-wall status after the assistant leg (the suite that must stay
green): layer 1 unchanged-green; layer 2 now pins the assistant names +
the two sanctioned homes + the TWO-site W2 pin; layer 3 adds the QUIC
dial-failure leg (a real RemoteExecutor dial to a dead QUIC address →
honest local fallback → zero calls) on top of the existing success/
runtime-less/tracker legs; the layer-2 W3 scan covers the new advisory
files (no chat/carrier type names).

## 4. Assumptions made

1. **The §4.2 artifact extension scope**: the task's enumerated rows
   (context-block field set, plane enum, `assistant.turn.*` as
   plane_own, the two gap values) are in; `peer_pattern_note`/
   `catalog_presentation_hint` are NOT (§7 step 4 is a later leg —
   adding unused vocabulary prematurely would repeat the forward-
   declaration discipline the previous leg avoided). `frozen_by`
   records this.
2. **`plane_telemetry_events` stays the digest plane's three rows** —
   the task text pins the assistant rows into `emit_site_events` as
   plane_own (the classification the R-1 tie diffs); the `plane_enum`
   distinguishes the planes.
3. **Assistant history is the PLANE's bounded memory** (the last 12
   COMPLETED turns, §3.5): the idle reset closes the CONVERSATION
   (fresh turn budget, new id) but does not wipe history — the ADR's
   letter carries history per-plane, bounded. Pinned by test.
4. **The desktop Settings toggle (`assistant_enabled`, default TRUE) is
   the §3.6 "turn it off any time" control**; the §3.4 master switch
   (`assistant.enabled` in advisory.json, default FALSE) stays the
   owner's — both must be on for cloud mode. The disclosure ack
   (`assistant_acknowledged`, default FALSE) additionally gates the
   FIRST call.
5. **The context builders pass values through; the artifact gate is the
   authority** (unknown-to-vocabulary labels drop at the gate with a
   count) — the builder TYPES, it never sanitizes.
6. **`status.error_counts`/`finish_reason_counts`/`advisory_gap_counts`
   exist in the artifact and the gate but are omitted (absent = honest
   unknown) today** — the desktop does not tally these yet; the
   emit-site wiring is the digest plane's natural next leg.
7. **The jsdom venue + the committed harness page** stand in for the
   in-browser run (the standing headless-Edge-dead note); the geometry
   assertions are in `ui-harness/assistant.html` for the Integrator's
   standing in-browser pass.
8. `env_lock()` (A-4) serializes env-riding tests within each test
   binary; the two wall tests hold it across awaits deliberately
   (`clippy::await_holding_lock` allowed with a comment — the node
   start READS the env inside).

## 5. Unresolved risks / open items

1. **The A-1/R-1 T/R mirror half**: `tests/fixtures/advisory-egress/`
   inventory + the guards two-sided tie over the (now extended)
   artifact, and the CI-native layer-1 step — Test and Release's items;
   the artifact rows exist for the mirror (every `emit_site_events` row
   added has a real source emit site).
2. **The full §9.2(7) A3 corpus sweep** (scheduler/session/sim/bench
   corpora) remains T/R's venue item; this leg pinned the node/desktop
   seam (item 14's picker/ladder/telemetry-filter pins).
3. **First live assistant conversation** (provider round-trip) is
   OWNER-LOCAL ONLY evidence under `docs/verification/` per the §6
   venue matrix — not run here (no live key use in CI/agents).
4. **In-browser 1152×648 geometry pass** over `ui-harness/assistant.html`
   rides the Integrator's standing run (headless Edge dead on this
   host).
5. **The R-2 scanner is deliberately literal-token**: `#[cfg(all(test,…))]`
   and `pub(crate) mod` forms under cfg(test) are not stripped (they
   scan as production — the over-inclusive, SAFE direction; the tree
   has one such module, serving.rs's `lease_helpers`, which carries no
   advisory name).
6. The rust.yml guards step asserting webview surfaces covers ADR-036's
   names; extending it to the assistant IPC names/disclosure is T/R's
   option (the jsdom suite + harness pin them from this side).

## 6. Suggested next task for the integrator

1. Route to Protocol + T/R review: the §4.2 artifact extension rows
   (context vocabularies copied from their sources of record — spot-
   check the copies), the layer-2 exemption for the desktop assistant
   module, and the W2 two-site pin; then T/R mirrors the fixture
   inventory + two-sided tie and runs the in-browser harness geometry
   leg.
2. ADR-037 §7 step 4 (the management additions: `peer_pattern_note`,
   `catalog_presentation_hint` as a small digest-plane extension riding
   the same client) may proceed once this handoff's reviews land.
3. Digest-plane production emit-site wiring (feeding the typed
   accumulators from the real emit sites) remains the natural next leg
   of the digest plane and would populate the `status.*_counts` context
   fields honestly.
