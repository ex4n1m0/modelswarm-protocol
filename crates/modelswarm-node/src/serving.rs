//! F0(3) serving bridge: inbound framed sessions → local executor →
//! outbound token frames (Phase F0; see docs/reviews/phase-f-research).
//!
//! The listener hands us authenticated sessions (the QUIC handshake
//! verifies the remote's libp2p PeerId = the ADR-020 derivation). For
//! each session we read one `InferenceRequest`, check the hub-signed
//! lease gate (ADR-026, at session open and per request), clamp
//! requester-supplied limits (msp-v1 §6.4), drive the node's
//! [`InferenceExecutor`] (the same executor the local gateway uses —
//! ADR-025 ChatML templating applies), and stream
//! `TokenDelta`/`Usage`/`Completed` frames back.
//!
//! Privacy: peer-supplied frame payloads are never logged or echoed —
//! diagnostics carry variant names and stable codes only (AGENTS.md
//! rule 5). Admission control (session cap, per-peer cap, request-id
//! dedup) is prototype-grade hostile-peer resistance per
//! docs/threat-model.md §4.2.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use modelswarm_gateway::{
    ExecutorEvent, InferenceExecutor, NormalizedMessage, NormalizedRequest, DEADLINE_MS_MAX,
    DEADLINE_MS_MIN, DEFAULT_MAX_OUTPUT_TOKENS, MAX_PROMPT_BYTES,
};
use modelswarm_telemetry::Telemetry;
use modelswarm_transport::libp2p_backend::{Libp2pListener, Libp2pSession};
use modelswarm_transport::message::{
    ChatMessage, Completed, StreamError, TokenDelta, Usage, WireMessage,
};

const IO_DEADLINE: Duration = Duration::from_secs(30);

/// Serves accepted sessions until `shutdown` flips to true OR its watch
/// sender is dropped (no owner left to signal stop ⇒ stop serving). Callers
/// that want serving to continue MUST hold the sender for the bridge's
/// lifetime. Never panics on a bad session: malformed input becomes a
/// `StreamError` frame, then the session closes.
pub async fn serve_sessions(
    mut listener: Libp2pListener,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
    lease_policy: LeasePolicy,
    telemetry: Arc<Telemetry>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let admission = Admission::defaults();
    loop {
        let session = tokio::select! {
            // watch::changed() fires immediately for the unseen INITIAL
            // value — only an actual true flip stops serving. A CLOSED
            // channel (every Sender dropped) must ALSO stop serving: a
            // closed channel makes changed() permanently Ready, and a
            // permanently-ready branch cancels the in-flight `accept` on
            // (nearly) every poll — dropping the listener's mid-handshake
            // QUIC upgrade, which quinn then application-closes as
            // transport error 0xC while the connection is unconfirmed.
            // The dialer sees exactly "aborted by peer: the application
            // or application protocol caused the connection to be closed
            // during the handshake" (the 2026-10-09 LAN pass-1 bug). With
            // no Sender left, nobody can ever signal stop, so stop now,
            // visibly, instead of silently killing every fresh connection.
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
        tokio::spawn(async move {
            let _session_slot = match session_admission.admit_session(&peer_id) {
                Ok(guard) => guard,
                Err(reason) => {
                    session_telemetry
                        .warn("serving.refused", &[("peer", &peer_id), ("reason", reason)]);
                    let mut session = session;
                    send_error(&mut session, "overloaded", reason).await;
                    return;
                }
            };
            if let Err(code) = serve_one(
                session,
                executor,
                profile,
                &policy,
                &session_admission,
                &session_telemetry,
            )
            .await
            {
                // `code` is a stable reason code by construction — never
                // peer-supplied payload text.
                session_telemetry.warn("serving.session_ended", &[("code", code.as_str())]);
            }
        });
    }
}

/// Wire-request limits (msp-v1 §6.4): the serving peer clamps
/// requester-supplied values exactly like the local gateway does, so a
/// lease-holder cannot pin the engine with a 49-day deadline or an
/// unbounded token budget. Returns `(deadline_ms, max_tokens)` and a
/// rejection code when the prompt itself exceeds the byte cap.
pub(crate) fn clamp_wire_request(
    deadline_ms: u32,
    max_tokens: u32,
    prompt_bytes: usize,
) -> Result<(u32, u32), &'static str> {
    if prompt_bytes > MAX_PROMPT_BYTES {
        return Err("payload_too_large");
    }
    let deadline_ms = deadline_ms.clamp(DEADLINE_MS_MIN, DEADLINE_MS_MAX);
    let max_tokens = max_tokens.clamp(1, DEFAULT_MAX_OUTPUT_TOKENS);
    Ok((deadline_ms, max_tokens))
}

