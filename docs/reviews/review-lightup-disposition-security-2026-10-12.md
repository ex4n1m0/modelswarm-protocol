# Security disposition check — ADR-032 rev-4 migrations + light-up surfaces (owner light-up decision)

Date: 2026-10-12. Reviewer: Security Engineer (independent blocking seat).
Scope: the revision-4 migration commit `4b35d81` (offer form v3 / §8e both
arms / RC-5(b) reclassification with the amended pin / §2 carrier sender
check / vocabulary-disjointness assertion) and the light-up commits
`f9ec2ff..4b35d81` security-relevant surfaces (composed daemon demux,
engine bridge seam, `FamilyLeaseSource` fail-closed default), against
`docs/adr/ADR-032-batched-speculative-verification.md` **revision 4**.
This review rules on the LIGHT-UP DISPOSITION (whether the family may
carry traffic), distinct from the ADR text, which stands accepted.

## Verdict

**APPROVE WITH REQUIRED CHANGES** for the light-up disposition.

Blocker: **0**. Required changes: **1** (RC-L1 — a wiring condition on
the light-up event itself, not a defect in the landed commits: the
production `FamilyLeaseSource` must enforce profile-bound eligibility
before the family carries traffic). Advisories: **4** (A-1 pinned by a
new test added by this review; A-2/A-3 resource-exhaustion bounds;
A-4 classification notes, no security effect).

The five landed migrations faithfully implement the revision-4 rulings;
the amended A1 pin freezes the ruled semantics exactly; the G2 carrier
rationale holds as implemented; the G3 offer binding closes the recorded
fail-closed gap with no consequential new oracle; the composed daemon
has no bypass or confusion surface I can drive; the disjointness test is
load-bearing in both directions. The one required change gates the
owner's production wiring, which is exactly the §7 item-(iii) work the
ADR already holds owner-gated.

## 1. Ruling on the A1 amendment (RC-5(b) stale-commit semantics) — FAITHFUL

ADR rev 4 §8 RC-5(b): a `PrefixCommit` for a round "already abandoned —
round handled but uncommitted, no outstanding result left to reproduce
against" is `CancelRound("round_mismatch")` with no state change, no
violation increment, never applied; commits failing RC-4 reproduction
while the result is outstanding remain violations.

Landed condition (`crates/modelswarm-session/src/verifydrafts.rs:1350-1355`):
`outstanding.is_none() && commit.round <= last_handled_round &&
commit.round > last_committed_round` → `CancelRound(RoundMismatch)` —
placed after the torn-down/session/params/sender/profile checks and the
committed-duplicate check, before the parent-chain check. Verified
against the ruling clause by clause:

- Post-expiry (abandoned, result discarded by `expire_wait_commit_if_due`
  → `abandon_outstanding`, `verifydrafts.rs:1413-1425`, `1436-1439`):
  exempt no-op, no violation, never applied (I2), no teardown across
  repeats, gap-forward retry admits — all frozen by the AMENDED pin
  `pin_verifier_late_commit_after_expiry_is_exempt_stale_round_mismatch`
  (`crates/modelswarm-session/tests/verifydrafts_security_review_pins.rs:294-389`),
  including the honest-learning path (stale reject → next round's
  `parent_prefix_hash` unchanged → admits).
- While-outstanding RC-4 failure remains a violation: frozen by the new
  scope pin
  `pin_commit_mismatch_while_result_outstanding_still_violates` (same
  file, `391-447`) — a hash-VALID commit over different ids while the
  result is outstanding is `commit_mismatch`, ONE violation, no teardown
  at one.
- Profile-mismatch commits still tear down even post-expiry (the profile
  check at `verifydrafts.rs:1318-1322` precedes the stale branch):
  consistent with the ruling, whose exemption is scoped to the
  UNKNOWABLE reproduction question — profile equality at commit time is
  fully knowable and is the "§8 commit-time equality" enforcement
  authority the ADR preserves.
- The pre-revision-4 pin (`pin_verifier_late_commit_after_expiry_is_commit_mismatch`)
  is gone, superseded in-file by the amended pin with the rev-4 citation
  — the freeze now matches the ruling, not the old behavior.

One corner deviates from a literal round-scoped reading of the ADR —
see advisory A-1 (pinned; unreachable for honest coordinators; charges
strictly MORE, never less).

