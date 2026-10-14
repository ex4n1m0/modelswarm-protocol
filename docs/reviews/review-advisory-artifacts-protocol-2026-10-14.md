# Review — Protocol Architect — 2026-10-14 — ADR-037 §7 steps 1–2 artifacts + chat-wall gate for step 3

Seat: Protocol Architect (contract owner of `protocol/advisory-egress-v1.json` and
`protocol/advisory-clamp-table-v1.json` per ADR-033 §9.4 revision-2 routing).
Input: handoff `docs/reviews/handoff-windows-product-engineer-2026-10-14-advisory.md`
at HEAD `429103f`. Scope: (1) confirm the two NOW-FROZEN contract artifacts, (2) rule
the chat-wall suite against ADR-037 §4.4 item 10 for the step-3 (assistant surface)
gate, (3) rule the bench-dependency question. This review is the Protocol-side
confirmation the handoff routes; where it specifies artifact-row corrections, those
corrections are the creation-confirmation completing §9.4's "created by the first
implementation session … then frozen" — after they land, the freeze attaches and
later changes require an ADR-033 amendment.

## Verdict

**APPROVE-WITH-REQUIRED-CHANGES** for the ADR-037 §7 step-3 gate. Two required
changes (Findings R-1, R-2 below) must land before any assistant-surface code; both
are small and neither invalidates the landed digest-plane core. Everything else is
approved as-is or rides with the assistant leg.

## 1. `protocol/advisory-egress-v1.json` — APPROVED with one required completion

