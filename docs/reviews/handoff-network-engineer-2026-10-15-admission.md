# Handoff — Network Engineer, ADR-038 §6.4 rows 6-7 (2026-10-15)

The relay-side admission gate and the client-side pre-flight — the D12
close's implementation half. Built on `ba5e4a8` (the ticket TYPE, golden
vectors, and tracker issuance were already landed by the Protocol and
Tracker seats; this handoff consumes them, never redefines them).

## 1. Changed files and why

| Path | Change |
|---|---|
| `crates/modelswarm-relay/src/admission.rs` (NEW, ~900 lines) | The `/msp/relay-ticket/1` pre-flight: strict frozen-protobuf codec (byte-pinned against `relay-admission-frame-1.json`, malformed corpus included), the §3.4 ten-check first-failure-wins verification over `modelswarm-eligibility`'s `relay_ticket` API, the ADR-020 check-8 derivation (`base58(SHA-256(pubkey))` from the PeerId's embedded key, reusing `modelswarm-identity::installation_id_for`), the memory-only deadline-evicted `AdmissionTable` (4096 PeerIds, injectable clock), the `gate_limiters()` closures, and the `NetworkBehaviour`/`ConnectionHandler` pair that answers the protocol on every connection and emits the frozen log events (no ticket bytes ever). |
| `crates/modelswarm-relay/src/main.rs` | Behaviour gains `admission`; `relay_config` now takes the shared table and (a) prepends the gate limiter FIRST to both limiter vectors, (b) runs the frozen §3.5 policy — `reservation_duration` 600 s, `max_circuit_duration` 1800 s, `max_circuit_bytes` 16 MiB — replacing the uncapped defaults (operator flags still override; `--help` states the change honestly); `--tracker-key <PATH>` flag + `resolve_tracker_key` + the `--public`-requires-key boot rule (exit 2; also exit 2 loopback-side because no ungated mode exists); the event loop prints the frozen admission log vocabulary; the test suite was rewritten around a gated in-process relay (raw-QUIC presenter helper, fixture-key minting, settable clock). |
| `crates/modelswarm-relay/Cargo.toml` | + `modelswarm-eligibility`, `modelswarm-identity`, `ed25519-dalek`, `hex`, `time`; dev + `serde_json`, `base64`. |
| `crates/modelswarm-transport/src/relay_client.rs` | The client half of the pre-flight: `TicketSource` (the opaque-string seam for WP row 8) + `no_ticket_source()` default; the strict `AdmissionMessage`/`AdmissionStatus` codec (byte-pinned); `mss_offer` (echo vs `na`); `admission_pre_flight` run after `dial_relay_leg` and BEFORE the hop exchange in BOTH `reserve` and `dial_circuit`; the renewal re-presentation rule (`should_re_present`: re-present iff `expires_at` passed or within 120 s, deadline parsed syntax-only from the node's own cached ticket via `RelayAdmissionTicket::from_wire`). Non-ok statuses fail the leg with the relay's verbatim `status_name` label and NO hop stream. |
| `crates/modelswarm-transport/src/libp2p_backend.rs` | `Libp2pTransport` gains the `relay_tickets` cache field + `set_relay_ticket_source` (the WP row-8 seam; default = empty cache, so pre-wiring behavior is byte-identical); `reserve_relay`/`dial_with_fallback` pass the source down; the test double's relay config now mirrors the binary's frozen ADR-038 policy; new §6.2 test-5 presentation suite (raw-QUIC admission double). |
| `crates/modelswarm-transport/Cargo.toml` | `modelswarm-eligibility` as an optional dep gated under `libp2p-backend` (the ADR §6.4 row-7 edge); dev + `time` (fixture ticket timestamps). |
| `Cargo.lock` | The new dependency edges (relay → eligibility/identity; transport → eligibility optional). |

Seam decision to note for reviewers: when the ticket cache is EMPTY the
client skips the pre-flight OFFER entirely (nothing to present) —
third-party relays dial exactly as before, and an MSP relay's gate then
refuses the hop exchange with the upstream `resource_limit_exceeded(201)`
label, surfaced verbatim and never reinterpreted (ADR-038 §3.6 item 5).
The alternative (offering and presenting an empty PRESENT) would surface a
misleading `malformed_message(400)` for what is really "no cached ticket".

## 2. Exact commands run and outcomes

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy -p modelswarm-relay --all-targets -- -D warnings` — clean.
- `cargo clippy -p modelswarm-transport --features libp2p-backend --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 79 test binaries/suites, all `ok`, 0 failed.
- `cargo test -p modelswarm-transport --features libp2p-backend` — 56 + 12 + 0, 0 failed.
- `cargo test -p modelswarm-relay` — **28 passed, 0 failed, THREE consecutive runs** (5.79 s / 5.81 s / 5.83 s — the standing stability rule).
- Boot smoke (real binary): `--help` exit 0; `--public --tracker-key <garbage>` → `refusing to start (--public requires a valid tracker key...)` **exit 2**; default loopback boot prints `--limit-summary reservations=4096 ... max_circuit_duration=1800s max_circuit_bytes=16777216 reservation_duration=600s admission=ADR-038-ticket` + the admission-gate line.

## 3. Test evidence (§6.2 mapping)

- **Test 1 (fail-closed proofs, real relay stack)** —
  `gate_fails_closed_admits_on_ticket_and_recloses_on_expiry`: a standard
  libp2p client that never runs the pre-flight is refused reservation AND
  circuit with `ResourceLimitExceeded` (the circuit denial also pins that
  the gate limiter runs BEFORE the `NoReservation` lookup in the pinned
  0.22.0 source order); fixture-key tickets (0x38 seed, real ADR-020
  derivations, minted at the virtual clock's t0) admit the SAME PeerIds —
  fresh swarms, same keys (libp2p's client closes its listener after a
  denied reservation; admission is PeerId-keyed, so same-key re-dials are
  the honest reading of "the same client") — reserve → circuit →
  bidirectional ping; the virtual clock advanced past
  `expires_at + 120 s` re-closes both gates. Unit halves:
  `gate_limiters_refuse_unadmitted_peers` (gate is FIRST in both vectors),
  `gate_predicate_is_deadline_bounded`, `admission_table_capacity_and_deadline_eviction`.
- **Test 2 (forge corpus + order)** —
  `forge_corpus_rows_match_the_frozen_pairs` re-derives all 12 rows
  (bad_signature, wrong_key, peer_mismatch, installation_mismatch,
  unknown_field, missing_field, non_canonical_segment, oversized_ticket,
  wrong_ttl_delta, nonce_wrong_charset, bad_timestamp, single_segment)
  against the frozen status+reason pairs;
  `first_failure_wins_order_is_pinned` pins syntax ≺ signature ≺ peer
  binding ≺ installation ≺ capacity. Vectors:
  `relay_ticket_1_vector_admits_with_the_recorded_deadline` (ok(100),
  deadline 12:12:00Z), `relay_ticket_2_expired_vector_is_denied_expired`
  (202/expired at 12:20).
- **Test 3 (frozen caps + announcement)** — relay side:
  `absent_flags_mean_the_frozen_adr038_policy` (600/1800/16 MiB + gate
  prepended), `limits_carry_streams_and_scale_reservations`,
  `limit_summary_reports_the_effective_posture`, flag-path cut proofs
  (`flag_armed_duration_cap_cuts_the_circuit`,
  `flag_armed_byte_cap_cuts_the_circuit`) now behind the gate; transport
  side: `reservation_surfaces_the_relay_terms` asserts the reservation
  response ANNOUNCES 1800 s / 16 MiB (T5) and the 600 s lease
  (`renewal_interval` → 450 s cadence is unit-pinned).
- **Test 4 (boot discipline)** —
  `public_boot_requires_a_resolvable_tracker_key` (embedded key resolves;
  valid `--tracker-key` file overrides; missing file/wrong length/
  uppercase/non-hex refused naming the shape; exit-2 path is
  `resolve_tracker_key`'s Err, main turns it into exit 2 — verified live
  above), `tracker_key_flag_parses`,
  `parse_args_rejects_malformed_input` (valueless flag fails), and the
  loopback default is unchanged
  (`non_loopback_bind_requires_the_explicit_public_opt_in`).
- **Test 5 (client paths, transport)** — codec:
  `admission_present_encode_is_byte_pinned_to_the_frame_vector`,
  `admission_status_frames_decode_to_the_vector_codes`,
  `admission_malformed_corpus_is_refused` (the relay-side decoder runs the
  same corpus in `admission::tests::malformed_corpus_is_refused...`);
  presentation: `pre_flight_ok_then_hop_proceeds` (order
  `[admission, hop]`, exactly ONE PRESENT, terms recorded),
  `pre_flight_refused_means_no_hop_stream_and_verbatim_label` (202 → no
  hop stream on reserve AND dial-with-fallback, label verbatim),
  `na_relay_dials_directly_with_and_without_a_ticket` (third-party
  interop, with and without a cached ticket),
  `renewal_re_presents_only_when_the_ticket_is_near_expiry` (2 s lease →
  ~1 s renewals: fresh ticket presented once; near-expiry ticket
  re-presented every renewal), plus the unit pins
  `renewal_re_presentation_rule_matches_the_frozen_window` and
  `cached_ticket_deadline_parses_from_the_vector_token`.
- **Test 6 (redaction, relay half)** — `log_lines_carry_no_ticket_bytes`
  (no serialized ticket, no base64 payload; the exact frozen vocabulary).
- **Tests 7-8** are T/R's (wire-compat job, guards) — not touched here;
  the workspace's existing byte-identity guards ran green.

## 4. Assumptions

1. An empty ticket cache skips the pre-flight offer (documented decision
   above, code-documented in `relay_client`'s module docs).
2. Admission is PeerId-keyed with a time deadline (§3.4 check 10), NOT
   connection-lifetime-keyed; §6.1 S7's "connection-bound presentation"
   is read as "the authenticated PeerId comes from the connection the
   PRESENT arrived on" (check 7), which is how the tests and the handler
   implement it.
3. The boot rule exits 2 in EVERY mode when no valid key resolves (the
   ADR's "no ungated mode exists"); in practice the embedded
   `protocol/keys/hub-public.hex` always resolves, so only a bad
   `--tracker-key` file can trip it.
4. The golden vectors' `installation_id` is a placeholder
   (`b58-installation-relay-0001`) that cannot satisfy check 8 under the
   REAL ADR-020 derivation; the vectors therefore pin checks 1-6 + the
   order (their own descriptions say from_wire+verify), and the binding
   checks are pinned by re-derived fixture tickets with real derivations
   — the test `real_derivation_rejects_the_vector_placeholder_installation`
   asserts this honestly rather than papering over it.
5. The relay test suite's cap-cut proofs keep using flag OVERRIDES (2 s /
   4096 B) to see a cut in test time; the frozen numbers themselves are
   pinned by config/announcement tests, per §6.2 test 3's intent.

## 5. Unresolved risks

1. **Vector/check-8 placeholder drift (for the Protocol seat)**: if the
   intent was for `relay-ticket-1.json` to be end-to-end admittable, its
   `installation_id` needs to be the real derivation of its (seed-9)
   PeerId's key — currently the token can never pass a spec-compliant
   check 8. No code drift in `modelswarm-eligibility` (its API matched
   the ADR exactly; one cosmetic note: `verify()` reports
   `Expired { overdue_secs: 0 }` for a future `issued_at`, which maps to
   the same frozen reason).
2. **Renewal after admission lapse kills the reservation** (by design):
   if the node's cache is stale past the 120 s window and a re-present
   gets 202, the reservation is marked dead rather than retried — honest,
   but Windows Product's row-8 wiring must refresh the cache from every
   heartbeat (the 15-30 s cadence leaves ~570 s of slack).
3. **The live relay is NOT redeployed** (operator-gated per my brief);
   until it restarts with this binary, the deployed F2B relay still runs
   the uncapped, ungated posture. The real-network §6.3 verification
   entry (ticketed cross-NAT completion + ticketless refusal from a
   second machine) remains open and owner-gated.
4. Gate closure sequencing: D12 closes only after Security's §6.1 review,
   T/R's §6.2 sign-off, and the §6.3 entry exist — this handoff is the
   implementation, not the closure record.
5. `AdmissionTable::is_live` lazily evicts only the QUERIED peer's
   expired entry; a full sweep happens inside `admit` (check 9's
   "expired entries evicted first"), so stale-but-unqueried entries can
   occupy slots until the next admit — bounded by capacity 4096.

## 6. Suggested next task

Integrator: route to **Windows Product Engineer** for ADR-038 §6.4 row 8 —
the node's ticket cache: store `HeartbeatResponse.relayTicket`, call
`Libp2pTransport::set_relay_ticket_source` with a closure over that cache,
and surface the honest degradation label when the cache is empty during a
tracker outage (the transport surfaces
`resource_limit_exceeded(201)`/`permission_denied(202)` verbatim already).
After that, Security (§6.1 adversary matrix + threat-model D12 row) and
T/R (§6.2 sign-off + wire-compat extension), then the owner-gated §6.3
real-network verification entry against the redeployed relay.

---

## Addendum 2026-10-15 (post-review cleanups N1 + F-6, commit ef847ae)

Two routed items from the ADR-038 reviews, applied to
`crates/modelswarm-relay/src/main.rs` only; no behavior change except
corrected refusal strings — every enforced posture identical to ca28601.

**N1 (T/R review — stale "uncapped" wording).** Four flag-surface spots
still claimed omitting a cap flag leaves the relay uncapped (false since
the §3.5 frozen defaults: absent flag = 1800 s / 16 MiB). Swept to name
the ADR-038 frozen default: the `parse_duration_secs` /
`parse_circuit_bytes` doc comments and both 0-refusal error strings
("omit the flag for the ADR-038 frozen default (1800 s /
16777216 bytes)"). The 0-refusal test now pins the wording (the named
escape must be the frozen default; no refusal may claim an uncapped
escape), so the sweep cannot silently regress. `--help` was already
honest; every remaining "uncapped" in the crate is historical framing
("replaces the former uncapped posture"), verified by sweep. No test
name lied; none renamed.

**F-6 (Security review, low — the S8 pin).** New
`gate_denials_leave_the_per_ip_rate_limit_bucket_untouched`: 200
unadmitted peers from ONE shared-NAT IP (TEST-NET-3) denied through the
exact upstream accept-site call (`limiters.iter_mut().all(try_next)` —
the pinned libp2p-relay 0.22.0 semantics at both grant sites), then each
vector's live per-IP allowance measured for admitted peers at that same
IP must EQUAL a never-hammered twin config's (full allowance, no
hardcoded library budgets). Mutation-checked before commit: moving the
gate to the END of the vectors fails the test exactly on the S8
assertion — a reordered or dropped gate cannot regress silently.

Commands and outcomes (all green): `cargo fmt --check` (workspace);
`cargo clippy -p modelswarm-relay --all-targets -- -D warnings`;
`cargo test -p modelswarm-relay` 3x consecutive — 29 passed / 0 failed /
0 ignored each run (was 28; +1 is the F-6 pin), the pin green in all
three. Committed ef847ae on main and pushed (remote confirmed at
ef847ae). No new assumptions; no new unresolved risks — the open items
in section 5 above stand unchanged.
