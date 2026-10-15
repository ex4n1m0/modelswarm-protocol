# Review — ADR-038 relay admission, Test and Release seat (2026-10-15)

Seat: Test and Release Engineer — the ADR-038 §6.2 review gate (one of the two
sign-offs D12's close requires; the other, §6.1, is the Security seat's).
Reviewed at HEAD `54d8e4e` on `main`: the implementation chain
`c9fcc9f` (ADR + frozen-document amendments) → `2f8cedb` (tracker minting)
→ `af1dc01` (ticket type + four golden vectors) → `ba5e4a8` (TS vector
cross-tie) → `13046f0` (relay admission gate + client pre-flight) →
`54d8e4e` (node ticket cache). Machine: Windows 10 26300 x64,
rust-toolchain pin 1.99.0, Node 22; nothing accepted on report — every
claimed venue was re-run from scratch on this machine.

**Sequencing note (same shape as the ADR-036 review):** the Security seat's
§6.1 review (`docs/reviews/review-adr038-admission-security-2026-10-15.md`,
verdict APPROVE) landed on the shared tree WHILE this review ran, together
with its `docs/threat-model.md` companion edit — both still uncommitted in
the working tree at the time of this writing and therefore NOT part of this
seat's commit (they are the Security seat's to land). Their arrival changed
no code surface this review re-ran: all gate matrices below executed at
`54d8e4e`, and the only interaction is §3's drift adjudication, where the
Security ruling is recorded as binding per its own §4.

## VERDICT: APPROVE

All eight §6.2 requirements pass on independent re-execution (§2). This
review also LANDS this seat's own §6.2(7) obligation — the wire-compat job
extension (§2 item 7, §4) — verified end-to-end against a real locally
started TS tracker. The known vector drift is adjudicated in §3 per the
Security seat's binding ruling (non-blocking, follow-up routed to Protocol).
Nothing blocks. **D12 itself closes only per ADR-038 §6.3's own structure:
this §6.2 sign-off + the Security §6.1 approval + the owner-gated
real-network verification entry against the REDEPLOYED relay (§5) — the
entry does not exist yet (`docs/verification/` has no `relay-admission-*.md`),
so until it does the deployed F2B relay must not run `--public` on the old
ungated binary.**

---

## 1. Independent re-execution matrix (this machine, HEAD 54d8e4e)

| Command | Result | Handoff claimed | Match |
|---|---|---|---|
| `cargo test -p modelswarm-relay` (run 1 / 2 / 3) | 28 passed / 0 failed; 5.83 s / 5.80 s / 5.78 s | 28/0 ×3 (5.79/5.81/5.83 s) | yes (stability rule held) |
| `cargo test -p modelswarm-transport --features libp2p-backend` | 56 + 12 passed / 0 failed | 56 + 12 / 0 | yes |
| `cargo test -p modelswarm-node --features libp2p-backend --lib relay_ticket` (×3) | 7 passed / 0 failed each (0.07 s) | 7/0 ×3 (0.07 s) | yes |
| `cargo test -p modelswarm-eligibility` | 20 + 4 passed / 0 failed | (in workspace count) | green |
| `cargo test -p modelswarm-eligibility --test relay_ticket_vectors` | 4 passed / 0 failed (accept / expired / forge corpus / frame composition) | — | green |
| `cargo test -p modelswarm-tracker-api` | 6 + 5 passed / 0 failed | 6 + 5 | yes |
| `cargo test --workspace` | 79 suites `ok`, 0 failed, exit 0 | 79 `ok`, 638 passed | yes |
| `cargo test --workspace --features modelswarm-node/libp2p-backend` | 79 suites `ok`, 0 failed, exit 0 | 79 `ok`, 672 passed | yes |
| `npx vitest run tests/relay-ticket.test.ts` (apps/tracker) | 26 passed / 0 failed | 26/26 | yes |
| `npm run test` (apps/tracker; no DATABASE_URL) | 162 passed / 2 skipped / 0 failed | 162/2/0 | yes |
| `npm run typecheck` (apps/tracker) | clean | clean | yes |
| `npm run build` (apps/tracker) | production build succeeded | succeeded | yes |
| `npm run validate:vectors` | PASS (all vectors incl. challenge-prompt-1) | — | green |
| `cargo fmt --check` | clean | clean | yes |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings | clean | yes |