**Value-typed discipline (ADR-033 §3.2): CONFIRMED.** The `contract.value_types`
surface is exactly the six typed shapes — `integer`, `boolean`, `timestamp_utc`,
`enum` (VALUE-matched against a frozen vocabulary named in the artifact),
`peer_ref_16hex`, `profile_ref_16hex` — with NO free-text/string class. RC-1 is
correctly specified for both reference forms (first 16 hex of SHA-256 over the full
base58btc PeerId string; the 16 hex following `msp1:`; raw base58 never emitted),
and the compiled gate enforces `^[0-9a-f]{16}$`
(`crates/modelswarm-node/src/advisory/egress.rs:214-218`, `:257-271`). Typed
accumulators are the only aggregation source
(`crates/modelswarm-node/src/advisory/accum.rs:76-121`: event names allowlisted,
timing metric names frozen to the reduction row, tally values frozen to vocabularies
— there is no stringly entry point, so a forged JSONL line has no parse path, which
is RC-4's structural form). Zero-surviving-fields and the RC-3 choke-point re-scan
are carried in `contract.zero_surviving_fields` / `contract.choke_point` and
implemented (`egress.rs:342-384`). Counts/labels only; no content-bearing field
type exists at the digest plane.

**Vocabulary copies: VERIFIED against sources of record, byte-accurate.**
- `msp_v1_section_6_5_error_registry` (16 values): the 13 base values at
  `protocol/msp-v1.md:327-330` plus `invalid_lease`/`bad_frame`/`executor_error`
  from the ADR-028 §5 amendment recorded at `protocol/msp-v1.md:323-325` and
  `:376-377`. Exact match.
- `finish_reason` (`stop|length|cancelled|error`): `protocol/msp-v1.md:305`. Exact.
- `served_by` (`local|swarm|failed`): the shipped provenance labels
  (`crates/modelswarm-desktop/src/app.rs:4924`, `:4383`, `:3706`). Exact.
- `divergence_verdict` (`match|divergence`): `AuditVerdict`
  (`crates/modelswarm-session/src/verifydrafts.rs:2332-2339`). Exact.
- `cancel_round_reason` (`parent_mismatch|round_mismatch|commit_mismatch`):
  ADR-032 §2 frozen strings (`docs/adr/ADR-032-batched-speculative-verification.md:518-519`).
  Exact.
- `advisory_gap` (12 values) and `suggestion_type` (4) + `confidence` (3): ADR-033
  §6 / §2 / §4. Exact; the golden enum ties the artifact, the `Gap` enum, and the
  ADR (`crates/modelswarm-node/src/advisory/state.rs:29-81`;
  `egress.rs:436-471` asserts the ADR-037 §4.3 values are ABSENT — premature
  addition reds the build; correct forward-declaration discipline).
- `serving_session_ended_code` (17 values = `recv` + `invalid_lease` + the 16-value
  registry): justified by the actual composite error returns at
  `crates/modelswarm-node/src/serving.rs:314` (`recv: {e}`), `:366`
  (`invalid_lease: {why}`), plus registry-coded returns (`:341`, `:354`, `:378`).
  The one uncoded shape (`:322`, "expected InferenceRequest, got {variant}") has no
  colon, so the whole string is the prefix, matches nothing, and drops with a count
  — fail-closed exactly per the rule.
- `reduction_rules`: all eight ADR-033 §3.2 table rows carried, including
  `per_site_code_sets` "initially EMPTY" for `error`/`detail` — the faithful reading
  of "drop by default; a site's errors enter only via a per-site frozen code set
  added … by amendment".

**Aggregation allowlist (5 events): all five exist at real emit sites** —
`serving.session_ended` (`serving.rs:114`, `verifydrafts/composed.rs:529`),
`serving.clamped` (`serving.rs:382`), `serving.refused` (`serving.rs:96`,
`composed.rs:510`), `verifydrafts.verifier_divergence`
(`verifydrafts/mod.rs:181-185`), `engine.gpu_fallback` (`lib.rs:321`). No dead
rows. The 5-event feed set is conservative and maps to ADR-033 §2.1's triage
inputs; the choice is a contract decision properly recorded in the
Protocol-owned artifact.

### Finding R-1 (REQUIRED) — the artifact is not yet mirrorable for the R-1 two-sided tie

ADR-033 R-1: "the guards completeness tie diffs the source emit-site vocabulary
against BOTH — a new event name without both a manifest row and a vocabulary row is
a red build", and §9.2(1) requires "one fixture row set per emitted event type".
The artifact's event-name surface is `aggregation_event_allowlist.events` (5) +
`plane_telemetry_events` (3) = 8 names. The production source emits ~34 distinct
event names (verified): node — `store.opened`, `tracker.configured`, `gateway.failed`,
`engine.gpu_fallback`, `identity.loaded`, `identity.created`, `serving.session_ended`,
`serving.clamped`, `serving.refused`, `serving.stopped`, `verifydrafts.verify_ms_lie`,
`verifydrafts.verifier_divergence`, `verifydrafts.budget_refused`,
`net.metrics.store_unavailable`, `net.metrics.invalid_key`,
`net.metrics.persist_failed`; desktop — `listener.error`, `listener.bound`,
`family.pin`, `family.fallback`, `family.engine_ready`, `family.enabled`,
`family.probe`, `family.engage`, `chat.cleared`, `chat.swarm.error`,
`chat.swarm.fallback`, `chat.swarm.empty`, `lease.earn`, `sched.shadow.policy`,
`sched.shadow.decision`, `sched.shadow.realized`; plane — the three `advisory.*`
rows. As frozen, Test and Release cannot build a green symmetric mirror: the tie
would be permanently red, or T/R would have to narrow it (weakening R-1's
new-event-name guarantee).

**Required correction (Protocol-specified; apply before T/R starts the mirror and
before the assistant leg's §4.2 extension, so the artifact extends an already-
mirrorable base).** Add one section to `protocol/advisory-egress-v1.json`:
`emit_site_events` — the EXHAUSTIVE event-name vocabulary over the node/desktop
production telemetry emit sites (classification: `feeds_digest` (also listed in
`aggregation_event_allowlist.events`) | `excluded` (never feeds a digest at this
leg) | `plane_own` (advisory.* — excluded from aggregation by RC-4); reclassification
or a new row is an ADR-033 amendment). Seed rows (verified by this review):
the 5 feeds_digest events; excluded — `serving.stopped`,
`verifydrafts.verify_ms_lie`, `verifydrafts.budget_refused`,
`net.metrics.store_unavailable`, `net.metrics.invalid_key`,
`net.metrics.persist_failed`, `store.opened`, `tracker.configured`,
`gateway.failed`, `identity.loaded`, `identity.created`, `listener.error`,
`listener.bound`, `family.pin`, `family.fallback`, `family.engine_ready`,
`family.enabled`, `family.probe`, `family.engage`, `chat.cleared`,
`chat.swarm.error`, `chat.swarm.fallback`, `chat.swarm.empty`, `lease.earn`,
`sched.shadow.policy`, `sched.shadow.decision`, `sched.shadow.realized`;
plane_own — `advisory.cycle.sent`, `advisory.cycle.gap`,
`advisory.suggestion.recorded`. The completing enumeration is the mechanical
source scan T/R's tie step performs (any name the scan finds beyond this list is
added as `excluded` in the same confirmation edit; after freeze, by amendment).
Accompanying code change (additive, no gate-semantics change): extend
`EgressVocabulary` in `crates/modelswarm-node/src/advisory/egress.rs:37-47` with
`#[serde(default, rename = "emit_site_events")] pub emit_site_events:
BTreeMap<String, String>` and add a golden tie test (artifact keys == the verified
source set; `feeds_digest` rows == `aggregation_event_allowlist.events`). The JSON
section alone does not break the compiled gate (serde ignores unknown fields), so
the artifact edit can land first. `aggregation_event_allowlist` and
`plane_telemetry_events` remain as-is (the classification references them).

