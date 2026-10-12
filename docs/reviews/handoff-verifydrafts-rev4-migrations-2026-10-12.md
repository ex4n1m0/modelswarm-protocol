# Handoff — ADR-032 revision-4 contract migrations (§7 light-up item (iv))

Date: 2026-10-12. Seat: implementation session working under the Protocol
Architect's contract (binding ADR:
`docs/adr/ADR-032-batched-speculative-verification.md`, **Accepted,
revision 4**). Base: `4e8fc18` (the composed daemon, light-up tier).
Scope executed: exactly the §7 item-(iv) migration list + the
revision-4 amendment note's numbered decisions + the G6 fold routed from
`docs/reviews/handoff-windows-product-verifydrafts-compose-2026-10-11.md`.

The five migrations (each before the family carries traffic, as §7
requires):

1. **§3 offer form v3 (G3)** — done.
2. **§8e budget seconds arm (T/R advisory A-2)** — done.
3. **§8 RC-5(b) stale-commit reclassification (Security advisory A1)** —
   done, Security pin amended.
4. **§2 `PrefixCommit` carrier `sender` check (G2)** — done, pinned.
5. **§2 rule (ii) vocabulary-disjointness assertion** — done.

Plus the §7 fixture obligations they carry: `session-offer-3.json`, the
`session-reject-1.json` two-reason extension, and the commit-form golden
vector (`verifydrafts-5-commit-carrier.json`).

## 1. Changed files and why

### `crates/modelswarm-session/src/negotiation.rs`
- `SessionOffer` gains REQUIRED `profile_id: String` and
  `generation_params_hash: String` — the THIRD canonical form (ADR-032
  rev 4 §3/G3, the F-6 mechanism). Absence fails schema (serde required
  fields; pinned by the new unit test
  `session_offer_form_v3_requires_both_binding_fields` — the base and v2
  forms both fail deserialization).
- `SessionRejectReason` extends to `ProfileMismatch`/`ParamsMismatch`
  (`profile_mismatch`/`params_mismatch`) — negotiation-level vocabulary,
  §6.5 registry untouched (the frame-level `profile_mismatch` code is a
  different enum, unchanged).

### `crates/modelswarm-session/src/verifydrafts.rs` (the gate)
- **§8e seconds arm:** stage 11 now refuses when
  `served_ms + (clamped deadline_ms + WAIT_COMMIT_GRACE_MS) >
  max_endpoint_seconds * 1000` — would-exceed on BOTH arms (the seconds
  arm reserves the round's deadline+grace worst case; one-outstanding
  makes it double-count-free; the LEDGER still advances on serve-loop
  deltas). The clamped deadline is computed once before stage 11 (it was
  previously computed at stage 12); **stage ORDER unchanged** — the
  120-pair conjunction sweep passes untouched.
- **RC-5(b) stale-commit reclassification:** a commit for a round
  already abandoned (handled but uncommitted, `outstanding == None`,
  `last_committed < round <= last_handled`) rejects
  `CancelRound("round_mismatch")` with NO state change and NO
  violation-counter increment; never applied (I2). Scoped exactly: a
  commit failing RC-4 reproduction WHILE the round is outstanding
  remains a `commit_mismatch` violation toward threshold-two teardown.
- **§2 carrier sender check:** the gate pins
  `installation_id_for(coordinator_key)` at construction; a commit whose
  `sender` differs rejects `bad_frame` before any state is touched (no
  violation, nothing consumed — the round stays outstanding for a
  correctly-sent retry). Enforcement lives in the gate (the single
  authority both venues and the fixture replays share); the serve loop's
  strict parse is the other half (below).

### `crates/modelswarm-node/src/verifydrafts/serve.rs`
- Offer validation: after the preset check, the loop compares
  `offer.profile_id` vs `config.profile_id` and
  `offer.generation_params_hash` vs the locally derived params hash,
  rejecting honestly (`SessionReject` + telemetry +
  `ServeEndReason::RejectedOffer`), before backend pairing and budget.
  Verifier-side pinning (stages 4/7, §8 commit-time equality) is
  unchanged — the fields are discovery, not trusted input.
