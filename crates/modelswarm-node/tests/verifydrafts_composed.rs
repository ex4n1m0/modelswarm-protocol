//! The composed-daemon demux (feature `verifydrafts` + `libp2p-backend`):
//! ONE QUIC listener serving BOTH session vocabularies behind ONE
//! admission gate and ONE ADR-026 lease discipline
//! (`serve_sessions_composed`).
//!
//! - `one_listener_serves_both_vocabularies`: a SpecMessage (msp-v1)
//!   session and a VerifyDrafts family session coexist on one listener —
//!   the required demux pin — with the family's engine disclosure coming
//!   from the INJECTED engine through the seam (real geometry, never the
//!   hand-set config field), and the msp-v1 path streaming through its
//!   executor unchanged.
//! - `family_session_refused_without_lease`: the fail-closed lease seam —
//!   no lease derivation for the authenticated coordinator PeerId means
//!   no family session (the honest fallback-to-single signal on the
//!   client), telemetered with a stable label.

#![cfg(all(feature = "verifydrafts", feature = "libp2p-backend"))]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use futures_util::stream;
use modelswarm_eligibility::{CapacityClass, EligibilityLease};
use modelswarm_gateway::{
    ExecutorError, ExecutorEvent, ExecutorStream, FinishReason, InferenceExecutor,
    NormalizedRequest,
};
use modelswarm_identity::InstallationIdentity;
use modelswarm_node::verifydrafts::composed::{
    serve_sessions_composed, FamilyServing, RefuseFamilyLeases, StaticLeaseExpiry,
};
use modelswarm_node::verifydrafts::engine_source::FamilyEngineSource;
use modelswarm_node::verifydrafts::{
    VerifyDraftsClient, VerifyDraftsClientConfig, VerifyDraftsServeConfig,
};
use modelswarm_runtime::SamplingParams;
use modelswarm_session::negotiation::{Backend, EngineDisclosure};
use modelswarm_session::verifydrafts::BatchVerifyEngine;
use modelswarm_telemetry::Telemetry;
use modelswarm_transport::libp2p_backend::Libp2pTransport;
use modelswarm_transport::message::{
    ChatMessage, InferenceRequest, Sampling as WireSampling, WireMessage,
};

/// A profile id that passes the msp1 form checks (serving.rs test pattern).
const TEST_PROFILE: &str = "msp1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const ACCEPT_DEADLINE: Duration = Duration::from_secs(10);

/// A mock batch-verify engine with a scripted outcome list (the QUIC
/// venue test's pattern): one outcome per dispatched round, in order.
struct ScriptedEngine {
    disclosure: EngineDisclosure,
    outcomes: Mutex<VecDeque<(u32, Option<u32>)>>,
}

impl ScriptedEngine {
    fn new(disclosure: EngineDisclosure, outcomes: Vec<(u32, Option<u32>)>) -> Arc<Self> {
        Arc::new(Self {
            disclosure,
            outcomes: Mutex::new(outcomes.into()),
        })
    }
}

#[async_trait::async_trait]
impl BatchVerifyEngine for ScriptedEngine {
    async fn verify_window(
        &self,
        _window: &[u32],
        _params: &SamplingParams,
    ) -> Result<(u32, Option<u32>), modelswarm_runtime::RuntimeError> {
        self.outcomes
            .lock()
            .expect("outcomes lock")
            .pop_front()
            .ok_or_else(|| {
                modelswarm_runtime::RuntimeError::MalformedResponse("script exhausted".to_string())
            })
    }

    fn capabilities(&self) -> EngineDisclosure {
        self.disclosure.clone()
    }
}

fn batch_engine(window_max: u32) -> EngineDisclosure {
    EngineDisclosure {
        backend: Backend::Cpu,
        batch_verify: true,
        batch_window_max: window_max,
    }
}

/// The deterministic msp-v1 executor (the serving-bridge test's
/// FixedExecutor): "Hello swarm" + usage + completed.
struct FixedExecutor;

