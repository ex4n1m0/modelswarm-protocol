# Handoff — Windows Product Engineer: ADR-032 light-up tier (VerifyDrafts serve-loop wiring + requester client), feature `verifydrafts`

Date: 2026-10-12 (task dated 2026-10-11; file name per the task directive).
Owner seat: Windows Product Engineer (`crates/modelswarm-desktop`,
`crates/modelswarm-node`, `crates/modelswarm-winjob`, `installer/`).
Base: `339ae9e` (Runtime's CapiVerifyEngine landed mid-task; see §7).

## 1. Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-node/Cargo.toml` | New OFF-by-default feature `verifydrafts = ["dep:modelswarm-transport", "dep:modelswarm-session", "dep:bs58"]`; optional deps `modelswarm-session`, `bs58 = "0.5"` added. Nothing production-visible: default builds compile none of it (`serving.rs` is untouched; the desktop, winjob, and installer trees are untouched — zero lines). |
| `crates/modelswarm-node/src/lib.rs` | Two lines: `#[cfg(feature = "verifydrafts")] pub mod verifydrafts;`. |
| `crates/modelswarm-node/src/verifydrafts/mod.rs` (new) | Module root: `unix_ms_now` (the gate's wall clock), `verifying_key_from_peer_id` (the ADR-020 identity-multihash INVERSE — recovers the Ed25519 key from a `12D3Koo…` PeerId, fail-closed on any foreign form, round-trip-guarded), `NodeVerifyTelemetry` (implements the session crate's `VerifyTelemetry` over node `Telemetry`; ids/codes/counts only), `http_adapter_disclosure` (the honest `batch_verify: false` + `batch_window_max: 1`), `clamp_budget_down`, `default_supported_presets` (all lossless presets; `deep` excluded), frame demux, and `WirePrefixCommit` (see gap G2). 7 unit tests. |
| `crates/modelswarm-node/src/verifydrafts/wire.rs` (new) | The transport port `VerifyDraftsWire` (`send_json`/`recv_json`/`remote_peer_id`; recv timeouts are NON-destructive wake-ups) + `StagedVerifyWire`: the concrete adapter over the staged TCP backend (`SignedFrameTransport`) with a dedicated pump task per session (the staged `Session::recv_json` CLOSES the session on deadline expiry, which would make RC-5(b) polling destructive; the pump isolates it behind channels — biased select, cancel-safe channels, no frame loss). `accept_staged` authenticates the client via `Listener::accept_with` (resolver = the trusted peer directory; `verify_handshake` pins the claimed PeerId to the resolved key — the staged equivalent of the QUIC-authenticated remote PeerId); `dial_staged` pins the server key. |
| `crates/modelswarm-node/src/verifydrafts/serve.rs` (new) | `serve_verifydrafts_session`: the §8 serve loop. Negotiation (offer → honest accept with the config-wired `EngineDisclosure`, or `SessionReject` `preset_unavailable`/`backend_mismatch`/`budget_infeasible`; effective budget clamped DOWN only); gate construction with the coordinator key PINNED from the transport-authenticated remote PeerId (or a roster-style `pinned_coordinator_key` override); the round loop with every RC-2 obligation (§3 mapping below); `VerifyDraftsServeReport` (counts/hashes/reason only — never token ids) and `ServeEndReason` (stable labels; every teardown carries an honest reason so the coordinator's fallback-to-single is never silent). `TimingEngine` wrapper measures the exact engine-pass wall time for `verify_pass_ms`. |
| `crates/modelswarm-node/src/verifydrafts/client.rs` (new) | `VerifyDraftsClient` (library-level; NOT wired into the desktop UI): `negotiate` (offer, accept/reject demux, `validate_session_accept`, `eligible_batch_verifier`; every failure is an explicit fallback-to-single signal) and `round(window)`: builds and signs `VerifyDrafts` (own RC-7 `seq`, fresh nonce, chained parent), `track_request`, requester-measured wall time, `admit_result` under RC-3 with the transport-supplied remote PeerId, per-verifier EWMA feed + `verify_pass_ms` cross-check (`verify_ms_lie` telemetry on a lie), then the reproducing `PrefixCommit` (RC-4/§8 WAIT_COMMIT → COMMITTED). Result admission includes the nonce echo (RC-3(b)) by construction via the coordinator. |
| `crates/modelswarm-node/tests/verifydrafts_wiring.rs` (new) | 5 integration tests over the REAL staged transport (real loopback sockets, real length-prefixed framing, the real Ed25519 §6.1 handshake, scripted mock engine): full lifecycle (3 engaged rounds: full acceptance, mid-window rejection, EOG present-as-null), backend mismatch → honest `SessionReject`, pinned-coordinator remote-peer mismatch → fail-closed `bad_frame` with zero engine work and session survival, budget exhaustion → `overloaded` + rejection-privacy assertions, WAIT_COMMIT timeout → round abandoned + next round gaps forward + commit advances. |
| `.github/workflows/rust.yml` | Two steps in the check job mirroring the libp2p-backend precedent (the task's explicit instruction that the wiring's tests run in the feature job): `Clippy (verifydrafts feature)` + `Tests (verifydrafts feature)`. **Cross-boundary note:** this file is Test-and-Release-owned; the two steps follow the established pattern exactly and are flagged here for T/R review. |
| `Cargo.lock` | The node seat's own entries (`bs58`, `modelswarm-session` under `modelswarm-node`) — per Runtime handoff §7's ask ("verify at merge time that the node seat's commit carries their own lock entries"). |

## 2. Exact commands run and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` | clean (after one `cargo fmt --all` normalization) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (default features) |
| `cargo test --workspace` | **459 passed / 0 failed / 6 ignored — exactly the recorded baseline**; default builds are behavior-identical (feature OFF; `serving.rs`, desktop, winjob, installer untouched) |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts -- -D warnings` | clean |
| `cargo test --workspace --features modelswarm-node/verifydrafts` | **493 passed / 0 failed / 7 ignored** |
| `cargo test --workspace --features modelswarm-node/libp2p-backend` (delta attribution only) | 482 / 0 / 7 — i.e. the verifydrafts run = 482 (the transport crate's libp2p tests come transitively with the node→transport dep spec, which hard-codes `features = ["libp2p-backend"]`) + **11 new tests of this change** (6 module unit + 5 integration) |

## 3. The RC-2 serve-loop obligations → implementation points

| ADR-032 §7 RC-2 item | Where it lives |
|---|---|
| **Draft signing** (the gate returns an unsigned draft; the serve loop signs) | `serve.rs::handle_request` — `sign_verify_drafts_result(&draft, &config.verifier_identity)` with the verifier identity; pinned by `wait_commit_timeout…` asserting `verify_verify_drafts_result_signature` under the server key over the real transport. The draft's `verify_pass_ms` is set to the wrapper-MEASURED engine-pass wall time (`TimingEngine` samples start/end around the single `verify_window` call — the gate trait is clock-free and takes caller-supplied clocks, so the two-step measure-then-write is the honest way to feed it; documented in-code). |
| **PeerId supply** (QUIC-/handshake-authenticated remote PeerId on every admission call) | `serve.rs` passes `wire.remote_peer_id()` into every `admit_verify_drafts`; the coordinator key is PINNED from that authenticated identity via `verifying_key_from_peer_id` (inverse of `peer_id_for`, fail-closed on non-Ed25519 multihashes), so the gate's own RC-1 pin (`peer_id_for(coordinator_key)`) is consistent by construction. `pinned_coordinator_key` allows roster-style pinning (exercised by the remote-peer-mismatch test: validly-signed frames from the wrong authenticated peer fail closed `bad_frame` BEFORE any stage, zero engine work). Client-side mirror: `admit_result(frame, wire.remote_peer_id())` (RC-3(a)). |
| **Budget seconds-ledger advancement on wall-clock deltas** | `serve.rs` loop head: `gate.advance_session_seconds(last_tick.elapsed())` on every iteration (§8e). Interpretation recorded in §5-A1: cumulative session wall time from the accept (the session holds the serving slot). |
| **WAIT_COMMIT expiry polling** (`deadline_ms + 250 ms` fixed grace) | Loop head `gate.expire_wait_commit_if_due(now_ms)`; the recv wake deadline is `min`-free: exactly the outstanding round's bound + `WAIT_COMMIT_GRACE_MS` when a round is outstanding, else the idle timeout. Non-destructive because `VerifyDraftsWire` recv timeouts are channel wake-ups (the pump owns the session). Pinned end-to-end by the gap-forward test (no commit → round abandoned at ~1.25 s → round 2 admitted, committed, chain advanced). |
| (Client-side RC-2) **timing EWMA feed** | `client.rs::round` — `timings.record_round(REQUESTER-measured wall ms)` after every admitted result (never the claimed value), `cross_check(verify_pass_ms, observed)` with `verify_ms_lie` telemetry through `NodeVerifyTelemetry`; per-verifier isolation via the session crate's `VerifierTimings`. |

## 4. API gaps found (the task's stop-and-record protocol)

- **G1 — BLOCKER (modelswarm-transport, Network Engineer's crate): the QUIC `Libp2pSession` has no raw-JSON passthrough.** ADR-032's family frames are bare JSON objects riding the ADR-018 amended staging; the staged TCP `Session` implements `send_json`/`recv_json`, but `Libp2pSession` speaks typed `WireMessage` frames only — a family frame cannot be SENT or RECEIVED on QUIC today. Per the task's gap protocol I STOPPED exactly these sub-parts: (a) the `serve_sessions`/`serve_one` QUIC extension (the module docs record this; `serving.rs` is untouched), and (b) the "loopback over the real QUIC transport" test venue. Everything else landed over the staged backend through the `VerifyDraftsWire` port. **Exact signatures needed** (mirroring `modelswarm_transport::Session`, including the stash discipline so typed and raw frames interleave safely):
  ```rust
  impl Libp2pSession {
      pub async fn send_json(&mut self, value: &serde_json::Value, deadline: Duration) -> Result<(), TransportError>;
      pub async fn recv_json(&mut self, deadline: Duration) -> Result<serde_json::Value, TransportError>;
  }
  ```
  Once landed, the adapter is a drop-in: `Libp2pSession::remote_peer_id` already exists and the port trait matches the TCP shapes. (Note: unlike the TCP backend, a QUIC recv timeout should NOT close the stream — that behavior is why the staged adapter needs its pump.)
- **G2 — wire-form decision needing adoption (modelswarm-session, Protocol Architect):** `CommitPrefix` carries no serde derives; the old vocabulary serializes it via `SpecMessage::PrefixCommit`. ADR-032 §2 says "the shipped `CommitPrefix` shape rides this family" and defers the binding migration to the freeze ADR. This tier serializes the shipped `SpecMessage::PrefixCommit` FIELD NAMES as a bare object (including `profile_id` for the state machine's `model_profile_id`, no `signature` — the commit rides the authenticated session and the gate enforces RC-4 reproduction), via the local `WirePrefixCommit` mirror (unit-pinned field-for-field). Ask: adopt or amend this exact family wire form in the freeze ADR (or give `CommitPrefix` serde + a strict parse).
- **G3 — negotiation binding (Protocol, ADR-level):** `SessionOffer` carries neither `profile_id` nor the generation-params hash. The verifier therefore pins both from node config (profile = the hosted exact profile; params = `config.sampling_params` → `batch_generation_params_hash`). A coordinator configured with different sampling parameters gets `bad_frame` on every round — honest fail-closed, but undiscoverable at negotiation. A future ADR could echo a params commitment in the accept.
- **G4 — already recorded by Runtime (their handoff §4):** the `BatchVerifyEngine` trait-cycle (session depends on runtime). Runtime's suggested option 4(a) — the node hosts the `CapiVerifyEngineAsTrait`-style newtype — is the natural next step for the production engine wiring and composes directly with this serve loop (it takes `Arc<dyn BatchVerifyEngine>`); intentionally NOT done here (the task pins the honest `batch_verify: false` default until the family's light-up gates run).
- **G5 — minor (session crate):** `VerifyDraftsCoordinator` has no resync for `committed_prefix_hash` (getter only). On a `parent_mismatch` CancelRound the client errors out honestly instead of resyncing from the rejection's `prefix_hash` echo; acceptable at this tier, recorded as a limitation.

## 5. Assumptions made

1. **Seconds ledger = cumulative session wall time from the accept** (the session holds the serving slot for that long; idle time burns the coordinator's own budget). The ADR names "engine seconds" as the harm; the gate's shipped stage-11 check is already-at-cap on `served_ms`. Protocol's A-2 advisory (would-exceed vs already-at-cap asymmetry) is still open and may refine this.
2. The ADR-026 lease is a serve-config input (`lease_expires_at_ms`); the composed daemon derives it from the lease the coordinator presented at session open (the family's offer wire carries no lease field — same family as G3).
3. A non-offer first frame, an offer with a wrong protocol version, or a non-Ed25519 remote PeerId ends the session WITHOUT a `SessionReject` (no frozen reason exists for those); all are labeled honestly in the report/telemetry.
4. The staged backend's server-side authentication is address/key pinning (its own honesty doc: the ack is unsigned); the cryptographic PeerId binding arrives with the QUIC adapter (G1). The client reports the PINNED server PeerId, never a payload claim.
5. `preset_unavailable` is served for `deep` only (approximate tier, out of this lossless family); `batch_verify_unsupported` is never auto-sent — an incapable peer ACCEPTS declaring `batch_verify: false` (ADR-032 §3/§4) and the client surfaces `Ineligible` as the fallback-to-single signal.
6. Engine failure is not a §6.5-family wire code (the `FrameRejectReason` set is frozen at 8): the serve loop ends the session with a stable local label (`engine_*`) and telemetry; engine detail never crosses the wire.

## 6. Unresolved risks

1. **G1 blocks the production venue**: until `Libp2pSession` grows the passthrough, the family cannot ride QUIC; the staged backend is loopback-only by the ADR-018 honesty guard, so no cross-machine VerifyDrafts traffic is possible. This is the top coordinator item.
2. The verifydrafts CI job transitively compiles the transport's libp2p-backend (the node→transport dep spec hard-codes it) — heavier than strictly needed, kept to avoid touching the existing feature's mechanics.
3. `WirePrefixCommit` (G2) is this tier's pick; a divergent freeze-ADR ruling would change the wire bytes (tests pin the current shape).
4. The pass-time measure (`TimingEngine`) includes trait-dispatch scheduling inside the pass window (start sampled at `verify_window` entry) — an upper-bounded approximation of the pure engine pass; the cross-check's 25 ms noise bound absorbs it on real engines, and mock-loopback pins assert `≥ 0` semantics only (ADR A4 discipline).
5. Multi-session composition (the daemon wiring `serve_verifydrafts_session` behind a real listener alongside the msp-v1 bridge, lease derivation, desktop UI surfaces) is deliberately not in this task; nothing production-visible changed.

## 7. Concurrent-workspace note (for the integrator)

Runtime's `339ae9e` (CapiVerifyEngine) landed mid-task; their §7 stash/restore of this seat's untracked WIP lost the `src/verifydrafts/` directory once — it was restored from in-context copies and re-gated (nothing of theirs was touched; no overlap: their commit touches `modelswarm-runtime` + docs only). This commit carries its own Cargo.lock entries (Runtime's §7 ask). The untracked `zcode-master-prompt.md` at the repo root predates this task and is not part of this change.

## 8. Suggested next task for the integrator

1. **Route G1 to the Network Engineer** (the two `Libp2pSession` signatures above); their landing unblocks the QUIC `serve_sessions` demux + the QUIC-venue loopback test, after which this port's QUIC adapter is a small follow-up on this seat.
2. Route G2/G3 to the Protocol Architect (family commit wire form; offer/params binding) before any second consumer exists.
3. T/R review of the two new CI steps (this file's only cross-boundary edit).
4. Then (owner-gated, per ADR-032 §7): the node-hosted `CapiVerifyEngineAsTrait` bridge (Runtime's 4(a)) + the capi flag flip + item-2 exactness-pin reruns.
