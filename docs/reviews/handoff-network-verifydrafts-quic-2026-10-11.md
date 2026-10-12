# Handoff — Network Engineer: VerifyDrafts QUIC venue (G1 passthrough + wiring + loopback proof), 2026-10-11 task

Date: 2026-10-12 (task dated 2026-10-11; file name per the task directive).
Owner seat: Network Engineer (`crates/modelswarm-transport`,
`crates/modelswarm-relay`), plus the `verifydrafts` feature-gated parts of
`crates/modelswarm-node` as sanctioned by the task.
Base: `f9ec2ff` (the ADR-032 light-up wiring whose handoff recorded G1).

## 1. Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-transport/src/libp2p_backend.rs` | **The G1 ask, exactly the recorded signatures**: `Libp2pSession::send_json(&mut self, value: &serde_json::Value, deadline)` and `recv_json(&mut self, deadline) -> serde_json::Value`, additive on the `libp2p-backend` feature (the same gate the session type rides). ONE JSON document per u32-BE-length-prefixed frame — identical framing to the typed path; payloads > 256 KiB refused `TooLarge` BEFORE any socket write (the staged backend's writer task enforces the same cap; msp-v1 §6.4). Stash discipline mirrors the staged `Session`: `recv` stashes raw frames for `recv_json` and vice versa, in order, both directions; control PINGs are answered transparently on both recv paths. Decode is staged-identical: `WireMessage` first, raw `Value` fallback, non-JSON = session corruption. **Two internal strengthenings the port contract required** (documented in-code, no observable change for frames that decoded before): (a) frame reads now go through a session-owned cancel-safe partial buffer — `futures`' `read_exact` silently loses partially-read bytes when its future is dropped, and recv deadlines on this backend are WAKE-UPS (the ADR-032 serve loop polls WAIT_COMMIT bounds through them), so a cancelled recv now resumes the same frame instead of desynchronizing the stream; the buffer also handles QUIC coalescing multiple frames into one read (the body is truncated to the announced length, the tail stays buffered). (b) `close_graceful(&mut self, drain)`: FINs the stream then waits bounded — plain drop triggers quinn's implicit `CONNECTION_CLOSE`, which can overtake queued data and lose the LAST sent frame(s) (the race `serving.rs`'s `linger` works around at the call site); EOF at a frame boundary now surfaces as `Closed` (peer FIN) instead of an I/O error. 3 new tests: passthrough/typed interleaving with stash discipline both directions; timeouts-are-wake-ups incl. a ~240 KiB frame crossing 1 ms wake-up cycles intact; oversized send refused before writing with the session surviving. |
| `crates/modelswarm-node/src/verifydrafts/wire.rs` | **The QUIC adapter the port was shaped for**: `QuicVerifyWire` implementing `VerifyDraftsWire` directly over `Libp2pSession` — NO pump (a QUIC recv timeout is non-destructive by construction, the property the staged adapter needs its pump for), `remote_peer_id()` = the QUIC-TLS-authenticated ADR-020 derivation (RC-1 cryptographically, not resolver-approximated). `Drop` FINs + bounded-drains via a spawned `close_graceful` (1 s, the msp-v1 bridge's `linger` bound), so a caller's last frame — the client's final `PrefixCommit`, the server's `SessionReject` — is delivered before the connection closes; outside a runtime it degrades to today's abrupt drop. Helpers `accept_quic` (no resolver directory — the handshake IS the authentication) and `dial_quic` (PeerId verified against the pinned server key's ADR-020 derivation). Module docs updated: the "QUIC gap (honest)" section is now the two-venue description. The staged adapter, pump, and their tests are untouched. |
| `crates/modelswarm-node/src/verifydrafts/serve.rs` | **The parallel serve function the wiring module expected** (its `serve_sessions` design; `serving.rs` itself is untouched): `serve_verifydrafts_quic(listener, config, engine, telemetry, shutdown)` — the same watch lifecycle as the msp-v1 bridge (stop on `true` flip OR sender-dropped; the closed-channel/cancelled-accept lesson is copied verbatim), 1 s accept polls, one spawned `serve_verifydrafts_session` per accepted session. Honest limitation recorded in-code: the bridge's `Admission` (session/per-peer caps) lives behind the node `libp2p-backend` feature and is not reused; per-connection admission belongs to the composed daemon wiring. |
| `crates/modelswarm-node/src/verifydrafts/mod.rs` | Exports for the new items; the "Transport venue (honest limitation)" section rewritten as "Transport venues (2026-10-12 update)" — both backends now carry the family. |
| `crates/modelswarm-node/Cargo.toml` | The `verifydrafts` feature comment only (was: "QUIC integration BLOCKED"). No dependency or feature-graph change of any kind. |
| `crates/modelswarm-node/tests/verifydrafts_quic.rs` (new) | **The QUIC-venue loopback test** (two in-process libp2p endpoints over real QUIC on `127.0.0.1:0`): full lifecycle (negotiate → 3 engaged rounds: full acceptance + bonus, mid-window rejection + replacement, EOG present-as-null → commits → teardown), wrong-remote-peer `bad_frame` (pinned third key; validly-signed request fails closed BEFORE any admission stage, zero engine work, session survives), backend-mismatch `SessionReject` (`backend_mismatch`, honest fallback signal), and the serve-loop venue test (`serve_verifydrafts_quic` serves a negotiated round end-to-end and exits on sender-drop shutdown, with `verifydrafts.session_ended`/`client_closed` telemetry asserted). Both sides' PeerIds asserted as the QUIC-authenticated derivations. |
| `docs/reviews/handoff-network-verifydrafts-quic-2026-10-11.md` | This file. |