/// Variant name only — peer-supplied payloads must never reach logs or
/// error echoes (privacy rule 5; a hostile peer writes arbitrary text
/// into Debug output otherwise).
fn wire_kind(m: &WireMessage) -> &'static str {
    match m {
        WireMessage::Handshake(_) => "Handshake",
        WireMessage::HandshakeAck(_) => "HandshakeAck",
        WireMessage::InferenceRequest(_) => "InferenceRequest",
        WireMessage::TokenDelta(_) => "TokenDelta",
        WireMessage::Usage(_) => "Usage",
        WireMessage::StreamError(_) => "StreamError",
        WireMessage::Cancelled(_) => "Cancelled",
        WireMessage::Completed(_) => "Completed",
        WireMessage::Cancel(_) => "Cancel",
        WireMessage::Control(_) => "Control",
    }
}

/// Executor error → stable code for the wire and logs (no payload text).
fn executor_error_code(e: &modelswarm_gateway::ExecutorError) -> String {
    use modelswarm_gateway::ExecutorError;
    match e {
        ExecutorError::NoPeer => "no_peer".into(),
        ExecutorError::Retryable { .. } => "executor_retryable".into(),
        ExecutorError::Fatal { code } => code.clone(),
    }
}

/// Serving admission control: bounded concurrent sessions, a per-peer
/// session cap, and a bounded request-id dedup window (threat model
/// §4.2: a captured request frame must not replay into fresh inference
/// for the lease's lifetime).
#[derive(Clone)]
pub struct Admission {
    inner: Arc<std::sync::Mutex<AdmissionState>>,
    max_sessions: usize,
    max_sessions_per_peer: usize,
}

struct AdmissionState {
    sessions: usize,
    per_peer: HashMap<String, usize>,
    seen_requests: HashMap<(String, String), Instant>,
}

/// RAII session slot — releasing on drop keeps the count correct on every
/// exit path (panic, error return, client vanish).
pub struct SessionGuard {
    admission: Admission,
    peer_id: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.admission.release_session(&self.peer_id);
    }
}

impl Admission {
    /// Defaults sized for the prototype swarm: 8 concurrent sessions
    /// total, 2 per peer.
    pub fn defaults() -> Self {
        Self::new(8, 2)
    }

    pub fn new(max_sessions: usize, max_sessions_per_peer: usize) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(AdmissionState {
                sessions: 0,
                per_peer: HashMap::new(),
                seen_requests: HashMap::new(),
            })),
            max_sessions,
            max_sessions_per_peer,
        }
    }

    pub fn admit_session(&self, peer_id: &str) -> Result<SessionGuard, &'static str> {
        let mut state = self.inner.lock().expect("admission lock");
        if state.sessions >= self.max_sessions {
            return Err("session_limit");
        }
        if state.per_peer.get(peer_id).copied().unwrap_or(0) >= self.max_sessions_per_peer {
            return Err("per_peer_session_limit");
        }
        state.sessions += 1;
        *state.per_peer.entry(peer_id.to_string()).or_insert(0) += 1;
        Ok(SessionGuard {
            admission: self.clone(),
            peer_id: peer_id.to_string(),
        })
    }

    fn release_session(&self, peer_id: &str) {
        let mut state = self.inner.lock().expect("admission lock");
        state.sessions = state.sessions.saturating_sub(1);
        if let Some(count) = state.per_peer.get_mut(peer_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.per_peer.remove(peer_id);
            }
        }
    }

    /// `true` when `(peer, request_id)` is fresh; `false` = duplicate
    /// within the replay window (reject as a replay).
    pub fn check_request_fresh(&self, peer_id: &str, request_id: &str) -> bool {
        const WINDOW: Duration = Duration::from_secs(600);
        const HARD_CAP: usize = 8192;
        let mut state = self.inner.lock().expect("admission lock");
        if state.seen_requests.len() > HARD_CAP {
            state.seen_requests.retain(|_, at| at.elapsed() < WINDOW);
        }
        let key = (peer_id.to_string(), request_id.to_string());
        let now = Instant::now();
        match state.seen_requests.get(&key) {
            Some(at) if at.elapsed() < WINDOW => false,
            Some(_) | None => {
                state.seen_requests.insert(key, now);
                true
            }
        }
    }
}

