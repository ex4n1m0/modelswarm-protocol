# Handoff — Test and Release Engineer — 2026-10-14 — ADR-037 assistant review + A3 sweep + harness-hang classification

Verdict review: `docs/reviews/review-adr037-assistant-testrelease-2026-10-14.md`
(APPROVE). This handoff covers the seat's changed files, commands, and
evidence for that review plus the standing mirror-work countersign.

## 1. Changed files and why

1. `crates/modelswarm-desktop/ui-harness/assistant.html` — the browser-hang
   fixes (review F-1: `render()` now builds cells via `textContent` — the
   old innerHTML interpolation parsed scenario B's hostile-reply detail,
   fetched `/ui-harness/x`, and `onerror=alert(1)` wedged the tab in a
   blocking modal; F-2: the 4000-char cap assertion reads the reply text
   node, not reply+meta). Windows-Product-owned path, edited under the
   dispatch's explicit "fix it if small" authorization — flag for that
   seat's acknowledgment.
2. `crates/modelswarm-desktop/ui-harness/pairing-chat.html` — F-3: the
   Setup-section geometry bound 260 → 420 px (the ADR-037 Settings row
   legitimately grew it to 375 px, measured in Edge 155 at 1152×648);
   cause recorded in-file. Same authorization.
3. `.github/workflows/rust.yml` — F-4: the CI-native chat-wall layer-1
   step was VACUOUS (`d.get('package')` exists only for renamed deps, so
   `dependents` was always empty and the assert unreachable — printed
   "dependents []"); now matches `d['name']` with kind semantics mirroring
   the in-tree layer-1 test and requires the expected set exactly
   {desktop, bench}. F-5: new guards step asserting the ten assistant
   panel element ids + four IPC names exist in the webview markup.
4. `crates/modelswarm-node/tests/advisory_a3_corpus_sweep.rs` — NEW: the
   §9.2(7) FULL A3 corpus sweep (scheduler decision full matrix, node
   workload telemetry windows with declared normalizations, accumulator
   feed corpus, assistant plane in-process), per-push in the default
   workspace run.
5. `docs/reviews/review-adr037-assistant-testrelease-2026-10-14.md` — the
   verdict review (APPROVE) with all findings and the 91-row countersign.

## 2. Exact commands run and outcomes

- `cargo fmt --all && cargo fmt --all --check` — clean.
- Clippy (all green, exit 0): `--workspace --all-targets`;
  `-p modelswarm-node --all-targets --features verifydrafts` (vd-ALONE);
  workspace `--features modelswarm-node/libp2p-backend`; workspace
  `--features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`;
  workspace `--features modelswarm-runtime/capi-adapter`; workspace
  `--features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend`;
  `TAURI_CONFIG={"bundle":{"resources":[]}} cargo clippy -p modelswarm-desktop
  --all-targets --features tauri-shell`.
- `cargo test --workspace` — **603/0/6** (602 baseline + the sweep).
- `cargo test -p modelswarm-desktop --features tauri-shell` — **69/0/0**.
- `cargo test --workspace --features modelswarm-node/libp2p-backend` —
  **626/0/7**; `--features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`
  — **669/0/7**.
- `cargo test -p modelswarm-node` × 5 — **98/0/5** every run.
- `python tests/fixtures/advisory-egress/scan_emit_sites.py` — **PASS
  91/91/91** (source == artifact == inventory).
- Independent countersign grep (raw, no test-stripping) over
  node/desktop `src` — 91 names, exact set equality with the artifact rows
  both directions.
- rust.yml YAML parse + the fixed layer-1 python replica (dependents
  exactly {modelswarm-bench, modelswarm-desktop}) + the new
  assistant-surface greps — all pass locally.
- Real-browser harness runs (python http.server + headless Edge 155 via
  CDP `--remote-debugging-port`; `--dump-dom` stays dead on this host):
  `assistant.html` pre-fix — non-interruptible wedge (Runtime.evaluate AND
  Debugger.pause time out; server log shows `GET /ui-harness/x`; with
  dialogs auto-dismissed: exactly one `alert("1")`, completes 21/22 — the
  classification evidence); post-fix **22/22 ALL PASS in 4 s**.
  `adr036-carriage.html` 17/17; `pairing-chat.html` 14/14 (post-fix);
  `census` 6/6; `monitor` 9/9; `updater` 17/17.

## 3. Test evidence

See §2 and the review's §6 table. Key named results: the sweep
`a3_full_corpus_plane_absent_vs_armed_erroring` ok × 5 consecutive;
`desktop_canary_suite_transcript_and_carrier_absent`,
`disclosure_blocks_the_first_cloud_call_until_acknowledged`,
`opt_out_yields_zero_calls_and_static_help`,
`static_help_covers_every_frozen_vocabulary_label`,
`assistant_a3_extension_plane_absent_vs_erroring`,
`layer1_decision_crates_do_not_depend_on_the_advisory_host`,
`layer2_the_provider_call_executes_from_exactly_one_site` — all green in
the workspace/desktop runs above.

## 4. Assumptions

1. The dispatch's "fix it if small" covers the two harness files (they
   are review-seat instruments; the product UI and daemon code are
   untouched).
2. The A3 file-diff's declared normalizations (ts, ephemeral port,
   installation ids) are minimal; new boot-varying row fields will red
   the sweep honestly.
3. The protocol-seat countersign of the 91-row enumeration is still
   pending and is not waived by this seat's countersign.

## 5. Unresolved risks

1. Rendered-harness evidence is headless-Edge-155-via-CDP on one host
   (dump-dom dead; CDP reliable — standing note amended in the review).
2. The provider round-trip remains owner-local-only evidence (not run,
   not blocking per ADR-037 §6).
3. The sweep's digest-cycle leg uses a 400 ms window; failure mode is an
   honest red, never a silent pass.

## 6. Suggested next task for the integrator

1. Land this commit; route the two harness edits to Windows Product for
   acknowledgment; request the protocol-seat countersign; green-light the
   v0.2.24 train WITH the review's §9 checklist items (restate the
   deferred key rotation; name the assistant surface honestly).
2. ADR-037 §7 step 4 (management additions) may proceed once its reviews
   land; the digest-plane production emit-site wiring remains the natural
   follow-up leg to populate the `status.*_counts` context fields.
