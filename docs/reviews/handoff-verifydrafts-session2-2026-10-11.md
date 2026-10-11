# Handoff — VerifyDrafts session 2: fixture generator + committed golden vectors + consumer golden test

Agent: implementation session under the Protocol Architect contract
(ADR-032 revision 2; plan `docs/implementation/adr-032-verifydrafts-plan.md`
§2, step 2 — EXACTLY §2, nothing from §3/§4).
Base: `fddb972` (session 1: types/serialization, landed).
Date: 2026-10-11.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-session/examples/gen-verifydrafts-vectors.rs` (new) | Plan §2.2: the deterministic golden-vector generator. No arguments, no clock, no RNG; fixture-only seeds `[0x11;32]` (coordinator) / `[0x22;32]` (verifier) recorded in every generated file (F-7); single time value `verify_at = "2026-10-12T00:00:00Z"`; builds each message with the session-1 structs, signs the signature-omitted canonical form (base64 STANDARD, `SigningKey::from_bytes(&seed)`), and writes `serde_json::to_string_pretty` + trailing newline. Also self-checks: the F-1 params-preimage pin (`batch_generation_params_hash(SamplingParams::default())` must equal the sha256 of the literal `{"seed":null,"temperature":1,"top_k":40,"top_p":1}` canonical bytes) and the three unix-ms anchors (verified against independent `date -u`: 1791763200000 / 1791763140000 / 1791759600000) via an internal days-from-civil conversion. |
| `protocol/vectors/verifydrafts-1-accept.json` (new) | ADR-032 §7 row 1: w=8 full acceptance. Pins request/result canonical bytes + sha256 + signatures; `accepted_prefix_len == len(window) == 8`; `divergence_point` null PRESENT-AS-NULL; `correction` = bonus 999; `committed = window[..8] ++ [999]`; `new_prefix_hash` VALUE + construction inputs; `nonce_echo`; seq pair request 12 / result 4 (§1/RC-7 illustration made normative bytes). |
| `protocol/vectors/verifydrafts-2-reject.json` (new) | Row 2: mid-window rejection at k=3<8; `divergence_point == accepted_prefix_len == 3`; `correction` = replacement 777; `committed = [201,202,203,777]`; chain hash recomputed. |
| `protocol/vectors/verifydrafts-3-boundary.json` (new) | Row 3: len-32 accepted with `batch_window_max = 32`, plus the len-33 admission case (`over_cap_case`, schema- and signature-valid so the reject fires AT the cap stage; `expected_reject: payload_too_large`, `engine_passes_before_reject: 0`). |
| `protocol/vectors/verifydrafts-4-eos.json` (new) | Row 4 / F-2: EOG inside the accepted path; `correction` null PRESENT-AS-NULL; commit covers the accepted prefix only (`committed = [601,602,603]`, no correction token in the hash preimage); `expected.eog = true`. |
| `protocol/vectors/verifydrafts-malformed.json` (new) | Row 5: top-level ARRAY of the 12 admission-rejection cases, each self-contained `{name, description, verify_at, wire, canonical_json, signature, canonical_json_sha256, context, expected_reject}` + the recorded fixture keys: empty window (`bad_frame`); bad signature (`bad_frame`); wrong `profile_id` (`profile_mismatch`); duplicate nonce / duplicate seq / non-monotone seq (`replayed_request` ×3); deadline-past vs `verify_at` (`deadline_exceeded`); unknown session (`bad_frame`); torn-down session (`cancelled_by_peer`); lease-expired (`expired_token`); window over `batch_window_max` (`payload_too_large`); budget-exhausted (`overloaded`). |
| `protocol/vectors/session-offer-2.json`, `session-accept-2.json`, `session-reject-1.json` (new) | ADR-032 §7 last row / A-4: extended canonical forms (`verifier_backend`; `engine {backend, batch_verify, batch_window_max}`; `SessionReject` with one primary case `backend_mismatch` + a `reasons` list of all four canonical serializations). |
| `protocol/vectors/session-offer-1.json`, `session-accept-1.json` (new) | Plan §2.2 addition beyond the ADR table (authorized by the plan): base ADR-029 forms WITHOUT the additive fields — recorded as raw JSON because the extended structs cannot express them; they pin the base shape the additive fields extend. |
| `crates/modelswarm-session/tests/verifydrafts_vectors.rs` (new) | Plan §2.3 consumer golden test: `include_str!` all 10 fixtures; recompute canonical bytes from `fields` and assert equality with `canonical_json` AND `canonical_json_sha256`; verify signatures under the recorded public keys and assert `installation_id_for(pubkey) == sender`; recompute `new_prefix_hash` via `compute_prefix_hash` from the recorded construction inputs and assert equality; classification-level assertions for every malformed case; `TODO(session 3)` hooks where the §3 admission gate will replay fixtures. |
| `docs/reviews/handoff-verifydrafts-session2-2026-10-11.md` | This handoff. |

Scope check: only `crates/modelswarm-session/**`, `protocol/vectors/**`,
and `docs/reviews/` touched. `protocol/msp-v1.md`,
`protocol/msp-cooperative-v1.md`, and every other path untouched. No
dependencies added (example + test use existing crate deps only; the
days-from-civil helper avoids a `time` dependency).

## 2. Exact commands and outcomes

Run from the repo root, Windows/Git Bash, workspace default features:

```
$ cargo run -p modelswarm-session --example gen-verifydrafts-vectors
    Finished `dev` profile [unoptimized + debuginfo] target(s)
    Running `target\debug\examples\gen-verifydrafts-vectors.exe`
wrote ...\protocol\vectors\verifydrafts-1-accept.json        (x10 lines)
verifydrafts fixture generation complete: 10 files (idempotent — rerun leaves git diff empty)
```

Idempotency proof (per task instruction: run twice, confirm git diff
empty): the 10 fixture files were staged (`git add`), then the generator
was run twice more:

```
$ git add protocol/vectors/verifydrafts-*.json protocol/vectors/session-*.json
$ cargo run -p modelswarm-session --example gen-verifydrafts-vectors   # rerun 1
$ git diff --stat -- protocol/vectors/          # (no output)
$ git status --porcelain protocol/vectors/      # only "A " lines (staged, unmodified)
$ cargo run -p modelswarm-session --example gen-verifydrafts-vectors   # rerun 2
$ git diff --stat -- protocol/vectors/          # (no output)
$ git status --porcelain protocol/vectors/      # only "A " lines
```

Both reruns left `git diff` empty (no unstaged modifications). The
`LF will be replaced by CRLF` notices are the repo's `* text=auto`
policy (index LF / Windows checkout CRLF, identical to the existing
`lease-hubkey-1.json`); git normalizes on comparison, so idempotency
holds on LF and CRLF checkouts alike.

## 3. Test evidence (gates)

| Gate | Command | Result |
|---|---|---|
| fmt | `cargo fmt --all --check` | clean (after `cargo fmt --all`) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| tests | `cargo test --workspace` | **409 passed, 0 failed**, 6 ignored (pre-existing owner-gated suites; baseline after session 1 was 399 passed → +10 = the new golden tests) |
| scoped | `cargo test -p modelswarm-session --test verifydrafts_vectors` | 10 passed, 0 failed |

Golden-test coverage detail (all 10 green):
`golden_verifydrafts_1_accept_full_acceptance`,
`..._2_reject_mid_window`, `..._3_boundary_cap_edge` (incl. the len-33
`over_cap_case` with `engine_passes_before_reject: 0`),
`..._4_eos_correction_null` (present-as-null + prefix-only commit),
`golden_verifydrafts_malformed_admission_cases` (all 12 cases:
expected-code registry mapping, canonical+sha pins, per-case
classification inputs), `golden_session_offer_2_extended_canonical_form`,
`golden_session_accept_2_engine_disclosure` (incl. budget clamped DOWN),
`golden_session_reject_1_all_four_reasons`,
`golden_session_offer_and_accept_1_base_forms_pin_f6` (base forms pin
canonical bytes AND fail to deserialize into the extended structs),
`golden_fixture_set_shares_one_anchor_and_keypair` (single verify_at,
single keypair, seeds recorded in every file, installation ids derive
from the recorded public keys).

Development note (test earned its keep): the golden test caught two
generator bugs before commit — (a) fixtures 1–4 initially recorded
`sender: ""` (violating §1 rule 5), and (b) three malformed cases
mutated fields AFTER signing (invalidating signatures) and the
empty-window case never actually emptied the window. All fixed; the
committed fixtures are the fixed output.

## 4. F-flags byte-pinned by these fixtures

- **F-1** (`generation_params_hash` rendering): every family fixture
  records the full construction — the params object, its canonical bytes
  `{"seed":null,"temperature":1,"top_k":40,"top_p":1}`, and the sha256 —
  and the golden test asserts equality with
  `batch_generation_params_hash(&SamplingParams::default())`. The
  generator itself asserts this at generation time (fails loudly if the
  runtime default ever changes).
- **F-2** (`correction` at EOG): `verifydrafts-4-eos.json` pins
  `correction` PRESENT-as-null at EOG, the prefix-only commit, and the
  hash over exactly the accepted ids; the test asserts the wire fields
  contain the null key.
- **F-6** (`verifier_backend` required): `session-offer-2.json` /
  `session-accept-2.json` pin the extended canonical forms; the base
  `-1` vectors pin the ADR-029 shapes and the test asserts they do NOT
  deserialize into the extended structs (required-field discipline).

## 5. Assumptions, deviations, and ADR-vs-plan notes (no byte rule improvised)

- **No ADR/plan conflict found** requiring escalation; where the plan
  left fixture-JSON shape details open, the additions are recording-only
  (never new wire semantics): a `generation_params` block, the
  `expected.new_prefix_hash_construction` object (the plan demands the
  construction be recorded; this is its encoding), `over_cap_case` inside
  `verifydrafts-3-boundary.json` (plan §2.2 row 3 asks for the len-33
  admission input "pairs with" that fixture), per-case key blocks +
  canonical/signature pins in the malformed array (plan: "seeds recorded
  in every file"; self-contained entries per the consumer-neutrality
  rule), and `coordinator_peer_id` alongside the plan's key fields
  (referenced by `proposer_id`).
- **Malformed raw-JSON note:** the plan says raw-JSON entries appear
  "where the structs cannot express the input". All 12 enumerated cases
  are struct-expressible (each wire is a fully signed `VerifyDrafts`
  value whose reject reason lives in recorded context pre-state), so no
  malformed entry needed raw JSON. The two base negotiation vectors
  (`session-offer-1/-accept-1`) ARE raw JSON — the extended structs
  cannot express the base forms. The session-3 admission API accepting
  `serde_json::Value` remains the plan's requirement, unchanged.
- **Deadline-past encoding:** `deadline_ms` is a duration, so the
  deadline-past case records its anchor as context pre-state
  (`request_arrival` 2026-10-11T23:59:00Z + `deadline_ms` 1000 < the
  `verify_at` admission clock), with unix-ms integers for the session-3
  injected-clock API. These pre-state times are recorded context, not
  fixture clocks; `verify_at` remains the single fixture time value.
- **Session-1 observation (NOT modified; flagged for session 3):**
  `verifydrafts.rs`'s own unit-test helper `sample_result` constructs
  `new_prefix_hash = compute_prefix_hash(GENESIS, &[999])` while claiming
  `accepted_prefix_len: 8` over an 8-token window — i.e. it hashes only
  the correction, not `(parent ‖ accepted prefix ‖ [correction])` as
  ADR-032 §1 rule 2 pins. It is a test-local self-consistent sample, not
  a code path (no production code computes the hash yet), and its test
  only asserts the sample equals itself. These fixtures byte-pin the
  ADR construction (full-acceptance hash covers window ++ bonus). When
  session 3 implements the gate, it must follow the ADR (as pinned
  here); if the architect prefers, session 3 should also align that
  unit-test sample — left untouched here to avoid editing landed,
  reviewed session-1 tests.
- Session-3 TODO hooks are marked inline in
  `tests/verifydrafts_vectors.rs` (gate replay with mock engine + clock
  fixed at `verify_at`; zero-`verify_window`-call assertions for the cap
  rejects; exact-code assertions per malformed case). Until §3 lands,
  admission cases are asserted at the classification level only — this
  is the "assert what is assertible now" reading of the tasking.

## 6. Unresolved risks

1. The §3 admission gate does not exist yet; the malformed fixtures'
   expected codes are pinned data, not executed behavior (hooks in
   place; activation is session 3's first task).
2. F-7 holds: fixture keys are recorded seeds; installation ids derive
   from the recorded public keys, so no production collision is
   possible — but the seeds are visible and must never be used for real
   identities.
3. `apps/tracker/scripts/validate-vectors.mjs` does not know these
   files; extending it is Tracker-owned and out of this session's scope
   (plan §2.1 already assigns it as a later task).
4. Line endings follow the repo `* text=auto` policy (index LF); the
   generator always writes LF — reruns stay diff-clean on both LF and
   CRLF checkouts (proven above on CRLF).

## 7. Suggested next task for the integrator

Proceed to session 3 (plan §3): the verifier-side admission gate +
`BatchVerifyEngine` trait + the two property suites
(`verifydrafts_admission_properties.rs`,
`verifydrafts_divergence_properties.rs`), activating the TODO(session 3)
hooks in `tests/verifydrafts_vectors.rs` so every committed fixture —
including all 12 malformed cases and the len-33 over-cap wire — replays
through the real gate with the mock engine and the fixed `verify_at`
clock.