## 2. `protocol/advisory-clamp-table-v1.json` — APPROVED as-frozen, no changes

Exactly the three ADR-033 §4 / RC-7 rows: `MSP_SHADOW_QUEUE_ADVERTISED_WEIGHT`
[0.0, 1.0], `MSP_SHADOW_QUEUE_MEASURED_WEIGHT` [0.0, 1.0],
`MSP_SHADOW_QUEUE_CAP_MS` [1000.0, 120000.0], each with source-of-truth citation and
`promise_metric: ttft_p50_ms (down)`. Verified against
`crates/modelswarm-scheduler/src/shadow.rs:77-85`. Absent-target behavior is
structurally `requires_adr`, never an improvised bound
(`egress.rs:405-422` `HintAdmission::RequiresAdr`; pinned by
`clamp_property_over_the_frozen_table` at `egress.rs:626-629`). `frozen_by`
correctly records UNCHANGED-by-ADR-037 §4.2. No fourth row exists. Approved.

## 3. The chat-wall suite vs ADR-037 §4.4 item 10 — satisfied in substance; one required soundness fix

**Layer 1 (dependency direction): SATISFIED.**
`layer1_decision_crates_do_not_depend_on_the_advisory_host`
(`crates/modelswarm-node/tests/advisory_chat_wall.rs:136-218`) bars the frozen
nine-crate decision set from any dependency on `modelswarm-node` (the advisory host
under the sanctioned `modelswarm-node::advisory` module placement), bars any
`modelswarm-advisory` dedicated crate, bars dev-deps for the five §9.2(6)-named
crates, and asserts the host's normal dependents are exactly {desktop, bench}. The
bench edge is ruled on in §4 below.

**Layer 2 (source scope, directional): SATISFIED in coverage; one soundness fix
required.** Advisory names are confined to the advisory module + the composition
host `lib.rs` + `*/tests/` (`advisory_chat_wall.rs:225-269`); the named hot zones
(gateway crate, `serving.rs`, `verifydrafts/**`) are re-checked explicitly; the
`send_chat`/`try_swarm_chat` scopes are extracted and checked against the full
advisory vocabulary AND the bare word "advisory" case-insensitively (`:294-314`)
— the chat path must not know the plane exists, which is W1's strongest
source-level form; the advisory modules cannot name chat/carrier/serving types
(`:317-346`; forbidden set includes `chat_log`, `ChatTurn`, `chat-delta`,
`prompt_tokens`, `pin_commitment`, `InferenceRequest`, `WireMessage`,
`GatewayState`, `render_chatml`); the enumerated-call-sites pin holds
(`:349-385`: `transport.send(&request` executes only in `runner.rs`;
`advisory::wire(` only in `lib.rs`; `ReqwestTransport::new(` only inside the
advisory module).

### Finding R-2 (REQUIRED) — the layer-2 scanner's test-module strip is unsound for mid-file modules

`advisory_chat_wall.rs:54-71` (`production_source`): `text.find("#[cfg(test)]\nmod ")`
strips from the FIRST marker to EOF. If a production file ever carries a mid-file
`#[cfg(test)]` module, ALL production code after it silently leaves every layer-2
scan — the under-inclusive (unsafe) direction. The doc comment at `:48-53` and
handoff risk 7 claim the opposite ("the scanner then flags it, which errs on the
safe side"); the code does not do that. Today no exploit exists (all test modules
in the scanned set are trailing, and the assertions hold), so this is drift
protection — but the assistant leg is precisely the incoming change this scanner
polices, so it must be sound before that code lands. Fix: strip only a module that
is the file's last item (brace-match `mod X { … }` from the marker and require only
whitespace/comments after its close), FAIL LOUDLY on a non-trailing marker, and pin
with a unit fixture (a synthetic mid-file-marker file must make the helper error).
Optional hardening while there: match `\r?\n` so a CRLF checkout cannot silently
disable the strip (CRLF failure direction is over-inclusive, i.e. safe, but pinning
it is free).