- Commit arm: the carrier parse is now STRICT per the §2 G2 letter —
  `deny_unknown_fields` (unknown fields / wrong types fail
  deserialization) plus the form check below; every failure is
  `bad_frame` before the gate is touched.

### `crates/modelswarm-node/src/verifydrafts/mod.rs`
- `WirePrefixCommit` is the FORMAL §2 carrier (rev 4): doc updated;
  `#[serde(deny_unknown_fields)]` added; new
  `passes_strict_carrier_schema()` (bare-64-hex on the three hash
  fields, `msp1:<64 hex>` profile form). New unit test
  `wire_prefix_commit_strict_schema_pins_the_g2_carrier` (valid carrier
  passes; `signature`/envelope-tag keys fail deserialization;
  `sha256:`-prefixed / short hashes / bad profile form fail the form
  check).

### `crates/modelswarm-node/src/verifydrafts/client.rs`
- `negotiate` sends the v3 offer (`profile_id` from config,
  `generation_params_hash` from the already-derived family hash). The
  two new reject reasons surface through the existing
  `ClientOpenError::Rejected` fallback-to-single path.

### `crates/modelswarm-node/src/verifydrafts/engine_source.rs` (G6 fold)
- Module docs record how the offer v3 fields interact with the
  pinned-prompt prefill: profile/params mismatch can no longer open a
  session, so the prefilled KV can never be consumed by a mismatched
  pairing; the PROMPT itself remains out-of-band with NO on-wire
  commitment (recorded, not improvised — see §4 below).

### `crates/modelswarm-session/tests/*`
- `verifydrafts_admission_properties.rs`: `budget_exhaustion_refuses_with_overloaded`
  re-pins the seconds-arm boundaries under would-exceed semantics
  (reservation = clamped 2_500 + 250 grace = 2_750 ms; admits at
  `served + 2_750 == 120_000` exactly, refuses one ms past; plus a
  clamp-ceiling-deadline long-reservation refusal). The deadline-clamp
  test moves to a wide-seconds gate (a 120 s cap can never fit a
  120_250 ms clamp-ceiling reservation — that interaction is itself the
  new arm working). `Cond::BudgetSeconds` still fires (at-cap ⇒
  over-reservation); the full conjunction sweep is unchanged and green.
- `verifydrafts_security_review_pins.rs`: the late-commit pin is
  RE-SCOPED (`pin_verifier_late_commit_after_expiry_is_exempt_stale_round_mismatch`,
  citing ADR-032 rev 4) to freeze the NEW semantics — zero violations,
  no teardown across repeated late commits, gap-forward retry admits;
  new `pin_commit_mismatch_while_result_outstanding_still_violates`
  freezes the exemption's scope; new
  `pin_carrier_sender_mismatch_is_bad_frame_before_state` pins the §2
  sender check (bad_frame, round stays outstanding, pinned-sender retry
  applies).
- `verifydrafts_vectors.rs`: `session-offer-2` flips to superseded
  failing-data; new `golden_session_offer_3_third_canonical_form`
  (round-trip + live params-hash cross-check); reject test extended to
  the six-reason vocabulary; new
  `golden_verifydrafts_5_commit_carrier_bare_object_form` (exact 8-key
  bare object, no envelope/signature, canonical bytes + sha256, §1
  rule-2 construction, sender binding, and a full gate replay: fixture-1
  request → one pass → THIS carrier applies → `Advanced`).
- `verifydrafts_coordinator_properties.rs`: `sample_offer` carries the
  v3 fields (compile ripple, no behavioral change — the coordinator
  side has no offer validation to extend).
