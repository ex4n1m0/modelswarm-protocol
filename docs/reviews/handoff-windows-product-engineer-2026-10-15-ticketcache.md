# Handoff — Windows Product Engineer, ADR-038 §6.4 row 8 (2026-10-15)

The node's relay admission ticket cache — the last implementation row of
the D12 close's client side. Built on `13046f0` (the Network seat's
admission gate + client pre-flight and its `TicketSource` seam; the
ticket type, vectors, and tracker issuance were already landed by the
Protocol and Tracker seats). This handoff CONSUMES the seam, never
redefines it: no relay, transport, tracker, protocol, or workflow path
was touched.

## 1. Changed files and why

| Path | Change |
|---|---|
| `crates/modelswarm-node/src/relay_ticket_cache.rs` (NEW) | `RelayTicketCache` — the single-slot cache (one `Mutex<Option<String>>`): `store_heartbeat(&HeartbeatResponse)` (a present, well-formed `relayTicket` REPLACES; absent keeps the previous value — the §6.2(7) deploy-skew case; malformed/oversized is dropped with the last good kept, outcome in `TicketStore::{Stored,Absent,Dropped}`), `current()`, `ticket_source()` (the closure for the Network seat's seam), and `attach_to(&Libp2pTransport)` (the one-liner every relay-leg construction site uses). Validation is the frozen `RelayAdmissionTicket::from_wire` SYNTAX parse only (§3.4 checks 3-4, incl. the 1024-byte bound) — no signature check: the relay owns check 5, the seam contract keeps the string opaque, and the golden vectors' tokens are fixture-key-signed (a node-side signature check would reject them by construction). NO freshness policy: a stale ticket is supplied verbatim; the transport's `should_re_present` renewal window and the relay's check-6 own freshness. Six tests read `protocol/vectors/relay-ticket-1.json` from the file (never inlined). |
| `crates/modelswarm-node/src/lib.rs` | Module registration, `#[cfg(feature = "libp2p-backend")]` (same gating as `remote`/`serving` — `TicketSource` only exists under the transport's `libp2p-backend`). |
| `crates/modelswarm-node/src/remote.rs` | `RemoteExecutor` gains `relay_tickets: Option<Arc<RelayTicketCache>>` + the `#[must_use] with_relay_tickets` builder; `execute()` attaches the source to every FRESH dial transport right after `Libp2pTransport::new` — the requester dial path's relay legs then run the pre-flight with the cached ticket. `None` (all pre-row-8 callers) is byte-identical to before. New test `attached_relay_tickets_leave_direct_dials_untouched`: with the golden vector token cached, a DIRECT dial still serves — the pre-flight lives on relay legs only. |
| `crates/modelswarm-desktop/src/app.rs` | (1) The heartbeat task consumes the response it previously discarded: `store_heartbeat` on every successful heartbeat (failures leave the cache untouched). (2) `set_hosting` creates the per-hosting-session cache, stores it in `Inner.relay_tickets`, hands a clone to the relay reservation supervisor AND moves one into the heartbeat task. (3) `reserve_relay_attempt` (the serving-side reservation, fresh transport per attempt) attaches the source before `reserve_relay`. (4) `try_swarm_chat`'s pooled `RemoteExecutor` gets `.with_relay_tickets(...)` when the cache exists. (5) Both hosting toggles retire the pooled swarm executor (hosting-on: it may predate the new cache; hosting-off: its cache is dead) — the next swarm turn rebuilds it correctly. NOTHING user-facing: no switch, no UI element, no status field, no new telemetry event (the owner's everything-on-by-design directive; the ticket rides the heartbeat invisibly). |

Wiring shape: the cache INSTANCE lives in the desktop's app state
because the heartbeat loop is the desktop's roster task (the node
daemon's `Node::start` makes no network calls by design); the cache
TYPE and all semantics live in `crates/modelswarm-node` per the ADR's
§6.4 routing. The serving LISTENER itself needs no ticket — it accepts
direct connections; the only serving-side relay leg is the
reservation, which the supervisor owns.

## 2. Exact commands run and outcomes

- `cargo fmt --check` — clean (after one `cargo fmt` pass).
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` — clean (mirrors rust.yml).
- `cargo test --workspace` — 79 suites `ok`, 638 tests passed, 0 failed.
- `cargo test --workspace --features modelswarm-node/libp2p-backend` — 79 suites `ok`, 672 passed, 0 failed (the +34 includes the 7 new tests).
- `cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend` — 715 passed, 0 failed.
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --features tauri-shell` — 72 passed, 0 failed.
- `cargo test -p modelswarm-node --features libp2p-backend --lib relay_ticket` — **7 passed, 0 failed, THREE consecutive runs** (0.07 s each; the standing stability discipline).

## 3. Test evidence (§6.2(5), node half — "ticket cache refresh from heartbeat responses")

All in `relay_ticket_cache::tests`, driving `store_heartbeat` with the
EXACT §3.3 response JSON (field present vs absent — the serde-default
deploy-skew pair) and the golden vector token read from
`protocol/vectors/relay-ticket-1.json`:

1. `heartbeat_with_ticket_populates_cache_and_source` — populates the
   cache AND the source closure hands back the same string.
2. `heartbeat_without_ticket_keeps_previous_value` — absent field (old
   tracker) keeps the previous value; the empty-string encoding too; a
   fresh cache stays empty.
3. `malformed_and_oversized_tickets_are_dropped_honestly` — not-a-ticket,
   wrong-payload segments, and a 1200-byte string all `Dropped`, the
   last good value survives each; empty cache stays empty.
4. `fresh_ticket_replaces_never_accumulates` — a second well-formed
   ticket (fresh nonce, re-signed with the vector's own fixture seed)
   replaces the first: one slot, no history.
5. `stale_ticket_is_supplied_verbatim_the_transport_governs_freshness`
   — the cache consults NO clock; the vector token (deadline long past)
   is supplied verbatim. The freshness rules stay where they are
   already pinned: transport
   `renewal_re_presentation_rule_matches_the_frozen_window`, relay
   check-6. (My reading of the task's "stale-past-120s case honestly
   lets the transport skip": the node must NOT add a second expiry
   policy — supplying as-is is exactly what lets the transport's
   documented rule govern.)
6. `attach_to_wires_a_real_transport_without_side_effects` — the
   composition call against a real `Libp2pTransport` (the pre-flight
   itself is byte-pinned transport-side: `pre_flight_ok_then_hop_proceeds`
   and siblings).
7. `remote::failover_tests::attached_relay_tickets_leave_direct_dials_untouched`
   — end-to-end through the REAL serving bridge: with the vector token
   cached, a direct dial + lease-gated turn still answers REMOTE — the
   ticket never perturbs non-relay paths.

Desktop half (the two-line wiring in the heartbeat task) is exercised
by the tauri-shell suite (72 green) at compile+behavior level; the
tracker-api crate already pins both wire shapes
(`heartbeat_response_relay_ticket_deploy_skew_tolerance`), and the
cache tests above pin the node's handling of each — no live tracker is
reachable from CI (per the standing venue rule).

## 4. Assumptions

1. The task's "node state" = the desktop's app state instance of the
   node-crate type: the heartbeat loop is the desktop's roster task;
   `Node::start` makes no network calls (its documented contract), so
   the cache cannot be filled inside the daemon itself.
2. Syntax-only validation in the cache (`from_wire`, checks 3-4) —
   signature verification stays relay-side (check 5); node-side
   verification would reject the fixture-keyed golden vectors and add
   a second trust decision the ADR assigns elsewhere.
3. "Malformed" = anything `from_wire` refuses (the same parse the
   transport runs for its renewal window — one definition across node
   and transport, per the seam's opaque-string contract).
4. Tickets ride heartbeats ONLY (§3.3); the register path does not
   mint, so nothing is stored there.
5. The requester-side cache exists because the installation itself
   hosts (project rule) — its own heartbeat supplies its own ticket
   for its own relay legs (check-7 binding is to the DIALING peer).
6. The pooled `swarm_executor` is retired at hosting toggles so it can
   never outlive its cache (a stale-cache executor would otherwise
   persist until the peer key changed).

## 5. Unresolved risks

1. **No counts-only telemetry line for the dropped-ticket case.** The
   task allowed one only if the frozen vocabulary had a suitable event;
   it does not (`roster.error`/`listener.error` are free-text,
   other-phase events; ADR-038 §6.2(6) wants counts/codes only). The
   outcome is returned (`TicketStore`) and currently ignored by the
   heartbeat task — if T/R wants a `relay.ticket` event family, that is
   an ADR/vocabulary decision, not something I may invent here.
2. Desktop-half behavior (heartbeat → cache) is compile+suite-pinned,
   not live-wire-pinned: no mock-tracker harness exists for the desktop
   heartbeat loop (pre-existing gap; the wire shapes are pinned
   tracker-api-side). The §6.3 real-network entry will exercise it
   end-to-end once the relay is redeployed (owner-gated, Network risk
   3).
3. A tracker outage > TTL (600 s) means new relayed legs present stale
   tickets and earn honest `permission_denied(202)/expired` — the ADR's
   documented degradation (§3.6 item 5), surfaced verbatim by the
   transport. The cache deliberately does nothing about it.
4. `dial_quic` (the family/elected carrier path) is direct-only and
   carries no ticket — unchanged, per the recorded "family-over-relay"
   follow-up in `app.rs`'s own comments.

## 6. Suggested next task

Integrator: ADR-038 §6.4 is now fully implemented (rows 2-8). Route to
Security for the §6.1 admission adversary matrix + threat-model D12
row, and to Test and Release for the §6.2 sign-off (the wire-compat
job's `relayTicket` extension + the vectors in `rust.yml` — my seven
tests run under the existing `modelswarm-node/libp2p-backend` step, no
workflow change needed). The §6.3 real-network verification entry
against the redeployed relay remains owner-gated (Network risk 3).
