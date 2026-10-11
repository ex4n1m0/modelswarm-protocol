//! Security-review pin tests — post-implementation review of the landed
//! VerifyDrafts family (ADR-032 §7 checklist item 8; review document
//! `docs/reviews/review-verifydrafts-implementation-security-2026-10-11.md`).
//!
//! These tests PIN current implementation behavior that the review found
//! behavior-determining but not explicitly pinned by the ADR or the existing
//! property suites. They change NO production behavior; they freeze the
//! behavior so any later amendment is a deliberate, review-visible change
//! (the repo's own precedent: session 3 pinned the over-constrained
//! `deadline-past` fixture's literal-seeding classification before the
//! session-4 fixture repair).
//!
//! Pins carried here:
//! 1. The coordinator-side drop-vs-violation partition of
//!    [`VerifyDraftsCoordinator::admit_result`] (review advisory A2): the
//!    consumed-nonce and no-outstanding drops run AFTER the signature and
//!    remote-peer checks, so a GENUINE replay drops with no state change
//!    while a frame that fails signature/remote-peer counts a violation
//!    even when it carries a consumed nonce or no round is outstanding
//!    (session-4 handoff assumption 3).
//! 2. The verifier-side treatment of a LATE `PrefixCommit` after the
//!    WAIT_COMMIT grace has expired (review advisory A1): the reproducing
//!    commit is classified `CancelRound(commit_mismatch)` and counted as a
//!    protocol violation, and the SECOND such commit by the same peer
//!    tears the session down. The ADR pins the threshold-two rule for
//!    coordinator-side results only; the verifier-side counting policy is
//!    the implementation's (recorded, unpinned) choice — flagged in the
//!    review for the owning session/architect to confirm or amend.

use modelswarm_identity::InstallationIdentity;
use modelswarm_runtime::SamplingParams;
use modelswarm_session::negotiation::{
    Backend, CancelRoundReason, EngineDisclosure, SessionBudget,
};
use modelswarm_session::verifydrafts::{
    batch_generation_params_hash, sign_verify_drafts, sign_verify_drafts_result, AdmissionDecision,
    BatchVerifyEngine, CancelRoundFrame, CommitApplyOutcome, DispatchOutcome, ResultAdmission,
    ResultDrop, ResultViolation, VerifyDrafts, VerifyDraftsCoordinator,
    VerifyDraftsCoordinatorConfig, VerifyDraftsGate, VerifyDraftsGateConfig, VerifyDraftsResult,
    TEARDOWN_VIOLATION_THRESHOLD, WINDOW_CAP,
};
use modelswarm_session::{compute_prefix_hash, CommitPrefix, GENESIS_PREFIX_HASH};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 2026-10-12T00:00:00Z (the shared fixture anchor).
const NOW: u64 = 1_791_763_200_000;
const SESSION: &str = "sess-sec-review";
const WINDOW: [u32; 3] = [9_600_001, 9_600_002, 9_600_003];

fn fixture_id(seed: u8) -> InstallationIdentity {
    InstallationIdentity::from_bytes(&[seed; 32])
}

fn profile() -> String {
    format!("msp1:{}", "ab".repeat(32))
}

fn params_hash() -> String {
    batch_generation_params_hash(&SamplingParams::default()).unwrap()
}

// ---------------------------------------------------------------------------
// Shared builders (minimal; self-contained, not shared with the property
// suites so a suite edit cannot silently move these pins)
// ---------------------------------------------------------------------------

fn build_request(
    coordinator: &InstallationIdentity,
    seq: u64,
    round: u64,
    nonce: &str,
) -> VerifyDrafts {
    let mut msg = VerifyDrafts {
        session_id: SESSION.to_string(),
        protocol_version: "msp-cooperative-v1".to_string(),
        seq,
        round,
        profile_id: profile(),
        parent_prefix_hash: GENESIS_PREFIX_HASH.to_string(),
        generation_params_hash: params_hash(),
        window_tokens: WINDOW.to_vec(),
        proposer_id: "12D3KooProposer".to_string(),
        nonce: nonce.to_string(),
        deadline_ms: 2_500,
        sender: coordinator.installation_id(),
        signature: String::new(),
    };
    msg.signature = sign_verify_drafts(&msg, coordinator).unwrap();
    msg
}

