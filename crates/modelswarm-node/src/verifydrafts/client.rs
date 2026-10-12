//! The VerifyDrafts requester client (ADR-032; feature `verifydrafts`) —
//! the coordinator side, library-level (tests + future desktop use; NOT
//! wired into the desktop UI in this tier).
//!
//! The client opens a session (offer → accept/reject), then drives rounds
//! against [`modelswarm_session::verifydrafts::VerifyDraftsCoordinator`]:
//! builds and signs `VerifyDrafts` (the coordinator's own RC-7 `seq`
//! counter, fresh single-use nonce, chained parent hash), tracks the
//! outstanding request, admits exactly one result per request (RC-3,
//! including the nonce echo and the transport-authenticated remote PeerId),
//! feeds the per-verifier timing EWMA with the REQUESTER-measured round
//! wall time (RC-2 — `verify_pass_ms` is only the lower-bound sanity term),
//! and sends the reproducing `PrefixCommit` (RC-4/§8 WAIT_COMMIT →
//! COMMITTED).
//!
//! Fallback honesty (ADR-032 §3): a rejected offer or an ineligible accept
//! surfaces as [`ClientOpenError`] — the caller falls back to single
//! (`SingleDecode`), never silently.

use std::time::{Duration, Instant};

use modelswarm_identity::{new_nonce, InstallationIdentity};
use modelswarm_runtime::SamplingParams;
use modelswarm_session::negotiation::{
    Backend, FrameRejectReason, Preset, SessionAccept, SessionBudget, SessionOffer, SessionReject,
    COOPERATIVE_PROTOCOL_VERSION,
};
use modelswarm_session::verifydrafts::{
    batch_generation_params_hash, eligible_batch_verifier, sign_verify_drafts,
    validate_session_accept, CancelRoundFrame, CrossCheckOutcome, RejectionFrame, ResultAdmission,
    ResultDrop, ResultViolation, SessionAcceptError, VerifierTimings, VerifyDrafts,
    VerifyDraftsCoordinator, VerifyDraftsCoordinatorConfig, VerifyDraftsResult, VerifyTelemetry,
    WINDOW_CAP,
};
use modelswarm_session::CommitPrefix;
use modelswarm_telemetry::Telemetry;
use modelswarm_transport::ed25519_dalek::VerifyingKey;

use super::wire::{VerifyDraftsWire, WireError};
use super::{NodeVerifyTelemetry, FAMILY_IO_DEADLINE};

/// The frozen `CancelRound` reason vocabulary as a wire label (mirrors
/// [`modelswarm_session::negotiation::CancelRoundReason`] for callers that
/// do not depend on the session crate directly).
pub type CancelRoundLabel = modelswarm_session::negotiation::CancelRoundReason;

/// Why [`VerifyDraftsClient::negotiate`] could not reach a usable
/// batch-verify session.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientOpenError {
    /// The peer refused honestly (`backend_mismatch`, `preset_unavailable`,
    /// `batch_verify_unsupported`, `budget_infeasible`) — the mandatory
    /// fallback-to-single signal.
    Rejected(&'static str),
    /// The accept validated but the peer cannot batch-verify under this
    /// session's pairing (`batch_verify: false` or a backend mismatch) —
    /// also a fallback-to-single signal, carrying the accept for
    /// diagnostics.
    Ineligible { accept: Box<SessionAccept> },
    /// The accept failed the requester-side honesty check (echo mismatch,
    /// invalid disclosure, inflated budget).
    InvalidAccept(String),
    /// The first frame was neither an accept nor a reject, or the offer
    /// round-trip failed.
    Protocol(&'static str),
    /// Local misconfiguration (non-integral sampling params, malformed
    /// profile id).
    Misconfigured(String),
    /// Transport failure (label only).
    Io(&'static str),
}

impl std::fmt::Display for ClientOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientOpenError::Rejected(reason) => write!(f, "peer rejected the offer: {reason}"),
            ClientOpenError::Ineligible { .. } => write!(
                f,
                "peer cannot batch-verify under this session's pairing (fallback to single)"
            ),
            ClientOpenError::InvalidAccept(why) => write!(f, "invalid session accept: {why}"),
            ClientOpenError::Protocol(why) => write!(f, "protocol violation: {why}"),
            ClientOpenError::Misconfigured(why) => write!(f, "misconfigured: {why}"),
            ClientOpenError::Io(what) => write!(f, "transport failure: {what}"),
        }
    }
}

