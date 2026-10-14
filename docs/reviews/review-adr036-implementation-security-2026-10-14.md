# Review — ADR-036 implementation, Security seat (2026-10-14)

Reviewer: Security Engineer (ADR-036 §6 review seat — one half of the
carrier-traffic gate; the Test and Release seat's sign-off is the other
half and carrier traffic stays blocked until it lands).

Subject: commits `2e9e468` (node), `39a8850` (desktop), `7aa22f2`
(harness + privacy.md) at HEAD `a9b9ca3`, reviewed against
`docs/adr/ADR-036-family-user-content-sessions.md` and the handoff
`docs/reviews/handoff-windows-product-engineer-2026-10-13-adr036.md`
(incl. the Integrator's audit addendum).

## Verdict

**APPROVE** — the Security seat signs off. Findings below are LOW
severity, advisory/informational; none block the gate. This approval
plus the T/R seat's approval unblocks carrier traffic per ADR-036 §6.

## What was reviewed and how

Line-by-line review of the admission path
(`crates/modelswarm-node/src/verifydrafts/serve.rs`,
`composed.rs`, `client.rs`, `engine_source.rs`, `wire.rs`), the
session-crate carrier types
(`crates/modelswarm-session/src/negotiation.rs`,
`verifydrafts.rs`), the desktop surfaces
(`crates/modelswarm-desktop/src/app.rs`, `ui/index.html`), the golden
vectors (`protocol/vectors/pin-carrier-*.json`), and the test suites;
independent gate runs (commands and outcomes in §Gates); adversarial
additions authored by this seat under `tests/`
(`crates/modelswarm-node/tests/verifydrafts_pin_carrier_security.rs`,
NEW, 3 tests); and the ADR-036 §6 threat-model updates landed in
`docs/threat-model.md` (this seat's owned deliverable).

## Scope item 1 — the admission path (first-failure order, binding,
refusal honesty, replay seeding, deadline clamp, oversize)

**Verified correct.**

- `admit_pin_carrier` (serve.rs:556-624) is row-for-row faithful to
  ADR-036 §3.2's table, including the subtle deferrals: the
  commitment-BIND `validate()` fault is caught early but deferred to
  row 7 (so schema < signature/sender < carriage < profile < params <
  pin ordering holds — `PinCarrierError::CommitmentMismatch` falls
  through at serve.rs:569-573); sender binding runs BEFORE the
  Ed25519 check (verifydrafts.rs `verify_family_signature`,
  `installation_id_for(expected_key) == sender` first); and the
  transport PeerId ↔ pinned-key equality is enforced independently
  (serve.rs:578-580). The ADR's `sender` row is fully satisfied:
  `sender` == installation id derived from the signing key AND the
  QUIC-authenticated remote PeerId derives from the same key.
- The carriage refusal is honest: `ContentCarriage::Off` →
  `SessionRejectReason::ContentCarriageRefused` (serve.rs:586-588),
  sent as a codes-only `SessionReject` (composed.rs:79-92) echoing
  only session id + protocol version + reason — byte-pinned by the
  shipped test. A refusal is never mislabeled `pin_mismatch`, and a
  mismatch is never softened to a refusal.
- Replay/nonce ledger seeding: `serve_verifydrafts_session_with_pin`
  seeds the gate via the public `seed_pre_state` with
  `last_seq[sender] = carrier.seq` and the carrier nonce
  (serve.rs:894-900). The carrier's `sender` is verified equal to
  `installation_id_for(coordinator_key)`, which is exactly the key the
  gate uses for later `VerifyDrafts` frames (verifydrafts.rs:1146,
  stage-3 sender binding) — the baseline provably binds. The shipped
  staged-transport test drives a real carrier session (first round
  seq 2 / round 1) and a carrier-nonce reuse → `replayed_request`
  with the engine untouched.
- Deadline: clamped `[1000, 120000]` at admission
  (serve.rs:609-614, `arrival + clamp(deadline_ms)`), re-checked
  immediately before prefill dispatch (composed.rs:722-730) — before
  `engine_for_root`, so a late deadline never constructs an engine.
- `payload_too_large` before parse: serve.rs:455 checks the serialized
  length before the strict `WirePinCarrier` parse; the transport layer
  independently enforces `MAX_FRAME_BYTES` before the body is read or
  allocated (frame.rs). See finding F2 for the label nuance.
- Open-sequence order: `read_session_open` implements the fixed
  `[lease]? → [pin]? → offer` with all four valid shapes and the
  deviation classes (`OutOfOrder`/`BadPinCarrier`/`PayloadTooLarge`),
  pinned by tests. A carrier arriving mid-session is `FamilyFrame::Unknown`
  → session end; it never reaches the frame router.
- The offer↔carrier `session_id` binding (serve.rs:707-712) had NO
  direct test — this seat added one
  (`offer_session_id_must_equal_the_carriers`): mismatched ids end the
  session `BadOffer("carrier session id mismatch")` before any accept,
  any send, and any engine pass. It passes.

## Scope item 2 — the commitment oracle

**Verified: the implementation matches the ADR's pre-analysis.**
`WirePinCarrier::commitment_matches` (negotiation.rs:327) recomputes
`sha256(prompt_tokens LE-u32)` over the payload the serving peer
ALREADY holds — pure self-consistency. The only outputs are a boolean
and, on mismatch, the frozen reason string `pin_mismatch` sent to the
already-authenticated sender. No content-derived value (the
commitment included) reaches telemetry, logs, receipts, or any other
frame — verified by grep sweep and pinned by two tests (the shipped
codes-only test and this seat's
`commitment_check_is_pure_self_consistency`). No third-party probing
surface exists: the check is unreachable without first passing the
lease gate and the signature/sender binding.

## Scope item 3 — redaction/hygiene in practice

**Verified counts-only discipline.**

- Workspace grep: `pin_commitment` appears only in the carrier
  construction/signature code, the types, and tests (positive
  constructions or negative assertions). No telemetry/log/status call
  site carries it.
- Node telemetry: `verifydrafts.pin_carrier_admitted` carries peer,
  token COUNT, seq; `pin_carrier_refused` carries peer + reason label
  (composed.rs:674-681, 693-695). The shipped test asserts the whole
  exchange's telemetry contains no `pin_commitment` and no token
  arrays.
- Desktop: the status surface pin
  (`adr036_carrier_fields_never_reach_the_status_surface`) asserts no
  `prompt_tokens`, no `pin_commitment`, no arrays; the carriage object
  is exactly `{enabled, disclosure}`. The neutral `pin.pin_hash`
  remains allowed per ADR-034 rule 7, and the §4.4 non-transfer to
  content-derived commitments is respected (the commitment appears
  nowhere).
- Receipts: untouched — the three commits modify no
  eligibility/receipt code; ReceiptV2 stays counters/digests only.
- Hub content-blindness: no tracker surface is touched; the elected
  path (`try_family_verified_chat`) performs no hub call at all (the
  pre-existing `tracker.lookup` runs before it, in `try_swarm_chat`);
  the carrier rides P2P QUIC only.

## Scope item 4 — the disclosure contract (both ends)

**Verified verbatim and always-visible.**

- Serving text (`CARRIAGE_DISCLOSURE_SERVING`, app.rs:225-228) and
  requester text (`CARRIAGE_DISCLOSURE_REQUESTER` + `{N}`
  substitution, app.rs:243-252) match ADR-036 §3.1 byte-for-byte,
  including the em-dashes. The requester text states the honest cost
  ("slower than single-peer chat, not faster"), the readable-form fact
  ("in a form the receiving computers can read"), and the hub-blind
  fact. No implication of encryption or speed anywhere; the opt-out
  line is the honest off statement. Pinned by
  `adr036_disclosure_texts_are_verbatim`.
- UI: `#carriage-line` is visible iff the family is ON and renders
  `fam.carriage.disclosure` verbatim; `#verified-chat-line` renders
  the daemon's `verified.disclosure` while elected (index.html:561-570,
  498-506). The reply meta labels the mode "slower by design".
  Default postures: carriage ON (fresh installs AND pre-ADR-036
  configs via serde default TRUE), election OFF — pinned by
  `adr036_postures_default_on_serving_off_requester`.