async fn serve_one(
    mut session: Libp2pSession,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
    lease_policy: &LeasePolicy,
    admission: &Admission,
    telemetry: &Telemetry,
) -> Result<(), String> {
    let peer_id = session.remote_peer_id().to_string();
    // One QUIC session, many sequential requests (connection reuse): the
    // loop waits for the next request until the client closes. The
    // between-requests wait is generous (chat gaps); a recv error ends
    // the session and the client re-dials self-healingly.
    const BETWEEN_REQUESTS: Duration = Duration::from_secs(300);
    loop {
        let request = match session.recv(BETWEEN_REQUESTS).await {
            Ok(WireMessage::InferenceRequest(r)) => r,
            Ok(other) => {
                // Variant name only — the payload is peer-controlled text.
                let code = format!("expected InferenceRequest, got {}", wire_kind(&other));
                send_error(
                    &mut session,
                    "bad_frame",
                    "expected an InferenceRequest frame",
                )
                .await;
                linger(&mut session).await;
                return Err(code);
            }
            Err(e) => {
                // Transport errors are our own diagnostics text; the peer
                // sees the code, the log keeps the detail short.
                send_error(&mut session, "bad_frame", "frame receive failed").await;
                linger(&mut session).await;
                return Err(format!("recv: {e}"));
            }
        };
        if request.profile_id != profile_id {
            send_error(
                &mut session,
                "profile_mismatch",
                "this node serves another profile",
            )
            .await;
            linger(&mut session).await;
            return Err("profile_mismatch".into());
        }

        // Replay protection: a repeated (peer, request_id) is rejected —
        // each replay would otherwise re-execute inference on the lease.
        if !admission.check_request_fresh(&peer_id, &request.request_id) {
            send_error(
                &mut session,
                "replayed_request",
                "request id already served",
            )
            .await;
            linger(&mut session).await;
            return Err("replayed_request".into());
        }

        // F3: the hub-signed lease gate. The remote's PeerId comes from the
        // QUIC handshake, so a lease issued to anyone else is worthless here.
        if let Err(why) = lease_policy.check(
            &request.capability_token,
            &profile_id,
            &session.remote_peer_id().to_string(),
        ) {
            send_error(&mut session, "invalid_lease", "lease check failed").await;
            linger(&mut session).await;
            return Err(format!("invalid_lease: {why}"));
        }

        // §6.4 clamps — wire values are attacker-controlled even with a
        // valid lease; the local gateway applies the same table.
        let prompt_bytes: usize = request.messages.iter().map(|m| m.content.len()).sum();
        let (deadline_ms, max_tokens) =
            match clamp_wire_request(request.deadline_ms, request.max_tokens, prompt_bytes) {
                Ok(v) => v,
                Err(code) => {
                    send_error(&mut session, code, "prompt exceeds the byte cap").await;
                    linger(&mut session).await;
                    return Err(code.into());
                }
            };
        if deadline_ms != request.deadline_ms || max_tokens != request.max_tokens {
            telemetry.info(
                "serving.clamped",
                &[
                    ("peer", peer_id.as_str()),
                    (
                        "deadline_ms",
                        &format!("{}->{}", request.deadline_ms, deadline_ms),
                    ),
                    (
                        "max_tokens",
                        &format!("{}->{}", request.max_tokens, max_tokens),
                    ),
                ],
            );
        }

        let normalized = NormalizedRequest {
            request_id: request.request_id.clone(),
            profile_id: request.profile_id.clone(),
            capability_token: Some(request.capability_token.clone()),
            messages: request
                .messages
                .into_iter()
                .map(|m: ChatMessage| NormalizedMessage {
                    role: m.role,
                    content: m.content,
                })
                .collect(),
            sampling: modelswarm_gateway::Sampling {
                temperature: request.sampling.temperature as f32,
                top_p: request.sampling.top_p as f32,
                top_k: request.sampling.top_k,
                seed: request.sampling.seed,
            },
            max_tokens,
            deadline_ms,
            stream: true,
        };

        let mut events = match executor.execute(normalized).await {
            Ok(stream) => stream,
            Err(e) => {
                let code = executor_error_code(&e);
                send_error(&mut session, "executor_error", &code).await;
                linger(&mut session).await;
                return Err(format!("executor_error: {code}"));
            }
        };
        while let Some(event) = events.next().await {
            let frame = match event {
                ExecutorEvent::Accepted { .. } => continue,
                ExecutorEvent::TokenDelta { delta, index } => WireMessage::TokenDelta(TokenDelta {
                    request_id: request.request_id.clone(),
                    delta,
                    index,
                }),
                ExecutorEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                    prefill_ms,
                    decode_ms,
                } => WireMessage::Usage(Usage {
                    prompt_tokens,
                    completion_tokens,
                    prefill_ms: prefill_ms as u32,
                    decode_ms: decode_ms as u32,
                }),
                ExecutorEvent::Completed { finish_reason } => WireMessage::Completed(Completed {
                    request_id: request.request_id.clone(),
                    finish_reason: format!("{finish_reason:?}").to_lowercase(),
                }),
                ExecutorEvent::Error {
                    code, retryable, ..
                } => {
                    send_error(&mut session, &code, &format!("retryable={retryable}")).await;
                    linger(&mut session).await;
                    return Err(code);
                }
            };
            if session.send(&frame, IO_DEADLINE).await.is_err() {
                return Err("client vanished mid-stream".into());
            }
        }
    } // loop: await the next request on this session
}

