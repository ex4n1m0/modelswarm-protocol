# Review — ADR-036 implementation, Test and Release seat (2026-10-14)

Seat: Test and Release Engineer (ADR-036 §6 review gate; the ownership map's
required reviewer for the Windows Product leg).
Reviewed: implementation commits `2e9e468` → `39a8850` → `7aa22f2` at base
`a9b9ca3` (the ADR/protocol seat `1e371ed`/`2d3d060` is already merged
beneath them). **Sequencing note (corrected in the follow-up commit):** the
Security seat's review `21c3dc0` (verdict APPROVE; threat-model rows + 3
adversarial pins) landed on the shared tree WHILE this review ran. This
seat's full gate matrix below executed at `a9b9ca3` — proven by the exact
suite counts (589/0/7 vf+lp, which EXCLUDES the Security commit's 3 new
tests; at `21c3dc0` the same venue counts 592). After `21c3dc0` landed,
this seat re-verified its additions at the new HEAD: `cargo fmt --all
--check` clean; `cargo test -p modelswarm-node --features
verifydrafts,libp2p-backend --test verifydrafts_pin_carrier_security` →
**3/3 ok** (offer↔carrier session-id binding, both-keys discriminant,
commitment self-consistency); `cargo clippy -p modelswarm-node
--all-targets --features verifydrafts,libp2p-backend -- -D warnings`
clean. The Security commit touches only the threat model, its review doc,
and the new test file — no overlap with any surface this review re-ran at
`a9b9ca3`, so the matrix below stands as the verification of the Windows
Product implementation exactly as handed off.

## VERDICT: APPROVE-WITH-REQUIRED-CHANGES

One required change (R1, below — Integrator-owned, ~15 minutes). Everything
else passes on independent re-execution. Carrier-traffic gate status from
THIS seat: **approved conditional on R1 landing**; the Security seat's
review remains the other half of the gate per ADR-036 §6. Nothing in the
implementation blocks beyond R1.

---

## 1. Independent gate re-execution (this machine, clean re-run, HEAD a9b9ca3)

