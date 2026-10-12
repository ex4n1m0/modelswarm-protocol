# Review — pre-light-up release disposition (Test and Release seat)

Date: 2026-10-12. Seat: Test and Release Engineer (release gate). Base:
`4b35d81` (main). Inputs: (1) the ADR-032 revision-4 disposition
(`docs/adr/ADR-032-batched-speculative-verification.md` rev 4, the §7
item-(iv) migrations at `4b35d81`); (2) the light-up CI steps
(`f9ec2ff` verifydrafts feature steps, `4e8fc18` verifydrafts-capi
combo, in `.github/workflows/rust.yml`); (3) the two cross-boundary
`SessionOffer` construction-site edits in `apps/modelswarm-sim`
(`4b35d81`, flagged for this seat). Method: independent re-runs of
every claimed gate on this machine (rustc/cargo 1.99.0, matching the
`rust-toolchain.toml` pin and the CI `dtolnay/rust-toolchain@1.99.0`
step), a clean-worktree baseline re-run at the parent commit `4e8fc18`
for count reconciliation, and programmatic fixture verification
against `git show` of the parent commit. No self-reported number in
this review is trusted without a re-run.

## Verdicts

| Input | Verdict |
|---|---|
| 1. ADR-032 rev-4 disposition (§7 item-(iv) migrations, `4b35d81`) | **APPROVE** (1 record correction + 2 advisories) |
| 2. CI steps (`f9ec2ff`, `4e8fc18`, current `rust.yml`) | **APPROVE WITH REQUIRED CHANGES** — the required change is APPLIED by this seat in this review (explicit `libp2p-backend` in both feature steps) |
| 3. Cross-boundary sim edits (`apps/modelswarm-sim`, `4b35d81`) | **APPROVE** |
| Overall light-up gate disposition | **NOT BLOCKED from the release seat** — see the disposition statement |

## 1. ADR-032 revision-4 disposition — APPROVE

Does the landed code match the decided text? Yes, on all five
migrations plus the fixture obligations.

- **§3 offer form v3 (G3).** `SessionOffer` gains `profile_id: String`
  and `generation_params_hash: String` as plain serde-required fields
  (no `#[serde(default)]`) — absence fails schema, the F-6 mechanism
  as a THIRD canonical form. `SessionRejectReason` extends to
  `ProfileMismatch`/`ParamsMismatch` (`snake_case` wire forms
  `profile_mismatch`/`params_mismatch`), negotiation-level vocabulary;
  the frame-level `profile_mismatch` code enum is untouched (§6.5
  registry unchanged). The client `negotiate` sends both fields from
  real sources (config profile + the already-derived family params
  hash); the serve loop compares both against its own pinned config
  and rejects honestly (`SessionReject` + telemetry +
  `ServeEndReason::RejectedOffer`) BEFORE backend pairing and budget,
  with verifier-side pinning (stages 4/7, §8 commit-time equality)
  unchanged — discovery, not trusted input, exactly the decided
  letter. Pinned by `session_offer_form_v3_requires_both_binding_fields`
  (base and v2 forms fail deserialization), the
  `golden_session_offer_3_third_canonical_form` vector test, and
  `offer_v3_profile_and_params_mismatch_reject_honestly` over the real
  staged transport.
- **§8e would-exceed on BOTH arms (T/R advisory A-2 — this seat's own
  advisory, now decided).** Stage 11 replaces the landed
  already-at-cap seconds check (`served_ms >= cap`) with
  `served_ms.saturating_add(clamped_deadline_ms + WAIT_COMMIT_GRACE_MS)
  > max_endpoint_seconds * 1000` — byte-for-byte the §8e formula
  (clamped deadline + 250 ms grace reservation, saturating arithmetic
  per the earlier overflow advisory). The token arm was already
  would-exceed and is unchanged; the stage ORDER is unchanged (the
  clamp computation moved above stage 11 with identical semantics for
  stage 12); the seconds ledger still advances on serve-loop deltas
  (`advance_session_seconds`). Boundary pins are re-based exactly:
  admits at `117_250 + 2_750 == 120_000` (would-not-exceed), refuses
  at `120_001`, refuses a clamp-ceiling reservation (`120_250 ms`)
  against a 120 s cap, and the deadline-clamp test moves to a
  wide-seconds gate (3_600 s) because that interaction IS the new arm
  working. My re-run of the admission property suite: **15/0 green**,
  including `first_failure_wins_all_exhaustive_pairs` (the conjunction
  sweep — stage order preserved) and
  `budget_exhaustion_refuses_with_overloaded` (the re-based
  boundaries).
