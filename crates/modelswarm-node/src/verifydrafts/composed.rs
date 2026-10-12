//! The composed daemon: ONE QUIC listener serving BOTH session
//! vocabularies (feature combination `verifydrafts` + `libp2p-backend`;
//! the light-up handoff's composed-daemon seam, closed here).
//!
//! [`serve_sessions_composed`] is the msp-v1 bridge
//! ([`crate::serving::serve_sessions`]) with the VerifyDrafts family
//! demultiplexed in behind the SAME listener, the SAME session admission
//! (serving.rs's `Admission`: total + per-peer session caps — one gate
//! for both vocabularies), and the SAME ADR-026 lease discipline:
//!
//! - **msp-v1 sessions** (typed `WireMessage` frames): the unchanged
//!   [`crate::serving::serve_one`] loop — per-request hub-signed lease
//!   check (`LeasePolicy`), §6.4 clamps, executor streaming;
//! - **family sessions** (raw-JSON ADR-032 frames): the unchanged
//!   [`super::serve::serve_verifydrafts_session`] loop, with the
//!   coordinator's lease derived AT SESSION OPEN from the
//!   QUIC-authenticated PeerId through the [`FamilyLeaseSource`] seam
//!   (fail-closed: no lease ⇒ no family session — the family's offer wire
//!   carries no lease field, wiring handoff §5-A2/G3; the production
//!   source is the daemon's eligibility/roster state, injected here).
//!
//! # Demux
//!
//! A session's vocabulary is classified by its FIRST frame, with zero
//! added latency for msp-v1 sessions: the loop awaits a TYPED frame
//! (1 s polls inside the between-requests budget); a typed frame returns
//! immediately, while a raw family frame is STASHED by the typed recv and
//! picked up by a 1 ms raw recv (the transport's stash discipline — stash
//! first, never hidden behind an expired deadline). The peeked frame is
//! handed to the winning serve path pre-read ([`crate::serving::serve_one_seeded`]
//! / [`PrefacedQuicWire`]) — never decoded twice.
//!
//! Default builds (the feature combination off) keep today's
//! single-vocabulary `serve_sessions` byte-identically; this module is
//! additive.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use modelswarm_gateway::InferenceExecutor;
use modelswarm_telemetry::Telemetry;
use modelswarm_transport::libp2p_backend::{Libp2pListener, Libp2pSession};
use modelswarm_transport::message::WireMessage;
use modelswarm_transport::TransportError;

use crate::serving::{send_error, serve_one_seeded, Admission, LeasePolicy};

use super::engine_source::FamilyEngineSource;
use super::serve::{serve_verifydrafts_session, VerifyDraftsServeConfig};
use super::wire::{QuicVerifyWire, VerifyDraftsWire, WireError};

/// How long a composed session may wait for its FIRST frame (mirrors the
/// msp-v1 loop's between-requests wait — both vocabularies get the same
/// session treatment).
const FIRST_FRAME_IDLE: Duration = Duration::from_secs(300);

/// The typed-recv poll inside the first-frame wait (raw family frames are
/// stashed by the typed recv and collected on the next poll boundary —
/// bounded classification latency for family sessions, zero for msp-v1).
const DEMUX_POLL: Duration = Duration::from_secs(1);

/// ADR-026 lease derivation for family sessions AT SESSION OPEN — the
/// composed daemon's seam. Given the QUIC-authenticated coordinator
/// PeerId, returns the lease expiry (unix ms) the session serves under;
/// `None` = NO lease ⇒ the session is refused fail-closed (the
/// coordinator's client surfaces a transport failure and falls back to
/// single, never silently).
///
/// The family's offer wire carries no lease field (wiring handoff
/// G3/§5-A2); the production source is the daemon's eligibility/roster
/// state for the authenticated peer, wired in here.
pub trait FamilyLeaseSource: Send + Sync {
    /// The lease expiry for `coordinator_peer_id` (the transport-
    /// authenticated remote PeerId), or `None` to refuse.
    fn lease_expires_at_ms(&self, coordinator_peer_id: &str) -> Option<u64>;
}

/// Fail-closed default: refuses every family session (the honest default
/// until the daemon wires a real lease source — serving family sessions
/// without a lease derivation would break the ADR-026 discipline the
/// msp-v1 path enforces per request).
pub struct RefuseFamilyLeases;

impl FamilyLeaseSource for RefuseFamilyLeases {
    fn lease_expires_at_ms(&self, _coordinator_peer_id: &str) -> Option<u64> {
        None
    }
}

