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
//!   Since the RC-L1 discharge (ADR-032 §7 disposition item i,
//!   2026-10-12) the seam is PROFILE-bound: [`RosterFamilyLeases`]
//!   validates the coordinator's lease against the SERVED profile with
//!   every binding the msp-v1 per-request gate enforces
//!   ([`FamilyLeaseRefusal`] mirrors its classification).
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use modelswarm_eligibility::EligibilityLease;
use modelswarm_gateway::InferenceExecutor;
use modelswarm_telemetry::Telemetry;
use modelswarm_transport::libp2p_backend::{Libp2pListener, Libp2pSession};
use modelswarm_transport::message::WireMessage;
use modelswarm_transport::TransportError;

use crate::serving::{rfc3339_now, send_error, serve_one_seeded, Admission, LeasePolicy};

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

/// Why a family session was refused at the composed daemon's ADR-026
/// lease gate. One variant per binding the msp-v1 per-request check
/// enforces (`crate::serving::LeasePolicy::check`, RC-L1): a lease that
/// fails ANY binding refuses the family session, classified exactly like
/// that gate classifies it. Labels only — lease contents never reach
/// telemetry (privacy rule 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyLeaseRefusal {
    /// No lease is on file for the authenticated coordinator (the msp-v1
    /// gate's "no lease presented").
    NoLease,
    /// The lease failed decoding or hub verification — signature,
    /// profile-id form, expiry/freshness beyond the ±120 s skew, or the
    /// lease cap (that gate's "bad lease" / "lease invalid" family).
    InvalidLease,
    /// The lease names another profile than the one this daemon serves
    /// ("lease is for another profile").
    ProfileMismatch,
    /// `can_consume` is false or `slots` is zero ("lease grants no
    /// consume right").
    NoConsumeRight,
    /// The lease is bound to a different peer than the transport
    /// authenticated ("lease is not bound to this peer").
    PeerMismatch,
}

impl FamilyLeaseRefusal {
    /// Stable telemetry label for the refusal (ids/codes only).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoLease => "no_lease",
            Self::InvalidLease => "invalid_lease",
            Self::ProfileMismatch => "profile_mismatch",
            Self::NoConsumeRight => "no_consume_right",
            Self::PeerMismatch => "peer_mismatch",
        }
    }
}

impl std::fmt::Display for FamilyLeaseRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The msp-v1 per-request gate's log reason for the same binding
        // failure, verbatim (no lease contents — ever).
        match self {
            Self::NoLease => write!(f, "no lease presented"),
            Self::InvalidLease => write!(f, "lease invalid"),
            Self::ProfileMismatch => write!(f, "lease is for another profile"),
            Self::NoConsumeRight => write!(f, "lease grants no consume right"),
            Self::PeerMismatch => write!(f, "lease is not bound to this peer"),
        }
    }
}

/// ADR-026 lease derivation for family sessions AT SESSION OPEN — the
/// composed daemon's seam. Given the QUIC-authenticated coordinator
/// PeerId, returns the lease expiry (unix ms) the session serves under —
/// validated against the SERVED profile with every binding the msp-v1
/// per-request gate enforces (RC-L1) — or the refusal; a refusal means NO
/// family session (the coordinator's client surfaces a transport failure
/// and falls back to single, never silently).
///
/// The family's offer wire carries no lease field (wiring handoff
/// G3/§5-A2); the production source is the daemon's eligibility/roster
/// state for the authenticated peer, wired in here
/// ([`RosterFamilyLeases`]).
pub trait FamilyLeaseSource: Send + Sync {
    /// The validated lease expiry (unix ms) for `coordinator_peer_id` (the
    /// transport-authenticated remote PeerId) serving `served_profile`
    /// (this daemon's pinned profile), or the refusal.
    fn lease_for(
        &self,
        coordinator_peer_id: &str,
        served_profile: &str,
    ) -> Result<u64, FamilyLeaseRefusal>;
}