**Layer 3 (behavioral zero-calls): SATISFIED for the gate.** Success path
(`crates/modelswarm-node/src/advisory/mod.rs:179-249`: full gateway turn over the
mock runtime, plane armed + keyed + fed past threshold, zero calls/bodies/key
presentations), the local-fallback surface (the same loopback gateway),
runtime-less failures (`advisory_chat_wall.rs:434-484`: no_peer 503 +
unknown-profile), and `try_swarm_chat` tracker-unreachable + no-eligible-peer
(`crates/modelswarm-desktop/src/app.rs:6904-6981`). Non-vacuity is proven by the
counter-sensitivity control (`mod.rs:252-313`: the armed, fed plane DOES record a
cycle on the double). The deep QUIC dial/mid-stream failure paths of
`try_swarm_chat` are NOT behaviorally driven (handoff risk 6) — item 10's letter
says "every failure path". Ruling: acceptable to ride with the assistant leg,
because the untested paths run through the same source already proven name-free at
layer 2 and cannot reach the plane through any transport site other than the
digest runner (the W2 pin); a behavioral dial-failure leg SHOULD land with the
item-11 canary work to close the letter. This is a recorded follow-up, not a
step-3 blocker.

**Must close BEFORE the assistant surface starts:** Findings R-1 and R-2 (R-2 may
be the first commit of the step-3 leg, before any assistant code exists; R-1 lands
before T/R's mirror and before the §4.2 artifact extension).
**Rides WITH the assistant leg:** the QUIC-failure behavioral legs; the CI-native
guards layer-1 step, the R-1 tie step, and the fixture inventory (T/R's half);
the full §9.2(7) A3 corpus sweep (T/R venue; the node-seam pin stands per the
handoff); advisories A-1..A-4 below.

## 4. Ruling — the bench exemption (modelswarm-bench's optional dep on modelswarm-node)

**ACCEPTABLE under item 10; the Scheduler seat does NOT unwind it.** Grounds:

1. The edge predates the advisory plane and serves another purpose:
   `crates/modelswarm-bench/Cargo.toml:46` declares `modelswarm-node` `optional =
   true`, gated behind the `quic-runner` feature whose comment pins its role —
   "transport-backed runner over REAL serving bridges (modelswarm-node
   `serve_sessions` over QUIC, the F15/shadow test seam)". It is the 9.6 pass-1
   measurement binding, not advisory reach.
2. ADR-033 §9.2(6) records the exemption in the same mechanism item 10 extends:
   "`bench` exempt as the sanctioned shadow-ledger consumer", and §9.2(6) layer
   2's allowlist already permits the suggestion type name in `modelswarm-bench`
   and the desktop display renderer. The ADR family contemplates bench as a
   sanctioned advisory-adjacent consumer.
3. Item 10's phrase "the advisory crate is depended on ONLY by
   modelswarm-node/modelswarm-desktop" was authored against the dedicated-crate
   contingency; under the actual sanctioned module placement, its object is the
   advisory HOST (`modelswarm-node`), and the host's dependents are {desktop,
   bench-optional}. The threat item 10 polices — chat/serving surfaces reaching the
   provider, advisory reaching decision crates — is unaffected: bench is not in the
   nine-crate decision set, is not in any serving path (it DRIVES serving bridges
   to measure them), and layer 2 proves bench sources contain zero advisory names
   today.

Standing conditions of this ruling: (a) layer 2's bench advisory-name prohibition
stays enforced — when the Scheduler seat lands the §9.2(5) shadow-join extension
(`sched.shadow.advisory` rows) and legitimately needs advisory names in bench, that
is sanctioned by ADR-033 §9.2(6) layer 2's recorded allowlist and the scanner
allowlist extends WITHOUT a new ADR (see Advisory A-3); (b) any NON-optional bench
dependency on node, or any third dependent of node, reds layer 1 — correct;
(c) this recorded interpretation should be folded as a one-line note in the next
ADR-033/037 amendment that touches these sections, so letter and mechanism stay
aligned.

## Findings summary