The handoff's numbers were NOT accepted on report; every venue was re-run
from scratch on this machine (Windows 10 26300 x64, rust-toolchain pin
1.99.0, no shared state with the implementer's runs).

| Gate (command) | Result | Handoff claimed | Match |
|---|---|---|---|
| `cargo fmt --all --check` | clean (exit 0) | clean | yes |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean | clean | yes |
| `cargo test --workspace` | **526 / 0 / 6** (73 suites, all ok) | 526/0/6 | EXACT |
| clippy `modelswarm-node/libp2p-backend` | clean | clean | yes |
| clippy `verifydrafts,libp2p-backend` | clean | clean | yes |
| `cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend` | **589 / 0 / 7** (73 suites, all ok) | 589/0/7 | EXACT |
| clippy `modelswarm-runtime/capi-adapter` | clean | clean | yes |
| clippy `verifydrafts-capi,libp2p-backend` | clean | clean | yes |
| `cargo test --workspace --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend` | **608 / 0 / 13** (73 suites, all ok) | 608/0/13 | EXACT |
| clippy `-p modelswarm-desktop --features tauri-shell` | clean | clean | yes |
| `cargo test -p modelswarm-desktop --features tauri-shell` | **56 / 0 / 0** | 56/0/0 | EXACT |

The pre-existing `modelswarm-winjob` close-all flake did NOT fire in this
re-run (noted honestly: it is machine-state-dependent; the handoff observed
it once and re-ran clean).

Every ADR-036-named test was individually confirmed `ok` in the re-run log:
`golden_vectors_replay_through_the_admission_table`,
`admission_first_failure_order_is_pinned`,
`open_sequence_order_is_fixed_and_neutral_stays_neutral`,
`carrier_session_seeds_the_gate_and_rejects_replays`,
`carrier_rejects_carry_codes_only_never_content`,
`engine_handle_teardown_bounds_the_session` (node); the 5
`pin_carrier_property.rs` tests + the golden replays (session);
all 6 `adr036_*`/`family_status_carries_the_carriage_disclosure_when_on`
(desktop).

## 2. Review scope findings

### 2.1 Fixture/vector coverage — PASS

- **The three vectors replay through the REAL admission path.**
  `crates/modelswarm-node/tests/verifydrafts_pin_carrier.rs:156-312` replays
  `pin-carrier-1-accept.json` (strict parse + the canonical-bytes round-trip
  byte-pin + accept with seq/nonce/token facts),
  `pin-carrier-2-mismatch.json` (re-signed honestly over the mutated form so
  the mismatch is reachable under a valid signature → `pin_mismatch`), and
  every row of `pin-carrier-malformed.json` (12 rows) through
  `admit_pin_carrier` (`serve.rs:556-624`) — the same table the composed
  daemon calls (`composed.rs:657`), not a session-crate parse shim.
- **Malformed array structural cases**: all 12 rows carry their pinned label
  (unknown field / envelope / non-bare-hex / non-u32 / empty tokens / wrong
  protocol_version / round≠0 / seq 0 / corrupted signature / sender≠key →
  bad_frame-class with the pure-validate labels riding verbatim), and the
  two admission-path outcomes are exercised where testable: carriage-off →
  `content_carriage_refused` (test :272-287) and context-overflow →
  `budget_infeasible` with the cap explicitly supplied (test :288-308).
- **session-reject-1.json extension consistency**: programmatically compared
  against the pre-ADR-036 revision — the six original canonicalizations are
  BYTE-IDENTICAL (preset_unavailable, backend_mismatch,
  batch_verify_unsupported, budget_infeasible, profile_mismatch,
  params_mismatch) and exactly two new entries exist (`pin_mismatch`,
  `content_carriage_refused`). G3 precedent followed.
- **Frozen-surface byte-identity**: `git diff 2e9e468~1..HEAD --
  protocol/msp-v1.md protocol/messages.proto
  protocol/vectors/lease-hubkey-1.json` is EMPTY — the frozen §6 surfaces
  and the lease vector are untouched (ADR-036 §5.3). The three
  implementation commits touch only node/desktop/ui/docs files (diffstat
  verified).

### 2.2 Property/pin suite (node + desktop) — PASS (one advisory)

- **First-failure order sweep** (`verifydrafts_pin_carrier.rs:317-475`):
  pairwise in the ADR's row order — schema beats signature (fault applied
  AFTER signing so both are live), signature beats carriage-off (a broken
  signature under an opted-out posture is bad_frame, not the refusal — the
  refusal is stated only to an authenticated peer), carriage beats profile,
  profile beats params, params beats pin, pin beats prefill (bad_pin applied
  after too_long so both stay live), prefill beats deadline, and the clean
  baseline admits. Faithful to ADR-036 §3.2's table.
- **Open-sequence order** (:539-622): all four valid shapes (lease→pin→offer,
  pin→offer, lease→offer, offer) plus the deviation classes (pin→lease,
  repeated lease, carrier-after-carrier) as `OutOfOrder` bad_frame ends, and
  the oversize carrier as `PayloadTooLarge` BEFORE parse (1.4 MB of ids).
- **seq/round/nonce invariants over the real staged transport**
  (:632-773): full carrier session over `SignedFrameTransport` — the
  client's first round rides seq 2 / round 1; a hostile re-send of seq 2 is
  refused `replayed_request` with the engine NEVER touched (calls()==1).
  The nonce-ledger arm is pinned in the teardown test (:998-1029): a fresh
  well-formed frame REUSING the carrier's nonce → `replayed_request`,
  engine calls 0, then the legit strictly-greater round succeeds.
- **Teardown-bounded engine** (:855-1092): a drop-counting wrapper on the
  per-session engine handle proves the serve loop's handle is dropped at
  session end (dropped==1) — the KV-residency bound at the seam. The
  requester side mirrors it in `try_family_verified_chat`
  (`app.rs:4043-4134`: `engine_for_root(Some(&root))` self-check engine,
  `drop(selfcheck)` after the loop).