## Scope item 5 — KV residency and the threat-model rows

- The bound: `engine_for_root(Some(tokens))` constructs a FRESH
  per-session engine under the Capi seam (engine_source.rs:114-148);
  the serve loop's handle drops at session end (drop-counted at the
  seam by the shipped test), and the requester's self-check engine is
  dropped at turn end (`drop(selfcheck)` in the elected path). The
  `Shared` test venue ignores the root by design (no KV).
- **Threat-model updates LANDED by this seat** (docs/threat-model.md):
  the §3 adversary-B restatement under default-on carriage (every
  cohort member reads the conversation; owner-accepted; mitigations
  stated without overstatement; the consent-UX downgrade and the
  posture-not-on-the-wire honest boundary), the §4.2 carrier-exposure
  row plus the malformed-carrier and replay/reorder rows, the §4.3
  KV-residency row, the §5.2 extension ("every verifying peer of an
  elected family-verified chat reads the whole conversation it
  verifies"), and the §6 ADR-036 abuse-test index.

## Scope item 6 — rotation-deferral interplay

**Verified: nothing forces or assumes key rotation.** The carrier is
signed by the EXISTING installation identity
(`identity.installation_id()`, client.rs negotiate); the serving side
pins the coordinator key from the transport-authenticated PeerId
exactly as round frames already do; no new key material, no key
lifecycle change, no rotation trigger anywhere in the three commits
(grep-verified). The standing owner directive is respected.

## Findings (none blocking)

### F1 — LOW, advisory: the lease is validated after the whole open
exchange is read, weakening the ADR's disclosure-minimizing rationale

`read_session_open` reads `lease → pin → offer` to completion
(composed.rs:545-597) before `family_lease.lease_for` runs
(composed.rs:610-620). ADR-036 §3.2 states the rationale "the tiny
lease precedes the content-bearing carrier, so lease refusal happens
before the coordinator has sent the conversation" — under this
implementation the conversation bytes are always received and
JSON-parsed before any lease refusal. Reproduction: any coordinator
without a valid lease sends lease → pin carrier (≤256 KiB) → offer;
the serving daemon reads and parses all three frames, then refuses
(`verifydrafts.family_refused` / `no_lease`) — the token ids sat in
memory.

Severity rationale (why not blocking): this is msp-v1 parity — the
msp-v1 per-request `capability_token` rides INSIDE the
prompt-bearing `InferenceRequest`, so the full prompt text equally
arrives before the lease check on the §6 path; the family carrier is
bounded (256 KiB, admission-gated sessions); nothing is processed,
prefilled, logged, or telemetered on refusal; and ADR-036 §3.1's own
honest-boundary note (bytes of a dialed session cannot be un-received)
covers the adjacent carriage case. Recommended future hardening (a
seam change, protocol-adjacent, its own small change record): validate
the PRESENTED lease between frame 1 and frame 2 so the stream can be
dropped before the content frame is read.

### F2 — LOW, informational: transport-refused oversize carriers label
as `wire_failure`, not `payload_too_large`

The transport frame reader enforces `MAX_FRAME_BYTES` before the body
is read (frame.rs:54), so an oversize carrier over the real transport
surfaces as `SessionOpenError::Wire` → telemetry
`serving.session_ended wire_failure` (composed.rs:592-596); the
`PayloadTooLarge` branch (serve.rs:455) fires only for in-hand
pre-read values, which the bounded transports cannot produce. Both are
session ends before parse; only the operator-facing label differs from
ADR-036 §3.2 row 1. No action required; noted for observability
correlation.

### F3 — LOW, informational: a single-key partial carrier degrades to a
neutral offer, not a bad_frame

`is_pin_carrier_frame` (serve.rs:389-391) classifies on BOTH
`pin_commitment` AND `prompt_tokens` — consistent with ADR-036 §4.3's
plural "discriminant keys pin_commitment and prompt_tokens". Because
`SessionOffer` is a legacy ADR-032 shape without
`deny_unknown_fields`, a frame carrying only `prompt_tokens` (or only
`pin_commitment`) plus valid offer fields parses as a NEUTRAL offer
and the stray content-bearing key is silently dropped unprocessed
(never prefilled; a coordinator that intended a carrier then sees
honest divergence at round 1). Pinned by this seat's test
`partial_carrier_keys_in_the_offer_position_stay_neutral`. Optional
stricter future: classify on either key → bad_frame-class end.

## Gates (run by this seat, 2026-10-14)

- `cargo fmt --check` — clean (after one fmt fix in THIS seat's new
  test file; the implementation commits were already clean).
- `cargo clippy -p modelswarm-node --all-targets --features
  verifydrafts,libp2p-backend -- -D warnings` — clean.
- `cargo test -p modelswarm-node --features verifydrafts,libp2p-backend`
  — all green; `verifydrafts_pin_carrier` 6/6;
  `verifydrafts_pin_carrier_security` 3/3 (NEW);
  `verifydrafts_composed` 3/3; `verifydrafts_wiring` 8/8;
  `verifydrafts_quic` 4/4.
- `cargo test -p modelswarm-session` — all green, including the golden
  pin-carrier vectors (`golden_pin_carrier_1_accept`,
  `golden_pin_carrier_2_mismatch`), the property suite (incl.
  `pin_carrier_sender_mismatch_is_bad_frame_before_state`), and the
  vocabulary disjointness extension.
- `cargo test -p modelswarm-desktop --features tauri-shell` — 56
  passed / 0 failed (includes the six ADR-036 desktop tests).

## Changes landed by this seat with this review

- `crates/modelswarm-node/tests/verifydrafts_pin_carrier_security.rs`
  (NEW): the offer↔carrier session-id binding pin (engine untouched,
  nothing sent), the both-keys discriminant pin (both halves + the
  both-keys contrast), and the commitment self-consistency pin.
- `docs/threat-model.md`: the ADR-036 §6 rows this seat owns
  (§3 restatement, §4.2 rows, §4.3 KV row, §5.2 extension, §6 index,
  status note).
- This review document.

## Residual risks (accepted, stated for the record)

The owner-accepted N-party exposure is real and now lands on a live
wire surface; the disclosures and threat-model rows state it plainly.
Cross-machine carrier chat has not been rehearsed on live machines
(the gate preceded carrier traffic); the probe remains the pairing
diagnostic. The rendered-geometry UI leg was re-verified in-browser by
the Integrator (17/17) and re-checked textually by this seat.