/// Give the requester a moment to drain the last frame before the
/// session drops (see the post-completion linger note).
async fn linger(session: &mut Libp2pSession) {
    let _ = tokio::time::timeout(Duration::from_secs(1), session.recv(IO_DEADLINE)).await;
}

async fn send_error(session: &mut Libp2pSession, code: &str, detail: &str) {
    let _ = session
        .send(
            &WireMessage::StreamError(StreamError {
                request_id: String::new(),
                code: code.to_string(),
                message: detail.chars().take(200).collect(),
                retryable_peer_hint: false,
            }),
            IO_DEADLINE,
        )
        .await;
}

#[cfg(test)]
pub(crate) mod lease_helpers {
    use super::LeasePolicy;
    use ed25519_dalek::SigningKey;
    use modelswarm_eligibility::{CapacityClass, EligibilityLease};

    /// Direct gate round-trip: helper-issued lease passes check for its
    /// own peer, fails for any other binding.
    #[test]
    fn gate_accepts_own_and_rejects_foreign_binding() {
        let (policy, token) = policy_and_lease(TEST_PROFILE, "peer-x");
        policy
            .check(&token, TEST_PROFILE, "peer-x")
            .expect("own lease verifies");
        let err = policy.check(&token, TEST_PROFILE, "peer-y").unwrap_err();
        assert!(err.contains("not bound"), "got: {err}");
        let err = policy.check("", TEST_PROFILE, "peer-x").unwrap_err();
        assert!(err.contains("no lease"));
    }

