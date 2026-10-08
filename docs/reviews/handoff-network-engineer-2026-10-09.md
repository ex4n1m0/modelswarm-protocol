# Handoff — Network Engineer domain audit (master-prompt audit, 2026-10-09)

Read-only audit of `crates/modelswarm-transport` and `crates/modelswarm-relay`
(+ the transport-consuming seams in `crates/modelswarm-node/src/{remote,serving}.rs`
and `crates/modelswarm-desktop/src/app.rs`, read for transport-correctness only).
Baseline `730cebf` (branch `audit/master-prompt-2026-10-09`); `63b98e1` on main
is CI-only and touches nothing here. No code modified, no cargo/npm executed;
suite evidence read from `target/audit-logs/`.

## 1. Changed files and why

- `docs/reviews/handoff-network-engineer-2026-10-09.md` (this file) — the sole
  output, per the audit's read-only constraint.

## 2. Commands run and outcomes

Read-only: `git log --oneline`, `git status`, `grep`/`sed`/`find` over the two
owned crates, the consuming seams, `protocol/msp-v1.md` §6–7, ADR-014/018/026,
`docs/verification/phase-f-lan-2026-10-07.md`, `f2a-relay-2026-10-08.md`,
`docs/reviews/phase-f-research-2026-10-07.md`, the freeze record, and the
vendored `libp2p-quic-0.14.0` / `libp2p-relay-0.22.0` sources in the cargo
registry (to verify QUIC keepalive/idle/window defaults and the relay-client
API surface). `target/audit-logs/summary.txt`: fmt PASS, clippy (default and
libp2p-backend) PASS, `cargo test --workspace` PASS 301 tests / 49 suites,
libp2p-backend PASS 313 tests / 49 suites (0 failed), tracker all PASS.

## 3. Findings (severity-ranked; file:line at `730cebf`)

### HIGH-1 — Production QUIC path exposes zero RTT/loss telemetry; the measurement-plumbing prerequisite for M9/M10 and the scheduler does not exist

- Evidence: `Session::measure_rtt` + `Stats` (p50/p95/jitter) exist ONLY on the
  staged TCP backend (`crates/modelswarm-transport/src/transport.rs:366-393`);
  `Libp2pSession` has no ping/pong, no stats, nothing
  (`crates/modelswarm-transport/src/libp2p_backend.rs:55-168`); peer selection
  in the desktop is roster order, not latency
  (`crates/modelswarm-desktop/src/app.rs:1434-1450` "First peer with a real QUIC
  multiaddr"); the scheduler's EWMA inputs are unwired (master prompt §0 ground
  truth). Only `apps/modelswarm-sim` measures (loopback TCP, sim-only).