## 2. Ruling on the G2 carrier posture — SOUND AS IMPLEMENTED

The §2 carrier is the bare 8-key object, no envelope, no signature
(`crates/modelswarm-node/src/verifydrafts/mod.rs:243-274`;
`protocol/vectors/verifydrafts-5-commit-carrier.json`). The
authentication rationale verified end to end:

1. **Channel authentication in the commit's direction.** QUIC: the
   coordinator key is PINNED from the QUIC-TLS-authenticated remote
   PeerId via the inverse ADR-020 derivation
   (`verifying_key_from_peer_id`, `mod.rs:121-143`) — prefix-checked,
   round-trip-guarded, fail-closed on non-Ed25519 forms; `dial_quic`
   verifies the connected PeerId against the pinned derivation
   (`wire.rs:362-380`). Staged: `accept_staged` resolves the handshake
   PeerId against the trusted directory and the listener pins the key
   (`wire.rs:203-230`). A third party cannot inject a commit frame on
   either venue.
2. **Sender check cannot be evicted.** The pinned value is computed at
   gate construction from the pinned key
   (`verifydrafts.rs:884`), never from frame content; the frame's
   `sender` is compared against it at `verifydrafts.rs:1315-1317`
   BEFORE any state (no violation, nothing consumed, round stays
   outstanding for a correct retry) — pinned by
   `pin_carrier_sender_mismatch_is_bad_frame_before_state`
   (`verifydrafts_security_review_pins.rs:492-558`).
3. **Strict parse before the gate.** Serve-loop commit arm
   (`crates/modelswarm-node/src/verifydrafts/serve.rs:578-607`):
   `deny_unknown_fields` deserialization (unknown fields, envelope
   tags, `signature` all fail) + `passes_strict_carrier_schema`
   (bare-64-hex on all three hash fields, `msp1:<64 hex>` profile) →
   `bad_frame` RejectionFrame before the gate; pinned by
   `wire_prefix_commit_strict_schema_pins_the_g2_carrier`
   (`mod.rs:435-480`).
4. **Harm bounded to round abandonment.** To advance the chain a commit
   must reproduce the verifier's OWN outstanding result — exact token
   ids AND `new_prefix_hash` (`verifydrafts.rs:1362-1376`) — and
   recompute under the §1 rule-2 construction (`1374-1376`). A forged
   commit that chains validly but does not reproduce abandons the round
   (`commit_mismatch`), exactly the harm of silence; **I2 is
   unaffected** — there is no path by which an unverified token
   advances the chain, and no path by which a commit evicts or weakens
   the sender check (the check is upstream of every mutation, including
   the profile teardown).
5. **Smuggling dead ends.** A frame carrying BOTH `window_tokens` and
   `accepted_token_ids` demuxes Request (`mod.rs:218-226`) and dies at
   the stage-2 strict parse: `parse_strict` requires the key set to
   EQUAL the declared field set (`verifydrafts.rs:286-304`) — the extra
   key is `UnknownKey` → `bad_frame`. Combined-key and unknown-field
   smuggling both fail closed.
6. **I2 coordinator property untouched:** the coordinator still admits
   results only per RC-3 (signature, remote-peer, nonce echo, seq
   mirror, recomputed hash), unchanged by this migration.

The golden vector `verifydrafts-5-commit-carrier.json` pins the exact
8-key wire form, canonical bytes + sha256, the rule-2 construction, the
sender derivation from the recorded fixture key, and replays through
the real gate against fixture 1 (`golden_verifydrafts_5_commit_carrier_bare_object_form`,
`crates/modelswarm-session/tests/verifydrafts_vectors.rs`).

## 3. Ruling on the G3 offer binding — GAP CLOSED, NO CONSEQUENTIAL ORACLE