- **§8 RC-5(b) stale-commit reclassification (Security advisory A1).**
  The gate implements the operational definition exactly: a commit
  with `outstanding == None && last_committed_round < round <=
  last_handled_round` rejects `CancelRound("round_mismatch")` with no
  state change and no violation increment (`cancel_round_commit`
  touches no counter), never applied (I2). The exemption's scope is
  pinned: `pin_commit_mismatch_while_result_outstanding_still_violates`
  freezes that a commit failing RC-4 reproduction WHILE the result is
  outstanding remains a violation toward threshold-two teardown. The
  Security review's pin is re-scoped
  (`..._is_exempt_stale_round_mismatch`, citing rev 4) to freeze the
  NEW semantics.
- **§2 carrier `sender` check (G2).** The gate pins
  `installation_id_for(coordinator_key)` at construction; a carrier
  whose `sender` differs is `bad_frame` BEFORE any state is touched
  (it precedes even the profile-mismatch violation counter — nothing
  consumed, round stays outstanding for a correct retry). The serve
  loop's carrier parse is now STRICT per the G2 letter:
  `#[serde(deny_unknown_fields)]` plus
  `passes_strict_carrier_schema()` (bare-64-hex on the three hash
  fields, `msp1:<64 hex>` profile form), every failure `bad_frame`
  before the gate. Pinned three ways: gate pin
  (`pin_carrier_sender_mismatch_is_bad_frame_before_state`), unit pin
  (`wire_prefix_commit_strict_schema_pins_the_g2_carrier`, including
  `signature`/envelope-tag rejection), and the new commit-form golden
  vector.
- **§2 rule (ii) vocabulary-disjointness assertion.**
  `verifydrafts_vocabulary_disjointness.rs` is a per-push DEFAULT-feature
  session-crate test: it extracts the LIVE §6 `WireMessage` variant
  tags from serde's own unknown-variant error (panics loudly if
  serde's message shape changes — fail-closed), asserts the frozen
  ten, builds the family discriminant keys from the REAL wire shapes
  (proving each ADR-enumerated key is a live field; the carrier key
  `accepted_token_ids` is delegated to the carrier golden vector since
  the node mirror is feature-gated), and asserts disjointness both for
  the enumerated nine and the full live family field set. This
  satisfies rule (ii)'s "lands with the §7-recorded migrations at the
  latest".
- **Fixtures (independently verified).** The four pre-existing
  `session-reject-1.json` reason canonicalizations are byte-identical
  to the parent commit (verified programmatically against
  `git show 4b35d81~1:`; only the description text and the two new
  reasons differ). `session-offer-2.json` is description-only (the v2
  canonical block bytes unchanged) and now documented as superseded
  failing-data. `verifydrafts-5-commit-carrier.json` cross-checks
  cleanly against fixture 1: `new_prefix_hash` equals fixture 1's
  result hash, the §1 rule-2 construction recomputes exactly
  (`sha256(raw32(parent) || LE u32 ids)`), the canonical-JSON sha256
  matches, and the form is the exact 8-key bare object with no
  envelope and no signature. The generator writes 12 files; double-run
  idempotency re-verified by this seat (two runs, `git diff` empty).
- **G6 fold.** Documented at the engine seam
  (`engine_source.rs`): profile/params mismatch can no longer open a
  session, so the pinned-prompt prefill is never consumed by a
  mismatched pairing; the PROMPT itself remains out-of-band with the
  residual gap recorded (freeze-ADR item) — recorded, not improvised,
  matching the handoff.

**Record correction (owed, not blocking):** every claimed workspace
total in the `4b35d81` commit message and handoff under-counts by
exactly ONE test. The security-pins delta is +3, not +2: besides the
re-scoped late-commit pin and the two listed pins, the commit adds
`pin_stale_commit_for_older_round_while_newer_outstanding_is_charged`,
which the handoff's "New pins, by contract item" section does not
list. Correct numbers (mine, re-run):

| Config | Handoff claim | This re-run (4b35d81) |
|---|---|---|
| `cargo test --workspace` | 468/0/6 | **469/0/6** |
| `--features modelswarm-node/verifydrafts` | 513/0/7 | **514/0/7** |
| `--features modelswarm-node/verifydrafts-capi` | 531/0/12 | **532/0/12** |