- **NEW** `verifydrafts_vocabulary_disjointness.rs` (§2 rule (ii)):
  per-push, default features. Extracts the LIVE §6 `WireMessage`
  variant tags from serde's own unknown-variant error (a loud panic if
  serde's message shape ever changes), asserts the frozen ten, builds
  the family discriminant keys from the REAL wire shapes (proving each
  ADR-enumerated key is a live field; `accepted_token_ids` is pinned by
  the carrier golden vector — the node mirror is feature-gated), and
  asserts disjointness both for the enumerated nine and for the FULL
  live family field set.

### `crates/modelswarm-node/tests/verifydrafts_wiring.rs`
- New `offer_v3_profile_and_params_mismatch_reject_honestly` over the
  real staged transport: mismatched profile → `profile_mismatch`, wrong
  params hash → `params_mismatch`, both honest `SessionReject`s with
  `!accepted` and the matching `ServeEndReason`.
- The manual-offer test's `SessionOffer` carries the v3 fields.

### `crates/modelswarm-session/examples/gen-verifydrafts-vectors.rs`
- `fixture_offer_2` becomes raw-JSON DATA (the live struct serializes
  v3); new `fixture_offer_3`; `fixture_reject_1` extended to six reasons
  (the four existing canonicalizations are UNCHANGED BYTES — verified
  below); new `fixture_commit_carrier`. The generator writes 12 files,
  idempotent.