impl std::error::Error for ClientOpenError {}

/// Why one round failed (labels/counts only — no token ids).
#[derive(Debug, Clone, PartialEq)]
pub enum RoundFailure {
    /// The verifier answered with a frame-level reject.
    Rejected {
        /// The effective §6.5 registry code.
        code: FrameRejectReason,
        /// The verifier's current committed prefix hash (a hash echo is
        /// permitted in rejections).
        prefix_hash: Option<String>,
    },
    /// The verifier abandoned the round (`parent_mismatch` /
    /// `round_mismatch`); the client chains the next round on its own
    /// committed hash.
    Cancelled {
        /// The abandoned round.
        round: u64,
        /// The frozen cooperative-namespace reason.
        reason: CancelRoundLabel,
    },
    /// The coordinator admitted a protocol violation (first; the session
    /// is still up).
    Violation(ResultViolation),
    /// The second violation by the same peer tore the session down.
    TornDown(ResultViolation),
    /// The result was dropped with no state change (replay / no
    /// outstanding request / torn down).
    Dropped(ResultDrop),
    /// The result wait elapsed; the request was abandoned and its nonce
    /// consumed (a late genuine result is a harmless drop).
    TimedOut,
    /// The window exceeds the verifier's disclosed `batch_window_max`
    /// (ADR-032 §3: the coordinator MUST NOT send longer windows).
    WindowTooLarge {
        /// The offending length.
        requested: usize,
        /// The disclosed cap.
        cap: u32,
    },
    /// The coordinator is torn down; no more rounds may be sent.
    SessionTornDown,
    /// The result frame did not strictly parse after admission said
    /// otherwise (defensive; treated as a protocol violation upstream).
    UnparsableResult,
    /// Transport failure while driving the round (label only).
    Io(&'static str),
}

/// One completed round (counts and hashes only — the committed ids are
/// returned because the coordinator DRAFTED them; they are its own data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundOutcome {
    /// The round number.
    pub round: u64,
    /// Accepted prefix length out of the window.
    pub accepted_prefix_len: u32,
    /// Divergence point (None on full acceptance).
    pub divergence_point: Option<u32>,
    /// The verifier's correction/bonus token.
    pub correction: Option<u32>,
    /// The committed ids for this round (`window[..accepted] ++ [correction]`).
    pub committed: Vec<u32>,
    /// The new committed prefix hash.
    pub new_prefix_hash: String,
    /// The verifier-claimed engine-pass wall time (ms).
    pub verify_pass_ms: u32,
    /// The REQUESTER-measured round wall time (ms) — the EWMA input (RC-2).
    pub observed_wall_ms: u64,
    /// The RC-2 cross-check verdict on `verify_pass_ms`.
    pub cross_check: CrossCheckOutcome,
}

/// Requester-side configuration.
#[derive(Clone)]
pub struct VerifyDraftsClientConfig {
    /// The exact profile the swarm hosts (`msp1:<64 hex>`).
    pub profile_id: String,
    /// The negotiated contract tier.
    pub preset: Preset,
    /// The offered budget envelope (the serving side clamps DOWN only).
    pub budget: SessionBudget,
    /// The requester's own backend (E0 pairing: must equal the verifier's).
    pub verifier_backend: Backend,
    /// The verifier's pinned public key (RC-3 (a): every result verifies
    /// under exactly this key).
    pub verifier_key: VerifyingKey,
    /// The pinned sampling parameters — MUST equal the verifier's pinned
    /// parameters or every round rejects `bad_frame` (the family binds
    /// them through `generation_params_hash`; see the handoff's
    /// negotiation-gap note).
    pub sampling_params: SamplingParams,
    /// Per-round `deadline_ms` (clamped to `[1000, 120000]` by the gate).
    pub round_deadline_ms: u32,
    /// How long to wait for a result after sending a request.
    pub result_wait: Duration,
    /// Per-frame I/O deadline.
    pub io_deadline: Duration,
}