fn result_draft(
    verifier: &InstallationIdentity,
    req: &VerifyDrafts,
    seq: u64,
    accepted: u32,
    correction: Option<u32>,
    new_prefix_hash: String,
) -> VerifyDraftsResult {
    let divergence = (accepted < req.window_tokens.len() as u32).then_some(accepted);
    let mut msg = VerifyDraftsResult {
        session_id: req.session_id.clone(),
        protocol_version: "msp-cooperative-v1".to_string(),
        seq,
        round: req.round,
        accepted_prefix_len: accepted,
        divergence_point: divergence,
        correction,
        new_prefix_hash,
        verify_pass_ms: 35,
        nonce_echo: req.nonce.clone(),
        sender: verifier.installation_id(),
        signature: String::new(),
    };
    msg.signature = sign_verify_drafts_result(&msg, verifier).unwrap();
    msg
}

/// An internally-consistent (honest-shape) result for `req`.
fn honest_result(
    verifier: &InstallationIdentity,
    req: &VerifyDrafts,
    seq: u64,
) -> VerifyDraftsResult {
    let mut committed = req.window_tokens.to_vec();
    committed.push(999);
    let hash = compute_prefix_hash(&req.parent_prefix_hash, &committed).unwrap();
    result_draft(
        verifier,
        req,
        seq,
        req.window_tokens.len() as u32,
        Some(999),
        hash,
    )
}

struct MockEngine {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl BatchVerifyEngine for MockEngine {
    async fn verify_window(
        &self,
        window: &[u32],
        _params: &SamplingParams,
    ) -> Result<(u32, Option<u32>), modelswarm_runtime::RuntimeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((window.len() as u32, Some(999)))
    }
    fn capabilities(&self) -> EngineDisclosure {
        EngineDisclosure {
            backend: Backend::Cpu,
            batch_verify: true,
            batch_window_max: WINDOW_CAP,
        }
    }
}

fn gate(coordinator: &InstallationIdentity) -> VerifyDraftsGate {
    VerifyDraftsGate::new(VerifyDraftsGateConfig {
        session_id: SESSION.to_string(),
        profile_id: profile(),
        generation_params_hash: params_hash(),
        committed_prefix_hash: GENESIS_PREFIX_HASH.to_string(),
        coordinator_key: coordinator.verifying_key(),
        verifier_installation_id: fixture_id(0x22).installation_id(),
        engine: EngineDisclosure {
            backend: Backend::Cpu,
            batch_verify: true,
            batch_window_max: WINDOW_CAP,
        },
        budget: SessionBudget {
            max_endpoints: 1,
            max_endpoint_seconds: 120,
            max_output_tokens: 10_000,
        },
        lease_expires_at_ms: NOW + 3_600_000,
        sampling_params: SamplingParams::default(),
        verifier_next_seq: 4,
    })
    .unwrap()
}

// ---------------------------------------------------------------------------
// Pin 1 — coordinator drop-vs-violation partition (review advisory A2)
// ---------------------------------------------------------------------------

#[test]
fn pin_coordinator_replay_partition_genuine_drop_garbage_violation() {
    assert_eq!(TEARDOWN_VIOLATION_THRESHOLD, 2);
    let coordinator_id = fixture_id(0x11);
    let verifier = fixture_id(0x22);
    let request = build_request(&coordinator_id, 12, 3, "pin-nonce");
    let mut coord = VerifyDraftsCoordinator::new(VerifyDraftsCoordinatorConfig {
        session_id: SESSION.to_string(),
        profile_id: profile(),
        generation_params_hash: params_hash(),
        verifier_key: verifier.verifying_key(),
    })
    .unwrap();
    coord.track_request(&request).unwrap();

    let wire = |msg: &VerifyDraftsResult| -> Value { serde_json::to_value(msg).unwrap() };

    let result = honest_result(&verifier, &request, 4);
    assert_eq!(
        coord.admit_result(&wire(&result), &verifier.peer_id()),
        ResultAdmission::Accepted {
            committed: vec![9_600_001, 9_600_002, 9_600_003, 999]
        }
    );

    // (1) A replayed GENUINE result (consumed nonce, valid signature,
    // right peer) is dropped with NO state change — RC-3 tail.
    assert_eq!(
        coord.admit_result(&wire(&result), &verifier.peer_id()),
        ResultAdmission::Dropped(ResultDrop::ConsumedNonce)
    );
    assert_eq!(coord.violations_of(&verifier.peer_id()), 0);

    // (2) A frame carrying the CONSUMED nonce that fails the signature is
    // a VIOLATION, not a drop: the signature check precedes the
    // consumed-nonce drop (session-4 handoff assumption 3).
    let mut garbage = result.clone();
    garbage.signature = "AAAAnot-base64".to_string();
    assert_eq!(
        coord.admit_result(&wire(&garbage), &verifier.peer_id()),
        ResultAdmission::Violation(ResultViolation::Signature)
    );
    assert_eq!(coord.violations_of(&verifier.peer_id()), 1);
    assert!(
        !coord.is_torn_down(),
        "the FIRST violation never tears down"
    );

    // (3) A VALIDLY-signed result with no outstanding request and a fresh
    // nonce is a drop, not a violation.
    let mut orphan = honest_result(&verifier, &request, 5);
    orphan.nonce_echo = "fresh-unconsumed".to_string();
    orphan.signature = sign_verify_drafts_result(&orphan, &verifier).unwrap();
    assert_eq!(
        coord.admit_result(&wire(&orphan), &verifier.peer_id()),
        ResultAdmission::Dropped(ResultDrop::NoOutstandingRequest)
    );
    assert_eq!(
        coord.violations_of(&verifier.peer_id()),
        1,
        "the no-outstanding drop changes no state"
    );

    // (4) A SECOND signature violation by the same peer tears down at
    // exactly two (the threshold pin, mirrored from the property suite).
    let mut garbage2 = result.clone();
    garbage2.nonce_echo = "fresh-unconsumed-2".to_string();
    garbage2.signature = "AAAAnot-base64".to_string();
    assert_eq!(
        coord.admit_result(&wire(&garbage2), &verifier.peer_id()),
        ResultAdmission::TornDown(ResultViolation::Signature)
    );
    assert!(coord.is_torn_down());
}

