# Handoff — Windows Product Engineer, 2026-10-15 (F2B relay adoption)

Session record for adopting the Network Engineer's F2B surface
(docs/reviews/handoff-network-engineer-2026-10-15.md §7.1) in MY owned
paths (`crates/modelswarm-node`, `crates/modelswarm-desktop`) — commit
`364609e` on main. The live blocker this closes: the owner's third
machine outside the LAN elects family-verified chat (always on since
009bfbd), the cross-NAT direct QUIC dial fails, and the app honestly
falls back local. With this adoption, a serving peer that reserved on a
relay is reachable through its advertised circuit address, and the
requester dials it after the direct attempt fails — every step labeled.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-node/src/remote.rs` | Requester path: `RemoteExecutor::execute` replaces the single direct `dial` with `dial_with_fallback` over ALL advertised candidates, split into two BUDGET phases around it (the transport API has one per-attempt budget per call): direct candidates SHORT (5 s — a blackholed cross-NAT dial must not starve the relay leg; the F2B handoff's guidance), relay candidates generous (10 s). Ordering/classification stay the transport's own. Per-attempt honesty: every failed attempt — including the direct blackhole that PRECEDED a relay win — emits one `net.failure`/stage=`dial` row via the attached `PeerMetrics` (the frozen vocabulary; nothing invented) plus this module's existing stderr diagnostics; the full report (kind/addr/error, the relay's status vocabulary verbatim) is exposed via `last_dial_attempts()`; the winning path via `pooled_dial_path()` labels relayed replies. New `set_candidate_addrs` seam mirrors `set_advertised_queue_ms` (refreshed per roster lookup, primary deduped). New `relay_fallback_tests`: blackhole→relay e2e through the REAL serving bridge, relay-only-peer primary, candidate dedup. |
| `crates/modelswarm-node/src/serving.rs` | Serving path: `RelaySlot` (replaceable `Arc<Mutex<Option<RelayReservation>>>`; installing drops → releases the old lease) and `ServingInbox` (ONE bounded channel fed by forwarder tasks for the direct listener AND the relay slot — an accepted session is never lost to select-cancellation, the 2026-10-09 shutdown-race bug class). `serve_sessions_relayed` admits relayed circuits into the SAME admission- and lease-gated loop; `serve_sessions` delegates with an empty slot, so bench and every existing caller/test are unchanged. |
| `crates/modelswarm-node/src/verifydrafts/composed.rs` | `serve_sessions_composed` takes the `ServingInbox` — the family tier serves relayed circuits too (same listener-or-circuit sources, same demux, same RC-L1 lease gate). |
| `crates/modelswarm-node/tests/verifydrafts_composed.rs`, `.../verifydrafts_capi_composed_e2e.rs` | Mechanical: wrap each test's listener in `ServingInbox::new(listener, RelaySlot::empty())`. |
| `crates/modelswarm-node/Cargo.toml` (+`Cargo.lock`) | dev-dependency `libp2p 0.57` with `relay` — the same in-process relay double the transport suite uses (the relay binary's exact stack); dev-only. |
| `crates/modelswarm-desktop/src/app.rs` | (a) Picker/roster: the candidate predicate widens to DIALABLE (direct OR circuit; `0.0.0.0` undialable) — a relayed peer is reachable, not skipped; the full dialable list rides `set_candidate_addrs` per turn; the reply's `mode` gains `" (relayed)"` when the pooled session is relayed; dial-failure reasons fold in the per-attempt lines (existing `chat.swarm.fallback` reason text); `lookup_peers` classifies rows direct / `relay (circuit)` / `direct + relay` / no dialable address. (b) Serving: `<data_dir>/relay.addr` (DEFAULT-EMPTY) names the configured relay; a supervisor task (first tick immediate, then 30 s) reserves/re-reserves, logs success as `listener.bound(addr=circuit)` and failures as `listener.error(phase=relay_reserve,…)` throttled first+20th — both EXISTING events — and owns the heartbeat's advertised list (direct first, circuit only while a live reservation holds); the heartbeat registers the LIST and re-registers on change so the roster never claims a dead circuit. Hosting start never blocks on the relay. (c) `get_status` carries `relay_circuit_addr` (truthful). (d) The family probe and the shadow planner's frozen `has_direct_quic_addr` stay direct-only by name. (e) Three new tests (classification, relay.addr parsing, advertised-list truth). |
| `crates/modelswarm-desktop/ui/index.html` | The serving chip renders `remote serve: on (<direct> + relay)` when a circuit address is advertised; roster rows render the new classification. |

## 2. Exact commands and outcomes

All with `TAURI_CONFIG='{"bundle":{"resources":[]}}'` where the desktop
builds (the CI overlay — engines are staged only by installer flows):

- `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` — clean.
- `cargo test --workspace` — **622 passed / 0 failed / 6 ignored**.
- `cargo test --workspace --features modelswarm-node/libp2p-backend` — **648 / 0 / 7** (645 baseline + the 3 new relay tests).
- `cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend` — **691 / 0 / 7**.
- `cargo test --workspace --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend` — **710 / 0 / 13**.
- `cargo test -p modelswarm-desktop --features tauri-shell` — **72 / 0 / 0**.
- `cargo test -p modelswarm-node --features libp2p-backend relay_fallback` — 3/3, run 3× consecutive stable (6.1 s each).
- Not run locally (no Linux runner here): the CI non-Windows capi clippy
  job. This diff has no `cfg(windows)`/unsafe surface and the tauri-shell
  clippy compiled the capi path on Windows; risk is nil but CI will
  confirm.
- No winjob flake surfaced in any battery this session.

## 3. Test evidence (the adoption proofs)

- `direct_blackhole_falls_back_to_relay_through_the_serving_bridge`
  (node): the peer's primary advertised direct address is a HELD-SOCKET
  blackhole (the honest cross-NAT stand-in, bd469ce's de-flake shape);
  its candidate list also carries the circuit address it reserved on the
  relay double. The executor's direct phase burns its short budget, the
  relay phase wins, the REAL `serve_sessions_relayed` bridge admits the
  circuit through the `RelaySlot` and serves "ab"; exactly one circuit;
  `pooled_dial_path()` = `Relayed`; the failed direct attempt counted
  `net.failure`/stage=`dial` = 1 AND present in `last_dial_attempts()`
  with kind=direct despite the overall SUCCESS.
- `relay_only_peer_is_dialable_as_the_primary_candidate` (node): a peer
  whose ONLY address is the circuit is dialable — circuit as PRIMARY
  works, no direct phase runs, zero failures recorded.
- `candidate_addrs_dedup_the_primary` (node): the refresh seam.
- Desktop: `peer_address_classification_direct_circuit_and_placeholders`
  (circuit reachable, `0.0.0.0` placeholders not, direct-only predicate
  excludes circuits), `relay_addr_file_is_optional_and_comment_aware`
  (missing/blank/comment-only file = NO relay; first real line wins),
  `advertised_list_tracks_the_live_circuit_truthfully` (change-only
  signaling — a no-op update must not force a re-register; direct stays
  first; dead-circuit removal fires the change).

## 4. Assumptions

1. Circuit addresses ride the tracker's EXISTING `addresses` field — no
   schema change, exactly the F2B handoff's design. The requester learns
   the relay from peer records; nothing else.
2. The serving side needs the relay's OWN multiaddr to reserve. There is
   no in-band way to learn it before any peer advertises a circuit, so a
   client-side endpoint setting was unavoidable — implemented as
   `<data_dir>/relay.addr`, DEFAULT-EMPTY (no invented defaults pointing
   at nonexistent hosts), documented in-code and here. When present, use
   is automatic: no new user-facing switch (owner directive).
3. Two budget phases instead of one `dial_with_fallback` call: the API
   takes a single per-attempt deadline, and the handoff's guidance is
   short-direct/generous-relay. The split is by `/p2p-circuit`
   substring ONLY to route budgets; classification/order stay the
   transport's.
4. Per-attempt `net.failure` rows: one row (and failure-count
   increment) per failed ATTEMPT, including attempts that preceded a
   relay WIN. Single-candidate failures still count 1 (the pre-existing
   test's assertion is unchanged).
5. The heartbeat may register once with `[direct]` before the
   supervisor's first reservation lands; it re-registers with
   `[direct, circuit]` on the watch change (≤30 s). Honest either way.
6. `serve_sessions`' public signature is unchanged; bench
   (`modelswarm-bench`, NOT my path) needed zero edits — verified by
   reading its two `RemotePeer` literals and the full green workspace.

## 5. Unresolved risks / honest boundaries

1. **Missing telemetry labels (recorded, not invented)**: there is no
   dedicated event/field for relay lifecycle (`relay.reserved`,
   reservation lost, relayed-RTT labeling) — I reused `listener.bound`
   (addr=circuit) and `listener.error` (phase=`relay_reserve`), and
   `net.failure`/stage=`dial` per attempt. If Protocol/Telemetry want a
   first-class relay vocabulary, that is an ADR + inventory change.
   Likewise the `chat.swarm.completed` `served_by` field carries no
   relayed distinction — relayed honesty rides the UI `mode` label and
   the attempt report; the ADR-013 nat-path field remains the right
   scheduler-side home (unwired).
2. **Family-over-relay gap**: `dial_quic` (the ADR-036 elected carrier
   session) is direct-only this pass. On a relay-only peer the elected
   turn fails its dial and falls back to the plain swarm path — which IS
   relayed — with the existing `elected_fell_back` label. Threading
   candidates through `QuicVerifyWire` is the follow-up.
3. **Relayed far-end identity is relay-vouched** (the F2B handoff's risk
   5.2): our serving lease gate binds the relay-vouched source PeerId;
   end-to-end §6.1 Ed25519 over circuits is the Protocol/Session
   cross-path decision, unchanged by this work.
4. **measure_rtt on a relayed pooled session records the RELAYED RTT**
   (handoff risk 6): correct number, unlabeled as to path — folded into
   risk 1's vocabulary gap.
5. Supervisor's liveness signal is the transport's best-effort
   `alive()`; a silently-dead reservation can advertise a stale circuit
   for up to one 30 s supervisor tick before the list is corrected
   (requesters then fail that candidate honestly and fall back).
6. Real-network proof pending (owner-gated): deploy `modelswarm-relay`
   per the Network handoff §6 on the LAN-trusted host, write the relay
   address into each machine's `<data_dir>/relay.addr`, re-run the
   three-machine swarm, and record the first relayed completion +
   end-to-end "relayed, not direct" labeling in
   `docs/verification/f2b-relay-2026-10-1x.md`.

## 6. Suggested next task

The owner-gated real-network F2B exit (risk 6): stage the relay on the
second desktop (`modelswarm-relay.exe --listen 0.0.0.0:4001 --key
<stable path>\msp-relay.key --public`, UDP 4001 inbound + forwarded),
drop its `/ip4/<host>/udp/4001/quic-v1/p2p/<relay-peer-id>` into
`relay.addr` on the serving machines, and run the family swarm from
outside the LAN. Expect: `listener.bound` with the circuit addr, the
roster row `direct + relay`, the outside machine's turn served
`swarm — remote peer … (relayed)`, and `family_engage.reason =
elected_fell_back` (risk 5.2's label) until family-over-relay lands.

— Windows Product Engineer, 2026-10-15