The error direction is benign (MORE coverage than claimed, 0 failures
everywhere, ignored counts exact), and the baseline claim itself
reproduces exactly (clean worktree at `4e8fc18`: 462/0/6 across 64
binaries — the per-binary delta to `4b35d81` is precisely +3 pins, +2
vectors, +1 disjointness binary, +1 negotiation unit = +7). Gate-count
claims must still be exact; the implementer session should correct the
handoff record.

**Advisories (routed):**

1. **RC-5(b) corner-case wording → Protocol Architect.** The unlisted
   corner pin freezes a STRICT reading of the exemption's
   precondition "no outstanding result left to reproduce against": a
   stale commit for an ABANDONED older round arriving while a NEWER
   round's result is outstanding is CHARGED `commit_mismatch`
   (violation + abandonment of the newer round's result), because the
   exemption fires only when NO round is outstanding at all. This is
   defensible under the rev-4 letter (there IS an outstanding result
   at that instant; fail-closed; I2 unaffected either way;
   threshold-two still required for teardown) and matches the
   handoff's stated operational definition (`outstanding == None`).
   But the exemption's RATIONALE (an honest-but-slow coordinator's
   in-flight commit must not tear the session) is reachable in this
   corner too: coordinator sends the round-3 commit (in flight) →
   verifier expires round 3 → coordinator gap-forwards per RC-5(c)
   with round 5 → verifier admits round 5 → the round-3 commit
   finally arrives → one violation, and round 5's result is
   discarded. Recommend a rev-5 clarification of whether the
   exemption is per-round (round abandoned) or global (no outstanding
   round at all). Not a blocker.
2. **Stale-classification boundary (pre-existing, disclosed in
   handoff §4.2):** `apply_prefix_commit` is clock-free, so
   "already abandoned" is the outstanding-state operational
   definition; a commit in the instant between grace expiry and the
   next serve-loop poll is classified by the outstanding-state path
   (reproducing → applied slightly past grace, I2-safe;
   non-reproducing → `commit_mismatch`). Pre-existing poll
   granularity, unchanged by this migration — noted for the record.

## 2. CI steps review — APPROVE WITH REQUIRED CHANGES (change applied)

- **Feature spelling: correct.** `modelswarm-node/verifydrafts` and
  `modelswarm-node/verifydrafts-capi` both exist in
  `crates/modelswarm-node/Cargo.toml`; the combo is
  `verifydrafts-capi = ["verifydrafts", "modelswarm-runtime/capi-adapter"]`
  — the G4 bridge composition, matching the steps' intent. The steps
  mirror the established libp2p-backend precedent (clippy + test,
  `--workspace --all-targets` / `--workspace`).
- **Venue-matrix coverage (ADR §7 per-push rows): all covered.** The
  default `cargo test --workspace` step runs: item 1 golden vectors
  (`verifydrafts_vectors` 13/0), item 3 admission properties (15/0),
  item 5 divergence properties, the I2/SessionAccept-honesty/
  `verify_pass_ms` coordinator properties, item 4a (scheduler, 36/0),
  item 4b mixed-swarm sim (`apps/modelswarm-sim` `mixed_swarm` 6/0 —
  the T/R-owned venue, not a bench-only module), and the rev-4
  disjointness assertion (new default-feature binary, 1/0). Both
  standing statements hold: item 2 and the real-engine halves never
  go green per-push (the `real-model-e2e.yml` is untouched since
  `b4b4d43` and remains `workflow_dispatch`-only), and the
  `capi-adapter` clippy/test steps exist on every push. The
  feature-gated light-up tests (wiring, QUIC venue, composed, capi
  e2e, strict-schema unit pin) run in the verifydrafts and combo
  steps — verified by reading the cfg gates and by the run logs.