## 2. The §6.2 eight-point checklist

**(1) Fail-closed proofs — PASS.**
`gate_fails_closed_admits_on_ticket_and_recloses_on_expiry`
(`crates/modelswarm-relay/src/main.rs:1426-1671`) drives the REAL in-process
rust-libp2p relay stack through all three phases against a settable virtual
clock: (i) a standard libp2p client that NEVER runs the pre-flight is
refused reservation AND circuit with `ResourceLimitExceeded` (the circuit
denial arriving before any `NoReservation` outcome — the gate precedes the
reservation lookup in the pinned 0.22.0 source order); (ii) fixture-key
tickets for the same PeerIds admit — reserve → circuit → bidirectional
ping; (iii) the clock advanced past `expires_at + 120 s` re-closes BOTH
gates. Verified by reading the test AND by the 3× relay-crate runs above.
Unit halves also green: `gate_limiters_refuse_unadmitted_peers` (gate at
index 0 of BOTH limiter vectors — the S8 property),
`gate_predicate_is_deadline_bounded`,
`admission_table_capacity_and_deadline_eviction`.

**(2) Forge corpus with ORDER pins — PASS.**
All 12 `relay-ticket-forge-1.json` rows re-derived in code and asserted
against the frozen status+reason pairs:
`forge_corpus_rows_match_the_frozen_pairs` (relay, checks 2-10 with the
corpus context) and `relay_ticket_forge_corpus` (eligibility — the test
that READS the vector file's `authenticated_remote_peer_id` /
`derived_installation_id` context fields). ORDER pinned by
`first_failure_wins_order_is_pinned`: syntax ≺ signature ≺ peer binding ≺
installation ≺ capacity — a ticket failing both signature and binding
reports `bad_signature`; failing syntax and signature reports `malformed`.
The TS tracker-side mirror is in the 26 green TS tests.

**(3) Caps at the frozen numbers + announcement — PASS.**
`absent_flags_mean_the_frozen_adr038_policy` pins §3.5 exactly (absent
flags ⇒ 600 s reservation duration / 1800 s circuit duration / 16 MiB
circuit bytes / gate prepended);
`armed_cap_flags_override_the_frozen_defaults_exactly` and
`cap_flags_parse_validate_and_default_to_the_frozen_policy` pin the flag
paths; `limits_carry_streams_and_scale_reservations` and
`limit_summary_reports_the_effective_posture` pin the honest posture line;
`flag_armed_duration_cap_cuts_the_circuit` /
`flag_armed_byte_cap_cuts_the_circuit` prove the cut machinery (behind the
gate). T5 announcement:
`reservation_surfaces_the_relay_terms` (transport) asserts every
reservation response ANNOUNCES 1800 s and 16 MiB and the ~600 s lease;
`renewal_interval_renews_at_three_quarters_of_the_lease` pins the 450 s
renewal cadence. All green in the runs above.

**(4) Boot discipline — PASS (including an independent live smoke).**
`public_boot_requires_a_resolvable_tracker_key` (embedded key resolves;
valid `--tracker-key` override; missing/wrong-length/uppercase/non-hex
refused naming the shape), `tracker_key_flag_parses`,
`parse_args_rejects_malformed_input`, and the unchanged loopback default
(`non_loopback_bind_requires_the_explicit_public_opt_in`). This seat ALSO
re-ran the live binary smoke rather than trusting the handoff:
`cargo run -p modelswarm-relay -- --public --tracker-key <non-hex file>` →
`refusing to start (--public requires a valid tracker key...)` with **true
exit code 2**; `--help` exits 0.

