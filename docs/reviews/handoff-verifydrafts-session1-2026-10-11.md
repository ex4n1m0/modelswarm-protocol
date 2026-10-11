# Handoff — VerifyDrafts implementation session 1 (type + serialization layer)

Date: 2026-10-11. Agent seat: implementation session under the Protocol
Architect contract (plan §5.1 staging). Binding contracts read in full
before writing: `docs/adr/ADR-032-batched-speculative-verification.md`
(accepted, revision 2) and
`docs/implementation/adr-032-verifydrafts-plan.md` §1 (incl. §1.5 tests).
Scope executed: EXACTLY plan §1 (Step 1). Base: `main @ 30258fe`.

## 1. Changed files and why

| File | Change | Why |
|---|---|---|
| `crates/modelswarm-session/src/verifydrafts.rs` | NEW (≈1000 lines incl. tests) | §1.2 message structs (`VerifyDrafts`, `VerifyDraftsResult`, normative field names/types/serde), constants (`COOPERATIVE_PROTOCOL_VERSION`, `WINDOW_CAP=32`, `DEADLINE_CLAMP_MIN_MS=1000`, `DEADLINE_CLAMP_MAX_MS=120000`, `WAIT_COMMIT_GRACE_MS=250`, `VERIFY_PASS_CLOCK_NOISE_MS=25`, `TEARDOWN_VIOLATION_THRESHOLD=2`), declared-field-set constants, strict two-pass parse (`VerifyDrafts::parse` / `VerifyDraftsResult::parse` → `FrameParseError`; every variant is the `bad_frame` schema-failure path; missing OR unknown key fails; present-as-null enforced for `divergence_point` and EOG `correction`), byte-rule helpers per ADR §1 rules 1–5 (`is_bare_hash` rejecting `sha256:`-prefixed forms; `batch_generation_params_hash` with integral-only rendering and `ParamsHashError::NonIntegral` fail-closed; `signing_preimage` omitting `signature` ENTIRELY via `modelswarm_types::canonical_json`; `sign_verify_drafts`/`sign_verify_drafts_result` and `verify_verify_drafts_signature`/`verify_verify_drafts_result_signature` following `spec.rs::verify_commit_signature`'s discipline — base64 STANDARD, 64-byte `Signature::from_slice`, sender-binding FIRST), §1.5 unit tests (17). Prefix-hash chaining REUSES `crate::compute_prefix_hash` / `GENESIS_PREFIX_HASH` as-is — no new hash function. |
| `crates/modelswarm-session/src/negotiation.rs` | NEW (≈500 lines incl. tests) | §1.4 negotiation shapes: `Preset` (the five frozen ADR-029 values — no Rust type existed; see assumption A4), `Backend`, `SessionBudget`, `SessionOffer` (additive `verifier_backend`), `EngineDisclosure` (+ `is_valid_disclosure` range pin), `SessionAccept` (additive `engine`), `SessionReject`; frozen vocabularies with `Display`/`as_str`: `SessionRejectReason` (`preset_unavailable|backend_mismatch|batch_verify_unsupported|budget_infeasible`), `CancelRoundReason` (`parent_mismatch|round_mismatch|commit_mismatch`; old `proposal_failed` NOT reused), `FrameRejectReason` (the eight effective-§6.5 codes `bad_frame|replayed_request|expired_token|cancelled_by_peer|profile_mismatch|payload_too_large|overloaded|deadline_exceeded`, asserted in-test to be a subset of the effective msp-v1 §6.5 + ADR-028 §5 registry). Tests (8) pin exact wire shapes incl. canonicalizability (integer-only). |
| `crates/modelswarm-session/src/lib.rs` | +2 lines | Register both modules (`pub mod negotiation; pub mod verifydrafts;`). No other edit; existing `spec.rs` paths untouched. |
| `crates/modelswarm-session/Cargo.toml` | +1 dependency | `modelswarm-types = { path = "../modelswarm-types" }` for the `Result`-returning `canonical_json` (`FloatRejected`) — plan §1.1. Existing `modelswarm_identity::canonical_json` usage in `spec.rs` NOT switched (no cross-cutting edits); new code uses `modelswarm_types::canonical_json` exclusively. |
| `Cargo.lock` | dependency-edge update | MECHANICAL consequence of the authorized Cargo.toml addition (records the `modelswarm-session → modelswarm-types` edge; both packages already in the lock). Strictly outside the literal scope guard (`crates/modelswarm-session/**` + `docs/reviews/`) but unavoidable — committing the Cargo.toml without it would leave a stale lock. Flagged here for the integrator; no other workspace file touched. |
| this file | NEW | Handoff per AGENTS.md format. |

Not touched (scope guard respected): `protocol/`, `spec.rs`, other crates,
workflows, `experiments/`, `apps/`. The pre-existing untracked
`zcode-master-prompt.md` was left alone (not mine).

## 2. Exact commands run and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all` then `cargo fmt --all --check` | clean (diff applied to the two new files only) |
| `cargo clippy -p modelswarm-session --all-targets -- -D warnings` | clean, exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| `cargo test -p modelswarm-session` (default features; no new features added) | ok: 42 lib + 10 + 1 + 5 + 7 + 5 integration = 70 passed, 0 failed |
| `cargo test --workspace` (default features) | **399 passed; 0 failed; 6 ignored** (the 6 are pre-existing owner-gated ignores, same as the 2026-10-11 scheduler handoff baseline) |
| `git push origin main` | ok; `origin/main == main` verified (`git rev-list --count` both directions = 0) |

Fix iterations during development (before the gates above were final):
one `E0631` (`CanonicalError::Serde` needs `e.to_string()`), clippy
`field_reassign_with_default` ×3 and `needless_borrows_for_generic_args`
×4 in tests (rewritten), one test-logic bug (preimage field-presence
loop must skip `signature`) — all resolved; final gates are the green
runs recorded above.

## 3. Test evidence

- 25 NEW unit tests, all green: 17 in `verifydrafts::tests`, 8 in
  `negotiation::tests`.
- §1.5 coverage map: round-trip + exact `serde_json::to_value` shape for
  both messages (`request_wire_shape_is_snake_case_and_round_trips`,
  `result_wire_shape_is_snake_case_and_round_trips` — the latter also
  pins the §1/RC-7 seq-pair illustration: request 12, result 4);
  present-as-null (`divergence_point_is_present_as_null`,
  `eog_correction_is_present_as_null` — omission ⇒
  `FrameParseError::MissingKey`); strict key-set equality
  (`unknown_or_missing_keys_and_non_objects_are_schema_failures`);
  integer-only pins (`float_fields_fail_deserialization` — `35.5` AND
  `35.0` both fail `verify_pass_ms`; `12.0` fails `seq`); empty-window
  malformed (`empty_window_is_malformed`); bare-hash rule
  (`is_bare_hash_rejects_prefixed_and_malformed_forms`,
  `prefix_chain_reuses_the_shipped_hash_function` — genesis =
  hex(sha256("")) and chaining via the SHIPPED `compute_prefix_hash`);
  signatures (`signature_verifies_and_tampering_breaks_it`,
  `sender_binding_is_checked_first` — the sender error fires before any
  Ed25519 verdict, `signing_preimage_omits_signature_entirely` —
  `signature` key absent from the preimage, all other declared fields
  present, null-valued `divergence_point` survives, preimage ≠ canonical
  of the full message); params hash
  (`batch_generation_params_hash_is_stable_64_hex`,
  `..._renders_integrals_as_integers` — byte-exact expected preimage
  `{"seed":null,"temperature":1,"top_k":40,"top_p":1}`, seed set renders
  the full integer (F-1), `..._fails_closed_on_non_integral`,
  `family_params_hash_is_not_the_old_vocabulary_hash`); canonicalizer
  parity pin (`canonicalizers_agree_byte_for_byte_on_family_payloads` —
  `modelswarm_identity::canonical_json` ≡ `modelswarm_types::canonical_json`
  on family payloads, including float-rejection parity).
- Workspace total 399/0/6 vs the 374/0 baseline recorded in the scheduler
  handoff (commit `2b4fcd3`): +25 = exactly the new tests; no foreign
  failures anywhere in the workspace suites.

## 4. Assumptions made (interim decisions, all within plan wording)

- **A1 — Cargo.lock** (see table above): committed as the mechanical
  shadow of the authorized dependency addition.
- **A2 — empty window enforced at the type layer.** ADR §1: "An empty
  `window_tokens` is malformed"; plan §3.3 stage 10 note: "empty window →
  `bad_frame` (schema)". I enforce it in `VerifyDrafts::parse` as a new
  `FrameParseError::EmptyWindow` variant (distinct from serde's, so the
  §3 admission session can name it precisely; still one `bad_frame`
  outcome).
- **A3 — error-type shape.** The plan names `ParamsHashError::NonIntegral`
  (kept as a unit variant, exact name) and no parse-error type at all; I
  added `ParamsHashError::Canonical(CanonicalError)` for the
  practically-unreachable canonical failure (no `unwrap` in library code)
  and designed `FrameParseError` with the five variants above.
- **A4 — `Preset` defined fresh** in `negotiation.rs` (plan §1.4's
  `SessionOffer` sketch references a `Preset` type that did not exist
  anywhere in Rust; msp-cooperative-v1 §1 names "SessionOffer and the
  preset enum" as the namespace's content). Serialized lowercase, the
  five frozen strings, `Display`/`as_str` per the `RejectReason`/
  `FallbackReason` precedent.
- **A5 — sign returns the signature string** (plan sketch
  `fn sign(msg, identity) -> Result<String, SpecError>`), caller assigns
  it and sets `sender`; NO sender check on the sign side — the exact
  `spec.rs::sign_commit` discipline (verify enforces sender binding).
- **A6 — verify functions take the parsed structs** (reading `sender`/
  `signature` fields), matching `verify_commit_signature`. The plan's
  sketch parametrization `(msg_value, expected_key, claimed_sender)` is
  satisfied structurally: the §3 admission order (schema → signature)
  parses to the struct BEFORE signature verification, and the
  malformed-fixture raw-JSON cases (§2) can never reach signature
  checking anyway (they fail schema first). If session 3 needs a
  Value-level entry point, it wraps `parse` + these functions.
- **A7 — `EngineDisclosure::is_valid_disclosure`** is the
  `1..=WINDOW_CAP` range check on `batch_window_max` (field-level pin);
  the I4 requirement "`batch_verify: true` REQUIRES in-range" is implied
  by it, and with `batch_verify: false` any in-range value is
  schema-valid (guidance form `false`+`1` documented).
- **A8 — no strict two-pass parse for negotiation structs.** Plan §1.2's
  strict-parse rule is stated for the family messages; negotiation shapes
  are frozen-doc-governed and additive-evolution is vector-pinned (the
  "-2" vectors), so unknown-key rejection there could conflict with a
  future additive field. If the architect wants negotiation strictness,
  it is a one-liner to add `parse` methods later.
- **F-flags relied on (interim, per plan §6):** F-1 (integral-only
  rendering, seed rendered as the full integer), F-2 (`correction` FIELD
  always present, VALUE null exactly at EOG — enforced by the key-set
  check), F-6 (`verifier_backend` REQUIRED in the new canonical form —
  implemented as a required struct field, so its absence fails parse).
  All three are byte-pinned by the session-2 fixtures next.

## 5. Contract conflicts found

None on byte rules — no place was found where the plan and ADR-032
revision 2 disagree about bytes, and nothing was improvised. Two nuances
recorded (not conflicts, both already resolved by the plan):

- ADR §1 rule 3 writes `<num>` for `temperature`/`top_p` while the
  canonicalizer rejects floats — the plan's F-1 resolution (integral
  rendering, `NonIntegral` fail-closed) is implemented; the fixture set
  (§2) will byte-pin it; revision 3 of the ADR should confirm.
- ADR §1 rule 4 names only `divergence_point` for present-as-null;
  "correction is always present" (§1 semantics) vs the engine's
  `Option<bonus>` is the plan's F-2 — implemented as the plan pins
  (field present, null at EOG) and test-pinned here.

## 6. Unresolved risks

- The wire shapes' first INDEPENDENT byte check is the session-2 golden
  vectors; a divergence between this layer and the architect's fixture
  expectations would surface there (canonical key sorting makes field
  order moot; the only genuinely open renderings are the F-1/F-2 pins
  above).
- Sender binding's remote-PeerId half (QUIC-authenticated equality,
  msp-v1 §6.6) is enforced by the serve loop — intentionally NOT in this
  layer; §3/§4 sessions own it.
- `deadline_ms` clamping helper, admission order, state machine, EWMA:
  §3/§4 scope; only the constants exist here.
- `FrameRejectReason` is the family's subset view of the effective §6.5
  registry; the test embeds the 16-string effective registry and asserts
  membership — if a future ADR renames a registry code, that test fails
  loudly (intended).

## 7. Suggested next task for the integrator

Session 2 of the plan (§2): the
`crates/modelswarm-session/examples/gen-verifydrafts-vectors.rs`
generator plus the committed `protocol/vectors/verifydrafts-*.json` and
`session-{offer,accept,reject}-*.json` fixtures and the
`crates/modelswarm-session/tests/verifydrafts_vectors.rs` golden test —
it depends only on this layer and byte-pins the F-1/F-2/F-6 interim
decisions. Reviewers per the ownership map: Security + Test and Release.