- Impact: no fastest-eligible-peer selection, no direct-vs-relay cost class
  (ADR-014 #4), no hedged requests (§11), no WAN baselines (M9) — everything
  waits on this.
- Time-boxed fix: 1–2 d — add `Control` ping/pong handling + `measure_rtt` +
  an EWMA wrapper to `Libp2pSession` (the ping/pong frames already exist in
  `message.rs:162-198`; the loopback-hardened pattern to copy is
  `transport.rs`'s transparent in-band ping answering), export per-peer
  (rtt_ewma_ms, jitter, last_seen) into telemetry; then wire selection.

### HIGH-2 — Pre-first-token transport failures over a pooled session are reported non-retryable; ADR-007 would allow a retry/failover

- Evidence: client maps any mid-stream recv failure to
  `ExecutorEvent::Error{retryable:false}` regardless of tokens seen
  (`crates/modelswarm-node/src/remote.rs:140-151`); the serving side always
  sends `retryable_peer_hint:false` (`crates/modelswarm-node/src/serving.rs:419-430`);
  the gateway then never retries even at 0 emitted tokens
  (`crates/modelswarm-gateway/src/http.rs:507-518`). The stale-pool window is
  real: the server closes a pooled session after `BETWEEN_REQUESTS = 300 s`
  (`serving.rs:254`) while the client pool keeps the session forever with no
  TTL (`remote.rs:44-53`) — a chat gap >300 s (or a server restart) makes the
  next request race the server's close path.
- Impact: spurious user-visible failures exactly where ADR-007 permits failover;
  the desktop chat masks it via local fallback, the API path does not.
- Time-boxed fix: 2–4 h — in `stream_events`, set `retryable = (tokens seen in
  THIS stream == 0)` (the unfold already tracks that state); mark server-side
  transient codes (`over_limit`, `executor_retryable`) `retryable_peer_hint:true`.

### MEDIUM-1 — Fixed 30 s per-frame deadline can fire during long prefill (TTFT > 30 s) and kill an otherwise healthy request, non-retryably (compounds HIGH-2)

- Evidence: `IO_DEADLINE = 30 s` per frame on the requester path
  (`remote.rs:33,140`); the request itself may carry `deadline_ms` up to
  120 000 (`app.rs:1478`; clamps at `serving.rs:103-114`). Nothing is sent
  between `InferenceRequest` and the first `TokenDelta`, so prefill silence is
  indistinguishable from a dead peer at t+30 s.
- Time-boxed fix: 1–2 h — derive the per-frame deadline from the request's
  (clamped) `deadline_ms` for the pre-first-token phase; keep a shorter
  between-tokens deadline once streaming starts.

### MEDIUM-2 — Listener death is undetectable: serving looks alive with a dead socket

- Evidence: the driver task drops `ListenerError`/`ListenerClosed` events
  (`libp2p_backend.rs:249-252` "AddressChange/ListenerError/ListenerClosed:
  … not forwarded"); `accept()` then times out on a 1 s loop forever and
  `serve_sessions` continues (`serving.rs:48-62`) while heartbeats keep
  advertising the dead multiaddr.
- Time-boxed fix: 2–3 h — forward a terminal driver event and return
  `TransportError::Closed` from `accept`; node treats it as fatal telemetry
  and withdraws/refreshes the advertised address (coordination point with
  Windows Product for the heartbeat side).

### MEDIUM-3 — Wire-vs-spec drift vs msp-v1 §6 (confirms and extends the code audit's M4)

Verified drift, all on the raw-QUIC production path:

1. No `/msp/infer/1` protocol-id negotiation and no multistream-select — raw
   QUIC streams (`libp2p_backend.rs:288-324`). Also the structural blocker for
   F2b relay use (RESERVE/HOP are negotiated protocols).
2. No §6.1 handshake frames at all on this backend — QUIC TLS peer
   authentication replaces them (`libp2p_backend.rs:49-52`); consequently
   `incompatible_protocol` can never be signalled and profile/runtime intent
   arrives only per-request. (§6.6's "Handshake.peerId MUST equal the
   Noise-authenticated remote PeerId" is satisfied in spirit by the dial-side
   expected-PeerId check, `libp2p_backend.rs:311-315`, and ADR-026's
   QUIC-PeerId lease binding, `serving.rs:304-312`.)
3. No `accepted` stream event (§6.1/§6.3): `ExecutorEvent::Accepted` is
   skipped (`serving.rs:377`), no `Accepted` wire variant exists
   (`message.rs:202-215`). Queue position/etaMs never travel — also the M10
   bounded-wait-queue prerequisite.
4. Error codes off the frozen §6.5 set: the serving bridge emits
   `over_limit`/`session_limit`/`per_peer_session_limit`, `bad_frame`,
   `duplicate_request` (§6.5 says `overloaded`, `replayed_request`),
   `invalid_lease` (ADR-026, never added to §6.5), `executor_error`
   (`serving.rs:76,263,273,293,309,321,370`; frozen set at
   `error.rs:9-23`).
5. `StreamError.request_id` is always `""` (session-level conflation,
   `serving.rs:423`) vs §6.3's per-request error.
6. `Completed` carries no `job_receipt` — known, documented staging omission
   (`message.rs:141-151`); receipts v2 pending.
- Time-boxed fix: fold items 1–2 into the F2b ADR (either negotiate
  `/msp/infer/1` on Swarm streams or amend msp-v1 to record the QUIC-TLS
  staging shape); ~2–4 h spec/ADR work for items 4–5 (sync codes or extend the
  set via ADR); 1 d if the `accepted` event is added now (needed for M10
  queueing regardless).

### MEDIUM-4 — No Cancel support over the pooled serving path; cancellation is connection teardown

- Evidence: the serve loop accepts only `InferenceRequest`; any other frame —
  including `Cancel` — is a fatal `bad_frame` that closes the pooled session
  (`serving.rs:256-269`). Requester cancellation works only by dropping the
  stream/session (self-healing next dial), contradicting §6.1 "cancel, either
  side, anytime" and discarding the warm connection plus any batching hope.
- Time-boxed fix: 1–2 d — handle `Cancel` in `serve_one` (between-requests and
  a per-request cancellation channel into the executor loop) and expose
  cancellation on `RemoteExecutor`/`Libp2pSession` (the staged TCP backend's
  `request_cancel`, `transport.rs:352-363`, is the model).

### MEDIUM-5 — Stop/drain is not graceful: session tasks are spawn-and-forget

- Evidence: `serve_sessions` spawns per-session tasks with no `JoinSet`/
  tracking and returns on shutdown while in-flight sessions keep running
  (`serving.rs:69-94`); no drain window, no graceful listener close ordering.
  M7's Stop sequence ("reject new leases → drain or safely cancel") has no
  transport-side primitive.
- Time-boxed fix: 3–5 h — `JoinSet`, close listener first, broadcast shutdown
  into `serve_one`, bounded drain window (e.g. 10 s), then cancel; surface a
  drained-sessions count in telemetry.

### MEDIUM-6 — Relay: open access + uncapped per-circuit bytes (concurs with security audit M-1) + minor key-at-rest note

- Evidence: `relay/main.rs:31-42` (`max_circuit_bytes: u64::MAX`, no access
  control, `:10-12` documents the choice); the security audit's attack
  (bandwidth exhaustion / relay-as-amplifier against serving listeners;
  `docs/reviews/audit-security-findings.md` MEDIUM-1). Additional from this
  audit: the relay key file is written plaintext with default permissions
  (`relay/main.rs:59-61`) — same at-rest class as security M-3; and a missing
  closing paren in the startup banner (`relay/main.rs:181`).
- Design assessment: the unlimited-duration/unlimited-bytes choice is
  CORRECT for duration (u32-second max, stream-carrying requirement per
  phase-f research) and acceptable for bytes only while the relay is
  LAN-only. Before any router port-forward: cap `max_circuit_bytes` at ~4 GiB
  per circuit (a token stream is ~0.3 KB/s; pooled reuse amortizes circuits
  across requests — honest traffic never approaches the cap) and/or a
  per-peer byte allowance; reservation ACLs are an optional owner decision,
  not a v0.x requirement (lease gating stays at serving peers per ADR-026).
  Time-boxed fix: 1–2 h (byte cap + banner + 0600 key perms on Unix).

### LOW-1 — Deadline semantics differ between backends; libp2p timeout leaves a half-desynced session open

- Evidence: staged `Session` closes itself on deadline expiry
  (`transport.rs:279-283,310-316`); `Libp2pSession::send/recv` return
  `Timeout` leaving the stream mid-frame and the session usable
  (`libp2p_backend.rs:135-151`). Contained today only because both callers
  drop the session on any error; a future caller that retries on the same
  session after a Timeout would interleave garbage bytes.
- Time-boxed fix: 1 h — reset/close the stream on timeout, or document the
  terminal-error contract on the type.

### LOW-2 — Dial under the pool lock; fresh UDP endpoint per dial

- Evidence: `remote.rs:200-219` holds the pool mutex across a dial with up to
  `DIAL_DEADLINE = 10 s` — concurrent first requests serialize; each dial
  builds a new `quic::tokio::Transport`/socket (`libp2p_backend.rs:297`).
  Fine at k=1; relevant once hedging dials k peers in parallel.
- Time-boxed fix: fold into the multi-peer work (shared endpoint, lock-free
  take-then-dial).

### LOW-3 — QUIC config is stock defaults; verify the pooled-idle interplay once, empirically

- Evidence: `quic::Config::new` defaults (libp2p-quic-0.14.0 `src/config.rs:87-101`):
  idle timeout 10 s, keepalive 5 s, 256 bidi streams, 10 MB stream / 15 MB
  connection windows, handshake timeout 5 s. Backpressure and stream limits
  are therefore real (bounded windows; §6.4's "bounded libp2p buffers" holds).
  Pooled sessions should survive idle gaps because quinn's keepalive fires
  while ≥1 stream is open and our pooled session never closes its stream —
  inferred, not measured (see Risks).

### LOW-4 — Admission control numbers not tied to advertised slots (msp-v1 §6.4)

- Evidence: `Admission::defaults()` = 8 sessions / 2 per peer
  (`serving.rs:175-179`); §6.4 says max concurrent requests = advertised
  slots. Cross-ref security M-2 (admission runs before the lease gate).

## 4. Component classifications (salvage-matrix rows)

| Component | Current role | Evidence | Decision | Reason |
|---|---|---|---|---|
| `modelswarm-transport::frame` | 256 KiB length-prefixed codec, prefix validated before allocation | `frame.rs:41-95` + 6 unit tests incl. oversize-before-read | KEEP WITH TESTS | Exact ADR-018 staging; hostile-prefix tests exist |
| `modelswarm-transport::transport` (staged TCP) | CI/loopback backend: signed handshake, bounded queues (16/16), deadline-closes-session, RTT stats, cancel | `transport.rs` + `tests/loopback.rs` (18 integration tests: tampering, deadline, concurrency, passthrough) | KEEP WITH TESTS | ADR-018 staging path; the semantics reference implementation |
| `modelswarm-transport::message` / `error` / `handshake` | msp-v1 §6 typed surface + §6.5 codes + canonical-JSON signatures | proto-name tests `message.rs:244-271` | KEEP WITH TESTS | Field names pinned to `protocol/messages.proto`; drift is at the flow level (MEDIUM-3), not fields |
| `modelswarm-transport::libp2p_backend` | Production raw-QUIC transport; PeerId=ADR-020 derivation verified at dial; `remote_peer_id()` feeds the ADR-026 lease binding | LAN-proven (phase-f-lan doc); loopback tests in-file | KEEP WITH TESTS → REPLACE incrementally at F2b (same public surface) | No multistream/protocol-id, no relay/DCUtR, single stream per connection; ADR-018 names it the swap point |
| `modelswarm-node::remote` (pooled RemoteExecutor) | One-peer executor, single-session pool, self-heal dial on error | `remote.rs`; `two_requests_reuse_one_session` test | KEEP WITH TESTS + HIGH-2/MEDIUM-1 fixes | Pool poisoning: none reachable (any error drops the session; only a clean `Completed` returns it, and `Usage` provably precedes `Completed` on the wire, `executor.rs:183→194`); retryable-mapping bug instead |
| `modelswarm-node::serving` (bridge) | Session→executor→frames loop, lease gate, clamps, admission, replay dedup | serving.rs; lease tests + `serving_bridge_refuses_*` (ADR-026) | KEEP WITH TESTS + MEDIUM-3/4/5 fixes | Solid prototype; code-set drift + no cancel + no drain |
| `modelswarm-relay` | Standalone circuit-relay v2 host, raised limits, F2A-proven interop | relay/main.rs; 2 tests incl. real reserve+circuit+bidir ping | KEEP WITH TESTS + MEDIUM-6 hardening before internet exposure | Standard rust-libp2p; content-blind; three non-obvious deployment requirements documented |
| Multi-peer-per-request transport (hedged/k-parallel) | Does not exist | single session per executor; single stream per connection (`libp2p_backend.rs:60-74`) | NEEDS EXPERIMENT (then build) | k dials possible today; no per-parallel-stream limits, no hedging coordinator, no TTFT-percentile signals (HIGH-1) |
| Peer telemetry EWMA plumbing | Does not exist on the production path | HIGH-1 | NEEDS EXPERIMENT (measurement prerequisite per gap study/§11) | sim-only today |
| Network emulator configurations | Essentially none: sim does loopback RTT probes only (`apps/modelswarm-sim/src/lib.rs:207-380`); no loss/bandwidth/delay-injection harness | — | NEEDS EXPERIMENT | Required for reproducible WAN/degraded-network baselines (M9) and relay-vs-direct honesty numbers |

## 5. F2b — what is precisely missing, options, verdict

Missing for a production node to use the F2A relay: (a) multistream-select
negotiation on our QUIC streams (RESERVE `/libp2p/circuit/relay/0.2.0/reserve`
and HOP `/libp2p/circuit/relay/0.2.0/hop` are negotiated protocols; our
transport opens raw streams); (b) the reservation lifecycle — a persistent
stream, kept open, renewed (~2 min TTL), address bookkeeping; (c) `/p2p-circuit`
listen on the serving side and circuit dial in `RemotePeer`; (d) heartbeat
advertisement of relay multiaddrs (the roster schema already accepts them:
`apps/tracker/lib/schemas.ts:47`, up to 16 addresses); (e) dial-order policy
(direct first, circuit fallback) with explicit direct-failure reporting and a
`direct|relayed` path label per ADR-014 #4/#5.

**Option A — extend the raw-QUIC backend by hand.** Reuse the public
`multistream-select` crate (0.14, same crate libp2p uses) for negotiation, but
copy the relay protobuf definitions + write the reservation/HOP/STOP state
machines ourselves: everything in `libp2p-relay 0.22` is `pub(crate)`
(`src/protocol.rs:27-30`). Also requires multi-stream-per-connection support in
`Libp2pSession` FIRST (a reservation is a long-lived stream that must coexist
with request traffic; our session is one stream, `libp2p_backend.rs:60-74`) —
which is coincidentally a hedging prerequisite. Effort: 4–6 focused days incl.
interop tests against `modelswarm-relay` and a standard client. Risk: HIGH
permanent protocol-drift maintenance (every libp2p upgrade re-audits our copy;
the u32-duration panic found in F2A is exactly this bug class), and DCUtR would
be yet another hand-rolled protocol — this option does not advance ADR-014.

**Option B — migrate the serving + dial paths onto a libp2p Swarm (recommended).**
SwarmBuilder (QUIC + noise/yamux for the relay-client transport) with relay
client, identify, ping behaviours (dcutr/autonat at F2c); dial direct and
`/p2p-circuit` addresses; the serving bridge moves from `Libp2pListener` to
swarm events behind a thin custom stream-handling behaviour (~150–250 lines;
note `libp2p-stream` is not in the 0.57 dependency tree — verify availability,
else the minimal behaviour). The frame codec, lease gate, clamps, and pooled
executor logic carry over unchanged. Effort: 4–6 days including the ADR-018
gate re-runs (D/E suites, pooled-reuse re-proof, LAN cross-machine re-proof,
and a new relayed A→relay→B proof). Risk: bounded refactor of
`serving.rs`/`remote.rs`/`app.rs` (Windows Product coordination per ownership
map); dependency surface grows only by enabling features (`relay`, `dcutr`,
`autonat`) on the existing libp2p 0.57 dependency — no new external crates.
Decisive advantages: maintained code owns reservations/renewals; DCUtR/AutoNAT
arrive for free; `StreamProtocol` negotiation naturally carries `/msp/infer/1`,
closing MEDIUM-3 items 1–2; the relay binary and node speak the same stack.

**Verdict: Option B**, preceded by a half-day ADR (transport-swarm + relay
client + protocol-id/handshake + §6.5 code-set sync + `accepted` event
decision in one document). Total ~5–7 working days to a verified relayed
completion. Choose Option A only if the owner vetoes the Swarm refactor and
accepts relay-without-DCUtR indefinitely.

## 6. M8/M9 readiness (network domain)

| Capability | Status | Evidence | Gap to close |
|---|---|---|---|
| Identity/registration/heartbeat, real multiaddrs | LAN-proven | phase-f-lan doc; `app.rs:242-289` | — |
| Direct encrypted sessions (QUIC TLS, PeerId-bound dial) | LAN-proven | `libp2p_backend.rs:288-324` | — |
| ADR-026 lease binding via QUIC PeerId | LAN/CI-proven | `serving.rs:304-312`, `remote_peer_id()` `libp2p_backend.rs:78-80` | re-verify after F2b |
| Pooled connection reuse | loopback+CI proven; cross-machine re-probe PENDING (machine B offline at freeze) | `remote.rs` pool + reuse test; f2a doc "gated LAN re-probe" | re-run `lan_cross_machine_completion` |
| Drain/reconnect | MISSING | MEDIUM-5, MEDIUM-2 | JoinSet drain + listener-death signal |
| Hole-punch (DCUtR) | NOT STARTED; expect 60–70% best case, 30–50% of internet pairs relay-dependent (research, not our measurement) | phase-f-research §1 | after F2b (Swarm) |
| Relay fallback end-to-end | relay standalone proven; nodes cannot use it | f2a "honest boundary" | F2b + heartbeat relay addr + dial order + path label |
| Direct-vs-relay measurement/disclosure (ADR-014 #4/#5) | NOTHING measured | — | `path_type` telemetry + scheduler cost class + published split |
| WAN anything | UNPROVEN | master prompt §0 | post-F2b WAN baselines |
| Peer selection input (RTT EWMA) | sim-only | HIGH-1 | ping/pong on the QUIC path + wiring |
| Multi-peer per request (HEDGED prerequisite) | NOT STARTED | LOW-2 + classification table | k-executor fan-out, per-stream limits |

Performance-claim labels (per §0 honesty rule): pooled reuse = loopback/CI
proven, LAN pending; relay limits/throughput = in-process loopback QUIC only,
never under WAN load; RTT/jitter = sim/loopback only; hole-punch % =
literature expectation, unmeasured; all WAN claims: none exist.

## 7. Assumptions

1. Audit baseline is `730cebf` for all file:line references; `63b98e1` (CI
   overlay) touches no owned file.
2. Quinn's 5 s keepalive fires while at least one stream is open, so pooled
   sessions (whose stream is never closed) survive the 10 s idle timeout —
   inferred from libp2p-quic defaults; flagged as an unresolved risk until a
   30-minute experiment (idle >60 s pooled session, then reuse) pins it.
3. Security audit M-2 (admission before lease gate) and M-3 (seed at rest) are
   cross-referenced, not re-audited; my relay byte-cap assessment concurs with
   their M-1 and adds the key-file-perms note.
4. Desktop chat's local-executor fallback currently masks HIGH-2 for chat
   users; API consumers see it directly. Fix is in my domain regardless.
5. `MSP_LISTENER=1` non-loopback opt-in (`libp2p_backend.rs:217`) is the
   sanctioned F0 friendly-network flag; the honesty guard itself stays
   tested and intact.

## 8. Unresolved risks

- Assumption 2 (keepalive vs pooled idle): if wrong, every chat gap >10 s
  silently drops pooled sessions; self-heal hides it at a latency cost and
  HIGH-2 makes the failure non-retryable. Cheap experiment, do first.
- F2b Option B touches `serving.rs`/`remote.rs`/`app.rs` (Windows Product
  surface) — sequential integration + review required.
- The relay must not be port-forwarded before MEDIUM-6's byte cap.
- Scheduler/M9 work started before HIGH-1 lands would build on unmeasured
  inputs — measurement plumbing is the stated prerequisite, not polish.

## 9. Suggested next task for the integrator

Approve and sequence: (1) the HIGH-2 + MEDIUM-1 retryable/deadline fixes and
the keepalive experiment (≤1 day, unblocks honest pooled behavior);
(2) the F2b ADR (Option B) with the §6 drift decisions folded in; (3) F2b
implementation; (4) HIGH-1 measurement plumbing as F1.5 so M9 baselines and
the scheduler consume real RTT EWMA. Reviewers per ownership map: Security,
Test and Release.

---

Status: audit complete; read-only; one output file.
Owned paths: `crates/modelswarm-transport/`, `crates/modelswarm-relay/`,
transport integration tests, network emulator configurations.
Files changed: this file only.
Commands: read-only git + text/source inspection; no builds (logs from
`target/audit-logs/`, all PASS).
Tests: none run by me (constraint); frozen suite green: 301 (default) / 313
(libp2p-backend) tests, 0 failed; relay suite 2/2 green per F2A record.
Evidence: every finding carries file:line at `730cebf`; verification docs
phase-f-lan-2026-10-07, f2a-relay-2026-10-08.