/// Fail-closed default: refuses every family session (the honest default
/// until the daemon wires a real lease source — serving family sessions
/// without a lease derivation would break the ADR-026 discipline the
/// msp-v1 path enforces per request).
pub struct RefuseFamilyLeases;

impl FamilyLeaseSource for RefuseFamilyLeases {
    fn lease_for(
        &self,
        _coordinator_peer_id: &str,
        _served_profile: &str,
    ) -> Result<u64, FamilyLeaseRefusal> {
        Err(FamilyLeaseRefusal::NoLease)
    }
}

/// Fixed-expiry source (the TEST/stub seam ONLY — it skips every lease
/// binding by design and must never back a production daemon; the
/// production source is [`RosterFamilyLeases`]).
pub struct StaticLeaseExpiry(pub u64);

impl FamilyLeaseSource for StaticLeaseExpiry {
    fn lease_for(
        &self,
        _coordinator_peer_id: &str,
        _served_profile: &str,
    ) -> Result<u64, FamilyLeaseRefusal> {
        Ok(self.0)
    }
}

/// The production [`FamilyLeaseSource`] (RC-L1, ADR-032 §7 disposition
/// item i): the daemon's eligibility/roster state — the hub-signed lease
/// tokens the daemon holds per coordinator peer — validated AT SESSION
/// OPEN with the SAME discipline `crate::serving::LeasePolicy::check`
/// applies per msp-v1 request: hub signature + freshness, binding to the
/// daemon's pinned profile, `can_consume` with `slots >= 1`, and binding
/// to the transport-authenticated PeerId. The family wire carries no
/// lease field (ADR-032 §3; the offer-form-v3 oracle ruling), so the
/// ROSTER — not the peer — is where the token comes from; a peer with no
/// roster entry is refused fail-closed exactly like the empty-roster
/// default.
///
/// The roster trusts its FEEDER (the tracker/roster wiring that calls
/// [`Self::upsert`]); every lookup re-validates the token fully, so a
/// stale, misfiled, or revoked-epoch entry refuses exactly like a missing
/// one — the same "well-formed but policy-violating leases must fail at
/// every enforcing peer" stance `EligibilityLease::issue` documents.
pub struct RosterFamilyLeases {
    policy: LeasePolicy,
    roster: Mutex<HashMap<String, String>>,
}

impl RosterFamilyLeases {
    /// An empty roster under `policy` — refuses every family session until
    /// [`Self::upsert`] seeds a lease (the fail-closed production
    /// default).
    pub fn new(policy: LeasePolicy) -> Self {
        Self {
            policy,
            roster: Mutex::new(HashMap::new()),
        }
    }

    /// Records or refreshes the wire lease token for a coordinator peer
    /// (`base64url(json).base64url(sig)` — the tracker's issuance form).
    pub fn upsert(&self, coordinator_peer_id: &str, wire_lease: &str) {
        self.roster
            .lock()
            .expect("family lease roster lock")
            .insert(coordinator_peer_id.to_string(), wire_lease.to_string());
    }

    /// Drops a peer's entry (revocation / audit-sweep wiring).
    pub fn remove(&self, coordinator_peer_id: &str) {
        self.roster
            .lock()
            .expect("family lease roster lock")
            .remove(coordinator_peer_id);
    }

    /// Number of entries on file (diagnostics: the honest "verifying
    /// nobody yet" label for an empty roster).
    pub fn len(&self) -> usize {
        self.roster.lock().expect("family lease roster lock").len()
    }

    /// `true` when no lease is on file at all (diagnostics).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl FamilyLeaseSource for RosterFamilyLeases {
    fn lease_for(
        &self,
        coordinator_peer_id: &str,
        served_profile: &str,
    ) -> Result<u64, FamilyLeaseRefusal> {
        let token = self
            .roster
            .lock()
            .expect("family lease roster lock")
            .get(coordinator_peer_id)
            .cloned()
            .ok_or(FamilyLeaseRefusal::NoLease)?;
        validate_family_lease(&self.policy, &token, coordinator_peer_id, served_profile)
    }
}