- **Required fields + honest rejects:** `SessionOffer` v3 requires
  `profile_id` + `generation_params_hash` (absence fails schema; base
  and v2 forms preserved as failing data — pinned by
  `session_offer_form_v3_requires_both_binding_fields` and
  `golden_session_offer_2_is_superseded_failing_data`). The serve loop
  compares both against its own pinned config and rejects honestly
  (`serve.rs:345-380`: preset → profile → params → backend → budget)
  with `SessionReject` + telemetry + `ServeEndReason::RejectedOffer`;
  wired over the real staged transport by
  `offer_v3_profile_and_params_mismatch_reject_honestly`
  (`crates/modelswarm-node/tests/verifydrafts_wiring.rs`). The
  fail-closed gap (a params-mismatched pairing opening a session that
  fails every round at stage 4 with no teardown or discovery) is closed;
  a profile mismatch no longer costs a round + teardown to discover.
- **Oracle analysis (task question): does a prober learn the verifier's
  pinned profile/params by probing offers?** Yes in the trivial sense —
  the six reject reasons are distinguishable, so a probe confirms or
  refutes a CANDIDATE value. Ruled a non-issue under the threat model:
  (a) `profile_id` values are public swarm/catalog identifiers — the
  project rule means both sides already host exact P and the requester
  must know the profile to participate; confirming it discloses nothing
  protected. (b) The params-hash preimage domain is the pinned sampling
  object; presets are frozen public configurations already named in the
  offer, and a pinned seed is u64-width (confirmation-probing is
  infeasible). (c) Pre-revision-4 the same facts were discoverable at
  round 1 (stage 4 `bad_frame` vs stage 7 teardown) at HIGHER cost; the
  oracle moved information that was already probeable to a cheaper
  probe. (d) Probe cost is a full connection + handshake + one
  Admission slot (per-peer cap 2, total 8, `serving.rs:196-201`), and
  the lease seam refuses lease-less peers before the offer is even
  parsed in the composed venue (`composed.rs:279-286`). No asset in
  threat-model §2 is exposed. Not a finding.
- **No trusted input introduced:** verifier-side pinning is unchanged
  (stage 4 params equality, stage 7 profile + teardown, §8 commit-time
  equality) — the fields are discovery; a LYING offer now gets rejected
  at negotiation instead of opening a doomed session (strictly better).
  `SessionAccept` does not echo the fields; the frozen `EligibilityLease`
  shape is untouched (fixture byte-identity verified).
- Minor note (no security effect, not separately numbered): the offer's
  own deserialization tolerates unknown fields (the ADR pins strict
  schema for `VerifyDrafts` frames at stage 2, not for the offer);
  unknown offer keys are ignored and cannot collide with §6 tags
  without tripping the disjointness test.

## 4. Ruling on the composed daemon — NO BYPASS OR CONFUSION SURFACE FOUND

`serve_sessions_composed` (`crates/modelswarm-node/src/verifydrafts/composed.rs:199-322`):

- **One Admission gate, both vocabularies:** each accepted session holds
  one RAII slot for its whole lifetime regardless of vocabulary
  (`composed.rs:247-255`; guard release on drop, `serving.rs:185-194`) —
  total 8 / per-peer 2 defaults. Admission refusal answers the msp-v1
  `overloaded` StreamError; a family client cannot parse it and sees
  the session close — honest fail-closed both ways.