#[async_trait::async_trait]
impl InferenceExecutor for FixedExecutor {
    async fn execute(&self, request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
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

fn rfc3339(plus_secs: i64) -> String {
    time::OffsetDateTime::now_utc()
        .saturating_add(time::Duration::seconds(plus_secs))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

/// Issues a consume lease for (profile, peer) under a fresh test hub key;
/// returns (policy over that key, wire token) — the serving.rs
/// lease_helpers pattern (not reachable from an integration test).
fn policy_and_lease(
    profile: &str,
    peer_id: &str,
) -> (modelswarm_node::serving::LeasePolicy, String) {
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
    let policy = modelswarm_node::serving::LeasePolicy::from_hex(&hex::encode(
        signing.verifying_key().to_bytes(),
    ))
    .unwrap();
    (policy, token)
}

/// The required demux pin: a SpecMessage (msp-v1) session and a
/// VerifyDrafts family session COEXIST on one listener — classified by
/// their first frames, both admitted through the one `Admission` gate,
/// the family's lease derived from its authenticated PeerId, and the
/// family's engine disclosure reported from the INJECTED engine (the
/// seam's real geometry: window 8, batch true — not the hand-set
/// `honest()` default `batch_verify: false`).
#[tokio::test]
async fn one_listener_serves_both_vocabularies() {
    let server_identity = InstallationIdentity::from_bytes(&[0xE1; 32]);
    let coordinator = InstallationIdentity::from_bytes(&[0xE2; 32]);
    let requester = InstallationIdentity::from_bytes(&[0xE3; 32]);
    let telemetry = Arc::new(Telemetry::memory().0);

    let engine = ScriptedEngine::new(batch_engine(8), vec![(2, Some(901))]);
    let mut family_config =
        VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());
    // The hand-set disclosure is the honest `batch_verify: false`; the
    // composed loop must override it with the injected engine's REAL
    // geometry — pinned by the client-side assertion below.
    family_config.session_idle = Duration::from_secs(60);

    let transport = Libp2pTransport::new(&server_identity).expect("server transport");
    let listener = transport
        .listen("127.0.0.1:0")
        .await
        .expect("loopback quic");
    let addr = listener.bound_addr().to_string();
    let expected_server_peer = transport.peer_id();

    let (policy, token) = policy_and_lease(TEST_PROFILE, &requester.peer_id().to_string());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let serving_telemetry = Arc::clone(&telemetry);
    tokio::spawn(serve_sessions_composed(
        listener,
        Arc::new(FixedExecutor),
        TEST_PROFILE.to_string(),
        policy,
        FamilyServing {
            config: family_config,
            engine: Arc::new(FamilyEngineSource::Shared(engine)),
            lease: Arc::new(StaticLeaseExpiry(u64::MAX)),
        },
        serving_telemetry,
        shutdown_rx,
    ));

    // ---- msp-v1 arm: a SpecMessage session over the SAME listener ------
    let requester_transport = Libp2pTransport::new(&requester).expect("requester transport");
    let mut msp_session = requester_transport
        .dial(&addr, &expected_server_peer, ACCEPT_DEADLINE)
        .await
        .expect("msp-v1 dial into the composed listener");
    msp_session
        .send(
            &WireMessage::InferenceRequest(InferenceRequest {
                request_id: "req-1".into(),
                profile_id: TEST_PROFILE.into(),
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
        .expect("msp-v1 request through the demux");
    let mut deltas = String::new();
    let mut finish = String::new();
    for _ in 0..4 {
        match msp_session.recv(Duration::from_secs(10)).await.unwrap() {
            WireMessage::TokenDelta(d) => deltas.push_str(&d.delta),
            WireMessage::Usage(_) => {}
            WireMessage::Completed(c) => finish = c.finish_reason,
            other => panic!("unexpected msp-v1 frame: {other:?}"),
        }
    }
    assert_eq!(deltas, "Hello swarm");
    assert_eq!(finish, "stop");

    // ---- family arm: a VerifyDrafts session over the SAME listener -----
    let wire = modelswarm_node::verifydrafts::dial_quic(
        &addr,
        &coordinator,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("family dial into the composed listener");
    let tel = Telemetry::memory().0;
    let mut client = VerifyDraftsClient::negotiate(
        wire,
        coordinator.clone(),
        VerifyDraftsClientConfig::new(TEST_PROFILE.to_string(), server_identity.verifying_key()),
        &tel,
    )
    .await
    .expect("family offer accepted through the composed demux");
    // The disclosure is the INJECTED engine's real geometry (the seam
    // override), not the config's hand-set honest default.
    assert!(client.engine_disclosure().batch_verify);
    assert_eq!(client.engine_disclosure().batch_window_max, 8);
    assert_eq!(client.engine_disclosure().backend, Backend::Cpu);

    let r1 = client
        .round(&[71, 72], &tel)
        .await
        .expect("family round through the composed demux");
    assert_eq!(r1.committed, vec![71, 72, 901]);
    assert_eq!(client.committed_stream(), vec![71, 72, 901]);

    // The msp-v1 session is STILL live (connection reuse across the
    // family session's whole lifecycle — one listener, both vocabularies).
    drop(client.into_wire());
    drop(msp_session);
    drop(shutdown_tx);
}

/// The fail-closed ADR-026 seam: with no lease derivation for the
/// authenticated coordinator, the family session is refused BEFORE any
/// engine work (the scripted engine is never consumed) and the client
/// surfaces an honest transport failure — its fallback-to-single signal.
#[tokio::test]
async fn family_session_refused_without_lease() {
    let server_identity = InstallationIdentity::from_bytes(&[0xF1; 32]);
    let coordinator = InstallationIdentity::from_bytes(&[0xF2; 32]);
    let (telemetry, sink) = Telemetry::memory();
    let telemetry = Arc::new(telemetry);

    let engine = ScriptedEngine::new(batch_engine(8), vec![(1, Some(1))]);
    let family_config =
        VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());

    let transport = Libp2pTransport::new(&server_identity).expect("server transport");
    let listener = transport
        .listen("127.0.0.1:0")
        .await
        .expect("loopback quic");
    let addr = listener.bound_addr().to_string();
    let (policy, _unused) = policy_and_lease(TEST_PROFILE, "someone-else");
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let serving_telemetry = Arc::clone(&telemetry);
    let serving = tokio::spawn(serve_sessions_composed(
        listener,
        Arc::new(NeverExecutor),
        TEST_PROFILE.to_string(),
        policy,
        FamilyServing {
            config: family_config,
            engine: Arc::new(FamilyEngineSource::Shared(engine)),
            lease: Arc::new(RefuseFamilyLeases),
        },
        serving_telemetry,
        shutdown_rx,
    ));

    let wire = modelswarm_node::verifydrafts::dial_quic(
        &addr,
        &coordinator,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("dial itself is lease-agnostic");
    let tel = Telemetry::memory().0;
    match VerifyDraftsClient::negotiate(
        wire,
        coordinator,
        VerifyDraftsClientConfig::new(TEST_PROFILE.to_string(), server_identity.verifying_key()),
        &tel,
    )
    .await
    {
        // The server ends the session after classifying the offer frame:
        // the client sees the transport failure, never an accept.
        Err(modelswarm_node::verifydrafts::ClientOpenError::Io(_)) => {}
        Ok(_) => panic!("a lease-less family session must never be accepted"),
        Err(other) => panic!("expected an honest transport failure, got {other}"),
    }

    // The refusal is telemetered with a stable label (ids/codes only).
    let events = sink.snapshot().join("\n");
    assert!(
        events.contains("verifydrafts.family_refused"),
        "the refusal must be telemetered: {events}"
    );
    assert!(
        events.contains("\"reason\":\"no_lease\"") || events.contains("\"reason\": \"no_lease\""),
        "the no-lease reason must be stable: {events}"
    );
    drop(shutdown_tx);
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .expect("composed loop exits when the shutdown sender drops")
        .expect("composed loop task ok");
}

/// An executor that must never run in the lease-refusal test.
struct NeverExecutor;

#[async_trait::async_trait]
impl InferenceExecutor for NeverExecutor {
    async fn execute(&self, _request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
        unreachable!("no msp-v1 request is ever accepted in this test")
    }
}
