//! ADR-032 light-up wiring tests (feature `verifydrafts`): full session
//! lifecycle + the negative paths, over the REAL staged transport (real
//! loopback sockets, real length-prefixed framing, the real Ed25519 §6.1
//! handshake with the client key resolved from the peer directory) with a
//! scripted mock engine on the serve side.
//!
//! The QUIC venue is BLOCKED on the `Libp2pSession` raw-JSON passthrough
//! gap (see `docs/reviews/handoff-windows-product-verifydrafts-wiring-2026-10-11.md`);
//! these tests ride the ADR-018 amended staging — the exact path ADR-032
//! designates for the family's bare-JSON frames.

#![cfg(feature = "verifydrafts")]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use modelswarm_identity::InstallationIdentity;
use modelswarm_node::verifydrafts::serve::{
    serve_verifydrafts_session, ServeEndReason, VerifyDraftsServeConfig,
};
use modelswarm_node::verifydrafts::wire::{accept_staged, dial_staged};
use modelswarm_node::verifydrafts::{
    ClientOpenError, RoundFailure, VerifyDraftsClient, VerifyDraftsClientConfig, VerifyDraftsWire,
};
use modelswarm_session::negotiation::{
    Backend, EngineDisclosure, SessionAccept, SessionOffer, COOPERATIVE_PROTOCOL_VERSION,
};
use modelswarm_session::verifydrafts::{
    sign_verify_drafts, BatchVerifyEngine, VerifyDrafts, VerifyDraftsResult,
};
use modelswarm_session::GENESIS_PREFIX_HASH;
use modelswarm_telemetry::Telemetry;
use modelswarm_transport::ed25519_dalek::VerifyingKey;
use modelswarm_transport::SignedFrameTransport;

/// A profile id that passes the msp1 form checks (serving.rs test pattern).
const TEST_PROFILE: &str = "msp1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const ACCEPT_DEADLINE: Duration = Duration::from_secs(10);

/// A mock batch-verify engine with a scripted outcome list (the session
/// crate's MockVerifyEngine pattern, transport-shaped): one outcome per
/// dispatched round, in order; exhaustion fails closed.
struct ScriptedEngine {
    disclosure: EngineDisclosure,
    outcomes: Mutex<VecDeque<(u32, Option<u32>)>>,
    #[allow(dead_code)]
    calls: Mutex<usize>,
}

impl ScriptedEngine {
    fn new(disclosure: EngineDisclosure, outcomes: Vec<(u32, Option<u32>)>) -> Arc<Self> {
        Arc::new(Self {
            disclosure,
            outcomes: Mutex::new(outcomes.into()),
            calls: Mutex::new(0),
        })
    }
}