- **First-frame demux, race analysis:** typed frames return
  immediately from `session.recv` (zero added msp-v1 latency); raw
  frames are stashed by the typed recv and collected on ≤1 s poll
  boundaries via the stash-first raw recv (`composed.rs:132-156`). The
  transport's stash discipline drains the stash BEFORE any deadline
  check (`crates/modelswarm-transport/src/libp2p_backend.rs:231-294` —
  "stash first: a frame already received is never hidden behind an
  expired deadline"), recv timeouts are non-destructive wake-ups, and
  PINGs are answered below the vocabulary layer on both paths. No frame
  can be lost or double-read between the typed poll and the raw
  collect; the peeked frame is handed pre-read
  (`serve_one_seeded` / `PrefacedQuicWire`) and never decoded twice.
  The `serve_one` → `serve_one_seeded` extraction is behavior-identical
  for the msp-v1-only path (delegates with `None`; verified in the
  `4e8fc18` diff).
- **Vocabulary smuggling:** a family frame is swallowed by the typed
  path only if a §6 variant tag equals one of its top-level keys —
  excluded by the disjointness pin (§5 below) in BOTH directions; a
  frame with a §6 tag plus family keys fails typed decode
  (externally-tagged, extra key) AND fails the family strict parse
  (`UnknownKey`) → `bad_frame`. A malformed typed frame (valid JSON,
  §6 tag, wrong payload) classifies Raw → family offer parse fails →
  `BadOffer` session end — fail-closed, no reinterpretation (advisory
  A-4 records the classification nuance). Mid-session the family loop
  ends on any non-request/non-commit frame (`UnexpectedFrame`); the
  msp-v1 loop rejects non-request variants per its unchanged rules.
- **Lease-source fail-closed default:** `RefuseFamilyLeases` returns
  `None` for every peer → the family session is refused BEFORE the
  offer is parsed, telemetered `verifydrafts.family_refused/no_lease`,
  and the client's honest fallback-to-single signal is the transport
  failure — pinned by `family_session_refused_without_lease`
  (`tests/verifydrafts_composed.rs:304-376`). The expiry the seam
  supplies is enforced per request at the gate's stage 5. The lease is
  derived from the QUIC-authenticated PeerId, never a payload claim.
- **Engine seam:** the composed loop overrides the accept's
  `EngineDisclosure` with the injected engine's REAL capabilities
  (`composed.rs:301-303`) — honest geometry, pinned by
  `one_listener_serves_both_vocabularies` (the hand-set
  `batch_verify: false` default is never what a real engine answers).

## 5. Ruling on the vocabulary-disjointness test — LOAD-BEARING

`crates/modelswarm-session/tests/verifydrafts_vocabulary_disjointness.rs`
(default features, per-push; `rust.yml` runs it in every config):

- The §6 tags are extracted from the LIVE `WireMessage` enum via
  serde's own unknown-variant error — a probe that panics loudly if
  serde's wording changes (fail-closed, never a silent pass).
- `tags.len() == 10` + the ten expected names: ANY added variant fails
  the per-push build until the ADR + collision-test path of §2 rule (i)
  is exercised — this is what makes the assertion live rather than a
  stale copy.
- The family keys are built from the REAL wire shapes (offer, accept,
  reject, cancel, frame-reject, request, result), and every
  ADR-enumerated discriminant key except `accepted_token_ids` is proven
  a live field — the enumerated list cannot rot.
- Disjointness is asserted BOTH for the enumerated nine AND for the
  FULL live family field set (the mirror direction) — the mirror check
  is the one the composed demux actually depends on (classification is
  by typed-decode-success, so the hazard is "§6 tag == any family
  top-level key"), and the test is stronger than rule (i)'s letter.
- `accepted_token_ids` (the carrier's discriminant) is covered by the
  enumerated-nine disjointness assertion here, and its full field set
  is separately byte-pinned by the `verifydrafts-5-commit-carrier`
  golden vector (exact 8 keys + gate replay) — a two-layer composition
  that holds because both layers are per-push. Caveat recorded: the
  carrier's keys are fixture-pinned rather than live-extracted (the
  session-crate `CommitPrefix` is not serializable; the node mirror is
  feature-gated) — acceptable, and the in-test comment documents it.

## Findings

### Blocker

None.

### Required changes

**RC-L1 — the production `FamilyLeaseSource` must enforce PROFILE-BOUND
eligibility; the landed seam cannot express it.**
`crates/modelswarm-node/src/verifydrafts/composed.rs:75-79` reduces the
ADR-026 discipline to `lease_expires_at_ms(&peer_id) -> Option<u64>`:
the msp-v1 path's per-request check verifies a hub-signed lease for
exactly the served profile with consume right and slots
(`crates/modelswarm-node/src/serving.rs:1076-1106`: signature,
`lease.model_profile_id != served_profile`, `can_consume`, `slots`,
`lease.peer_id != remote_peer_id`), while the family seam's signature
drops every binding except peer identity — the served profile is not
even an input, so nothing in the type system forces the production
source to refuse a coordinator that hosts a DIFFERENT profile (the
project rule's serving-peer half for this family). The fail-closed
default and the ADR's "the production source is the daemon's
eligibility/roster state" wording make this a wiring obligation, not a
landed defect — but it is a CONDITION OF LIGHT-UP: before the family
carries traffic, either widen the trait (e.g. `(peer_id, profile_id)`
with the daemon's pinned profile) or pin, in the composition ticket and
its tests, that the production source returns `Some` only when the
authenticated peer holds eligibility for THIS daemon's exact profile
with consume right and a slot. Owner: Windows Product composition (§7
item iii) with Protocol review; the Security seat re-checks at the
light-up gate.

### Advisories

**A-1 — the stale-commit exemption is OUTSTANDING-gated, not
round-scoped: stricter than the rev-4 letter in one corner (pinned by
this review).** A late commit for an OLDER abandoned round that arrives
while a NEWER round is outstanding does not hit the exempt branch
(`verifydrafts.rs:1350`: `outstanding.is_none()` is false because round
n+k is outstanding); with a matching parent it falls to the RC-4 check,
is charged `commit_mismatch` (a violation), and the abandonment it
triggers ALSO discards the newer round's outstanding result. The ADR
letter is round-scoped ("a round already abandoned — no outstanding
result left to reproduce against" — the older round's result IS gone),
so the letter would classify this as an exempt stale no-op.
Reachability: NOT reachable by an honest coordinator — the shipped
client commits round n inside `round()` before it can request round n+k
(`crates/modelswarm-node/src/verifydrafts/client.rs:321-431`), and both
venues are ordered streams, so a delayed commit cannot arrive after a
later request it preceded; only a coordinator deliberately committing a
round it already abandoned hits it, and the charge lands on that
sender. The deviation charges MORE, never less; no bypass. Pinned by
the new
`pin_stale_commit_for_older_round_while_newer_outstanding_is_charged`
(`verifydrafts_security_review_pins.rs`) so any move to the round-scoped
letter is deliberate. Route: Protocol Architect (amend the condition or
record the deviation in the freeze ADR).

**A-2 — unbounded cross-vocabulary stash (memory exhaustion, bounded by
time and admission).** Under a family serve loop, typed §6-shaped
frames arriving mid-session are stashed into `stashed_typed` and never
consumed (`libp2p_backend.rs:290`); under the msp-v1 loop, raw frames
stash into `stashed_raw` (`:254`). Both queues are unbounded within the
session: a hostile peer can push 256 KiB frames (per-frame cap enforced
pre-allocation, `libp2p_backend.rs:347-359`) for as long as the session
lives. Bounds: the family session idles out 300 s after the last
SURFACED frame (stashed frames do not reset the idle timer), the msp-v1
loop's between-requests wait is 300 s, and Admission caps concurrent
sessions (8 total / 2 per peer) — so the exposure is a few hundred MB
at line rate per slot in the worst case, not unbounded growth. Still,
a bounded stash (teardown on overflow, or a cap with the honest
session-end) is the right hardening at the transport seam. Route:
Network Engineer (`crates/modelswarm-transport`), with a Security
re-check. Predates rev 4 (`cdaf378`); the composed daemon makes both
directions reachable from one listener.

**A-3 — family session residence is unbounded under frame dribble
(slot-holding without engine work).** Every successfully received frame
resets the family loop's idle timer — including frames that are
immediately rejected (`serve.rs:570` resets before demux; malformed
carriers get `bad_frame` at `:585-607` and the loop continues). A
hostile coordinator can hold a family session (and its Admission slot)
indefinitely with one garbage frame per <300 s. The §8e seconds arm
correctly stops new ADMITTED rounds past the cap, but nothing bounds
session LIFETIME; the msp-v1 venue has no equivalent residence (its
loop is per-request with the 300 s between-requests wait and no
activity reset from rejected frames). Bounded impact: per-peer cap 2
slots, zero engine work consumed, codes/counts-only telemetry. Recommend
a hard session-lifetime ceiling (e.g. the accepted budget's
`max_endpoint_seconds` as a wall clock, not just a round-admission
gate) at the composed/serve seam. Route: Windows Product composition +
Protocol ruling (it is a serve-loop policy choice the ADR does not
pin).

**A-4 — two demux classification nuances, both fail-closed (no security
effect; record in the freeze ADR).** (1) A malformed typed frame (valid
JSON, §6 tag, payload that fails the variant's schema) classifies Raw
at the transport (`from_slice::<WireMessage>` fails → Raw,
`libp2p_backend.rs:364-371`) and therefore Family at the composed demux
→ `BadOffer` session end, never an msp-v1 `bad_frame` answer. (2) A
family client sending a well-formed §6 frame (e.g. a non-PING control)
gets it stashed forever under `recv_json` — invisible, no reply, no
state change. Both end fail-closed with honest labels; neither
reinterprets a frame across vocabularies. Fold into the freeze ADR's
vocabulary record so second implementers match the classification.

## Tests added by this review (pin only — no behavior change)

`crates/modelswarm-session/tests/verifydrafts_security_review_pins.rs`
(+1 test, labeled, co-owned session-crate tests per mandate):
- `pin_stale_commit_for_older_round_while_newer_outstanding_is_charged`
  (freezes A-1's corner classification, the violation charge, the
  never-applied chain, and the newer-round force-abandonment side
  effect).

## Gates (run from the working tree at `4b35d81` + this review's test)

| Gate | Command | Result |
|---|---|---|
| fmt | `cargo fmt --all --check` | clean |
| clippy (default) | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| clippy (verifydrafts) | `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts -- -D warnings` | clean |
| clippy (verifydrafts-capi) | `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi -- -D warnings` | clean |
| tests (default) | `cargo test --workspace` | **469 / 0 / 6** (468 rev-4 baseline + this review's 1 pin) |
| tests (verifydrafts) | `cargo test --workspace --features modelswarm-node/verifydrafts` | **514 / 0 / 7** (513 + 1) |
| tests (verifydrafts-capi) | `cargo test --workspace --features modelswarm-node/verifydrafts-capi` | **532 / 0 / 12** (531 + 1) |
| scoped pins | `cargo test -p modelswarm-session --test verifydrafts_security_review_pins` | 6/0 (5 prior + 1 new) |

Independent verifications beyond the handoff's claims: the four
pre-existing `session-reject-1.json` canonicalizations are
byte-identical across `4e8fc18` → `4b35d81` (checked programmatically
against `git show 4e8fc18:`), with exactly the two new reasons added;
CI steps for all three feature configs exist in
`.github/workflows/rust.yml` (lines 47-66).

## Disposition conditions (what APPROVE WITH REQUIRED CHANGES means here)

The owner may light the family up when ALL of the following hold:
1. RC-L1 discharged (production `FamilyLeaseSource` profile-bound, with
   a test that a wrong-profile coordinator is refused).
2. The already-owner-gated §7 items ride along unchanged: (i) the
   `capi-adapter` flag flip, (ii) the item-2 family exactness-pin
   reruns (`real-model-e2e.yml` extension), (iii) the Windows-Product
   composition consumption + F2B cross-machine relay proof.
3. The remaining §7-recorded gaps stay recorded and do not widen: G5
   (no coordinator resync after `parent_mismatch`) and the G6 prompt
   commitment gap (out-of-band pinning; freeze-ADR item).

A-1..A-4 do not gate light-up; they route to their owners with the
pins/rechecks noted above.

## Changed files (this review)

- `crates/modelswarm-session/tests/verifydrafts_security_review_pins.rs`
  (+1 pin test, A-1 corner).
- `docs/reviews/review-lightup-disposition-security-2026-10-12.md`
  (this document).

## Assumptions

1. The ADR revision-4 text (`5a6c6a9`) is the binding contract; this
   review's A-1 advisory measures the implementation against its
   round-scoped letter, and the amended Security pin measures it
   against the ruled semantics — both are recorded, neither silently
   reinterpreted.
2. "Light-up" = the owner flipping the OFF-by-default features on in
   serving and letting the family carry traffic; all landed code is
   feature-gated OFF at HEAD, so nothing here is exploitable in the
   default build today.
3. Honest-coordinator reachability arguments assume one ordered stream
   per session on both venues (QUIC stream ordering; staged framing),
   which the client's sequential `round()` preserves.

## Unresolved risks

- RC-L1 until the composition work lands its production lease source.
- A-2/A-3 hardening is bounded-but-real resource exhaustion surface
  that grows with fleet exposure; re-check at light-up and after any
  relay introduction (relay changes the admission calculus).

## Suggested next task for the integrator

Route RC-L1 to the Windows Product composition ticket with Protocol
review; route A-1 to the Protocol Architect (condition-vs-letter
ruling), A-2 to Network, A-3 to Windows Product + Protocol, A-4 into
the freeze ADR's vocabulary record; record this disposition in ADR-032
§7 alongside the light-up ledger.