### `protocol/vectors/`
- **NEW** `session-offer-3.json` — the v3 form.
- **NEW** `verifydrafts-5-commit-carrier.json` — the §2 commit-form
  golden vector (bare-object carrier, reproducing
  `verifydrafts-1-accept`'s result).
- `session-reject-1.json` — +2 reason canonicalizations, 22 insertions /
  1 deletion (the deletion is the description text); the four existing
  `canonical_json` + `canonical_json_sha256` entries are byte-identical
  (verified programmatically against `git show HEAD:`).
- `session-offer-2.json` — description-only change (1/1); the v2
  canonical block bytes are unchanged.

### Cross-boundary (flagged for review — see §4)
- `apps/modelswarm-sim/src/mixed_swarm.rs` and
  `apps/modelswarm-sim/tests/mixed_swarm.rs` — T/R-owned paths,
  mechanically extended: the two `SessionOffer` construction sites gain
  the now-REQUIRED v3 fields (the frozen contract change makes the old
  literal un-compilable; workspace gates cannot pass otherwise). Values:
  the scenario's pinned profile label + the default-params family hash,
  with a comment marking them form-completeness (the sim is negotiation
  arithmetic, not a wire endpoint). No behavioral change; sim suites
  green.

## 2. Exact commands run and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` (after one `cargo fmt --all`) | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (0 errors/warnings) |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi -- -D warnings` | clean |
| `cargo test --workspace` | **468 passed / 0 failed / 6 ignored** = 462/0/6 baseline + exactly 6 new tests (negotiation v3-schema 1, disjointness 1, security pins +2, vectors +2) |
| `cargo test --workspace --features modelswarm-node/verifydrafts` | **513 passed / 0 failed / 7 ignored** = 505/0/7 baseline + 8 (the 6 above + the node strict-schema unit pin + the offer-v3 wiring test) |
| `cargo test --workspace --features modelswarm-node/verifydrafts-capi` | **531 passed / 0 failed / 12 ignored** = 523/0/12 baseline + the same 8 |
| `cargo run -p modelswarm-session --example gen-verifydrafts-vectors` (×2) | 12 files written; double-run diff empty (idempotency held) |
| Four-reason byte-identity check (python over `git show HEAD:` vs working tree) | all four existing `canonical_json`/`canonical_json_sha256` entries byte-identical; `primary` block identical |

## 3. Test evidence

- The three counts above (every gate in the task's mandate, all three
  feature configs).
- New pins, by contract item: offer v3 (1 unit + 1 golden + 1 wiring
  over the real transport), reject reasons (1 unit + 1 golden), seconds
  arm (re-pinned boundaries + unchanged conjunction sweep), RC-5(b) (2
  re-scoped/new security pins), carrier sender check (1 security pin +
  1 strict-schema unit pin + the carrier golden's gate replay),
  disjointness (1 default-feature per-push test).

## 4. Assumptions, recorded gaps, and conflicts

**Plan-vs-ADR conflicts: NONE found.** The ADR's stage order, reason
vocabularies, and semantics were implementable exactly as written; no
plan document governed these migrations beyond the ADR itself.

Recorded gaps and judgment calls (each recorded, not improvised):

1. **G6 residual (prompt surface):** the offer v3 fields close the
   profile/params pairing half of the prefill assumption. The PROMPT
   still has NO on-wire commitment — `FamilyEngineSource::Capi {
   prompt_tokens }` + `CapiBatchVerifyEngine::prefill` remain the
   out-of-band seam, and a misconfigured prompt pairing is detectable
   only through acceptance collapse / audit divergence. Closing this
   needs a freeze-ADR decision (a prompt-hash echo/commitment field);
   the seam is the single place to feed it. NOT improvised here.
2. **Stale-classification boundary:** `apply_prefix_commit` is
   clock-free, so "already abandoned" is implemented by its operational
   definition — no outstanding result (the serve loop's top-of-loop
   expiry polling bounds this; a commit arriving in the instant between
   grace expiry and the next poll is classified by the outstanding-state
   path: reproducing → applied slightly past grace, which is I2-safe,
   non-reproducing → commit_mismatch). Pre-existing poll granularity;
   unchanged by this migration.
3. **Carrier strictness beyond the sender check:** implementing the §2
   G2 "strict schema" sentence at the carrier's entry
   (`deny_unknown_fields` + bare-hex/profile form check) is a behavior
   change confined to MALFORMED carriers (previously permissive serde;
   non-bare-hex hashes classified downstream via recompute failure).
   This completes the G2 decision text; recorded as such.
4. **Cross-boundary edits:** the two `apps/modelswarm-sim` construction
   sites (T/R-owned) — compile-necessitated by the frozen v3 form,
   mechanical field additions only, flagged here for T/R review (the
   `rust.yml` precedent from the wiring handoffs).
5. **Offer-reject ordering:** the ADR pins no order among negotiation
   rejects (only one fires); this loop checks preset → profile → params
   → backend → budget (identity binding before capability). Recorded as
   the implementation's choice.
6. **Disjointness probe:** the §6 tag list is extracted from serde's
   unknown-variant error message; if serde's wording ever changes the
   test panics loudly (fail-closed) rather than passing silently.

## 5. Unresolved risks

1. The G6 prompt gap stands (owner-gated freeze-ADR item, §4.1).
2. The QUIC/composed lifecycle tests exercise the happy-path carrier;
   an over-the-wire FOREIGN-SENDER commit is pinned at the gate and
   parse layers (unit + gate pins) but not as a separate end-to-end
   transport test — the two enforcement layers are covered; a wire-level
   pin can fold into the next wiring pass if T/R wants it.
3. The sim's v3 field values are form-completeness only; if the sim ever
   becomes a wire consumer (ADR-032 §7 names it the natural second
   consumer), its profile label (`msp:sim-mock:…`, not `msp1:<64 hex>`)
   will need the real form — flagged in-code.

## 6. Suggested next task for the integrator

1. T/R review of the two sim construction-site edits (the only
   cross-boundary change).
2. With item (iv) discharged, the §7 REMAINING list is: (i) the
   `capi-adapter` flag flip at family light-up in serving, (ii) the
   item-2 family exactness-pin reruns (`real-model-e2e.yml` extension),
   (iii) the Windows-Product composition work (daemon listener demux
   consumption, desktop surfaces) + the F2B cross-machine relay proof —
   all owner-gated sequencing from here.