impl VerifyDraftsClientConfig {
    /// Defaults for loopback/tests: CPU pairing, `verified` preset, a
    /// 120-second budget envelope, 2.5 s round deadlines, 10 s result wait.
    pub fn new(profile_id: String, verifier_key: VerifyingKey) -> Self {
        Self {
            profile_id,
            preset: Preset::Verified,
            budget: SessionBudget {
                max_endpoints: 1,
                max_endpoint_seconds: 120,
                max_output_tokens: 4096,
            },
            verifier_backend: Backend::Cpu,
            verifier_key,
            sampling_params: SamplingParams::default(),
            round_deadline_ms: 2_500,
            result_wait: Duration::from_secs(10),
            io_deadline: FAMILY_IO_DEADLINE,
        }
    }
}

/// The requester-side coordinator for one VerifyDrafts session.
pub struct VerifyDraftsClient<W: VerifyDraftsWire> {
    wire: W,
    coordinator: VerifyDraftsCoordinator,
    timings: VerifierTimings,
    identity: InstallationIdentity,
    config: VerifyDraftsClientConfig,
    accept: SessionAccept,
    session_id: String,
    params_hash: String,
    next_seq: u64,
    next_round: u64,
    committed_stream: Vec<u32>,
}

impl<W: VerifyDraftsWire> VerifyDraftsClient<W> {
    /// Sends a `SessionOffer` and waits for the accept/reject. On success
    /// the returned client is ready to drive rounds; every error variant is
    /// an honest fallback-to-single signal (ADR-032 §3).
    pub async fn negotiate(
        mut wire: W,
        identity: InstallationIdentity,
        config: VerifyDraftsClientConfig,
        telemetry: &Telemetry,
    ) -> Result<Self, ClientOpenError> {
        let params_hash = batch_generation_params_hash(&config.sampling_params)
            .map_err(|why| ClientOpenError::Misconfigured(why.to_string()))?;
        let session_id = format!("vd-{}", new_nonce());
        let offer = SessionOffer {
            session_id: session_id.clone(),
            protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
            preset: config.preset,
            budget: config.budget.clone(),
            verifier_backend: config.verifier_backend,
        };
        let offer_value = serde_json::to_value(&offer).expect("offer serializes");
        wire.send_json(&offer_value, config.io_deadline)
            .await
            .map_err(|_| ClientOpenError::Io("send offer"))?;
        let reply = wire
            .recv_json(config.io_deadline)
            .await
            .map_err(|_| ClientOpenError::Io("recv accept"))?;
        let accept: SessionAccept = if reply.get("engine").is_some() {
            serde_json::from_value(reply).map_err(|_| ClientOpenError::Protocol("accept schema"))?
        } else if reply.get("reason").is_some() {
            let reject: SessionReject = serde_json::from_value(reply)
                .map_err(|_| ClientOpenError::Protocol("reject schema"))?;
            return Err(ClientOpenError::Rejected(reject.reason.as_str()));
        } else {
            return Err(ClientOpenError::Protocol(
                "first reply was neither accept nor reject",
            ));
        };
        if let Err(why) = validate_session_accept(&offer, &accept) {
            let label = match why {
                SessionAcceptError::EchoMismatch(field) => format!("{field} echo mismatch"),
                SessionAcceptError::InvalidDisclosure => "invalid engine disclosure".to_string(),
                SessionAcceptError::BudgetInflated(field) => {
                    format!("budget inflated: {field}")
                }
            };
            return Err(ClientOpenError::InvalidAccept(label));
        }
        if !eligible_batch_verifier(&offer, &accept) {
            telemetry.info(
                "verifydrafts.ineligible_verifier",
                &[("session", session_id.as_str())],
            );
            return Err(ClientOpenError::Ineligible {
                accept: Box::new(accept),
            });
        }
        let coordinator = VerifyDraftsCoordinator::new(VerifyDraftsCoordinatorConfig {
            session_id: session_id.clone(),
            profile_id: config.profile_id.clone(),
            generation_params_hash: params_hash.clone(),
            verifier_key: config.verifier_key,
        })
        .map_err(|why| ClientOpenError::Misconfigured(why.to_string()))?;
        Ok(Self {
            wire,
            coordinator,
            timings: VerifierTimings::new(),
            identity,
            config,
            accept,
            session_id,
            params_hash,
            next_seq: 1,
            next_round: 1,
            committed_stream: Vec::new(),
        })
    }