- **The coordinator's self-check still gates every draft**: in the elected
  path EVERY window runs `selfcheck.verify_window` BEFORE `client.round`
  (`app.rs:4090-4115`); a locally divergent window breaks out with
  `local_selfcheck` and never hits the wire. The v0.2.23 self-check suite
  (including the 0/8 warm-cache-flip arm) still runs green under
  tauri-shell.
- **Advisory A1**: a pin carrier arriving MID-SESSION (after the offer) has
  no dedicated test. Behavior is fail-closed by construction — it hits
  `demux_family_frame` (`verifydrafts/mod.rs:244-252`), matches neither
  `window_tokens` nor `accepted_token_ids`, and ends the session as
  `UnexpectedFrame`. A one-test pin of exactly that would complete the ADR
  §6 "never routed" property literally; not required (the construction is
  closed and the open-position cases are pinned).

### 2.3 Default-on posture, opt-out, disclosure visibility — PASS

- Fresh install: `adr036_postures_default_on_serving_off_requester`
  (`app.rs:6337`) — carriage ON / election OFF from an empty dir, from a
  pre-ADR-036 three-key config (serde defaults), and through the persisted
  round-trip. `VerifyDraftsServeConfig::honest()` defaults
  `ContentCarriage::On` (`serve.rs:232`) and `compose_family_serving`
  flows the persisted setting in (`app.rs:1039`) — a stock install serves
  carrier sessions with no extra configuration. (The full fresh-install
  E2E across two live machines remains the standing post-gate rehearsal,
  correctly not run before this review.)
- Opt-out refusal: `adr036_opt_out_refusal_is_honest` (`app.rs:6499`) — the
  OFF posture flows into the serve config and the node admission table
  refuses a well-formed SIGNED carrier with `content_carriage_refused`,
  never a misleading `pin_mismatch`; the wire frame is byte-pinned
  codes-only in the node hygiene test (`verifydrafts_pin_carrier.rs:808-818`).
- Disclosures: both §3.1 texts VERBATIM in-binary with N substitution and
  the honesty clauses (`app.rs:6373`); `FamilyStatus.carriage` shape
  (verbatim on / honest off / null while family down, `app.rs:6403`);
  the narrowed `blocked_content_contract` arm frozen (`app.rs:6564`).