**(5) Client paths incl. `na` / renewal — PASS.**
Transport presentation suite (all green):
`pre_flight_ok_then_hop_proceeds` (order `[admission, hop]`, exactly ONE
PRESENT, finite terms recorded), 
`pre_flight_refused_means_no_hop_stream_and_verbatim_label` (202 → no hop
stream on the connection for reserve AND dial, `permission_denied(202)`
verbatim), `na_relay_dials_directly_with_and_without_a_ticket`
(third-party interop both with a cached ticket and with the empty cache —
the pre-wiring default), `renewal_re_presents_only_when_the_ticket_is_near_expiry`
(fresh ticket presented once; near-expiry re-presented every renewal), plus
the unit pins `renewal_re_presentation_rule_matches_the_frozen_window` and
`cached_ticket_deadline_parses_from_the_vector_token`. Node cache half
(`relay_ticket_cache::tests`, 7/0 ×3): heartbeat populates the cache and
the source closure; absent field keeps the previous value (deploy skew);
malformed/oversized dropped with the last good kept; replace-never-
accumulate; stale supplied verbatim (no second freshness policy — the
transport window and relay check 6 own freshness); `attach_to` wires a
real transport; `attached_relay_tickets_leave_direct_dials_untouched`
(direct dials never perturbed). The empty-cache-skips-the-offer seam
decision is documented in the Network handoff and read by this seat — the
alternative would surface a misleading 400; accepted.

**(6) Content-blindness / redaction — PASS.**
T2 field set asserted EXACTLY in both stacks (TS issuance tests + the Rust
`deny_unknown_fields` canonical round-trip inside `from_wire`). Relay logs:
`log_lines_carry_no_ticket_bytes` (no serialized ticket, no base64 payload;
the frozen `admission accepted/denied` vocabulary verbatim). Node/desktop:
NO ticket telemetry event exists at all — the Windows Product seat's
honest note that no name in the frozen vocabulary fits a counts-only
dropped-ticket line is accepted (inventing an event name here would need
an ADR); this seat grep-verified `modelswarm-telemetry` carries no ticket
surface and the node/desktop references are cache/wiring only.

**(7) Wire-compat lockstep — PASS; this seat's obligation, LANDED in this
review.** Two halves:

- *Heartbeat field (live).* The harness
  `crates/modelswarm-node/tests/live_tracker.rs` step 3b (new): the Rust
  client asserts the TS tracker's heartbeat response carries `relayTicket`,
  parses it with the frozen `from_wire` (canonical bytes, exact field set,
  600 s TTL bound before any allocation), verifies the Ed25519 signature
  against the SAME pinned hub key the §5 lease check uses, and asserts the
  ADR-020 binding pair (`peer_id` == the registered row,
  `installation_id` == the authenticated identity's derivation) exactly as
  the relay's checks 7-8 would bind it — the cross-language surface where
  a TS/Rust derivation drift would otherwise ship invisibly. The empty-
  string/absent direction (OLD tracker, tracker-first deploy order) stays
  pinned tracker-api-side by
  `heartbeat_response_relay_ticket_deploy_skew_tolerance` (`#[serde(default)]`
  absence tolerance + `relayTicket` wire-name round-trip); against a
  current tracker the field must be PRESENT — the assert fails loudly on a
  wire-name drift or an un-deployed minting build, which is precisely the
  harness's job. **Verified locally end-to-end** (not just compiled): a
  real tracker (`next start`, MemoryStore, MSP_HUB_SEED=0707…, auto-approve,
  seeded + promoted profile) was started on 127.0.0.1:3010 and the ignored
  harness run against it TWICE:
  `MSP_LIVE=1 MSP_TRACKER=http://127.0.0.1:3010 MSP_LIVE_PROFILE=… 
  MSP_HUB_PUBLIC_KEY_HEX=ea4a6c63… cargo test -p modelswarm-node
  --features libp2p-backend --test live_tracker -- --ignored
  signed_flow_enroll_register_heartbeat_lookup` →
  `heartbeat relayTicket: TS-minted, Rust-verified, identity-bound
  (ADR-038 §6.2(7))` … 1 passed / 0 failed, on both runs (the full
  enroll→register→heartbeat→lookup→lease-earn→drain chain stayed green).
- *Vector (file).* The wire-compat job (`.github/workflows/tracker.yml`)
  gains a step running the EXISTING Rust vector consumer:
  `cargo test -p modelswarm-eligibility --test relay_ticket_vectors`
  (4 tests: accept canonical/sha/signature/TTL/deadline; expired;
  forge corpus; frame composition) — the SAME frozen files
  `apps/tracker/tests/relay-ticket.test.ts` pins, so the ticket
  bytes/canonical form/signature stay lockstep across both language stacks
  inside one CI job. The step was run locally: 4/0. The job's comment
  block documents the extension. YAML validated by parse (actionlint not
  installed on this machine).

**(8) Guards — PASS.**
`git diff c9fcc9f~1..HEAD -- protocol/messages.proto catalog/schema.json
catalog/schema-v2.json protocol/msp-cooperative-v1.md` is EMPTY — all four
frozen surfaces byte-identical since before the ADR landed (their
last-touch commits predate it entirely), i.e. the admission protocol
correctly stayed transport-layer. `npm run validate:vectors` PASS. The
msp-v1 amendments are exactly §4's texts, all verified in place: §3.3
heartbeat row (line 116, verbatim §4.1), §5a inserted between §5 and §6
(line 259), §9 changelog row (line 414, verbatim §4.3), and the ADR-014
revision note (§4.4). The workspace guards in `rust.yml` are unchanged and
green (their assertions ran inside the suites above).

## 3. Drift adjudication — the fixtures' coverage split (task item)

**Question:** the golden vectors' `installation_id`
(`b58-installation-relay-0001`) is a placeholder that cannot satisfy check
8's real ADR-020 derivation, so the vectors pin checks 1-6 + the order
while the binding checks are pinned by re-derived fixtures. Rule
(a) regenerate-with-derived-pair now, or (b) accept-with-documentation?

**Ruling: CONCUR with the Security seat's binding ruling (b) — accept the
split coverage for the D12 close; regeneration is a REQUIRED follow-up for
the Protocol seat in the next vector revision, not a condition of this
sign-off.** The Security review's §4 (which self-declares binding for this
seat) landed while this review ran; per the task's split procedure, this
seat defers to it and records it here so it is not re-litigated. This
seat's independent reading agrees on the merits: no security coverage gap
exists (checks 7-8 are pinned by validly-signed re-derived fixtures at
unit level AND end-to-end by the in-process proofs, which mint with real
ADR-020 derivations); the vector's own description scopes it to
`from_wire + verify` acceptance, which it delivers and which the TS↔Rust
byte-parity gate consumes; and a mid-gate regeneration would churn
Protocol-owned artifacts consumed by five Rust suites plus the TS parity
test during the close itself. This seat's residual conformance concern —
the ADR §6.2 table's "outcome ok(100)" row is only reachable through a
fabricated check-8 input — is honestly discharged by the landed test
`real_derivation_rejects_the_vector_placeholder_installation`
(admission.rs:1389-1417) and is exactly what the required follow-up fixes.