    /// A profile id that passes the ADR-011 format check in tests.
    pub const TEST_PROFILE: &str =
        "msp1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn rfc3339(plus_secs: i64) -> String {
        time::OffsetDateTime::now_utc()
            .saturating_add(time::Duration::seconds(plus_secs))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    /// Issues a consume lease for (profile, peer) under a fresh test hub
    /// key; returns (policy over that key, wire token).
    pub fn policy_and_lease(profile: &str, peer_id: &str) -> (LeasePolicy, String) {
        let signing = SigningKey::generate(&mut rand_core::OsRng);
        let lease = EligibilityLease::issue(
            peer_id,
            "test-installation",
            profile,
            rfc3339(-60),
            rfc3339(300),
            rfc3339(360),
            true,
            true,
            1,
            CapacityClass::GpuMid,
            1,
            "test-challenge",
            "test-nonce",
            &signing,
        );
        let token = lease.to_wire();
        let policy =
            LeasePolicy::from_hex(&hex::encode(signing.verifying_key().to_bytes())).unwrap();
        (policy, token)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    /// F0(4) same-host proof: the full serving-bridge round trip over real
    /// QUIC — dial, InferenceRequest in, TokenDelta/Usage/Completed out.
    #[tokio::test]
    async fn serving_bridge_round_trips_over_quic() {
        use super::serve_sessions;
        use futures_util::stream;
        use modelswarm_gateway::{
            ExecutorError, ExecutorEvent, ExecutorStream, FinishReason, InferenceExecutor,
            NormalizedRequest,
        };
        use modelswarm_identity::InstallationIdentity;
        use modelswarm_transport::libp2p_backend::Libp2pTransport;
        use modelswarm_transport::message::{
            ChatMessage, InferenceRequest, Sampling as WireSampling, WireMessage,
        };
        use std::time::Duration;

        struct FixedExecutor;
        #[async_trait::async_trait]
        impl InferenceExecutor for FixedExecutor {
            async fn execute(
                &self,
                request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                assert_eq!(request.messages.len(), 1);
                assert_eq!(request.messages[0].role, "user");
                Ok(Box::pin(stream::iter(vec![
                    ExecutorEvent::Accepted {
                        queue_position: 0,
                        eta_ms: 0,
                    },
                    ExecutorEvent::TokenDelta {
                        delta: "Hello ".into(),
                        index: 0,
                    },
                    ExecutorEvent::TokenDelta {
                        delta: "swarm".into(),
                        index: 1,
                    },
                    ExecutorEvent::Usage {
                        prompt_tokens: 3,
                        completion_tokens: 2,
                        prefill_ms: 1.0,
                        decode_ms: 2.0,
                    },
                    ExecutorEvent::Completed {
                        finish_reason: FinishReason::Stop,
                    },
                ])))
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[7u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[8u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let client = Libp2pTransport::new(&client_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();

        let bound = listener.bound_addr().clone();
        let expected_server_peer = server.peer_id();
        let (policy, token) = super::lease_helpers::policy_and_lease(
            super::lease_helpers::TEST_PROFILE,
            &client.peer_id().to_string(),
        );
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener,
            Arc::new(FixedExecutor),
            super::lease_helpers::TEST_PROFILE.into(),
            policy,
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            shutdown_rx,
        ));

        let dial_addr = bound.to_string();
        let mut session = client
            .dial(
                dial_addr.as_str(),
                &expected_server_peer,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        session
            .send(
                &WireMessage::InferenceRequest(InferenceRequest {
                    request_id: "req-1".into(),
                    profile_id: super::lease_helpers::TEST_PROFILE.into(),
                    capability_token: token,
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                    sampling: WireSampling {
                        temperature: 0.0,
                        top_p: 1.0,
                        top_k: 40,
                        seed: None,
                    },
                    max_tokens: 8,
                    deadline_ms: 10_000,
                    stream: true,
                }),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        let mut deltas = String::new();
        let mut saw_usage = false;
        let mut finish = String::new();
        for _ in 0..4 {
            match session.recv(Duration::from_secs(10)).await.unwrap() {
                WireMessage::TokenDelta(d) => deltas.push_str(&d.delta),
                WireMessage::Usage(_) => saw_usage = true,
                WireMessage::Completed(c) => finish = c.finish_reason,
                other => panic!("unexpected frame: {other:?}"),
            }
        }
        assert_eq!(deltas, "Hello swarm");
        assert!(saw_usage);
        assert_eq!(finish, "stop");
        shutdown_tx.send(true).ok();
    }

    /// 2026-10-09 LAN pass-1 bug (regression pin): dropping the shutdown
    /// watch SENDER must terminate the serving task — never spin with a
    /// permanently-ready `changed()` branch cancelling in-flight accepts
    /// (each cancellation dropped a mid-handshake QUIC upgrade; quinn then
    /// application-closed the unconfirmed connection as transport error
    /// 0xC, surfacing on the dialer as "libp2p stream open: aborted by
    /// peer: ... during the handshake"). The pre-fix task never exited,
    /// so a resolving JoinHandle is the pin.
    #[tokio::test]
    async fn serve_sessions_stops_when_shutdown_sender_drops() {
        use super::serve_sessions;
        use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedRequest};
        use modelswarm_identity::InstallationIdentity;
        use modelswarm_transport::libp2p_backend::Libp2pTransport;
        use std::time::Duration;

        struct NeverExecutor;
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for NeverExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                unreachable!("no request is ever accepted in this test")
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[67u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let serving = tokio::spawn(serve_sessions(
            listener,
            Arc::new(NeverExecutor),
            super::lease_helpers::TEST_PROFILE.into(),
            super::LeasePolicy::production(),
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            shutdown_rx,
        ));
        // Drop the last sender: no owner can ever signal stop — the
        // bridge must stop serving on its own, promptly.
        drop(shutdown_tx);
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .expect("serve_sessions exits when the shutdown sender drops")
            .expect("serve_sessions task did not panic");
    }

    /// The honest-refusal path (wrong profile → StreamError frame, then
    /// close). Previously flaky: the connection close raced the error
    /// frame — fixed by the post-error linger; kept as a regression pin.
    #[tokio::test]
    async fn serving_bridge_refuses_foreign_profile() {
        use super::serve_sessions;
        use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedRequest};
        use modelswarm_identity::InstallationIdentity;
        use modelswarm_transport::libp2p_backend::Libp2pTransport;
        use modelswarm_transport::message::{
            ChatMessage, InferenceRequest, Sampling as WireSampling, WireMessage,
        };
        use std::time::Duration;

        struct NeverExecutor;
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for NeverExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                unreachable!("wrong-profile requests never reach the executor")
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[17u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[18u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let client = Libp2pTransport::new(&client_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let bound = listener.bound_addr().to_string();
        let expected = server.peer_id();
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener,
            Arc::new(NeverExecutor),
            super::lease_helpers::TEST_PROFILE.into(),
            super::LeasePolicy::production(),
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            shutdown_rx,
        ));

        let mut session = client
            .dial(bound.as_str(), &expected, Duration::from_secs(10))
            .await
            .unwrap();
        session
            .send(
                &WireMessage::InferenceRequest(InferenceRequest {
                    request_id: "req-x".into(),
                    profile_id: "msp1:OTHER".into(),
                    capability_token: "f0-unverified".into(),
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                    sampling: WireSampling {
                        temperature: 0.0,
                        top_p: 1.0,
                        top_k: 40,
                        seed: None,
                    },
                    max_tokens: 8,
                    deadline_ms: 10_000,
                    stream: true,
                }),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        match session.recv(Duration::from_secs(10)).await.unwrap() {
            WireMessage::StreamError(e) => assert_eq!(e.code, "profile_mismatch"),
            other => panic!("expected StreamError, got {other:?}"),
        }
    }

    /// F3 gate: a valid profile request WITHOUT a lease is refused with
    /// invalid_lease — and never reaches the executor.
    #[tokio::test]
    async fn serving_bridge_refuses_leaseless_requests() {
        use super::serve_sessions;
        use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedRequest};
        use modelswarm_identity::InstallationIdentity;
        use modelswarm_transport::libp2p_backend::Libp2pTransport;
        use modelswarm_transport::message::{
            ChatMessage, InferenceRequest, Sampling as WireSampling, WireMessage,
        };
        use std::time::Duration;

        struct NeverExecutor;
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for NeverExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                unreachable!("leaseless requests never reach the executor")
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[47u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[48u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let client = Libp2pTransport::new(&client_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let bound = listener.bound_addr().to_string();
        let expected = server.peer_id();
        // Policy over a TEST hub key; the request presents NO lease at all.
        let (policy, _unused) = super::lease_helpers::policy_and_lease(
            super::lease_helpers::TEST_PROFILE,
            &client.peer_id().to_string(),
        );
        let (_tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener,
            Arc::new(NeverExecutor),
            super::lease_helpers::TEST_PROFILE.into(),
            policy,
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx,
        ));

        let mut session = client
            .dial(bound.as_str(), &expected, Duration::from_secs(10))
            .await
            .unwrap();
        session
            .send(
                &WireMessage::InferenceRequest(InferenceRequest {
                    request_id: "req-nolease".into(),
                    profile_id: super::lease_helpers::TEST_PROFILE.into(),
                    capability_token: String::new(),
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                    sampling: WireSampling {
                        temperature: 0.0,
                        top_p: 1.0,
                        top_k: 40,
                        seed: None,
                    },
                    max_tokens: 8,
                    deadline_ms: 10_000,
                    stream: true,
                }),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        match session.recv(Duration::from_secs(10)).await.unwrap() {
            WireMessage::StreamError(e) => assert_eq!(e.code, "invalid_lease"),
            other => panic!("expected StreamError, got {other:?}"),
        }
    }

    /// F3 gate: a VALID lease bound to a DIFFERENT peer is refused — the
    /// QUIC-authenticated PeerId binding makes stolen leases worthless.
    #[tokio::test]
    async fn serving_bridge_refuses_foreign_peer_lease() {
        use super::serve_sessions;
        use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedRequest};
        use modelswarm_identity::InstallationIdentity;
        use modelswarm_transport::libp2p_backend::Libp2pTransport;
        use modelswarm_transport::message::{
            ChatMessage, InferenceRequest, Sampling as WireSampling, WireMessage,
        };
        use std::time::Duration;

        struct NeverExecutor;
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for NeverExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                unreachable!("foreign-lease requests never reach the executor")
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[57u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[58u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let client = Libp2pTransport::new(&client_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let bound = listener.bound_addr().to_string();
        let expected = server.peer_id();
        // Lease issued to SOMEONE ELSE (a third identity), signed by the
        // same hub key the policy verifies with — signature valid, binding
        // wrong.
        let stranger = Libp2pTransport::new(&InstallationIdentity::from_bytes(&[59u8; 32]))
            .unwrap()
            .peer_id()
            .to_string();
        let (policy, stranger_token) =
            super::lease_helpers::policy_and_lease(super::lease_helpers::TEST_PROFILE, &stranger);
        let (_tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener,
            Arc::new(NeverExecutor),
            super::lease_helpers::TEST_PROFILE.into(),
            policy,
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx,
        ));

        let mut session = client
            .dial(bound.as_str(), &expected, Duration::from_secs(10))
            .await
            .unwrap();
        session
            .send(
                &WireMessage::InferenceRequest(InferenceRequest {
                    request_id: "req-stolen".into(),
                    profile_id: super::lease_helpers::TEST_PROFILE.into(),
                    capability_token: stranger_token,
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                    sampling: WireSampling {
                        temperature: 0.0,
                        top_p: 1.0,
                        top_k: 40,
                        seed: None,
                    },
                    max_tokens: 8,
                    deadline_ms: 10_000,
                    stream: true,
                }),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        match session.recv(Duration::from_secs(10)).await.unwrap() {
            WireMessage::StreamError(e) => {
                assert_eq!(e.code, "invalid_lease");
                // The wire message is static by policy (no lease internals
                // cross the wire); the specific reason stays in node logs.
                assert_eq!(e.message, "lease check failed");
            }
            other => panic!("expected StreamError, got {other:?}"),
        }
    }

    #[test]
    fn serving_module_wires_wire_shapes() {
        use modelswarm_transport::message::Completed;
        assert_eq!(
            Completed {
                request_id: "r".into(),
                finish_reason: "stop".into()
            }
            .finish_reason,
            "stop"
        );
    }

    /// §6.4 clamps on the wire path: a lease-holder sending a 49-day
    /// deadline or an unbounded token budget gets the same table the
    /// local gateway applies; an oversized prompt is rejected outright.
    #[test]
    fn wire_limits_clamp_like_the_gateway() {
        use modelswarm_gateway::{
            DEADLINE_MS_MAX, DEADLINE_MS_MIN, DEFAULT_MAX_OUTPUT_TOKENS, MAX_PROMPT_BYTES,
        };
        let (d, t) = super::clamp_wire_request(u32::MAX, u32::MAX, 100).unwrap();
        assert_eq!(d, DEADLINE_MS_MAX);
        assert_eq!(t, DEFAULT_MAX_OUTPUT_TOKENS);
        let (d, t) = super::clamp_wire_request(0, 0, 100).unwrap();
        assert_eq!(d, DEADLINE_MS_MIN);
        assert_eq!(t, 1);
        let (d, t) = super::clamp_wire_request(30_000, 64, 100).unwrap();
        assert_eq!((d, t), (30_000, 64));
        assert_eq!(
            super::clamp_wire_request(30_000, 64, MAX_PROMPT_BYTES + 1),
            Err("payload_too_large")
        );
    }

    #[test]
    fn admission_caps_sessions_and_replays() {
        let adm = super::Admission::new(2, 1);
        let a = adm.admit_session("peer-a").expect("first session admitted");
        assert!(adm.admit_session("peer-a").is_err(), "per-peer cap");
        let b = adm.admit_session("peer-b").expect("second peer admitted");
        assert!(adm.admit_session("peer-c").is_err(), "total cap");
        drop(a);
        drop(b);
        assert!(
            adm.admit_session("peer-c").is_ok(),
            "slots released on drop"
        );
        // request-id dedup: fresh once, replay rejected, other peer fresh.
        assert!(adm.check_request_fresh("peer-a", "req-1"));
        assert!(!adm.check_request_fresh("peer-a", "req-1"));
        assert!(adm.check_request_fresh("peer-b", "req-1"));
    }
}

// ---- F3: lease verification at session open (ADR-026) ----------------------
//
// A serving peer admits a session only when the request carries a
// hub-signed EligibilityLease in `capability_token` that (1) verifies
// against the pinned hub key, (2) names THIS node's profile, (3) grants
// can_consume with slots, and (4) is bound to the remote's
// QUIC-authenticated PeerId — a lease stolen from another peer is
// worthless. This closes the F0 TODO: serving is no longer
// protocol-speaking-anyone.

/// RFC 3339 "now" (shared by the gate and external verifiers).
pub fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("rfc3339 formats")
}

/// The pinned hub signing key (protocol/keys/hub-public.hex) — the same
/// key the tracker's catalog envelopes are verified against.
const HUB_PUBLIC_KEY_HEX: &str = include_str!("../../../protocol/keys/hub-public.hex");

#[derive(Clone)]
pub struct LeasePolicy {
    hub_pubkey: ed25519_dalek::VerifyingKey,
}

impl LeasePolicy {
    /// Production policy: the pinned hub key.
    pub fn production() -> Self {
        Self::from_hex(HUB_PUBLIC_KEY_HEX.trim()).expect("pinned hub key parses")
    }

    /// Test policy over an arbitrary key (hex of 32 bytes).
    pub fn from_hex(hex_str: &str) -> Result<Self, String> {
        let bytes = hex::decode(hex_str).map_err(|e| format!("hub key hex: {e}"))?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(
            &bytes[..32]
                .try_into()
                .map_err(|_| "hub key must be 32 bytes".to_string())?,
        )
        .map_err(|e| format!("hub key: {e}"))?;
        Ok(Self { hub_pubkey: key })
    }

    /// The pinned hub verifying key (for verification by other layers).
    pub fn hub_key(&self) -> &ed25519_dalek::VerifyingKey {
        &self.hub_pubkey
    }

    /// Full admission check for one inbound request.
    pub fn check(
        &self,
        capability_token: &str,
        served_profile: &str,
        remote_peer_id: &str,
    ) -> Result<(), String> {
        if capability_token.is_empty() || capability_token.starts_with("f0-unverified") {
            return Err("no lease presented".into());
        }
        let lease = modelswarm_eligibility::EligibilityLease::from_wire(capability_token)
            .map_err(|e| format!("bad lease: {e}"))?;
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| format!("clock: {e}"))?;
        lease
            .verify(&self.hub_pubkey, &now)
            .map_err(|e| format!("lease invalid: {e}"))?;
        if lease.model_profile_id != served_profile {
            return Err(format!(
                "lease is for another profile: {}",
                lease.model_profile_id
            ));
        }
        if !lease.can_consume || lease.slots == 0 {
            return Err("lease grants no consume right".into());
        }
        if lease.peer_id != remote_peer_id {
            return Err("lease is not bound to this peer".into());
        }
        Ok(())
    }
}
