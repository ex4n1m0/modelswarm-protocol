# Security review — ADR-038 relay admission (§6.1 gate), 2026-10-15

**Verdict: APPROVE.** The landed admission implementation matches ADR-038's
§3.4/§3.5/§3.7 claims at every enforcement point I attacked; the S1-S9
adversary matrix is adjudicated below with code-level evidence, all landed
test suites reproduce at HEAD (`54d8e4e`), and the known vector drift is
RULED ACCEPTABLE with a binding condition (§4 below). Gate **D12** therefore
closes CONDITIONALLY, exactly as ADR-038 §6.3 already requires: on this
§6.1 approval AND Test/Release's §6.2 sign-off AND the owner-gated §6.3
real-network verification entry against the REDEPLOYED gated relay (which
does not exist yet — `docs/verification/` has no `relay-admission-*.md`;
until it does, the deployed F2B relay must not run the old ungated binary
`--public`).

Reviewer: Security Engineer (independent release blocker). Reviewed HEAD
`54d8e4e` on `main`; tree clean apart from the untouchable
`zcode-master-prompt.md` and this change.

## 1. Scope and method

Read in full: `docs/adr/ADR-038-relay-admission-tickets.md`, the four
implementation handoffs (`docs/reviews/handoff-network-engineer-2026-10-15-admission.md`,
`handoff-windows-product-engineer-2026-10-15-ticketcache.md`,
`handoff-tracker-engineer-2026-10-15-relay-tickets.md`, and the §5/§6 D12
gap statement in `handoff-network-engineer-2026-10-15.md`), and the code:

- `crates/modelswarm-relay/src/admission.rs` (the pre-flight, ten-check
  verify, admission table, gate limiters) and `src/main.rs` (config wiring,
  boot rule, flags, event loop, in-process proofs).