/// Fixed-expiry source (tests, and a daemon pinning one roster entry).
pub struct StaticLeaseExpiry(pub u64);

impl FamilyLeaseSource for StaticLeaseExpiry {
    fn lease_expires_at_ms(&self, _coordinator_peer_id: &str) -> Option<u64> {
        Some(self.0)
    }
}

/// The family half of the composed daemon's configuration.
pub struct FamilyServing {
    /// The serve-loop config (profile, identity, budgets, presets, idle
    /// and I/O deadlines). `engine` is overridden at session open with the
    /// injected engine's REAL capabilities — the honest disclosure from
    /// real geometry, never hand-set.
    pub config: VerifyDraftsServeConfig,
    /// Where family sessions get their batch-verify engine (mock/stub or
    /// the production C-API bridge under `verifydrafts-capi`); one shared
    /// handle, asked for a fresh engine per accepted family session.
    pub engine: Arc<FamilyEngineSource>,
    /// The ADR-026 lease seam (see [`FamilyLeaseSource`]).
    pub lease: Arc<dyn FamilyLeaseSource>,
}

/// What the first-frame classifier found (the frame rides along — the
/// winning serve path consumes it pre-read).
enum FirstFrame {
    /// A typed msp-v1 frame won.
    MspV1(WireMessage),
    /// A raw family frame won.
    Family(Value),
    /// The session went idle or vanished before any frame arrived.
    Gone,
}

/// Classifies one accepted session by its first frame. msp-v1 frames
/// return from the typed recv immediately (no added latency); raw family
/// frames are stashed by the typed recv and collected on the poll
/// boundary via the stash-first raw recv.
async fn demux_first_frame(session: &mut Libp2pSession, idle: Duration) -> FirstFrame {
    let started = Instant::now();
    loop {
        let remaining = idle.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return FirstFrame::Gone;
        }
        let poll = remaining.min(DEMUX_POLL);
        match session.recv(poll).await {
            Ok(msg) => return FirstFrame::MspV1(msg),
            Err(TransportError::Timeout) => {
                // A raw family frame may have arrived and been stashed by
                // the typed recv: the stash-first raw recv returns it
                // immediately (the 1 ms bound only applies when nothing
                // was stashed).
                match session.recv_json(Duration::from_millis(1)).await {
                    Ok(value) => return FirstFrame::Family(value),
                    Err(TransportError::Timeout) => continue,
                    Err(_) => return FirstFrame::Gone,
                }
            }
            Err(_) => return FirstFrame::Gone,
        }
    }
}

/// Hands the peeked first family frame to the serve loop pre-read: the
/// first `recv_json` returns it, everything after delegates to the QUIC
/// wire (whose Drop still FINs + drains gracefully).
struct PrefacedQuicWire {
    first: Option<Value>,
    inner: QuicVerifyWire,
}

#[async_trait]
impl VerifyDraftsWire for PrefacedQuicWire {
    async fn send_json(&mut self, value: &Value, deadline: Duration) -> Result<(), WireError> {
        self.inner.send_json(value, deadline).await
    }

    async fn recv_json(&mut self, deadline: Duration) -> Result<Value, WireError> {
        match self.first.take() {
            Some(value) => Ok(value),
            None => self.inner.recv_json(deadline).await,
        }
    }

    fn remote_peer_id(&self) -> &str {
        self.inner.remote_peer_id()
    }
}