// ---------------------------------------------------------------------------
// Pin 2 — verifier-side LATE commit after WAIT_COMMIT expiry (review A1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pin_verifier_late_commit_after_expiry_is_commit_mismatch() {
    let coordinator = fixture_id(0x31);
    let engine = MockEngine {
        calls: AtomicUsize::new(0),
    };
    let mut gate = gate(&coordinator);
    gate.seed_pre_state(modelswarm_session::verifydrafts::GatePreStateSeed {
        last_seq: vec![(coordinator.installation_id(), 12)],
        last_handled_round: Some(1),
        ..Default::default()
    });

    let frame = build_request(&coordinator, 13, 2, "late-nonce");
    let AdmissionDecision::Admitted {
        deadline_expires_at_ms,
        ..
    } = gate.admit_verify_drafts(&serde_json::to_value(&frame).unwrap(), NOW, NOW)
    else {
        panic!("round must admit");
    };
    let DispatchOutcome::Dispatched(result) =
        gate.dispatch_outstanding(&engine, NOW, NOW + 35).await
    else {
        panic!("dispatch must succeed");
    };

    // WAIT_COMMIT grace expiry abandons the round and discards the result.
    assert!(gate.expire_wait_commit_if_due(deadline_expires_at_ms + 250));
    assert_eq!(gate.outstanding_round(), None);

    // The late REPRODUCING commit (it exactly matches the discarded
    // result) is classified commit_mismatch and counts ONE violation —
    // current implementation behavior, flagged as review advisory A1
    // (unpinned ADR policy; an honest-but-slow coordinator is counted).
    let mut committed = frame.window_tokens.to_vec();
    committed.push(999);
    let late = CommitPrefix {
        session_id: SESSION.to_string(),
        round: 2,
        previous_prefix_hash: GENESIS_PREFIX_HASH.to_string(),
        accepted_token_ids: committed,
        new_prefix_hash: result.new_prefix_hash.clone(),
        model_profile_id: profile(),
        generation_params_hash: params_hash(),
        sender: coordinator.installation_id(),
    };
    assert_eq!(
        gate.apply_prefix_commit(&late),
        CommitApplyOutcome::CancelRound(CancelRoundFrame {
            session_id: SESSION.to_string(),
            round: 2,
            reason: CancelRoundReason::CommitMismatch,
        })
    );
    assert_eq!(gate.violations_of(&coordinator.installation_id()), 1);
    assert_eq!(gate.committed_prefix_hash(), GENESIS_PREFIX_HASH);
    assert!(!gate.is_torn_down(), "first late commit does not tear down");

    // A SECOND late commit by the same peer tears the session down at the
    // threshold (the implementation's verifier-side policy, unpinned by
    // the ADR — see review advisory A1 for the requested ruling).
    assert_eq!(
        gate.apply_prefix_commit(&late),
        CommitApplyOutcome::CancelRound(CancelRoundFrame {
            session_id: SESSION.to_string(),
            round: 2,
            reason: CancelRoundReason::CommitMismatch,
        })
    );
    assert!(gate.is_torn_down());
    assert_eq!(gate.teardown_reason(), Some("commit_mismatch"));
}
