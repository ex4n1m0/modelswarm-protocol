//! F0(3) serving bridge: inbound framed sessions → local executor →
//! outbound token frames (Phase F0; see docs/reviews/phase-f-research).
//!
//! The listener hands us authenticated sessions (the QUIC handshake
//! verifies the remote's libp2p PeerId = the ADR-020 derivation). For
//! each session we read one `InferenceRequest`, drive the node's
//! [`InferenceExecutor`] (the same executor the local gateway uses —
//! ADR-025 ChatML templating applies), and stream
//! `TokenDelta`/`Usage`/`Completed` frames back.
//!
//! Honest F0 scope: the capability/lease token is NOT yet verified here
//! (TODO F3 — the tracker-signed lease check lands with hostile
//! hardening); a session is admitted if it speaks the protocol and asks
//! for the profile this node serves.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use modelswarm_gateway::{ExecutorEvent, InferenceExecutor, NormalizedMessage, NormalizedRequest};
use modelswarm_transport::libp2p_backend::{Libp2pListener, Libp2pSession};
use modelswarm_transport::message::{
    ChatMessage, Completed, StreamError, TokenDelta, Usage, WireMessage,
};

const IO_DEADLINE: Duration = Duration::from_secs(30);

/// Serves accepted sessions until `shutdown` flips to true. Never panics
/// on a bad session: malformed input becomes a `StreamError` frame, then
/// the session closes (one request per session in F0).
pub async fn serve_sessions(
    mut listener: Libp2pListener,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
    lease_policy: LeasePolicy,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let session = tokio::select! {
            // watch::changed() fires immediately for the unseen INITIAL
            // value — only an actual true flip stops serving.
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
                continue;
            }
            accepted = listener.accept(Duration::from_secs(1)) => match accepted {
                Ok(s) => s,
                Err(_) => continue, // timeout/accept miss: keep serving
            },
        };
        let executor = Arc::clone(&executor);
        let profile = profile_id.clone();
        let policy = lease_policy.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_one(session, executor, profile, &policy).await {
                eprintln!("modelswarm-node: serving session ended: {e}");
            }
        });
    }
}

async fn serve_one(
    mut session: Libp2pSession,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
    lease_policy: &LeasePolicy,
) -> Result<(), String> {
    // One QUIC session, many sequential requests (connection reuse): the
    // loop waits for the next request until the client closes. The
    // between-requests wait is generous (chat gaps); a recv error ends
    // the session and the client re-dials self-healingly.
    const BETWEEN_REQUESTS: Duration = Duration::from_secs(300);
    loop {
        let request = match session.recv(BETWEEN_REQUESTS).await {
            Ok(WireMessage::InferenceRequest(r)) => r,
            Ok(other) => {
                let code = format!("expected InferenceRequest, got {other:?}");
                send_error(&mut session, "bad_frame", &code).await;
                linger(&mut session).await;
                return Err(code);
            }
            Err(e) => {
                send_error(&mut session, "bad_frame", &e.to_string()).await;
                linger(&mut session).await;
                return Err(e.to_string());
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

        // F3: the hub-signed lease gate. The remote's PeerId comes from the
        // QUIC handshake, so a lease issued to anyone else is worthless here.
        if let Err(why) = lease_policy.check(
            &request.capability_token,
            &profile_id,
            &session.remote_peer_id().to_string(),
        ) {
            send_error(&mut session, "invalid_lease", &why).await;
            linger(&mut session).await;
            return Err(format!("invalid_lease: {why}"));
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
            max_tokens: request.max_tokens,
            deadline_ms: request.deadline_ms,
            stream: true,
        };

        let mut events = match executor.execute(normalized).await {
            Ok(stream) => stream,
            Err(e) => {
                let code = format!("{e:?}");
                send_error(&mut session, "executor_error", &code).await;
                linger(&mut session).await;
                return Err(code);
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
        use futures_util::stream;
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
        use futures_util::stream;
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
                assert!(e.message.contains("not bound"), "got: {}", e.message);
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