/// The composed serve loop: ONE listener, BOTH vocabularies, ONE
/// admission gate, ONE lease discipline. Lifecycle contract identical to
/// [`crate::serving::serve_sessions`]: stop when `shutdown` flips true OR
/// its watch sender drops. Per accepted session (in its own task, holding
/// one `Admission` slot for its whole lifetime regardless of vocabulary):
///
/// 1. admit (total + per-peer caps — refusal is the msp-v1 `overloaded`
///    `StreamError` and a telemetry label; a family client cannot parse
///    that frame and honestly sees the session close);
/// 2. classify by first frame (see [`demux_first_frame`]);
/// 3. msp-v1 → [`serve_one_seeded`] with the peeked frame (per-request
///    lease checks unchanged inside); family → lease seam (fail-closed),
///    engine from [`FamilyEngineSource::engine_for`] (per-session,
///    prefilled), disclosure overridden with the engine's real
///    capabilities, then [`serve_verifydrafts_session`].
pub async fn serve_sessions_composed(
    mut listener: Libp2pListener,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
    lease_policy: LeasePolicy,
    family: FamilyServing,
    telemetry: Arc<Telemetry>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let admission = Admission::defaults();
    loop {
        let session = tokio::select! {
            // The same semantics as the msp-v1 bridge: a CLOSED watch channel
            // (every Sender dropped) must ALSO stop serving — nobody can
            // ever signal stop otherwise, and a permanently-ready branch
            // would cancel every in-flight accept.
            changed = shutdown.changed() => match changed {
                Err(_) => {
                    telemetry.warn(
                        "serving.stopped",
                        &[("reason", "shutdown_sender_dropped")],
                    );
                    return;
                }
                Ok(()) => {
                    if *shutdown.borrow() {
                        telemetry.info("serving.stopped", &[("reason", "shutdown_signal")]);
                        return;
                    }
                    continue;
                }
            },
            accepted = listener.accept(Duration::from_secs(1)) => match accepted {
                Ok(s) => s,
                Err(_) => continue, // timeout/accept miss: keep serving
            },
        };
        let peer_id = session.remote_peer_id().to_string();
        let executor = Arc::clone(&executor);
        let profile = profile_id.clone();
        let policy = lease_policy.clone();
        let session_admission = admission.clone();
        let session_telemetry = Arc::clone(&telemetry);
        let family_config = family.config.clone();
        let family_engine = Arc::clone(&family.engine);
        let family_lease = Arc::clone(&family.lease);
        tokio::spawn(async move {
            let mut session = session;
            let _session_slot = match session_admission.admit_session(&peer_id) {
                Ok(guard) => guard,
                Err(reason) => {
                    session_telemetry
                        .warn("serving.refused", &[("peer", &peer_id), ("reason", reason)]);
                    send_error(&mut session, "overloaded", reason).await;
                    return;
                }
            };
            match demux_first_frame(&mut session, FIRST_FRAME_IDLE).await {
                FirstFrame::MspV1(frame) => {
                    if let Err(code) = serve_one_seeded(
                        session,
                        executor,
                        profile,
                        &policy,
                        &session_admission,
                        &session_telemetry,
                        Some(frame),
                    )
                    .await
                    {
                        // `code` is a stable reason code by construction.
                        session_telemetry.warn("serving.session_ended", &[("code", code.as_str())]);
                    }
                }
                FirstFrame::Family(offer) => {
                    // ADR-026 at family session open: the coordinator's
                    // lease derives from the QUIC-authenticated PeerId —
                    // fail-closed, no frozen family reject code exists for
                    // "no lease", so the honest answer is the session end
                    // (the client's fallback-to-single signal).
                    let Some(lease_expires_at_ms) = family_lease.lease_expires_at_ms(&peer_id)
                    else {
                        session_telemetry.warn(
                            "verifydrafts.family_refused",
                            &[("peer", &peer_id), ("reason", "no_lease")],
                        );
                        return;
                    };
                    let engine = match family_engine.engine_for().await {
                        Ok(engine) => engine,
                        Err(_) => {
                            // Local engine-load failure: stable label, no
                            // engine detail crosses the wire.
                            session_telemetry.warn(
                                "verifydrafts.family_refused",
                                &[("peer", &peer_id), ("reason", "engine_unavailable")],
                            );
                            return;
                        }
                    };
                    let mut config = family_config;
                    config.lease_expires_at_ms = lease_expires_at_ms;
                    // The honest disclosure from the engine's REAL
                    // geometry (RC-8) — never the hand-set field.
                    config.engine = engine.capabilities();
                    let wire = PrefacedQuicWire {
                        first: Some(offer),
                        inner: QuicVerifyWire::over_session(session),
                    };
                    // The report's end reason is telemetered inside the
                    // serve loop (`verifydrafts.session_ended`).
                    let _report =
                        serve_verifydrafts_session(wire, config, engine, &session_telemetry).await;
                }
                FirstFrame::Gone => {
                    // Idle/vanished before any frame: nothing honest to
                    // send in either vocabulary.
                    session_telemetry
                        .info("serving.session_ended", &[("code", "first_frame_timeout")]);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fail-closed default refuses every family session (no lease, no
    /// serving — the ADR-026 discipline the msp-v1 path enforces per
    /// request).
    #[test]
    fn fail_closed_lease_source_refuses_everything() {
        let source = RefuseFamilyLeases;
        assert_eq!(source.lease_expires_at_ms("12D3Kooanything"), None);
    }

    /// The static source supplies its pinned expiry (the test/roster
    /// venue).
    #[test]
    fn static_lease_source_supplies_expiry() {
        let source = StaticLeaseExpiry(u64::MAX);
        assert_eq!(source.lease_expires_at_ms("peer"), Some(u64::MAX));
    }
}