- **REQUIRED CHANGE (applied by this seat): composed-daemon coverage
  was implicit.** `verifydrafts/composed.rs` and
  `tests/verifydrafts_composed.rs` are cfg'd on
  `all(verifydrafts, libp2p-backend)` at the NODE level, and no CI
  step named that combination. The tests DID run in CI — but only
  through TWO stacked implicit mechanisms: (a) cargo's workspace-wide
  CLI-spec side effect (`--features modelswarm-node/X` also enables
  the bare feature named `modelswarm-node` on other members, where it
  exists as desktop's/bench's implicit optional-dep feature), which
  (b) activates desktop's dep-spec
  `modelswarm-node = { features = ["libp2p-backend"] }`. Verified
  empirically: `cargo test -p modelswarm-node --features verifydrafts
  --test verifydrafts_composed` → **0 tests**;
  `cargo test --workspace --features modelswarm-node/verifydrafts
  --test verifydrafts_composed` → **2 tests**. The compose handoff's
  own explanation ("workspace runs get libp2p-backend via desktop
  dep-spec unification") is incomplete — desktop's dep is optional
  and `tauri-shell`-gated, so it would not fire without mechanism
  (a). Silent breakage of either mechanism (a toolchain bump — the
  pin is deliberately bumped sometimes — or a `dep:`-syntax cleanup)
  would drop the demux tests to 0 tests while CI stays green: the
  exact blind-spot class the feature steps exist to close
  (2026-10-08 lease break). **Fix applied in this change:** both the
  verifydrafts steps and the combo steps now pass
  `modelswarm-node/libp2p-backend` EXPLICITLY. Verified
  coverage-equivalent: the updated commands produce the SAME counts
  (514/0/7 and 532/0/12; `verifydrafts_composed` 2/0 present in the
  log) and clippy is clean in both explicit combinations.
- **Advisory:** a bare `cargo test -p modelswarm-node --features
  verifydrafts` compiles zero composed coverage (the module is simply
  absent) — local-run guidance, already documented in the compose
  handoff §4; with the explicit step features, CI no longer depends
  on it.

## 3. Cross-boundary sim edits — APPROVE

`4b35d81` touched two `SessionOffer` construction sites in this
seat's crate, compile-necessitated by the frozen v3 form (the new
fields are serde-required, so the old literal cannot compile):

1. `apps/modelswarm-sim/src/mixed_swarm.rs` (`run_mixed_swarm_decision`,
   the item-4b scenario engine).
2. `apps/modelswarm-sim/tests/mixed_swarm.rs`
   (`honesty_edge_batch_false_single_and_backend_mismatch_continues`).

Both add exactly `profile_id: MIXED_PROFILE.to_string()` and
`generation_params_hash: batch_generation_params_hash(&SamplingParams::default())`.
Honesty of the values, checked:

- **The params hash is the genuine family hash** over default
  `SamplingParams`, computed by the landed family function — exactly
  what a default-parameters coordinator would send. The sim's
  scenarios pin no other sampling parameters, so nothing is
  misrepresented.
- **The profile value is internally consistent:** `MIXED_PROFILE`
  (`"msp:sim-mock:q4_k_m:v1"`) is the SAME constant the scenario
  already pins everywhere else (`ModelProfileId::new(MIXED_PROFILE)`
  for the scheduler candidates, the eligibility filter) — the offer
  now carries the scenario's actual pinned profile label, not an
  invented one.
