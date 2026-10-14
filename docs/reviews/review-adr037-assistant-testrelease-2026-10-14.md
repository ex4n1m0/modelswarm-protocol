# Review — Test and Release Engineer — 2026-10-14 — ADR-037 §7 step 3 (the assistant surface), VERDICT

Seat: Test and Release (ADR-037 §6 gate seat for the §4.4 item set and
fixtures). Reviewed at HEAD `e2c6c7c` plus this review's own landing commit
(changes listed in §7; all gates re-run at the exact tree being approved —
see §6). Scope: the ADR-037 §7 step-3 assistant surface per
`docs/reviews/handoff-windows-product-engineer-2026-10-14-assistant.md`
(`60ef75b` + `eba5ddf`), the §4.4 item (11)–(14) suites, the static-help
corpus coverage, the W4 panel contract, the settings/ack gating, the
§9.2(7) full A3 corpus sweep (this review's execution), the assistant
harness browser-hang classification (this review's investigation), and
the R-1 mirror work landed at `e2c6c7c` (countersign §5).

## Verdict

**APPROVE.** The assistant surface satisfies ADR-037 §4.4 items (11)–(14)
and the §3.6/§3.7 contracts as pinned; the chat wall (item 10) stayed
green through the assistant leg; the §9.2(7) full A3 corpus sweep PASSES
(byte-identical in both postures); and the harness browser hang is
CLASSIFIED HARNESS-BUG (two defects, both fixed in this review's commit —
the product was never at fault; evidence in §4). Four findings were fixed
in-review (F-1..F-4, all in T/R-owned or harness-instrument files); two
advisories ride with named owners (§8). With this commit landed, nothing
blocks the v0.2.24 train. Two train obligations are REQUIRED before the
release notes are finalized (§9, checklist items — process, not code).

## 1. Item (11) — the assistant redaction suite: SATISFIED

Node half (`crates/modelswarm-node/tests/advisory_assistant_suite.rs`):
canary bytes through every typed input channel (over-bound GPU-name and
picker-row strings, unknown field names, foreign enum values, poisoned
count maps) reach NO serialized request; the context block re-matches the
frozen allowlist POST-SERIALIZATION (`choke_point_rescan_assistant` on the
final bytes, asserted per body); `scrub_content` holds for sk-/JWT/Bearer/
hex-32 user text (raw secret-shaped bytes in NO body); post-gate corruption
of one context field (value-type swap AND name swap) makes the re-scan
reject the body; honest gap rows with the frozen label and NO key material
in telemetry. Desktop half (`app.rs` `desktop_canary_suite_transcript_and_
carrier_absent`): a transcript seeded with canary turns AND a carrier-shaped
`swarm_lease` payload in process state; assistant turns run; canary bytes,
`promptTokens`, and the key appear in NO captured body — the structural W3
argument (the builder is fed none of it) verified behaviorally. The
counting double's sensitivity is proven by the armed-plane control tests.
All green in this review's workspace run.

## 2. Items (12)/(13)/(14) — caps, disclosure/consent, A3: SATISFIED

**(12) cadence/cap property** (`advisory/assistant.rs` tests): the 20 000-
step adversarial flood on the virtual clock bounds day ≤ 300 / hour ≤ 60,
asserts ≤ 1 Call effect per user turn (no in-turn retry) and budget
consumption on failures; UTC-midnight and UTC-hour rollovers, the
conversation turn cap, the idle reset (fresh conversation id + budget,
bounded plane history), the `request_oversized`-at-zero-history arm, and
the 14-value golden gap enum (with the §4.3 additions) are pinned.

**(13) disclosure/consent**: `PersistedConfig.assistant_acknowledged`
default FALSE / `assistant_enabled` default TRUE with serde + Default
mirrors (`app.rs:506-548`), writer pins (`app.rs:1426-1431`), round-trip
survival of unknown keys; `assistant::turn` blocks the first cloud call
until ack (`NeedsAcknowledgement`, zero calls on the double), the
acknowledge IPC persists, opt-out yields ZERO calls + static help with the
`disabled` label, degraded mode renders the corpus with the honest gap.
The §3.6 texts are VERBATIM constants pinned in Rust and re-pinned in the
rendered harness (below).

**(14) A3 extension**: the desktop seam test (`assistant_a3_extension_
plane_absent_vs_erroring`) pins census/fit byte-identity and the filtered
telemetry equality with the plane emitting only `assistant.*` rows. The
FULL corpus sweep this review added and executed: see §3.

## 3. The §9.2(7) full A3 corpus sweep — EXECUTED, PASS

New per-push test `crates/modelswarm-node/tests/advisory_a3_corpus_sweep.rs`
(ADR-037 §4.5 routes §4.4 fixture work to T/R; the node tests tree is the
established home of the wall/A3 fixtures):

- **Corpus (a) — the scheduler/shadow decision surface, FULL matrix**: 96
  deterministic candidates spanning every capacity class × NAT path ×
  batch-verify flag × slot tier (incl. ineligible 0) × staleness tier
  (incl. over-cap 1500 ms) × measured-rate tier (incl. the 0.0 → INFINITY
  arm); `select_microswarm` at want ∈ {1, 4, 8} over three token profiles,
  `predicted_single_ms` per candidate, and the `should_engage_cooperative`
  boundary swept at exact `ratio × (1 + margin)` equality ± 1e-9 for four
  margin tiers plus the frozen ADR-013 floor rejection — all through the
  PRODUCTION imports (no re-implementation).
- **The node workload corpus**: the identical runtime-less gateway turn
  set (3 × no-peer 503, 2 × unknown-profile) through a REAL `Node::start`
  in both postures; `logs/node.jsonl` windows diffed byte-for-byte after
  filtering `advisory.*`/`assistant.*` and normalizing ONLY the log `ts`,
  the ephemeral gateway port, and installation ids (declared
  normalizations; everything else exact).
- **Corpus (e)**: posture B feeds the typed accumulators past threshold
  with the five feeds_digest event names — the digest runner CYCLES
  against the erroring double DURING the workload (`advisory.cycle.gap`
  rows observed and filtered; non-vacuity asserted), and the assistant
  plane is armed in-process over its own erroring double with driven
  turns while the decision corpus is evaluated.

**Result: PASS — decision corpus byte-identical; node telemetry
byte-identical (declared normalizations); plane rows exclusively
`advisory.*`/`assistant.*`; stable across 5 consecutive runs (0.49 s
each).** Corpus families (b) session admission/coordinator, (c) the sim
pinned-verdict scenarios, and (d) bench corpora execute BELOW the advisory
host in the crate graph — the chat-wall layer-1 test pins that they cannot
name the plane, so the absent posture is their only possible posture; they
re-ran green inside this review's workspace runs. The picker corpus
(family of item 14's desktop half) is covered by the desktop A3 pattern
test plus the structural fact that `app.rs` (the picker's home) carries no
advisory name at all (layer 2, unexempted) — see advisory A-1.

## 4. The assistant-harness browser hang — CLASSIFIED: HARNESS BUG (two defects; both fixed here; product clean)

Symptom reproduced first-hand in headless Edge 155 via CDP
(`--dump-dom` remains dead on this host per the standing `10d8ff1` note —
but CDP-driven headless Edge works; see §6):

1. `Runtime.evaluate` AND `Debugger.pause` both time out against
   `ui-harness/assistant.html` — a NON-INTERRUPTIBLE main-thread wedge,
   while `adr036-carriage.html` over the same UI completes 17/17 in 3 s.
2. The local server log shows `GET /ui-harness/x` — a relative fetch of
   `x`, exactly the `src=x` of scenario B's hostile reply
   `<img src=x onerror=alert(1)> …` — proving some layer PARSED that
   string as HTML.
3. Re-running with JS dialogs auto-dismissed via CDP: exactly ONE dialog
   (`{type: "alert", message: "1"}`) and the harness completes 21/22.

Root cause: **the harness's own `render()` interpolated page-derived
`detail` text into `tr.innerHTML`** — scenario B's detail is the first 40
characters of the hostile reply INCLUDING the literal `<img src=x
onerror=alert(1)>` text. The browser parsed it, fetched `/ui-harness/x`
(404), `onerror` fired `alert(1)`, and the blocking modal wedged the tab
forever (headless + CDP, nobody dismisses). jsdom neither loads images nor
blocks on `alert`, which is why the suite completed there. A harness that
parses the hostile bytes it is testing for non-parsing self-inflicts the
exact sink the product forbids. Fix: `render()` now builds every cell via
`textContent` (F-1).

The residual FAIL (4050 vs 4012 chars) was a second harness assertion bug:
the 4000-char-cap check read `lastElementChild.textContent`, which includes
the 38-char meta line (`cloud · turn 2 · conversation 01234567`; 4012 + 38
= 4050). Fix: read the reply text node (F-2).

**The product is NOT at fault — proven by the same runs:** the harness's
own "markup-looking reply renders PLAIN TEXT ONLY (no elements parse)"
check PASSES in the real browser; no innerHTML sink exists anywhere in the
assistant path of `ui/index.html` (all six innerHTML sinks are the model
cards / peer rows / HF-search rows). Post-fix, the full rendered harness
family passes in a real browser at 1152×648 (§6).

A third, related drift found by the same browser runs: the pairing-chat
harness's Setup-section geometry bound (`< 260px`, pre-assistant) is stale
— the ADR-037 Settings row legitimately grew the section to 375px. Bound
raised to 420 with the cause recorded in-file (F-3).

## 5. COUNTERSIGN — the 91-row completing enumeration (`emit_site_events`)

**Countersigned by this seat.** Evidence: (1) the two-sided scanner PASSes
at the reviewed tree (`emit-site scan: 91 source events; 91 artifact rows;
91 inventory rows — R-1 two-sided tie: PASS`, both directions red on
drift, classifications tied to `aggregation_event_allowlist` /
`plane_telemetry_events`); (2) an INDEPENDENT raw grep over
`crates/modelswarm-node/src` + `crates/modelswarm-desktop/src` (all
`.info/.warn/.error("…" )` literals plus the `log_event` wrapper's caller
literals, WITHOUT test-module stripping — a strict superset check) yields
exactly 91 names, identical to the artifact row set both directions
(0 missed sites, 0 dead rows); (3) classification consistency verified:
feeds_digest == the 5-event aggregation allowlist exactly; plane_own ==
the 3 `advisory.*` + 2 `assistant.*` rows; 81 excluded. The
protocol-seat countersign of the same enumeration is separately requested
by the artifact note and is PENDING at this review's writing (this T/R
countersign is the test-mechanism half and does not substitute for it).

## 6. Gate matrix re-run (this seat, at the approved tree)

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | CLEAN |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 errors |
| `cargo clippy -p modelswarm-node --all-targets --features verifydrafts -- -D warnings` (vd-ALONE) | 0 errors |
| `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings` | 0 errors |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings` | 0 errors |
| `cargo clippy --workspace --all-targets --features modelswarm-runtime/capi-adapter -- -D warnings` | 0 errors |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend -- -D warnings` | 0 errors |
| `TAURI_CONFIG={"bundle":{"resources":[]}} cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` | 0 errors |
| `cargo test --workspace` | **603 / 0 / 6** (602 baseline + 1: the A3 corpus sweep) |
| `cargo test -p modelswarm-desktop --features tauri-shell` | **69 / 0 / 0** |
| `cargo test --workspace --features modelswarm-node/libp2p-backend` | **626 / 0 / 7** |
| `cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend` | **669 / 0 / 7** |
| `cargo test -p modelswarm-node` × 5 consecutive | **98 / 0 / 5** every run (A-4 env-race fix holds; ignored = the standing env-gated live tests) |
| R-1 two-sided scanner (`tests/fixtures/advisory-egress/scan_emit_sites.py`) | PASS 91/91/91 |
| rust.yml YAML parse + fixed guards layer-1 python replica + new assistant-surface greps | PASS locally (dependents exactly {modelswarm-bench, modelswarm-desktop}) |
| Rendered harnesses, real browser (headless Edge 155 via CDP, 1152×648) | `assistant.html` **22/22 ALL PASS** (post-fix), `adr036-carriage.html` 17/17, `pairing-chat.html` 14/14 (post-fix), `census` 6/6, `monitor` 9/9, `updater` 17/17 |

Standing-note amendment: headless Edge is dead ONLY in `--dump-dom` mode
on this host; CDP-driven headless Edge (`--remote-debugging-port`) works
reliably and now carries the rendered-harness family evidence — the
"browser pass rests on the Integrator" caveat can be retired for harnesses
drivable via CDP.

## 7. Changes landing with this review (all disclosed)

1. `crates/modelswarm-desktop/ui-harness/assistant.html` — F-1 (render()
   via textContent; the hang), F-2 (cap assertion reads the reply text
   node). Windows-Product-owned path edited under the dispatch's explicit
   "fix it if small" authorization; flag to that seat for acknowledgment.
2. `crates/modelswarm-desktop/ui-harness/pairing-chat.html` — F-3 (stale
   geometry bound; same authorization).
3. `.github/workflows/rust.yml` — F-4 (the CI-native chat-wall layer-1
   step was VACUOUS: it matched `d.get('package')`, a key cargo metadata
   sets only for RENAMED deps, so `dependents` was always empty and the
   assert could never fire — it printed "dependents []" while appearing
   to verify {desktop, bench}; now matches `d['name']` with kind
   semantics mirroring the in-tree layer-1 test and requires the expected
   set exactly) + F-5 (new guards step "ADR-037 assistant surfaces exist
   in the webview": 10 panel element ids + 4 IPC names, string-level like
   the ADR-036 step).
4. `crates/modelswarm-node/tests/advisory_a3_corpus_sweep.rs` — NEW: the
   §9.2(7) full corpus sweep (§3), per-push in the default workspace run.

## 8. Findings summary

| # | Severity | Where | What |
|---|---|---|---|
| F-1 | FIXED in-review | `ui-harness/assistant.html` (render) | Harness innerHTML sink parsed the hostile reply detail → img fetch + `alert(1)` blocking modal → non-interruptible real-browser hang. Product clean (plain-text check passes; no sink in the assistant path). |
| F-2 | FIXED in-review | `ui-harness/assistant.html` (scenario B cap check) | Assertion read reply+meta (4050) not the reply node (4012). |
| F-3 | FIXED in-review | `ui-harness/pairing-chat.html:158` | Pre-assistant `< 260px` Setup bound stale (assistant Settings row → 375px measured); bound 420, cause recorded. |
| F-4 | FIXED in-review | `.github/workflows/rust.yml` (guards, chat-wall layer 1) | Vacuous python (`d.get('package')` never set for non-renamed deps — assert unreachable, printed `dependents []`). Now `d['name']` + kind + exact-set. |
| F-5 | ADDED in-review | `.github/workflows/rust.yml` (guards) | ADR-037 webview-surface existence step (10 ids + 4 IPC names). |
| F-6 | ADDED in-review | `crates/modelswarm-node/tests/advisory_a3_corpus_sweep.rs` | The §9.2(7) full corpus sweep, per-push. |
| A-1 | Advisory (Windows Product) | desktop item-14 test | Picker A3 corpus is a 2-card sample + census stub; extending it over the full 15-profile `catalog/candidate-profiles/` corpus × a hardware matrix would broaden corpus breadth (no new guarantee class — `app.rs` is layer-2-exempt from advisory names entirely, so the picker is structurally blind to the plane). Optional hardening. |
| A-2 | Advisory (Windows Product) | `ui-harness/adr036-carriage.html` (render) | Shares the old innerHTML render() shape; no hostile-text source exists there today (details are daemon-verbatim trusted texts). Harden opportunistically when next edited. |
| A-3 | Advisory (recorded) | protocol artifact note | The protocol-seat countersign of the 91-row enumeration remains PENDING (§5). |
| A-4 | Advisory (Windows Product) | `advisory/assistant.rs` (`run_turn` tail) | The `outcome.unwrap_or(Gap::EmptyAfterRedaction)` fallback is unreachable by construction (every step path emits an effect); harmless, worth a comment when next touched. |

## 9. v0.2.24 train obligations (REQUIRED before release notes finalize)

1. **Restate the deferred key rotation** in the release checklist
   (ADR-037 §8: "must be restated in every release checklist that carries
   advisory wiring") — this train carries advisory + assistant wiring.
2. **Name the assistant surface honestly in the notes**: ships in every
   install in static-help mode; cloud mode requires a locally configured
   key AND the verbatim first-use disclosure acknowledgement; the panel
   sends what the user types there plus typed machine facts (never swarm
   chats); opt-out is a Settings flip with zero functional loss. Carry
   over the standing v0.2.24 items from `db3c5cc` (both ADR-036
   disclosures verbatim, the slower-by-design honesty, the
   default-on-carriage behavior change for self-updaters).

## 10. Assumptions

1. The harness/pairing-chat edits (§7.1–7.2) are review-seat instrument
   fixes under the dispatch's explicit authorization; Windows Product
   acknowledgment is requested via the handoff, not assumed.
2. The A3 file-sink comparison's declared normalizations (ts, ephemeral
   port, installation ids) are the minimal set that makes cross-boot
   byte-comparison meaningful; any future row that embeds a boot-varying
   value outside these will (correctly) red the sweep.
3. The v0.2.24 train carries ADR-036 + ADR-037 (steps 1–3) only; §7 step 4
   (`peer_pattern_note`/`catalog_presentation_hint`) is NOT on the train
   and is not gated by this review.
4. The 91-row enumeration's protocol-seat countersign (A-3) is expected
   before the artifact's freeze note is considered fully discharged; this
   APPROVE does not waive it.

## 11. Risks

- The rendered-harness evidence rests on headless Edge 155 via CDP on one
  host; the standing in-person browser pass remains the owner's option.
- The assistant plane's live provider round-trip remains OWNER-LOCAL-ONLY
  evidence (§6 venue matrix) — NOT run here, NOT blocking per the ADR.
- The sweep's digest-cycle leg depends on a 400 ms window (min_interval
  30 ms); stable across repeated runs, but a very loaded CI runner could
  starve it — the failure mode is an honest red (non-vacuity assert), not
  a silent pass.

## 12. Handoff

- Integrator: land this review's commit; route §7.1–7.2 to Windows
  Product for acknowledgment; request the protocol-seat countersign
  (A-3); green-light v0.2.24 with §9's checklist items.
- Windows Product: A-1/A-2 advisories; A-4 comment.
- Scheduler: no action (no bench change in this review).