## 2. Exact commands run and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all` then `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (default) |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts -- -D warnings` | clean |
| `cargo test --workspace` | **462 passed / 0 failed / 6 ignored** — the recorded 459/0/6 baseline plus exactly this change's 3 new transport tests (the transport libp2p tests run in the default workspace config already: `modelswarm-bench` → `modelswarm-node` hard-codes the transport `libp2p-backend` feature, so the suite was always in the 459; no default-visible behavior changed — `serving.rs`, desktop, session, runtime untouched) |
| `cargo test --workspace --features modelswarm-node/verifydrafts` | **500 passed / 0 failed / 7 ignored** — the recorded 493/0/7 baseline + the same 3 transport tests + the 4 new QUIC-venue tests |
| `cargo test -p modelswarm-transport --features libp2p-backend` | 42/0 (30 lib incl. 3 new + 12 integration) |
| `cargo test -p modelswarm-node --features verifydrafts --test verifydrafts_wiring` | 5/0 — the staged wiring tests unchanged and green |
| `cargo test -p modelswarm-node --features verifydrafts --test verifydrafts_quic` | **4/0 — the QUIC-venue verdict: PASS** (repeated 5x, plus 3x transport re-runs: zero flakes) |

## 3. Test evidence

`cargo test -p modelswarm-node --features verifydrafts --test verifydrafts_quic`:

```
test backend_mismatch_rejects_honestly_over_quic ... ok
test remote_peer_mismatch_fails_closed_bad_frame_over_quic ... ok
test serve_verifydrafts_quic_serves_and_stops ... ok
test lifecycle_round_trips_over_real_quic ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured
```

The lifecycle asserts the full staged parity set over the real transport:
rounds 3/3 handled/committed, 21 committed tokens, committed stream =
window-prefixes ++ corrections, per-verifier EWMA fed, all `verify_pass_ms`
cross-checks `Ok`, `report.remote_peer_id ==` the client's ADR-020 PeerId
(QUIC-authenticated), teardown `ClientClosed`.

## 4. Assumptions made

1. "Do not change any existing method's behavior" binds observable
   behavior for frames/sequences that worked before. Three deliberate
   internal deltas, all required by the port contract and pinned by tests:
   the cancel-safe partial-buffer reads (a recv timeout mid-frame used to
   desync the stream silently); `send` now refuses > 256 KiB payloads
   locally (`TooLarge`) instead of writing a prefix the receiving side was
   already specified to reject (and > u32::MAX lengths no longer truncate);
   peer FIN at a frame boundary surfaces as `Closed` instead of an
   `Io(UnexpectedEof)`.
2. The QUIC adapter may live under plain `feature = "verifydrafts"` because
   the node→transport dep spec hard-codes `features = ["libp2p-backend"]`
   (the same fact the wiring handoff's §2 relied on for its test counts).
   If that dep spec ever drops the hard-coded feature, the QUIC adapter
   needs a `libp2p-backend` cfg too.
3. The graceful-close drain bound (1 s) mirrors `serving.rs`'s `linger`;
   it is a delivery window, not a guarantee — after it, teardown falls back
   to today's abrupt-drop semantics.
4. The verifydrafts CI job (T/R-reviewed steps at `f9ec2ff`) runs
   `cargo test --workspace --features modelswarm-node/verifydrafts`, which
   picks up the new tests with no workflow change needed.

## 5. Unresolved risks

1. The graceful-close Drop spawns a detached task holding the session; a
   process exit immediately after drop can still cut the last frame (the
   same residual risk class as `linger`, bounded by 1 s). Deterministic on
   loopback (5x green); a real-network pass has not been run (F2B remains
   the cross-machine venue).
2. The wakeup-resilience test's mid-flight-cancellation coverage is
   timing-dependent on loopback (the 240 KiB frame may arrive within one
   1 ms window); the property itself is structural (bytes live in the
   session struct) and code-reviewed rather than deterministically forced.
3. `serve_verifydrafts_quic` has no per-peer/session admission control
   (see §1); hostile-peer resistance there awaits the composed daemon
   wiring.
4. The two-pass decode (`WireMessage` then `Value`) means a future typed
   variant whose tag collides with a family discriminant would swallow
   family frames — the family's discriminants (`window_tokens`,
   `accepted_token_ids`, `engine`, `reason`, `nonce_echo`, `code`) do not
   collide with the frozen §6 tags today (externally-tagged single-key
   shapes); worth a line in the family's freeze ADR (wiring handoff G2/G3).

## 6. Suggested next task for the integrator

1. Fold this into the ADR-032 light-up record: G1 closed; the QUIC venue
   and its loopback proof exist (this file). Route assumption 4 to the
   Protocol Architect alongside G2/G3.
2. Windows Product follow-up: compose `serve_verifydrafts_quic` behind the
   daemon's listener demux with `serving.rs`'s `Admission` (the one seam
   this tier deliberately left open), plus lease derivation per the wiring
   handoff §5.
3. F2B (owner-gated): the real-network cross-machine relay proof — this
   venue is the transport it will ride.