    /// Drives ONE round: send `VerifyDrafts`, await `VerifyDraftsResult`,
    /// admit it (RC-3), feed the timing EWMA (RC-2), send the reproducing
    /// `PrefixCommit`. The window length must not exceed the verifier's
    /// disclosed `batch_window_max`.
    #[allow(clippy::too_many_lines)]
    pub async fn round(
        &mut self,
        window_tokens: &[u32],
        telemetry: &Telemetry,
    ) -> Result<RoundOutcome, RoundFailure> {
        let cap = WINDOW_CAP.min(self.accept.engine.batch_window_max) as usize;
        if window_tokens.is_empty() || window_tokens.len() > cap {
            return Err(RoundFailure::WindowTooLarge {
                requested: window_tokens.len(),
                cap: self.accept.engine.batch_window_max,
            });
        }
        if self.coordinator.is_torn_down() {
            return Err(RoundFailure::SessionTornDown);
        }
        let round = self.next_round;
        let request = VerifyDrafts {
            session_id: self.session_id.clone(),
            protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
            seq: self.next_seq,
            round,
            profile_id: self.config.profile_id.clone(),
            parent_prefix_hash: self.coordinator.committed_prefix_hash().to_string(),
            generation_params_hash: self.params_hash.clone(),
            window_tokens: window_tokens.to_vec(),
            proposer_id: self.identity.peer_id(),
            nonce: new_nonce(),
            deadline_ms: self.config.round_deadline_ms,
            sender: self.identity.installation_id(),
            signature: String::new(),
        };
        self.next_seq += 1;
        let mut signed = request.clone();
        signed.signature = sign_verify_drafts(&request, &self.identity)
            .map_err(|_| RoundFailure::Io("sign request"))?;
        self.coordinator
            .track_request(&signed)
            .map_err(|_| RoundFailure::SessionTornDown)?;
        let request_value = serde_json::to_value(&signed).expect("request serializes");
        let started = Instant::now();
        self.wire
            .send_json(&request_value, self.config.io_deadline)
            .await
            .map_err(|_| RoundFailure::Io("send request"))?;

        let reply = match self.wire.recv_json(self.config.result_wait).await {
            Ok(value) => value,
            Err(WireError::Timeout) => {
                // Deadline passed with no result: abandon (the nonce is
                // consumed; a late genuine result is a harmless drop).
                self.coordinator.abandon_outstanding_request();
                return Err(RoundFailure::TimedOut);
            }
            Err(_) => return Err(RoundFailure::Io("recv result")),
        };
        let observed_wall_ms = started.elapsed().as_millis() as u64;
        let remote_peer_id = self.wire.remote_peer_id().to_string();

        // Demux the reply: result, frame-level reject, or round cancel.
        if reply.get("nonce_echo").is_some() {
            let parsed = VerifyDraftsResult::parse(&reply).ok();
            match self.coordinator.admit_result(&reply, &remote_peer_id) {
                ResultAdmission::Accepted { committed } => {
                    self.next_round = round + 1;
                    self.committed_stream.extend_from_slice(&committed);
                    // RC-2: the EWMA feeds ONLY on the requester-measured
                    // wall time; verify_pass_ms is the lower-bound sanity
                    // term (cross-check → verify_ms_lie telemetry).
                    self.timings.record_round(&remote_peer_id, observed_wall_ms);
                    let verify_pass_ms = parsed.as_ref().map_or(0, |r| r.verify_pass_ms);
                    let cross_check =
                        self.timings
                            .cross_check(&remote_peer_id, verify_pass_ms, observed_wall_ms);
                    if cross_check == CrossCheckOutcome::Lie {
                        NodeVerifyTelemetry(telemetry)
                            .verify_ms_lie(&self.session_id, &remote_peer_id);
                    }
                    let parsed = parsed.ok_or(RoundFailure::UnparsableResult)?;
                    // §8 WAIT_COMMIT → COMMITTED: the reproducing commit.
                    // (RC-4: the ids and hash equal the admitted result by
                    // construction — the coordinator recomputed both.)
                    let commit = CommitPrefix {
                        session_id: self.session_id.clone(),
                        round,
                        previous_prefix_hash: request.parent_prefix_hash.clone(),
                        accepted_token_ids: committed.clone(),
                        new_prefix_hash: parsed.new_prefix_hash.clone(),
                        model_profile_id: self.config.profile_id.clone(),
                        generation_params_hash: self.params_hash.clone(),
                        sender: self.identity.installation_id(),
                    };
                    let commit_value = serde_json::to_value(super::WirePrefixCommit::from(&commit))
                        .expect("commit serializes");
                    self.wire
                        .send_json(&commit_value, self.config.io_deadline)
                        .await
                        .map_err(|_| RoundFailure::Io("send commit"))?;
                    Ok(RoundOutcome {
                        round,
                        accepted_prefix_len: parsed.accepted_prefix_len,
                        divergence_point: parsed.divergence_point,
                        correction: parsed.correction,
                        committed,
                        new_prefix_hash: parsed.new_prefix_hash,
                        verify_pass_ms: parsed.verify_pass_ms,
                        observed_wall_ms,
                        cross_check,
                    })
                }
                ResultAdmission::Violation(why) => Err(RoundFailure::Violation(why)),
                ResultAdmission::TornDown(why) => Err(RoundFailure::TornDown(why)),
                ResultAdmission::Dropped(why) => Err(RoundFailure::Dropped(why)),
            }
        } else if reply.get("code").is_some() {
            let rejection: RejectionFrame =
                serde_json::from_value(reply).map_err(|_| RoundFailure::Io("reject schema"))?;
            self.coordinator.abandon_outstanding_request();
            Err(RoundFailure::Rejected {
                code: rejection.code,
                prefix_hash: rejection.prefix_hash,
            })
        } else if reply.get("reason").is_some() {
            let cancel: CancelRoundFrame =
                serde_json::from_value(reply).map_err(|_| RoundFailure::Io("cancel schema"))?;
            self.coordinator.abandon_outstanding_request();
            self.next_round = round + 1;
            Err(RoundFailure::Cancelled {
                round: cancel.round,
                reason: cancel.reason,
            })
        } else {
            self.coordinator.abandon_outstanding_request();
            Err(RoundFailure::Io("unexpected reply frame"))
        }
    }

    /// The negotiated session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The accepted engine disclosure.
    pub fn engine_disclosure(&self) -> &modelswarm_session::negotiation::EngineDisclosure {
        &self.accept.engine
    }

    /// The effective budget (clamped down by the serving side).
    pub fn effective_budget(&self) -> &SessionBudget {
        &self.accept.budget
    }

    /// The committed token stream so far (the coordinator's own drafts —
    /// this is local data, never logged by this module).
    pub fn committed_stream(&self) -> &[u32] {
        &self.committed_stream
    }

    /// The committed prefix hash the next round chains onto.
    pub fn committed_prefix_hash(&self) -> &str {
        self.coordinator.committed_prefix_hash()
    }

    /// The per-verifier timing table (RC-2 EWMA + lie counts).
    pub fn timings(&self) -> &VerifierTimings {
        &self.timings
    }

    /// Whether the coordinator tore the session down.
    pub fn is_torn_down(&self) -> bool {
        self.coordinator.is_torn_down()
    }

    /// Releases the underlying wire (closing the session) and returns it.
    pub fn into_wire(self) -> W {
        self.wire
    }
}