**Required follow-up (Protocol seat, next vector revision — recorded, NOT
landed by this review):** regenerate `relay-ticket-1.json` /
`relay-ticket-2-expired.json` from a recorded peer identity seed whose
PeerId's real `installation_id_for` derivation is used verbatim, fix
`relay-ticket-forge-1.json`'s `derived_installation_id` (today it records
the placeholder as if it were the real derivation — the one place the
drift actively misstates a fact; note
`crates/modelswarm-eligibility/tests/relay_ticket_vectors.rs:149` is the
consumer that then asserts a fabricated binding succeeds), and update the
consuming tests in the same change. One precision this seat adds for that
revision: for the accept vector to be replayable END-TO-END by a second
implementation, the vector must ALSO record the peer's identity SEED (the
way the relay tests use `[9u8;32]`) — without it, checks 7-8 cannot be
exercised by any consumer that does not already hold the peer's key,
regardless of the `installation_id` value. Until the revision lands, the
drift is documented in: the honest-rejection test, the Network handoff
(assumption 4 / risk 1), the Security review §4, and
`docs/threat-model.md` §6 (the Security seat's companion edit).

## 4. Changes landed by this review (this seat's owned paths)

| Path | Change |
|---|---|
| `.github/workflows/tracker.yml` | wire-compat job: new step `Rust relay-ticket vector consumer (ADR-038 §6.2(7) lockstep)` running `cargo test -p modelswarm-eligibility --test relay_ticket_vectors` before the live harness step; job comment block extended to document the ADR-038 extension (heartbeat field asserted in-harness; vector consumed by the new step; additive-field tolerance pinned tracker-api-side). |
| `crates/modelswarm-node/tests/live_tracker.rs` | harness step 3b (under `#[cfg(feature = "libp2p-backend")]`): the heartbeat `relayTicket` end-to-end lockstep assertions described in §2(7) — present-field assert, frozen `from_wire` parse, hub-key Ed25519 verify, ADR-020 binding-pair asserts. No production code touched; both feature states compile (`--no-run` verified for `libp2p-backend` and default). |

Gates for the changes: `cargo fmt --check` clean; `cargo clippy
--workspace --all-targets -- -D warnings` clean; `cargo clippy -p
modelswarm-node --features libp2p-backend --all-targets -- -D warnings`
clean; `cargo test --workspace` and `cargo test --workspace --features
modelswarm-node/libp2p-backend` 79 suites ok / 0 failed each; live harness
1/0 twice against a real local tracker; YAML parses (actionlint
unavailable on this machine — careful manual review of the two edits
instead; both are additive within an existing job).

## 5. What remains for §6.3 (owner-gated real-network entry)

Per ADR-038 §6.3, gate D12 closes only when §6.1 + §6.2 approve (now both
done) AND `docs/verification/relay-admission-<date>.md` exists, recording
against the REAL deployed relay: (a) a ticketed cross-NAT reservation +
circuit + streamed completion (the F2B exit shape, now ticketed);
(b) the ticketless-refusal proof from a second machine; (c) the relay's
log lines for both; (d) the tracker heartbeat issuance capture (field
present, ticket verifies against `protocol/keys/hub-public.hex`).
Preconditions this review confirms are still open:

1. The deployed F2B relay still runs the pre-ADR-038 uncapped/ungated
   binary — the owner-gated redeploy (Network handoff risk 3) must happen
   first, and until the §6.3 entry exists the relay must NOT run `--public`
   on the old binary.
2. Production `modelswarm.deepflux.space` does not mint tickets until the
   owner deploys the minting build (Tracker handoff risk 4). Deploy order
   is tracker-first and safe (old Rust clients absorb the additive field —
   pinned); note that a MANUAL run of the extended harness against
   production before that deploy will now fail at step 3b by design — the
   honest signal that the tracker predates ticket minting.
3. The §6.3 venue is owner-gated per the F2B §8 precedent; CI never
   touches production, and the real-model-e2e workflow stays
   manual-dispatch only.

## 6. Non-blocking notes routed

| # | Seat | Note |
|---|---|---|
| N1 | Network | Stale pre-ADR-038 wording survives in two transport test comments (`libp2p_backend.rs`, the `announced_circuit_duration_limit_cuts_the_stream` / `announced_circuit_byte_limit_cuts_the_stream` doc comments still say "our production relay config leaves it uncapped"). Comment-only drift; the frozen posture is correctly pinned by `absent_flags_mean_the_frozen_adr038_policy`. |
| N2 | Protocol | The vector-regeneration follow-up, shaped as §3 above (including the peer-seed recording precision). |
| N3 | Network | Security F-6 (pin the S8 non-consumption property directly) — acknowledged as routed by the Security review; this seat will verify it when it lands. |