| # | Severity | Where | What |
|---|---|---|---|
| R-1 | REQUIRED before T/R mirror + assistant leg | `protocol/advisory-egress-v1.json:151-165` | Event-name surface not exhaustive (8 rows vs ~34 source names); R-1 tie not mirrorable. Add `emit_site_events` (spec in §1) + additive `EgressVocabulary` field + golden tie test. |
| R-2 | REQUIRED before assistant code lands | `crates/modelswarm-node/tests/advisory_chat_wall.rs:54-71` | Mid-file `#[cfg(test)]` strip is under-inclusive (unsafe direction); doc comment `:48-53` and handoff risk 7 state the opposite. Brace-match trailing-only strip + loud failure + fixture pin. |
| A-1 | Advisory (ride with item 11) | `crates/modelswarm-node/src/advisory/egress.rs:342-384` | Choke-point re-scan re-checks value shapes, not name→vocabulary binding. RC-3's letter is met; pin name bindings in the item-11 canary suite as defense-in-depth. |
| A-2 | Advisory | `crates/modelswarm-node/src/serving.rs:96`, `verifydrafts/mod.rs:191` | `serving.refused` `reason` / `verifydrafts.budget_refused` `code` are stringly with no reduction row or vocabulary — safe today by construction; if ever tallied, a frozen code set by amendment (RC-2). |
| A-3 | Advisory | `crates/modelswarm-node/tests/advisory_chat_wall.rs:102-103` | The in-test comment "extend ONLY via an ADR-037 amendment" overstates: ADR-033 §9.2(6) layer 2 already sanctions advisory names in `modelswarm-bench` and the desktop display renderer. Correct the comment at the Scheduler shadow-join leg; no ADR needed for those two sites. |
| A-4 | Advisory | `crates/modelswarm-node/src/advisory/mod.rs:188/:204/:266-268`, `tests/advisory_chat_wall.rs:409/:423`, `desktop/src/app.rs:6885/:6892` | `MSP_ADVISORY_KEY_FILE` set/remove across parallel tokio tests is racy within a binary (green today, 578/0). Add a serial guard or per-test key paths when the assistant leg adds more env-riding tests. |

## Evidence

Read and verified: `docs/adr/ADR-033-advisory-plane.md` (rev 2 + ADR-037 note),
`docs/adr/ADR-037-assistant-surface.md` (§3.1, §4.2–§4.4, §6, §7),
`docs/reviews/handoff-windows-product-engineer-2026-10-14-advisory.md`,
`protocol/advisory-egress-v1.json`, `protocol/advisory-clamp-table-v1.json`,
`protocol/msp-v1.md` §6.3/§6.5/§9, ADR-032 §2 (`:518-519`),
`crates/modelswarm-node/tests/advisory_chat_wall.rs`,
`crates/modelswarm-node/src/advisory/{mod,egress,accum,state,config,runner}.rs`,
`crates/modelswarm-node/src/serving.rs`, `crates/modelswarm-node/src/verifydrafts/mod.rs`,
`crates/modelswarm-session/src/verifydrafts.rs`, `crates/modelswarm-scheduler/src/shadow.rs`,
`crates/modelswarm-desktop/src/app.rs`, `crates/modelswarm-bench/Cargo.toml`.
No commands executed; no files modified (read-only seat). Test evidence relied
upon: the handoff's `cargo test --workspace` 578/0/6 (+50) and the desktop
tauri-shell 57/0/0, with the named wall tests green; the layer-1 dependents
assertion is itself green and authoritative for the {bench, desktop} fact.

## Assumptions

1. The artifact corrections in §1 are applied by the Integrator (or by this seat
   once write access is granted) as the creation-confirmation; the freeze attaches
   after they land.
2. The chat-wall tests remain in their current homes — rehoming them does not
   weaken the gate but must keep the trailing-module convention or fix R-2 first.
3. ADR-037 §4.2's artifact extension (assistant rows) lands as one edit that
   includes the R-1 completion, not before it.

## Handoff

- Integrator: commit this review; apply Finding R-1's artifact correction (or
  grant this seat write access to `protocol/` for it); then green-light ADR-037
  §7 step 3 for the Windows Product seat with R-2 as the leg's first commit.
- Test and Release: after R-1 lands, build the `tests/fixtures/advisory-egress/`
  inventory + guards completeness tie against `emit_site_events` (both directions:
  source-without-row and row-without-source are red), and the CI-native layer-1
  step; schedule the §9.2(7) A3 corpus sweep.
- Scheduler: no bench unwind (§4 ruling); when the shadow-join leg extends the
  layer-2 allowlist per ADR-033 §9.2(6), fold Advisory A-3's comment fix.