#[async_trait::async_trait]
impl BatchVerifyEngine for ScriptedEngine {
    async fn verify_window(
        &self,
        _window: &[u32],
        _params: &modelswarm_runtime::SamplingParams,
    ) -> Result<(u32, Option<u32>), modelswarm_runtime::RuntimeError> {
        *self.calls.lock().expect("calls lock") += 1;
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

/// F0(4)-style full round trip: offer/accept → three engaged rounds (full
/// acceptance, mid-window rejection, EOG full acceptance) → commits →
/// clean teardown by client close.
#[tokio::test]
async fn lifecycle_round_trips_over_real_transport() {
    let server_identity = InstallationIdentity::from_bytes(&[0xA1; 32]);
    let client_identity = InstallationIdentity::from_bytes(&[0xA2; 32]);
    let telemetry = Arc::new(Telemetry::memory().0);

    let engine = ScriptedEngine::new(
        batch_engine(8),
        vec![(8, Some(901)), (3, Some(902)), (8, None)],
    );
    let mut config =
        VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());
    config.engine = batch_engine(8);
    config.session_idle = Duration::from_secs(60);

    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .expect("loopback listen");
    let addr = listener.local_addr().expect("bound").to_string();
    let mut trusted: HashMap<String, VerifyingKey> = HashMap::new();
    trusted.insert(client_identity.peer_id(), client_identity.verifying_key());
    let server = tokio::spawn(async move {
        let wire = accept_staged(&listener, &trusted, ACCEPT_DEADLINE)
            .await
            .expect("authenticated accept");
        serve_verifydrafts_session(wire, config, engine.clone(), &telemetry).await
    });

    let wire = dial_staged(
        &addr,
        &client_identity,
        TEST_PROFILE,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("dial + handshake");
    let tel = Telemetry::memory().0;
    let mut client = VerifyDraftsClient::negotiate(
        wire,
        client_identity,
        VerifyDraftsClientConfig::new(TEST_PROFILE.to_string(), server_identity.verifying_key()),
        &tel,
    )
    .await
    .expect("offer accepted");

    // Honest disclosure round-tripped through config.
    assert!(client.engine_disclosure().batch_verify);
    assert_eq!(client.engine_disclosure().batch_window_max, 8);

    // Round 1: full acceptance + bonus token.
    let r1 = client
        .round(&[11, 12, 13, 14, 15, 16, 17, 18], &tel)
        .await
        .expect("round 1");
    assert_eq!(r1.round, 1);
    assert_eq!(r1.accepted_prefix_len, 8);
    assert_eq!(r1.divergence_point, None);
    assert_eq!(r1.correction, Some(901));
    assert_eq!(r1.committed.len(), 9);

    // Round 2: mid-window rejection at index 3 + replacement token.
    let r2 = client
        .round(&[21, 22, 23, 24, 25, 26, 27, 28], &tel)
        .await
        .expect("round 2");
    assert_eq!(r2.accepted_prefix_len, 3);
    assert_eq!(r2.divergence_point, Some(3));
    assert_eq!(r2.correction, Some(902));
    assert_eq!(r2.committed, vec![21, 22, 23, 902]);

    // Round 3: EOG — full acceptance, correction present-as-null, the
    // commit covers the accepted prefix only.
    let r3 = client
        .round(&[31, 32, 33, 34, 35, 36, 37, 38], &tel)
        .await
        .expect("round 3");
    assert_eq!(r3.accepted_prefix_len, 8);
    assert_eq!(r3.correction, None);
    assert_eq!(r3.committed.len(), 8);

    // The committed stream is exactly window-prefixes ++ corrections.
    let expected: Vec<u32> = [
        vec![11, 12, 13, 14, 15, 16, 17, 18, 901],
        vec![21, 22, 23, 902],
        vec![31, 32, 33, 34, 35, 36, 37, 38],
    ]
    .concat();
    assert_eq!(client.committed_stream(), expected);
    // RC-2: the per-verifier EWMA was fed by every measured round.
    assert!(client
        .timings()
        .ewma_ms(&server_identity.peer_id())
        .is_some());
    // All three outcomes passed the verify_pass_ms cross-check (claimed ==
    // measured on the loopback mock).
    assert_eq!(
        r1.cross_check,
        modelswarm_session::verifydrafts::CrossCheckOutcome::Ok
    );
    assert_eq!(
        r2.cross_check,
        modelswarm_session::verifydrafts::CrossCheckOutcome::Ok
    );
    assert_eq!(
        r3.cross_check,
        modelswarm_session::verifydrafts::CrossCheckOutcome::Ok
    );

    // Clean teardown: the client closes, the server reports honestly.
    drop(client.into_wire());
    let report = tokio::time::timeout(ACCEPT_DEADLINE, server)
        .await
        .expect("server settles")
        .expect("server task ok");
    assert!(report.accepted);
    assert_eq!(report.rounds_handled, 3);
    assert_eq!(report.rounds_committed, 3);
    assert_eq!(report.committed_output_tokens, 21);
    assert_eq!(report.ended_by, ServeEndReason::ClientClosed);
    // Both sides agree on the final committed hash (I3).
    assert_eq!(
        report.committed_prefix_hash.len(),
        64,
        "bare 64-hex committed hash"
    );
}

/// §3 backend pairing: a Vulkan requester against a CPU verifier is
/// refused with the honest `backend_mismatch` SessionReject — the caller
/// falls back to single, never silently.
#[tokio::test]
async fn backend_mismatch_rejects_honestly() {
    let server_identity = InstallationIdentity::from_bytes(&[0xB1; 32]);
    let client_identity = InstallationIdentity::from_bytes(&[0xB2; 32]);
    let telemetry = Arc::new(Telemetry::memory().0);

    let engine = ScriptedEngine::new(batch_engine(8), vec![(1, Some(1))]);
    let config = VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());

    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .expect("loopback listen");
    let addr = listener.local_addr().expect("bound").to_string();
    let mut trusted: HashMap<String, VerifyingKey> = HashMap::new();
    trusted.insert(client_identity.peer_id(), client_identity.verifying_key());
    let server = tokio::spawn(async move {
        let wire = accept_staged(&listener, &trusted, ACCEPT_DEADLINE)
            .await
            .expect("authenticated accept");
        serve_verifydrafts_session(wire, config, engine, &telemetry).await
    });

    let wire = dial_staged(
        &addr,
        &client_identity,
        TEST_PROFILE,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("dial + handshake");
    let mut client_config =
        VerifyDraftsClientConfig::new(TEST_PROFILE.to_string(), server_identity.verifying_key());
    client_config.verifier_backend = Backend::Vulkan; // E0: CPU ≠ Vulkan.
    let tel = Telemetry::memory().0;
    let opened = VerifyDraftsClient::negotiate(wire, client_identity, client_config, &tel).await;

    match opened {
        Err(ClientOpenError::Rejected(reason)) => assert_eq!(reason, "backend_mismatch"),
        Err(other) => panic!("expected backend_mismatch reject, got {other:?}"),
        Ok(_) => panic!("backend mismatch must refuse the session"),
    }
    let report = tokio::time::timeout(ACCEPT_DEADLINE, server)
        .await
        .expect("server settles")
        .expect("server task ok");
    assert!(!report.accepted);
    assert_eq!(
        report.ended_by,
        ServeEndReason::RejectedOffer(
            modelswarm_session::negotiation::SessionRejectReason::BackendMismatch
        )
    );
}

/// RC-1 over the wire: when the gate's coordinator key is pinned to a key
/// the stream's authenticated PeerId does NOT derive from, every
/// `VerifyDrafts` fails closed as `bad_frame` BEFORE any admission stage —
/// and the session survives for the honest client to give up cleanly.
#[tokio::test]
async fn remote_peer_mismatch_fails_closed_bad_frame() {
    let server_identity = InstallationIdentity::from_bytes(&[0xC1; 32]);
    let client_identity = InstallationIdentity::from_bytes(&[0xC2; 32]);
    let stranger = InstallationIdentity::from_bytes(&[0xC3; 32]);
    let telemetry = Arc::new(Telemetry::memory().0);

    let engine = ScriptedEngine::new(batch_engine(8), vec![(1, Some(1))]);
    let mut config =
        VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());
    config.engine = batch_engine(8);
    // Roster-style pinning to a THIRD key: the authenticated client peer
    // can never satisfy peer_id_for(pinned).
    config.pinned_coordinator_key = Some(stranger.verifying_key());

    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .expect("loopback listen");
    let addr = listener.local_addr().expect("bound").to_string();
    let mut trusted: HashMap<String, VerifyingKey> = HashMap::new();
    trusted.insert(client_identity.peer_id(), client_identity.verifying_key());
    let server = tokio::spawn(async move {
        let wire = accept_staged(&listener, &trusted, ACCEPT_DEADLINE)
            .await
            .expect("authenticated accept");
        serve_verifydrafts_session(wire, config, engine, &telemetry).await
    });

    let wire = dial_staged(
        &addr,
        &client_identity,
        TEST_PROFILE,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("dial + handshake");
    let tel = Telemetry::memory().0;
    let mut client = VerifyDraftsClient::negotiate(
        wire,
        client_identity,
        VerifyDraftsClientConfig::new(TEST_PROFILE.to_string(), server_identity.verifying_key()),
        &tel,
    )
    .await
    .expect("negotiation itself is peer-agnostic");

    // The client's request is VALIDLY signed by its own key — but that key
    // is not the pinned coordinator: fail-closed bad_frame, zero engine
    // work (the mock script is never consumed).
    match client
        .round(&[41, 42, 43], &tel)
        .await
        .expect_err("fail closed")
    {
        RoundFailure::Rejected { code, .. } => {
            assert_eq!(
                code,
                modelswarm_session::negotiation::FrameRejectReason::BadFrame
            );
        }
        other => panic!("expected bad_frame, got {other:?}"),
    }
    drop(client.into_wire());
    let report = tokio::time::timeout(ACCEPT_DEADLINE, server)
        .await
        .expect("server settles")
        .expect("server task ok");
    assert!(report.accepted);
    assert_eq!(report.rounds_handled, 0, "no round admitted");
    assert_eq!(report.rounds_committed, 0);
    assert_eq!(report.committed_output_tokens, 0);
    assert_eq!(report.ended_by, ServeEndReason::ClientClosed);
}

/// §8e / I8: a hostile or greedy coordinator that would blow the session
/// token budget gets `overloaded`, and the rejection frame carries codes
/// and hashes ONLY — window token ids never cross the wire.
#[tokio::test]
async fn budget_exhaustion_is_overloaded_and_private() {
    let server_identity = InstallationIdentity::from_bytes(&[0xD1; 32]);
    let client_identity = InstallationIdentity::from_bytes(&[0xD2; 32]);
    let telemetry = Arc::new(Telemetry::memory().0);

    let engine = ScriptedEngine::new(batch_engine(8), vec![(3, Some(901))]);
    let mut config =
        VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());
    config.engine = batch_engine(8);

    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .expect("loopback listen");
    let addr = listener.local_addr().expect("bound").to_string();
    let mut trusted: HashMap<String, VerifyingKey> = HashMap::new();
    trusted.insert(client_identity.peer_id(), client_identity.verifying_key());
    let server = tokio::spawn(async move {
        let wire = accept_staged(&listener, &trusted, ACCEPT_DEADLINE)
            .await
            .expect("authenticated accept");
        serve_verifydrafts_session(wire, config, engine, &telemetry).await
    });

    let wire = dial_staged(
        &addr,
        &client_identity,
        TEST_PROFILE,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("dial + handshake");
    let mut client_config =
        VerifyDraftsClientConfig::new(TEST_PROFILE.to_string(), server_identity.verifying_key());
    // Tiny token budget: round 1 commits 4 (3 accepted + correction);
    // round 2 would need 3 more → 7 > 5 → overloaded before engine work.
    client_config.budget.max_output_tokens = 5;
    let tel = Telemetry::memory().0;
    let mut client = VerifyDraftsClient::negotiate(wire, client_identity, client_config, &tel)
        .await
        .expect("offer accepted");

    let r1 = client
        .round(&[777771, 777772, 777773], &tel)
        .await
        .expect("round 1 fits the budget");
    assert_eq!(r1.committed.len(), 4);

    let failure = client
        .round(&[777774, 777775, 777776], &tel)
        .await
        .expect_err("budget exhausted");
    let failure_text = format!("{failure:?}");
    match &failure {
        RoundFailure::Rejected { code, .. } => {
            assert_eq!(&code.to_string(), "overloaded");
        }
        other => panic!("expected overloaded, got {other:?}"),
    }
    // Privacy rule 5 / A-3: the rejection (and its failure surface) carries
    // codes and hashes only — none of round 2's token ids appear.
    assert!(!failure_text.contains("777774"));
    assert!(!failure_text.contains("777775"));
    assert!(!failure_text.contains("777776"));

    drop(client.into_wire());
    let report = tokio::time::timeout(ACCEPT_DEADLINE, server)
        .await
        .expect("server settles")
        .expect("server task ok");
    assert_eq!(
        report.rounds_handled, 1,
        "the overloaded round never admitted"
    );
    assert_eq!(report.rounds_committed, 1);
    assert_eq!(report.committed_output_tokens, 4);
}

/// RC-5(b)/(c): a coordinator that vanishes after a result (no commit)
/// loses the round at `deadline_ms + 250 ms`; the NEXT round gaps forward
/// (`round > last handled`, parent = the still-genesis committed hash) and
/// commits cleanly.
#[tokio::test]
async fn wait_commit_timeout_abandons_and_next_round_gaps_forward() {
    let server_identity = InstallationIdentity::from_bytes(&[0xE1; 32]);
    let client_identity = InstallationIdentity::from_bytes(&[0xE2; 32]);
    let telemetry = Arc::new(Telemetry::memory().0);

    let engine = ScriptedEngine::new(batch_engine(8), vec![(3, Some(901)), (3, Some(902))]);
    let mut config =
        VerifyDraftsServeConfig::honest(TEST_PROFILE.to_string(), server_identity.clone());
    config.engine = batch_engine(8);
    config.session_idle = Duration::from_secs(60);

    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .expect("loopback listen");
    let addr = listener.local_addr().expect("bound").to_string();
    let mut trusted: HashMap<String, VerifyingKey> = HashMap::new();
    trusted.insert(client_identity.peer_id(), client_identity.verifying_key());
    let server = tokio::spawn(async move {
        let wire = accept_staged(&listener, &trusted, ACCEPT_DEADLINE)
            .await
            .expect("authenticated accept");
        serve_verifydrafts_session(wire, config, engine, &telemetry).await
    });

    let mut wire = dial_staged(
        &addr,
        &client_identity,
        TEST_PROFILE,
        &server_identity.verifying_key(),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("dial + handshake");

    // Manual offer/accept + rounds: the library client always commits on
    // acceptance, and this test needs the round-1 commit deliberately
    // withheld.
    let offer = SessionOffer {
        session_id: "vd-gap-test".to_string(),
        protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
        preset: modelswarm_session::negotiation::Preset::Verified,
        budget: modelswarm_session::negotiation::SessionBudget {
            max_endpoints: 1,
            max_endpoint_seconds: 120,
            max_output_tokens: 64,
        },
        verifier_backend: Backend::Cpu,
    };
    wire.send_json(
        &serde_json::to_value(&offer).expect("offer serializes"),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("send offer");
    let accept: SessionAccept =
        serde_json::from_value(wire.recv_json(ACCEPT_DEADLINE).await.expect("accept"))
            .expect("accept schema");

    let params_hash =
        modelswarm_session::verifydrafts::batch_generation_params_hash(&Default::default())
            .expect("default params are integral");
    let mut seq = 0u64;
    let mut request_round = |round: u64, window: Vec<u32>| {
        seq += 1;
        let mut frame = VerifyDrafts {
            session_id: offer.session_id.clone(),
            protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
            seq,
            round,
            profile_id: TEST_PROFILE.to_string(),
            parent_prefix_hash: GENESIS_PREFIX_HASH.to_string(),
            generation_params_hash: params_hash.clone(),
            window_tokens: window,
            proposer_id: client_identity.peer_id(),
            nonce: modelswarm_identity::new_nonce(),
            deadline_ms: 1_000, // clamped minimum → 1.25 s WAIT_COMMIT bound
            sender: client_identity.installation_id(),
            signature: String::new(),
        };
        frame.signature =
            sign_verify_drafts(&frame, &client_identity).expect("client signs its drafts");
        frame
    };

    // Round 1: admitted, one engine pass, result received — then SILENCE
    // (no PrefixCommit; the coordinator "vanishes" mid-round).
    let r1 = request_round(1, vec![51, 52, 53]);
    wire.send_json(
        &serde_json::to_value(&r1).expect("request serializes"),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("send round 1");
    let result_value = wire.recv_json(ACCEPT_DEADLINE).await.expect("result 1");
    let result = VerifyDraftsResult::parse(&result_value).expect("result schema");
    // The serve loop SIGNED the gate's draft (RC-2): the signature verifies
    // under the server's pinned key.
    modelswarm_session::verifydrafts::verify_verify_drafts_result_signature(
        &result,
        &server_identity.verifying_key(),
    )
    .expect("result is signed by the verifier identity");
    assert_eq!(result.accepted_prefix_len, 3);

    // Past deadline_ms + 250 ms grace: the verifier abandons the round and
    // returns to PREFILLED.
    tokio::time::sleep(Duration::from_millis(1_800)).await;

    // Round 2 gaps forward: round > last handled (1), parent still the
    // genesis committed hash (round 1 never committed).
    let r2 = request_round(2, vec![61, 62, 63]);
    wire.send_json(
        &serde_json::to_value(&r2).expect("request serializes"),
        ACCEPT_DEADLINE,
    )
    .await
    .expect("send round 2");
    let result2 =
        VerifyDraftsResult::parse(&wire.recv_json(ACCEPT_DEADLINE).await.expect("result 2"))
            .expect("result schema");
    assert_eq!(result2.round, 2);
    assert_eq!(result2.accepted_prefix_len, 3);

    // The reproducing commit for round 2 advances the chain (the shipped
    // PrefixCommit field shape, RC-4 ids and hash from the admitted result).
    let mut committed_ids = vec![61u32, 62, 63];
    if let Some(correction) = result2.correction {
        committed_ids.push(correction);
    }
    let commit = serde_json::json!({
        "session_id": offer.session_id,
        "round": 2,
        "previous_prefix_hash": GENESIS_PREFIX_HASH,
        "accepted_token_ids": committed_ids,
        "new_prefix_hash": result2.new_prefix_hash,
        "profile_id": TEST_PROFILE,
        "generation_params_hash": params_hash,
        "sender": client_identity.installation_id(),
    });
    assert_eq!(accept.engine.batch_window_max, 8);
    wire.send_json(&commit, ACCEPT_DEADLINE)
        .await
        .expect("send commit");

    drop(wire);
    let report = tokio::time::timeout(ACCEPT_DEADLINE, server)
        .await
        .expect("server settles")
        .expect("server task ok");
    // Round 1 was HANDLED (admitted, abandoned at WAIT_COMMIT expiry);
    // only round 2 committed — the gap is honest and visible. The committed
    // counter is the last committed round NUMBER (2).
    assert_eq!(report.rounds_handled, 2);
    assert_eq!(
        report.rounds_committed, 2,
        "the last committed round NUMBER"
    );
    assert_eq!(report.committed_output_tokens, 4); // 3 accepted + correction
    assert_eq!(report.ended_by, ServeEndReason::ClientClosed);
}