- **The non-wire-form profile string is harmless here because the sim
  is decision arithmetic, not a wire endpoint:** none of the three
  functions that consume the offer (`peer_reject_reason`,
  `validate_session_accept`, `eligible_batch_verifier`) reads either
  new field (verified by reading them — the reject surface is backend
  pairing only, the accept validation is echo/disclosure/budget
  only), so no scenario verdict depends on the values, and no serve
  loop or gate ever sees the sim's offer. The in-code comment marks
  the values as form-completeness, and the migration condition (the
  sim must adopt the real `msp1:<64 hex>` form if it ever becomes the
  family's second consumer per ADR §7) is recorded in the handoff's
  risk 3. Mechanical, honest, zero behavioral change; sim suites 6/0
  in every config this review ran.

Minor (no action): the two sites duplicate the params-hash
derivation expression; acceptable for two construction sites.

## Overall disposition for the light-up gate

**From the release seat: nothing in these three inputs blocks the
owner flag-flip package.** The §7 item-(iv) migration list — the
"before the family carries traffic" precondition the ADR attaches to
rev 4 — is discharged and independently verified at `4b35d81`
(input 1). The per-push CI the ADR venue matrix owes is present and
now robust (input 2; the required change is the explicit
`libp2p-backend` combination, applied and verified coverage-equivalent
in this change — push it WITH the flag-flip package). The
cross-boundary sim edits are correct and honest (input 3).

The remaining §7 rev-4 owner-gated items stand UNCHANGED and remain
preconditions/obligations of the flag flip itself, not blockers
introduced by this review: (i) the `capi-adapter` flag flip in
serving; (ii) the item-2 family exactness-pin reruns (the
`real-model-e2e.yml` extension or an owner-local run with a
`docs/verification/` record — venue-matrix obligation at light-up);
(iii) the Windows-Product composition completion (desktop surfaces,
the `serve_sessions_composed` plumb) and the F2B cross-machine relay
proof. When the family carries real traffic, the `MSP_LIVE=1`
live-wire harness obligation applies (`crates/modelswarm-node/tests/live_tracker.rs`
precedent). Record corrections owed by the implementer session: the
468/513/531 totals → 469/514/532, and the unlisted RC-5(b) corner pin.
One advisory routes to the Protocol Architect (RC-5(b) corner-case
wording, §1 advisory 1 above).

## Fixes made by this seat (this change)

- `.github/workflows/rust.yml`: both verifydrafts feature steps and
  both combo steps now enable `modelswarm-node/libp2p-backend`
  explicitly (with a comment recording why), making composed-daemon
  CI coverage independent of cargo's implicit workspace feature
  mechanisms. Coverage-equivalent (same counts, same green).

## Exact commands and outcomes (this seat, 2026-10-12)

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings` (new CI form) | clean |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend -- -D warnings` (new CI form) | clean |
| `cargo test --workspace` | **469 / 0 / 6** (claim 468 — off by the unlisted pin; baseline at `4e8fc18` re-run in a clean worktree: 462/0/6, exact) |
| `cargo test --workspace --features modelswarm-node/verifydrafts` | **514 / 0 / 7** (claim 513) |
| `cargo test --workspace --features modelswarm-node/verifydrafts-capi` | **532 / 0 / 12** (claim 531) |
| `cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend` (new CI form) | **514 / 0 / 7**, `verifydrafts_composed` 2/0 present |
| `cargo test --workspace --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend` (new CI form) | **532 / 0 / 12** |
| `cargo test -p modelswarm-session --test verifydrafts_admission_properties` | **15 / 0** (conjunction sweep green; §8e boundary pins re-based) |
| `cargo test -p modelswarm-session --test verifydrafts_security_review_pins` | **6 / 0** (was 3 at `4e8fc18`) |
| `cargo test -p modelswarm-session --test verifydrafts_vectors` | **13 / 0** |
| `cargo test -p modelswarm-session --test verifydrafts_vocabulary_disjointness` | **1 / 0** |
| `cargo test -p modelswarm-sim --test mixed_swarm` / `-p modelswarm-scheduler` | **6 / 0** / **36 / 0** |
| `cargo run -p modelswarm-session --example gen-verifydrafts-vectors` (×2) | 12 files; double-run `git diff` empty (idempotent) |
| `cargo test -p modelswarm-node --features verifydrafts --test verifydrafts_composed -- --list` | **0 tests** (the implicit-coverage proof) |
| `cargo test --workspace --features modelswarm-node/verifydrafts --test verifydrafts_composed -- --list` | **2 tests** (unification proof) |
| Fixture checks (python vs `git show 4b35d81~1:`) | reject-1 four existing canonicalizations byte-identical; offer-2 description-only; carrier golden: rule-2 recompute + canonical sha + 8-key form + fixture-1 hash match all TRUE |

## Assumptions

1. The three claimed configs are the complete per-push gate surface
   for this change (the libp2p-backend and tauri-shell steps predate
   the light-up commits and were reviewed in their own gates; both
   still run — the libp2p-backend step unchanged, and it now shares
   explicit features with the verifydrafts step).
2. Test-count equality across environments holds (same toolchain pin
   1.99.0 local and CI; the +1 delta is a handoff accounting error,
   proven by the clean-worktree baseline and the per-binary diff).
3. The corner-pin reading of RC-5(b) is treated as contract-compliant
   pending the Architect clarification (advisory 1); it is not
   grounds to block because it fails closed and I2 holds either way.

## Unresolved risks

1. The RC-5(b) corner semantics (strict global-no-outstanding
   exemption) is pinned by a test but the ADR text is ambiguous —
   routed to the Protocol Architect (advisory 1).
2. The G6 prompt-commitment gap stands (owner-gated freeze-ADR item,
   correctly recorded, not improvised).
3. A wire-level foreign-sender commit over the real transport is
   pinned at the gate and parse layers but not as a separate
   end-to-end transport test (implementer's handoff risk 2 — the two
   enforcement layers are covered; fold into the next wiring pass if
   desired).
4. CI wall time: the two explicit-combination steps re-run suites
   that overlap the plain feature steps; acceptable now (the check
   job's timeout is 90 min), revisit if the workspace grows.

## Suggested next task for the integrator

1. Merge this review + the `rust.yml` explicit-features fix WITH the
   owner flag-flip package (it is that package's CI).
2. Ask the rev-4 implementer session to correct the handoff/commit
   totals (469/514/532) and add the unlisted corner pin to the
   record.
3. Route advisory 1 (RC-5(b) corner wording) to the Protocol
   Architect for a rev-5 clarification.