- **UI, independently re-run by this seat**: the ui-harness scenarios were
  replicated over the REAL `ui/index.html` with a stubbed Tauri bridge
  (jsdom executes the app's inline script; scenario objects identical to
  `ui-harness/adr036-carriage.html`'s): **16/16 PASS** — serving disclosure
  always-visible + verbatim, carriage toggle default ON + writes
  `set_content_carriage(false)`, election default OFF + hidden-while-
  unelected, opt-out honest line + toggle OFF, hosting-off hides the
  disclosure, elected N=1 verbatim + cost + hub-blind statements + toggle
  ON + writes `set_family_verified_chat(false)`, N=3 actual-cohort
  substitution. The 17th (rendered-geometry height at 1152×648) needs real
  layout: headless Edge is dead on this host (the standing 10d8ff1 note;
  verified again today), so that leg stands on the Integrator's in-browser
  17/17 (audit addendum, 2026-10-13) — recorded as Advisory A2, not
  blocking; the standing owner browser pass remains available.

### 2.4 Redaction suite — PASS on the enumerated assertions; ONE REQUIRED structural gap

What is landed and green:
- Carrier token ids and the content-derived commitment asserted ABSENT from
  the telemetry event stream on both the admitted-session and refusal paths
  (`verifydrafts_pin_carrier.rs:766-772`, `:819-822`) — the event stream IS
  the node log pipeline (every field passes the mandatory redactor at
  `modelswarm-telemetry/src/lib.rs:174`).
- Rejection frames codes-only, byte-pinned (:808-818); every new reject
  reason rides `ClientOpenError::Rejected` verbatim (:834-847).
- The webview status surface NEVER carries `prompt_tokens` /
  `pin_commitment` / arrays; the carriage object is exactly
  `{enabled, disclosure}`; the neutral `pin.pin_hash` stays allowed per
  ADR-034 rule 7 with the §4.4 non-transfer respected
  (`app.rs:6446-6490`).
- Receipts and the hub: the diff touches NO tracker or receipt code (the
  carrier rides P2P QUIC only; ReceiptV2 unmodified) — nothing new to
  test there, per ADR-036 §5.1/§5.4.

**REQUIRED CHANGE R1** — the structural redaction surface was NOT extended
to the carrier's field names or shapes. `Redactor::FORBIDDEN_FIELDS`
(`crates/modelswarm-telemetry/src/redact.rs:12-36`) contains
`token`/`tokens_text` but NOT `prompt_tokens` or `pin_commitment`.
Empirically: `redact_field("prompt_tokens", "[151644, 11, 22, 33]")` passes
the name check, and the value survives `scrub_content` (commas fragment the
hex runs below the ≥32 scrub bar at redact.rs:107-127; the array is not
secret-shaped and under the 256-char truncation) — a careless future caller
CAN leak a carrier payload through the very module whose documented
contract is "so a careless caller cannot leak by accident". ADR-036 §3.2
("never logged, never telemetered") and §6 ("family frames join the
prompt-redaction surface") make this the surface-joining half of the
redaction gate; the landed tests cover the current emit sites, but the
backstop must know the new content-bearing names and the token-array SHAPE.
Note the asymmetry: `pin_commitment`'s VALUE is already scrubbed by the
≥32-hex HEXBLOB rule, so its exposure risk is name-level only;
`prompt_tokens`-shaped arrays are the real vector — under ANY field name.

Exact required change (Integrator-owned crate — this seat does not edit it
without an ADR-log approval):
1. Append `"prompt_tokens"` and `"pin_commitment"` to `FORBIDDEN_FIELDS`
   (`crates/modelswarm-telemetry/src/redact.rs:12`). Grep-verified safe:
   NO production telemetry emit site uses either name as a field (the only
   `("prompt_tokens", …)` emit is the TEST count at
   `crates/modelswarm-session/tests/adversarial.rs:1769`, whose survival
   the test does not assert).
2. Add a SHAPE-based scrub for token-id arrays — the actual leak vector —
   in `Redactor` (either in `redact_field` or as a rule in
   `scrub_content`): a value matching a bare u32-array form
   (`^\[\s*\d+(\s*,\s*\d+)*\s*\]$`, length ≥ 1) becomes
   `[REDACTED:TOKEN_ARRAY]` under ANY field name. Do NOT add `"tokens"` to
   the forbidden list: seven legitimate counts-only sites emit
   `("tokens", <count>)` (desktop `app.rs:892,915,931,4180`;
   node `serve.rs:1027,1081`; `composed.rs:678`) and ADR-036 §3.2
   explicitly permits token COUNTS — the shape rule catches
   `("tokens", "[1,2,3]")` while leaving `("tokens", "8")` legible.
3. Pin tests in the same file's `mod tests`:
   `redact_field("prompt_tokens", "[151644, 11]")` → `[REDACTED:`;
   `redact_field("pin_commitment", &"a".repeat(64))` → `[REDACTED:`;
   `redact_field("note", "[151644, 11, 22]")` → contains
   `[REDACTED:TOKEN_ARRAY]`; `redact_field("tokens", "8")` stays `"8"`.
4. Re-run `cargo test -p modelswarm-telemetry` and the workspace default
   suite (the adversarial redaction test at `adversarial.rs:1764-1816`
   stays green — it asserts only CANARY absence and the drop markers).

R1 gates this seat's half of the carrier-traffic sign-off. It is small and
mechanical; everything else is approved.

### 2.5 Feature/venue matrix — CONFIRMED, no venue gap

The four reported combos re-ran EXACT (table in §1). CI mapping verified in
`.github/workflows/rust.yml`: the default step (:35), the libp2p-backend
step (:43), the **verifydrafts step (:56) runs the new
`verifydrafts_pin_carrier.rs` suite** (feature `verifydrafts` gates it,
`rust.yml:53-56`), the capi steps (:64, :75), and the **tauri-shell step
(:90) runs the 6 desktop ADR-036 tests** (tauri-shell chains
`verifydrafts-capi` → `verifydrafts` per
`crates/modelswarm-desktop/Cargo.toml:42,51-60`; the un-gated postures test
additionally runs in the default venue). The session property suite runs in
the default step. Remaining pre-existing limitation (not an ADR-036 gap):
all Rust TEST execution is windows-latest; the non-Windows job is
clippy-only — unchanged standing posture.

**T/R seat addition (this review)**: CI previously had ZERO coverage of
`ui/index.html` (the webview). Added a guards-job step
`.github/workflows/rust.yml` — "ADR-036 disclosure surfaces exist in the
webview" — asserting the four contract element IDs
(`carriage-line`, `carriage-switch`, `verified-chat-line`,
`verified-chat-switch`) and both IPC names exist in the markup. This is
string-level drift detection (a UI refactor dropping a disclosure surface
now fails CI instead of silently violating the always-visible contract);
the rendered 1152×648 harness stays a review-seat pass. Validated locally:
6/6 greps green, YAML parses.

### 2.6 Release readiness, v0.2.24 train — no blockers; obligations below

- **The version-bump trigger auto-fires (ba0ed1a)**: confirmed
  `Cargo.toml`/`Cargo.lock` are in `desktop-installer.yml`'s push paths —
  the v0.2.24 bump commit builds correctly-named artifacts immediately
  (this fixes the v0.2.23 near-miss where the auto-run built
  old-version-named installers).
- **The two-push window persists BY DESIGN**: the rust.yml guards job's
  SHA256SUMS check fails at the bump commit (sums still name 0.2.23) and
  goes green at the staging commit — the established 0.2.22/0.2.23
  discipline. The train should not "fix" this; it should stage promptly.
- **Release-notes obligations for v0.2.24** (this seat will check them at
  the train):
  1. Both §3.1 disclosures carried VERBATIM (serving + requester texts).
  2. §3.4 honesty: any sentence presenting family-verified chat as a speed
     feature is a release defect — the mode is slower by design.
  3. The serving-side BEHAVIOR CHANGE stated plainly: clients
     self-updating 0.2.23 → 0.2.24 acquire DEFAULT-ON carriage of other
     users' conversations (pre-ADR-036 configs read the serde default
     TRUE; owner-sanctioned, disclosed in-UI, opt-out honored) — it must
     be said in the notes, not just the UI.
  4. Standing rules unchanged: all-OS installers together from one bump;
     binaries never committed; `.sig` content never re-encoded;
     `--app-version` mandatory at manifest signing; deploy from the tree
     with the LF-normalized SHA256SUMS.

## 3. Assumptions

- The Integrator's audit addendum (gates + rendered harness 17/17) is
  treated as evidence for exactly one item — the rendered-geometry leg
  (A2); everything else was re-executed by this seat.
- The Security seat reviews separately (adversary matrix, disclosure
  contract, replay/reorder); this review does not pre-empt it.
- R1's minimum is the two name additions + the shape-based token-array
  scrub; if the Integrator prefers a different mechanism that provably
  redacts token-id arrays under arbitrary names AND the two carrier names,
  that satisfies the requirement.

## 4. Changed files by this review

- `.github/workflows/rust.yml` — the new guards step (§2.5; T/R-owned).
- `docs/reviews/review-adr036-implementation-testrelease-2026-10-14.md`
  — this file.

No production behavior was modified by this seat.

## 5. Carrier-traffic gate statement

Security seat: **APPROVED** (`21c3dc0`, 2026-10-14 — findings F1–F3 all
LOW/advisory; this seat independently re-ran the 3 new adversarial pins,
3/3 ok).
T/R seat: **APPROVED conditional on R1 landing** (the Redactor extension:
the two carrier field names + the shape-based token-array scrub + pin
tests, per §2.4). Carrier traffic is unblocked once the Integrator merges
R1 — that is the LAST open condition. Recommended order: R1 → re-run
`cargo test -p modelswarm-telemetry` + the workspace default suite →
the first cross-machine carrier chat (handoff §5.5) with the probe as the
pairing diagnostic.