/// The RC-L1 gate: one place, mirroring `crate::serving::LeasePolicy::check`
/// binding for binding — decode + hub verification (signature,
/// `msp1:` profile form, expiry within the ±120 s skew, lease cap),
/// profile binding, consume right + slots, peer binding — and returning
/// the token's operative `expires_at` as unix ms (the session's stage-5
/// lease bound; ADR-026 caps the token at the underlying lease + 60 s
/// grace, so `expires_at` is the latest an honest session may serve).
fn validate_family_lease(
    policy: &LeasePolicy,
    wire_token: &str,
    authenticated_peer_id: &str,
    served_profile: &str,
) -> Result<u64, FamilyLeaseRefusal> {
    let lease =
        EligibilityLease::from_wire(wire_token).map_err(|_| FamilyLeaseRefusal::InvalidLease)?;
    lease
        .verify(policy.hub_key(), &rfc3339_now())
        .map_err(|_| FamilyLeaseRefusal::InvalidLease)?;
    if lease.model_profile_id != served_profile {
        return Err(FamilyLeaseRefusal::ProfileMismatch);
    }
    if !lease.can_consume || lease.slots == 0 {
        return Err(FamilyLeaseRefusal::NoConsumeRight);
    }
    if lease.peer_id != authenticated_peer_id {
        return Err(FamilyLeaseRefusal::PeerMismatch);
    }
    // Freshness past the open check is the gate's hard stage-5 bound: the
    // session serves until the token's own operative expiry, no longer.
    let expires_at = OffsetDateTime::parse(&lease.expires_at, &Rfc3339)
        .map_err(|_| FamilyLeaseRefusal::InvalidLease)?;
    let seconds =
        u64::try_from(expires_at.unix_timestamp()).map_err(|_| FamilyLeaseRefusal::InvalidLease)?;
    Ok(seconds * 1000 + u64::from(expires_at.millisecond()))
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
///    lease checks unchanged inside); family → the profile-bound lease
///    seam ([`FamilyLeaseSource::lease_for`] against the served profile,
///    fail-closed with the [`FamilyLeaseRefusal`] classification), engine
///    from [`FamilyEngineSource::engine_for`] (per-session, prefilled),
///    disclosure overridden with the engine's real capabilities, then
///    [`serve_verifydrafts_session`].
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
                    // ADR-026 at family session open (RC-L1): the
                    // coordinator's lease derives from the
                    // QUIC-authenticated PeerId and the daemon's lease
                    // source, validated against the SERVED profile with
                    // every binding the msp-v1 per-request gate enforces —
                    // fail-closed; no frozen family reject code exists for
                    // a refused lease, so the honest answer is the session
                    // end (the client's fallback-to-single signal).
                    let lease_expires_at_ms = match family_lease.lease_for(&peer_id, &profile) {
                        Ok(expires_at_ms) => expires_at_ms,
                        Err(refusal) => {
                            session_telemetry.warn(
                                "verifydrafts.family_refused",
                                &[("peer", &peer_id), ("reason", refusal.as_str())],
                            );
                            return;
                        }
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
    use ed25519_dalek::SigningKey;
    use modelswarm_eligibility::{CapacityClass, EligibilityLease};

    /// A profile id that passes the msp1 form checks (serving.rs's test
    /// pattern).
    const TEST_PROFILE: &str =
        "msp1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    /// A DIFFERENT well-formed profile (the wrong-profile binding arm).
    const OTHER_PROFILE: &str =
        "msp1:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn rfc3339(plus_secs: i64) -> String {
        OffsetDateTime::now_utc()
            .saturating_add(time::Duration::seconds(plus_secs))
            .format(&Rfc3339)
            .unwrap()
    }

    /// Issues a consume lease under `key` with overridable bindings and
    /// returns the wire token (the serving.rs lease_helpers pattern,
    /// parameterized for each refusal arm).
    #[allow(clippy::too_many_arguments)]
    fn lease_wire(
        key: &SigningKey,
        profile: &str,
        peer_id: &str,
        can_consume: bool,
        slots: u8,
        expires_in_secs: i64,
    ) -> String {
        EligibilityLease::issue(
            peer_id,
            "test-installation",
            profile,
            rfc3339(-60),
            rfc3339(expires_in_secs),
            rfc3339(expires_in_secs + 60),
            true,
            can_consume,
            slots,
            CapacityClass::GpuMid,
            1,
            "test-challenge",
            "test-nonce",
            key,
        )
        .to_wire()
    }

    fn policy_over(key: &SigningKey) -> LeasePolicy {
        LeasePolicy::from_hex(&hex::encode(key.verifying_key().to_bytes())).unwrap()
    }

    /// The fail-closed default refuses every family session (no lease, no
    /// serving — the ADR-026 discipline the msp-v1 path enforces per
    /// request).
    #[test]
    fn fail_closed_lease_source_refuses_everything() {
        let source = RefuseFamilyLeases;
        assert_eq!(
            source.lease_for("12D3Kooanything", TEST_PROFILE),
            Err(FamilyLeaseRefusal::NoLease)
        );
    }

    /// The static source supplies its pinned expiry (the TEST/stub seam —
    /// binding-free by design; production uses RosterFamilyLeases).
    #[test]
    fn static_lease_source_supplies_expiry() {
        let source = StaticLeaseExpiry(u64::MAX);
        assert_eq!(source.lease_for("peer", TEST_PROFILE), Ok(u64::MAX));
    }

    /// RC-L1: an empty production roster refuses fail-closed (NoLease) —
    /// the honest default until the daemon's roster wiring seeds a lease.
    #[test]
    fn empty_roster_refuses_fail_closed() {
        let key = SigningKey::generate(&mut rand_core::OsRng);
        let roster = RosterFamilyLeases::new(policy_over(&key));
        assert!(roster.is_empty());
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::NoLease)
        );
    }

    /// RC-L1: a VALID lease on file serves — and the expiry handed to the
    /// session is the token's own operative `expires_at` (unix ms), the
    /// ADR-026-capped bound the gate's stage-5 check enforces.
    #[test]
    fn roster_accepts_valid_lease_and_returns_token_expiry() {
        let key = SigningKey::generate(&mut rand_core::OsRng);
        let token = lease_wire(&key, TEST_PROFILE, "peer-a", true, 2, 300);
        let roster = RosterFamilyLeases::new(policy_over(&key));
        roster.upsert("peer-a", &token);
        assert_eq!(roster.len(), 1);
        let expires_ms = roster
            .lease_for("peer-a", TEST_PROFILE)
            .expect("valid lease serves");
        // within (300 s - 120 s skew, 300 s] of now, in ms
        let now_ms = OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000;
        assert!(expires_ms > now_ms + 120 * 1000, "{expires_ms} vs {now_ms}");
        assert!(
            expires_ms <= now_ms + 300 * 1000 + 1100,
            "{expires_ms} vs {now_ms}"
        );
        // The served profile was part of the validation: the same token
        // under another served profile is a ProfileMismatch, not a serve.
        assert_eq!(
            roster.lease_for("peer-a", OTHER_PROFILE),
            Err(FamilyLeaseRefusal::ProfileMismatch)
        );
        // And the authenticated peer matters: another peer id has no
        // entry at all.
        assert_eq!(
            roster.lease_for("peer-b", TEST_PROFILE),
            Err(FamilyLeaseRefusal::NoLease)
        );
        // Removal (revocation wiring) refuses again.
        roster.remove("peer-a");
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::NoLease)
        );
    }

    /// RC-L1, binding-by-binding (each refuses with the msp-v1 gate's own
    /// classification): wrong profile, expired beyond skew, can_consume
    /// false, slots zero, lease bound to another peer, and a token signed
    /// by a foreign hub.
    #[test]
    fn roster_refuses_each_broken_binding_with_the_msp_v1_classification() {
        let key = SigningKey::generate(&mut rand_core::OsRng);
        let policy = policy_over(&key);

        // Wrong profile: lease for OTHER_PROFILE on file under peer-a.
        let token = lease_wire(&key, OTHER_PROFILE, "peer-a", true, 1, 300);
        let roster = RosterFamilyLeases::new(policy_over(&key));
        roster.upsert("peer-a", &token);
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::ProfileMismatch)
        );

        // Expired beyond the ±120 s skew (issued 10 min ago, expired
        // 5 min ago): fails hub verification.
        let expired = EligibilityLease::issue(
            "peer-a",
            "test-installation",
            TEST_PROFILE,
            rfc3339(-600),
            rfc3339(-300),
            rfc3339(-240),
            true,
            true,
            1,
            CapacityClass::GpuMid,
            1,
            "test-challenge",
            "test-nonce",
            &key,
        )
        .to_wire();
        let roster = RosterFamilyLeases::new(policy_over(&key));
        roster.upsert("peer-a", &expired);
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::InvalidLease)
        );

        // can_consume false.
        let token = lease_wire(&key, TEST_PROFILE, "peer-a", false, 1, 300);
        let roster = RosterFamilyLeases::new(policy_over(&key));
        roster.upsert("peer-a", &token);
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::NoConsumeRight)
        );

        // Slots zero.
        let token = lease_wire(&key, TEST_PROFILE, "peer-a", true, 0, 300);
        let roster = RosterFamilyLeases::new(policy_over(&key));
        roster.upsert("peer-a", &token);
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::NoConsumeRight)
        );

        // Bound to another peer (misfiled/stolen token: filed under
        // peer-a, signed for peer-b — the signature binds peer-b, so the
        // authenticated-peer check refuses).
        let token = lease_wire(&key, TEST_PROFILE, "peer-b", true, 1, 300);
        let roster = RosterFamilyLeases::new(policy_over(&key));
        roster.upsert("peer-a", &token);
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::PeerMismatch)
        );

        // Signed by a FOREIGN hub: signature verification fails.
        let stranger = SigningKey::generate(&mut rand_core::OsRng);
        let token = lease_wire(&stranger, TEST_PROFILE, "peer-a", true, 1, 300);
        let roster = RosterFamilyLeases::new(policy.clone());
        roster.upsert("peer-a", &token);
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::InvalidLease)
        );

        // Undecodable garbage on file: fails closed the same way.
        let roster = RosterFamilyLeases::new(policy);
        roster.upsert("peer-a", "not-a-lease");
        assert_eq!(
            roster.lease_for("peer-a", TEST_PROFILE),
            Err(FamilyLeaseRefusal::InvalidLease)
        );
    }

    /// The refusal labels are stable (telemetry contract) and the Display
    /// mirrors the msp-v1 gate's log reasons.
    #[test]
    fn refusal_labels_and_display_mirror_the_msp_v1_gate() {
        assert_eq!(FamilyLeaseRefusal::NoLease.as_str(), "no_lease");
        assert_eq!(FamilyLeaseRefusal::InvalidLease.as_str(), "invalid_lease");
        assert_eq!(
            FamilyLeaseRefusal::ProfileMismatch.as_str(),
            "profile_mismatch"
        );
        assert_eq!(
            FamilyLeaseRefusal::NoConsumeRight.as_str(),
            "no_consume_right"
        );
        assert_eq!(FamilyLeaseRefusal::PeerMismatch.as_str(), "peer_mismatch");
        assert_eq!(
            FamilyLeaseRefusal::NoLease.to_string(),
            "no lease presented"
        );
        assert_eq!(
            FamilyLeaseRefusal::ProfileMismatch.to_string(),
            "lease is for another profile"
        );
        assert_eq!(
            FamilyLeaseRefusal::NoConsumeRight.to_string(),
            "lease grants no consume right"
        );
        assert_eq!(
            FamilyLeaseRefusal::PeerMismatch.to_string(),
            "lease is not bound to this peer"
        );
    }
}