- `crates/modelswarm-transport/src/relay_client.rs` (client pre-flight,
  `TicketSource` seam, renewal rule) and the `libp2p_backend.rs` wiring
  (`set_relay_ticket_source`, `reserve_relay`/`dial_with_fallback`
  pass-through, the test double's mirrored frozen config).
- `crates/modelswarm-node/src/relay_ticket_cache.rs` + `remote.rs` wiring +
  the desktop `app.rs` wiring points (heartbeat store, hosting-session cache
  lifecycle, executor retirement at hosting toggles).
- `apps/tracker/lib/relay-ticket.ts`, the heartbeat route
  (`app/api/v1/peers/heartbeat/route.ts`), `lib/crypto.ts` (`randomId128`),
  `lib/constants.ts`, `apps/tracker/tests/relay-ticket.test.ts`.
- `crates/modelswarm-eligibility/src/relay_ticket.rs`,
  `crates/modelswarm-identity/src/installation.rs` (the ADR-020 derivations).
- Vectors: `protocol/vectors/relay-ticket-1.json`, `relay-ticket-2-expired.json`,
  `relay-ticket-forge-1.json`, `relay-admission-frame-1.json`.

**Independently verified against the pinned upstream source** (the decisive
§2 claim of the ADR, not taken on trust): in
`~/.cargo/registry/src/index.crates.io-.../libp2p-relay-0.22.0/src/behaviour.rs`,
the reservation accept site denies when
`!self.config.reservation_rate_limiters.iter_mut().all(|limiter| limiter.try_next(event_source, endpoint.get_remote_address(), now))`
with status `ResourceLimitExceeded` (lines ≈578-586), and the circuit-source
accept site has the identical shape over `circuit_src_rate_limiters`
(lines ≈699-707) — evaluated BEFORE the destination-reservation lookup, so
a gate denial precedes even `NoReservation`. `.all()` short-circuits on the
first `false`, and `try_next` is never called on later limiters — the S8
token-preservation property rests on this and on the gate being at index 0.

## 2. The S1-S9 adversary matrix (ADR-038 §6.1), adjudicated

| # | Attack | Verdict | Code-level evidence |
|---|---|---|---|
| S1 | Ticketless flood, incl. clients that never run the pre-flight | **Mitigated (fail-closed at the grant sites)** | `admission::gate_limiters` (admission.rs:243-257) closes over the shared table; `relay_config` inserts it at INDEX 0 of BOTH vectors (`config.reservation_rate_limiters.insert(0, …)`, `circuit_src_rate_limiters.insert(0, …)`, main.rs:115-120); upstream consults both vectors at the two accept sites (verified in the 0.22.0 source above). Proof: `gate_fails_closed_admits_on_ticket_and_recloses_on_expiry` (main.rs:1425-1671) — a REAL libp2p client that never presents is refused reservation AND circuit with `ResourceLimitExceeded`; admission-expiry on a virtual clock re-closes both gates; renewals re-hit the gate (a renewal is a new RESERVE on the same upstream path). Pre-flight-refused clients open no hop stream at all (relay_client.rs:516-521). Residual cost for the attacker is QUIC-handshake CPU only — inherent to any public listener, not claimed otherwise by the ADR. |
| S2 | Forged signature | **Mitigated** | Ed25519 over the canonical signature-omitted JSON against the pinned key: `verify()` (relay_ticket.rs:167-189); key resolution fail-closed at boot — `resolve_tracker_key` (main.rs:356-385) rejects wrong length/case/charset, and `main` exits 2 in EVERY mode when no key resolves (main.rs:443-457 — stronger than the ADR's `--public`-only rule, honestly documented). Forge corpus: `forge_corpus_rows_match_the_frozen_pairs` re-derives `bad_signature` AND `wrong_key` rows against the frozen 202/`bad_signature` pair (admission.rs:1166-1202). |
| S3 | Farmed/stolen ticket | **Mitigated as designed (bounded, not prevented)** | Check 7 peer binding (relay_ticket.rs:194-203) + check 8 ADR-020 re-derivation (`derived_installation_id`, admission.rs:269-277, cross-checked against `modelswarm_identity::installation_id_for` by test at admission.rs:1389-1417): the presenting connection's NOISE-authENTICATED PeerId must equal `ticket.peer_id` and its embedded key must derive `ticket.installation_id`; the table keys on the PRESENTER (`verifier.table.admit(*remote, …)`, admission.rs:351), never on ticket input — a stolen ticket is presentable only by the victim itself. Farming = enrolled sybils: bounded by `DEVICE_APPROVAL_CAP`/admin approval (§3.6) and the frozen per-peer caps (§3.5). Tracker mints only from authenticated rows (heartbeat route: `issueRelayAdmissionTicket(installationId, lease.peerId, …)` after `leaseBindingError`, route.ts:27-54; failure paths return before minting). |
| S4 | Tracker-key compromise | **Accepted residual (pre-existing class) + NEW instant lever confirmed** | One key guards catalog/leases/tickets (§3.4); compromise ⇒ arbitrary ticket minting until rotation. The rotation runbook step EXISTS in code: `--tracker-key <PATH>` (parse at main.rs:312-317, resolution at 356-385, live exit-2 smoke in the Network handoff) — a relay restart under the new key invalidates every outstanding ticket instantly. Recorded honestly as threat-model §4.6 residual 1, including the node-side lead time (deployed nodes pin the OLD key until updated — the §4.5 key-lifecycle shape). |
| S5 | Admission-table exhaustion | **Mitigated (enrollment-bounded)** | 4096 PeerId-keyed entries, memory-only (`AdmissionTable`, admission.rs:164-227); `admit` performs a FULL expired-sweep before the capacity check (`entries.retain(…)` at 209) — so `table_full` (201) can only ever be reached with 4096 LIVE entries, each of which required a valid tracker-signed ticket naming that distinct PeerId. One installation can never hold more than one slot (key = PeerId). `is_live` evicts lazily (queried peer only) — a mild staleness, bounded by capacity, documented by the Network seat (risk 5) and confirmed benign here (finding F-4). |
| S6 | Clock-skew games | **Mitigated** | One shared injectable clock feeds check 6, `admit`, and the gate (`AdmissionVerifier.table.now()`, admission.rs:331; production `wall_clock()`); windows are the frozen ±120 s (relay_ticket.rs:179-188) with deadline `expires_at + 120 s` (224-227) and strict `deadline > now` liveness (admission.rs:194). Pinned by `relay-ticket-2-expired.json` (202/`expired` at 12:20) and the virtual-clock re-close proof. Boundary note: finding F-2 (informational). The relay's own clock is operator-trusted — a hostile OPERATOR clock only weakens its own relay. |
| S7 | Pre-flight injection | **Mitigated** | Strict frozen protobuf: `decode_present` (admission.rs:438-464) refuses unknown fields, wrong wire types (only 0/2 accepted, 433-line `parse_fields` rejects others), duplicates, missing type/ticket, wrong type values (a STATUS in a request), truncations, overruns, non-UTF-8. Size-before-allocation: `read_frame` validates the announced length ∈ [1, 4096] BEFORE waiting for or allocating the body (587-603; pinned async at 1019-1094 — uvarint(4097) with NOTHING else arriving is refused as malformed, not a timeout). One frame each way; every exchange deadline-bounded at 15 s (`PRESENT_DEADLINE`, :68); transport errors become 400 answers, never hangs (`exchange`, 637-671). Presentation is bound to the connection's authenticated PeerId (Handler carries `self.peer` from `handle_established_*_connection`, 698-717). Client-side strict mirror: `decode_admission_status` (relay_client.rs:402-432). |
| S8 | Rate-limiter token theft via shared NAT | **Mitigated — the gate is genuinely FIRST in both vectors** | Three independent confirmations: (1) code — `insert(0, …)` at main.rs:115-120; (2) test — `gate_limiters_refuse_unadmitted_peers` (main.rs:925-950) asserts index 0 of BOTH vectors refuses the unadmitted peer while the stock buckets behind have free tokens; (3) upstream source — `.all()` short-circuits, so `try_next` is never CALLED on the per-peer/per-IP buckets when the gate denies (behaviour.rs ≈578-586/699-707, quoted in §1). Unadmitted peers therefore cannot drain a shared NAT's per-IP bucket. Residual: an ADMITTED rogue behind the NAT still shares that bucket — inherent to per-IP accounting, recorded (threat-model residual 5). |
| S9 | Restart / state | **Mitigated (memory-only)** | `AdmissionTable` is a plain in-memory `HashMap` (admission.rs:164-227) — no persistence anywhere in the crate (grep-verified: no fs write paths outside the relay key file). A restart empties the table; every peer re-presents one cheap frame (~430 B) on its next leg; reservations/circuits die with the relay's connections, so there is no admission state to forge, steal, or replay across restarts. `T4` holds: `reservation_duration` 600 s ≤ TTL (+120 s grace), test-pinned (`absent_flags_mean_the_frozen_adr038_policy`). |

**LEAK surface (the §6.1 first half).** Relay logs carry PeerIds, admission
reasons, and deadlines only — the frozen vocabulary is enforced by the
`Event` type (no ticket field exists on it, admission.rs:113-145) and pinned
by `log_lines_carry_no_ticket_bytes` (no serialized ticket, no base64
payload). Flow-metadata and plaintext-at-relay exposure is real, unchanged
in kind from F2B risk 2, and now recorded unsparingly as threat-model §4.6
residual 3. Ticket content-blindness (T2) is structural: the type is exactly
five fields with `deny_unknown_fields` and a canonical round-trip check
(relay_ticket.rs:96-162), minted from identity + registered row + clock
only (route.ts:49-54) — no profile, address, or payload metadata exists to
leak.

## 3. Findings

Severity scale: blocker / high / medium / low / informational. No blocker,
no high.

**F-1 (medium — documentation/conformance, ruled on in §4): the golden
accept vector cannot pass check 8 end-to-end.**
`relay-ticket-1.json` (and the expired sibling) carry
`installation_id: "b58-installation-relay-0001"` — a placeholder that is not
the ADR-020 derivation of the vector's PeerId (it is not even valid base58:
it contains `-`). Under the real check 8 the token is always
`installation_mismatch` — honestly asserted by the landed test
`real_derivation_rejects_the_vector_placeholder_installation`
(admission.rs:1389-1417). Worse as a false friend:
`relay-ticket-forge-1.json`'s `derived_installation_id` field RECORDS the
placeholder as if it were the real derivation for
`authenticated_remote_peer_id`. No enforcement path trusts either field
(the relay derives at runtime; the tracker mints from real rows), so this is
not exploitable — it is conformance-documentation debt on Protocol-owned
artifacts. Ruling in §4.

**F-2 (informational — boundary semantics at exactly `expires_at + 120 s`).**
Check 6 accepts at `now == expires_at + 120` (`overdue == 0` is not `> 0`,
relay_ticket.rs:182-187) while `is_live` requires `deadline > now` strictly
(admission.rs:194) — a ticket presented in that instant yields `ok(100)`
for an instantly-dead admission (the client's next hop exchange is gate-
denied 201). Fail-closed direction, one-second window, honest labels on
both sides. No change required; noted so nobody "fixes" it into the
permissive direction.

**F-3 (informational — bounded stall amplification on admission streams).**
Each inbound `/msp/relay-ticket/1` stream holds one task + ≤ ~4 KiB buffer
for up to 15 s (`PRESENT_DEADLINE`); a hostile client can stall many streams
in parallel, bounded by the QUIC stack's concurrent-stream limit and
identical in kind to what identify/ping already expose on any libp2p host.
No allocation amplification exists (size-before-allocation, S7). No change.

**F-4 (informational — lazy eviction in `is_live`).** Only the queried
peer's expired entry is evicted on gate lookups (admission.rs:191-201); the
full sweep lives in `admit` (209). Consequence: stale-but-unqueried entries
occupy slots until the next admit — but because every admit sweeps first,
`table_full` still requires 4096 LIVE entries (S5). Confirmed benign;
already documented by the Network seat (risk 5).

**F-5 (informational — admitted-rogue NAT bucket sharing).** S8 closes the
UNADMITTED thief; an ADMITTED rogue behind a shared NAT still consumes that
NAT's per-IP bucket (60/min) alongside honest peers. Inherent to per-IP
accounting, bounded per-rogue by the per-peer bucket (30/2 min). Recorded
(threat-model residual 5).

**F-6 (low, recommended test — Network seat, non-blocking): pin S8's
non-consumption property directly.** The token-preservation half of S8 (the
per-IP bucket is not merely "behind" the gate but UNTOUCHED on a gate
denial) is today established by code inspection + upstream source + the
index-0 test. An in-crate test asserting the stock per-IP limiter's state
is unchanged after a gate-denied `try_next` would pin the property against
future upstream `.all()`-semantics changes. `modelswarm-relay` is the
Network seat's path — routed there, not landed by this seat.

**F-7 (informational — dead branch).** `verify_present`'s
`Err(TicketError::InvalidSignature)` arm for `from_wire` (admission.rs:326)
is unreachable (`from_wire` never returns that variant; base64 signature
failures are `Malformed`). Harmless; maps to the same frozen reason anyway.

Positive deviations worth recording (both honest and in the fail-closed
direction): the boot rule exits 2 in EVERY mode when no key resolves
(stronger than ADR §3.4's `--public`-only requirement); a transport failure
during the pre-flight is ANSWERED as `malformed_message(400)` rather than
silently dropped; and admission survives a failed STATUS ack because the
gate, not the acknowledgment, is the enforcement (admission.rs:655-660) —
the correct T1 reading.

## 4. Drift ruling (BINDING for Test and Release; routed to Protocol)

**Question:** the golden vectors' `installation_id` placeholder cannot pass
check 8's real ADR-020 derivation; binding coverage is provided by
re-derived fixtures instead. (a) Regenerate the vector with a properly
derived `(peer_id, installation_id)` pair BEFORE D12 closes, or (b) accept
the split coverage with the drift documented?

**Ruling: (b) — the split coverage is acceptable for the D12 close; the
regeneration is a REQUIRED follow-up for the Protocol seat in the NEXT
vector revision, not a condition of this approval.** T/R's §6.2 sign-off
must reference this ruling (and the drift note now in
`docs/threat-model.md` §6) so it is not re-litigated.

Grounds:

1. **No security coverage gap.** Checks 7-8 are pinned by validly-SIGNED
   re-derived fixtures at unit level (`peer_mismatch`,
   `installation_mismatch`, and the order pins `signature ≺ peer ≺
   installation`, admission.rs:1166-1382) and END-TO-END by the in-process
   fail-closed proofs, which mint tickets with REAL ADR-020 derivations and
   prove the genuine PeerId is admitted through the actual QUIC/mss/frame
   path (main.rs `mint_ticket_for`/`admit_client`, 1115-1126/1393-1403).
   What the golden vector alone cannot do is replay end-to-end to
   `ok(100)` — a conformance convenience, not an enforcement surface.
2. **The vector's declared purpose is still met.** Its own description
   promises `from_wire + verify` acceptance in TS and Rust at `verify_at`
   (checks 1-6 + the deadline computation) — which it delivers and which
   the TS↔Rust byte-parity gate (landed `ba5e4a8`) consumes. It does not
   promise end-to-end relay admission.
3. **Mid-gate regeneration is churn without security gain.** The vectors
   are consumed by five Rust test suites (relay, transport, node cache)
   plus the TS parity test; regenerating now would force a synchronized
   cross-seat re-landing during the close, touching Protocol-owned
   artifacts under review.
4. **The misleading part is named.** `relay-ticket-forge-1.json`'s
   `derived_installation_id` field must be corrected to the real
   derivation (or explicitly annotated as a placeholder) in that revision —
   it is the one place the drift actively misstates a fact.

Required follow-up (Protocol seat, next vector revision): regenerate
`relay-ticket-1.json`/`relay-ticket-2-expired.json` from a recorded seed
whose PeerId's real `installation_id_for` derivation is used verbatim, fix
`relay-ticket-forge-1.json`'s `derived_installation_id`, and update the
consuming tests in the same change. Until then the drift is documented in
THREE places: the relay test that asserts the honest rejection, the Network
handoff (assumption 4 / risk 1), and now this review + the threat model.

## 5. Threat-model diff summary (this review's companion change)

`docs/threat-model.md`, updated 2026-10-15:

- Header updated (ADR-038 §6.1 rows).
- §1 surface list: added surface 7 (the operator-hosted circuit relay).
- NEW §4.6 "Circuit relay + admission tickets": the conditional D12 close
  statement (this review + T/R §6.2 + the owner-gated §6.3 entry against
  the REDEPLOYED relay), the S1-S9 mitigation table as adjudicated above,
  and the five residual rows the task required — (1) tracker-key compromise
  window WITH the rotation runbook order (tracker redeploy → relay
  `--tracker-key` restarts → node release train, and the node-side
  adoption-cycle lead time), (2) sybil farming bounded by enrollment + caps
  (≈ 32 Mbps per admitted rogue; the operator's pipe is the global bound),
  (3) operator flow-metadata + plaintext-at-relay (the msp-v1 §6.1
  end-to-end handshake as the queued standing mitigation), (4) expiry-only
  revocation tail ≤ 12 min, (5) tracker-outage degradation for NEW relayed
  circuits (+ the shared-NAT admitted-rogue caveat).
- §5.4 restated: relayed paths exist and are admission-gated but carry the
  relay-vouched trust residuals; DCUtR remains future.
- §6 abuse-test index: added the ADR-038 suite map and the documented
  drift ruling.

## 6. Evidence (commands and outcomes, at HEAD `54d8e4e`)

- `cargo test -p modelswarm-relay` — **28 passed, 0 failed, THREE
  consecutive runs** (5.83 s / 5.81 s / 5.83 s; the standing stability
  rule). Includes the §6.2 tests 1-4 + 6 mapped in the Network handoff.
- `cargo test -p modelswarm-transport --features libp2p-backend` — 56 + 12
  passed, 0 failed (the §6.2 test-5 presentation suite included).
- `cargo test -p modelswarm-node --features libp2p-backend --lib
  relay_ticket` — 7 passed, 0 failed.
- `cargo test -p modelswarm-eligibility relay_ticket` — 3 passed, 0 failed.
- `cargo fmt --check`; `cargo clippy --workspace -- -D warnings`;
  `cargo test --workspace` — run for this seat's handoff gate (docs-only
  change; results in the handoff).
- Upstream source inspection (quoted §1): libp2p-relay 0.22.0
  `src/behaviour.rs` reservation accept ≈578-586 and circuit-source accept
  ≈699-707, `Cargo.lock` pin confirmed.

## 7. Required changes routed (none block this approval)

| # | Seat | Change | Blocking? |
|---|---|---|---|
| 1 | Protocol | Vector regeneration per §4 (accept/expired vectors + `derived_installation_id` fix + consumer test updates) | No — next vector revision, REQUIRED |
| 2 | Test and Release | §6.2 sign-off referencing the §4 ruling; §6.2(7) wire-compat extension; guards | Yes for D12 close (ADR's own gate), not for this review |
| 3 | Network | F-6 non-consumption pin; owner-gated relay redeploy + the §6.3 entry (with T/R) | No / owner-gated |
| 4 | — | No code changes required by this review; none landed outside this seat's owned paths (threat model + this review) | — |

## 8. Assumptions

1. The ADR's "connection-bound presentation" (§6.1 S7) is read, with the
   Network seat's documented assumption 2, as "the PRESENT binds to the
   Noise-authenticated PeerId of the connection it arrived on, and the
   admission table keys on that PeerId" — verified in code (§2 S7/S3).
2. The deployed F2B relay is still running the pre-ADR-038 binary until the
   owner-gated redeploy (Network risk 3) — the §6.3 precondition.
3. The heartbeat mint path's lease-binding guarantees are the Tracker
   seat's §6.4-reviewed surface; this review verified the mint inputs are
   server-side rows (route.ts:27-54) and the nonce/TTL/canonicalization
   constants match the Rust consumer (`randomId128` = 32 lowercase hex,
   `RELAY_TICKET_TTL_MS` = 600_000, `toISOString` = RFC-3339 ms UTC).
