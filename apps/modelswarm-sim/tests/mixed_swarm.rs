//! ADR-032 §7 item 4b — the mixed-swarm fallback scenarios, pinned as
//! per-push sim tests (default features; execution-venue matrix row:
//! `apps/modelswarm-sim`, NOT bench quic-runner-only modules).
//!
//! These are DECISION-outcome tests: the negotiation runs through the
//! landed session-crate API and the engage decision through the landed
//! scheduler surface, so a verdict here fails if either contract changes
//! behavior — the sim asserts outcomes, never bench measurements.
//!
//! The four scenario families (plan
//! `docs/implementation/adr-032-verifydrafts-plan.md` §5.2 row 5):
//!
//! 1. HTTP-only cohorts never engage (the option-A wire-true term blocks).
//! 2. Adapter cohorts engage only when the predicted (1+margin) advantage
//!    beats the fastest eligible single; an honest `batch_window_max` can
//!    flip engage to fallback.
//! 3. Mixed swarms fall back to the fastest eligible single regardless of
//!    class — batch capability never overrides the comparator.
//! 4. Honesty edge: `batch_verify: false` peers are first-class plain
//!    singles; a backend-mismatched peer rejects with `backend_mismatch`
//!    and the requester continues with eligible peers (no silent drops).

use modelswarm_scheduler::{
    predicted_single_ms, should_engage_cooperative, Candidate, VerifyTerm,
    DEFAULT_CONFIDENCE_MARGIN,
};
use modelswarm_session::negotiation::{
    Backend, EngineDisclosure, Preset, SessionAccept, SessionBudget, SessionOffer, SessionReject,
    SessionRejectReason, COOPERATIVE_PROTOCOL_VERSION,
};
use modelswarm_session::verifydrafts::{
    eligible_batch_verifier, peer_reject_reason, validate_session_accept,
};
use modelswarm_sim::mixed_swarm::{
    pinned_mixed_cases, run_mixed, run_mixed_swarm_decision, MixedSwarmDecision, MixedSwarmSpec,
    PeerNegotiation, SimPeerSpec, MIXED_PROFILE, VERIFY_BATCH_STEPS,
};
use modelswarm_types::ModelProfileId;
use serde_json::json;

fn decision(case: &str) -> MixedSwarmDecision {
    let (spec, _) = pinned_mixed_cases()
        .into_iter()
        .find(|(s, _)| s.case == case)
        .unwrap_or_else(|| panic!("pinned case {case} missing"));
    run_mixed_swarm_decision(&spec).expect("decision runs")
}

/// Family 1: an HTTP-only cohort (every peer `batch_verify: false`, the
/// pinned llama.cpp HTTP adapter class) NEVER engages speculation — the
/// landed option-A wire-true term blocks it, mirroring the pass-2
/// never-engage conclusion (enforced, not advised). The swarm falls back
/// to the fastest eligible single, and the wire-true verification charge
/// alone structurally exceeds that single's whole decode budget.
#[test]
fn http_only_cohorts_never_engage_speculation() {
    let decision = decision("http-only-never-engages");
    assert_eq!(decision.verify_term, VerifyTerm::SequentialWire);
    assert!(!decision.engaged, "HTTP-only cohorts must never engage");
    assert_eq!(decision.fallback_peer.as_deref(), Some("http-1"));
    assert_eq!(decision.fastest_single_peer.as_deref(), Some("http-1"));
    // Every HTTP peer negotiated an honest accept and NONE is eligible as
    // a batch verifier (first-class singles, never fake verifiers).
    assert_eq!(decision.negotiation.len(), 3);
    for entry in &decision.negotiation {
        match entry {
            PeerNegotiation::Accepted {
                peer_id,
                batch_verify,
                batch_verifier_eligible,
                ..
            } => {
                assert!(!*batch_verify, "{peer_id}: HTTP class");
                assert!(
                    !*batch_verifier_eligible,
                    "{peer_id}: an HTTP peer is never a batch verifier"
                );
            }
            other => panic!("HTTP-only roster must not reject: {other:?}"),
        }
    }
    // The structural never-engage fact, asserted on the decision's own
    // numbers: wire verification alone exceeds the single's decode budget.
    let wire_verify_ms = f64::from(decision.rounds)
        * VerifyTerm::SequentialWire
            .decode_steps_per_round(decision.effective_window, VERIFY_BATCH_STEPS)
        / 0.10; // http-1's measured decode rate (tokens/ms)
    let single_decode_ms = 16.0 / 0.10;
    assert!(
        wire_verify_ms > single_decode_ms,
        "wire verify {wire_verify_ms} must exceed the single decode budget {single_decode_ms}"
    );
    // And the full wire prediction loses outright, before any margin.
    let predicted = decision.predicted_swarm_ms.expect("prediction present");
    assert!(
        predicted > decision.fastest_single_ms.expect("fastest present"),
        "wire prediction {predicted} must lose to the fastest single outright"
    );
    assert!(!should_engage_cooperative(
        predicted,
        decision.fastest_single_ms.unwrap(),
        DEFAULT_CONFIDENCE_MARGIN
    )
    .unwrap());
}

/// Family 2: adapter cohorts (`batch_verify: true`, honest
/// `batch_window_max`) engage ONLY when the predicted (1+margin)
/// advantage beats the fastest eligible single. Three pins: the
/// comfortable cohort engages; a faster single anywhere in the roster
/// flips it to fallback; an honest `batch_window_max` below the desired
/// window shrinks the effective window (more rounds) and flips an
/// otherwise-engaging cohort to fallback — the disclosure degrades the
/// prediction, never inflates it.
#[test]
fn adapter_cohorts_engage_only_when_prediction_beats_the_fastest_single() {
    // (a) Comfortable advantage: engages under the batch term.
    let engaged_case = decision("adapter-cohort-engages");
    assert_eq!(engaged_case.verify_term, VerifyTerm::Batch);
    assert!(
        engaged_case.engaged,
        "the batch-term prediction must engage"
    );
    assert_eq!(engaged_case.fallback_peer, None);
    assert_eq!(engaged_case.effective_window, 8);
    assert_eq!(engaged_case.rounds, 2);
    let predicted = engaged_case.predicted_swarm_ms.unwrap();
    let fastest = engaged_case.fastest_single_ms.unwrap();
    assert!(
        should_engage_cooperative(predicted, fastest, DEFAULT_CONFIDENCE_MARGIN).unwrap(),
        "prediction {predicted} vs single {fastest} must clear the margin"
    );

    // (b) A faster single exists (adapter class): falls back to it — the
    // comparator wins regardless of the cohort's capability.
    let faster = decision("adapter-faster-single-wins");
    assert_eq!(faster.verify_term, VerifyTerm::Batch);
    assert!(!faster.engaged);
    assert_eq!(faster.fallback_peer.as_deref(), Some("ada-fast"));
    assert!(
        faster.predicted_swarm_ms.unwrap() * (1.0 + DEFAULT_CONFIDENCE_MARGIN)
            > faster.fastest_single_ms.unwrap()
    );

    // (c) Honest batch_window_max = 4 caps the effective window to 4:
    // rounds double (2 -> 4), the prediction rises, and the SAME rates
    // that engaged at window_max 32 now fall back.
    let capped = decision("adapter-honest-window-cap-falls-back");
    assert_eq!(capped.verify_term, VerifyTerm::Batch);
    assert!(!capped.engaged, "the honest window cap must degrade engage");
    assert_eq!(capped.fallback_peer.as_deref(), Some("ada-cap"));
    assert_eq!(capped.effective_window, 4, "window capped by disclosure");
    assert_eq!(capped.rounds, 4, "smaller window => more rounds");
    assert_eq!(capped.cohort.first().map(String::as_str), Some("ada-cap"));
    assert!(
        capped.predicted_swarm_ms.unwrap() > engaged_case.predicted_swarm_ms.unwrap(),
        "the cap must raise the prediction, never lower it"
    );
    // Same peer rates as (a) — ONLY the disclosure differs.
    assert_eq!(
        capped.ranked_singles[0].predicted_single_ms,
        engaged_case.ranked_singles[0].predicted_single_ms
    );
}

/// Family 3: class-blind mixed swarms. When the fastest eligible single
/// wins the prediction — EITHER adapter class or HTTP class — the swarm
/// falls back to it; batch capability never overrides the comparator.
/// And the presence of an HTTP peer does not poison a cohort whose
/// verifier IS adapter-class (the HTTP peer rides as proposer).
#[test]
fn mixed_swarms_fall_back_to_the_fastest_single_regardless_of_class() {
    // (a) The fastest eligible single is an HTTP peer: the mixed cohort's
    // verifier is that peer (fastest-ranked), the cohort is charged the
    // sequential wire term, and the swarm falls back to the HTTP single —
    // although adapter peers that WOULD engage are in the roster.
    let http_wins = decision("mixed-http-fastest-single-wins");
    assert_eq!(http_wins.verify_term, VerifyTerm::SequentialWire);
    assert!(!http_wins.engaged);
    assert_eq!(http_wins.fallback_peer.as_deref(), Some("http-fast"));
    assert_eq!(http_wins.fastest_single_peer.as_deref(), Some("http-fast"));
    assert_eq!(
        http_wins.cohort.first().map(String::as_str),
        Some("http-fast")
    );

    // The twin roster WITHOUT the HTTP peer engages — proving it is the
    // class-blind comparator (not the mixedness) that blocks (a).
    let mut twin = MixedSwarmSpec::new("mixed-twin-adapter-only", Backend::Cpu);
    twin.peers = vec![
        SimPeerSpec::adapter("ada-1", Backend::Cpu, 32, 5.0, 0.0, 3.0, 0.10),
        SimPeerSpec::adapter("ada-2", Backend::Cpu, 32, 5.0, 0.0, 3.0, 0.08),
    ];
    let twin_decision = run_mixed_swarm_decision(&twin).expect("twin runs");
    assert!(
        twin_decision.engaged,
        "the adapter-only twin must engage — the HTTP single was the blocker"
    );
    assert_eq!(twin_decision.verify_term, VerifyTerm::Batch);
    // Class-blindness on the numbers: the comparator equaled the HTTP
    // peer's single prediction, strictly below every adapter's.
    assert!(http_wins.fastest_single_ms.unwrap() < twin_decision.fastest_single_ms.unwrap());

    // (b) An HTTP peer aboard a cohort whose verifier is adapter-class
    // does not block engagement (it may act as proposer — first-class,
    // never a batch verifier).
    let engages_with_http = decision("mixed-adapter-cohort-engages-with-http-proposer");
    assert_eq!(engages_with_http.verify_term, VerifyTerm::Batch);
    assert!(engages_with_http.engaged);
    assert_eq!(
        engages_with_http.cohort.first().map(String::as_str),
        Some("ada-1")
    );
    assert!(engages_with_http.cohort.contains(&"http-slow".to_string()));
    for entry in &engages_with_http.negotiation {
        if let PeerNegotiation::Accepted {
            peer_id,
            batch_verify,
            batch_verifier_eligible,
            ..
        } = entry
        {
            if peer_id == "http-slow" {
                assert!(!*batch_verify);
                assert!(
                    !*batch_verifier_eligible,
                    "HTTP proposer is never a batch verifier"
                );
            }
        }
    }

    // (c) The fastest eligible single is ADAPTER class: the batch-term
    // prediction still loses to it; fallback to it. Either class wins.
    let adapter_wins = decision("mixed-adapter-fastest-single-wins");
    assert_eq!(adapter_wins.verify_term, VerifyTerm::Batch);
    assert!(!adapter_wins.engaged);
    assert_eq!(adapter_wins.fallback_peer.as_deref(), Some("ada-fast"));
}

/// Family 4 (honesty edge): a peer declaring `batch_verify: false` is
/// selected as a plain single; a backend-mismatched peer answers
/// `SessionReject(backend_mismatch)` and the requester CONTINUES with the
/// eligible peers — no silent drops, no errors, and the mismatched peer
/// (whose measured facts are the best in the roster) is excluded from the
/// ranking, the cohort, and the comparator.
#[test]
fn honesty_edge_batch_false_single_and_backend_mismatch_continues() {
    let decision = decision("honesty-backend-mismatch-continues");

    // No silent drops: every roster peer appears in the negotiation
    // ledger with a decided outcome.
    assert_eq!(decision.negotiation.len(), 3);
    let rejected: Vec<&PeerNegotiation> = decision
        .negotiation
        .iter()
        .filter(|n| matches!(n, PeerNegotiation::Rejected { .. }))
        .collect();
    assert_eq!(rejected.len(), 1, "exactly the vulkan peer rejects");
    match rejected[0] {
        PeerNegotiation::Rejected { peer_id, reason } => {
            assert_eq!(peer_id, "ada-vulkan");
            assert_eq!(reason, &SessionRejectReason::BackendMismatch);
        }
        other => panic!("expected a reject, got {other:?}"),
    }

    // The mismatched peer had the BEST measured facts (decode 0.50) yet is
    // excluded everywhere — ranking, cohort, comparator.
    let ranked_ids: Vec<&str> = decision
        .ranked_singles
        .iter()
        .map(|e| e.peer_id.as_str())
        .collect();
    assert!(!ranked_ids.contains(&"ada-vulkan"));
    assert!(!decision.cohort.contains(&"ada-vulkan".to_string()));
    assert_ne!(decision.fastest_single_peer.as_deref(), Some("ada-vulkan"));

    // The requester CONTINUED: the decision completed over the survivors
    // and the fastest eligible single — the batch_verify:false peer — is
    // the plain-single fallback (a first-class single).
    assert!(!decision.engaged);
    assert_eq!(decision.fallback_peer.as_deref(), Some("http-best"));
    assert_eq!(decision.fastest_single_peer.as_deref(), Some("http-best"));
    assert_eq!(decision.fastest_single_ms, Some(120.0));

    // The underlying session-crate surfaces, exercised directly on the
    // same offer/accept pair: the mismatch rejects with backend_mismatch;
    // the batch_verify:false accept validates but is never a batch
    // verifier; the reject frame serializes the frozen reason string.
    let offer = SessionOffer {
        session_id: "sim-mixed-honesty-backend-mismatch-continues".to_string(),
        protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
        preset: Preset::Balanced,
        budget: SessionBudget {
            max_endpoints: 8,
            max_endpoint_seconds: 600,
            max_output_tokens: 16,
        },
        verifier_backend: Backend::Cpu,
        profile_id: MIXED_PROFILE.to_string(),
        generation_params_hash: modelswarm_session::verifydrafts::batch_generation_params_hash(
            &modelswarm_runtime::SamplingParams::default(),
        )
        .expect("default sampling params are integral"),
    };
    let vulkan_engine = EngineDisclosure {
        backend: Backend::Vulkan,
        batch_verify: true,
        batch_window_max: 8,
    };
    assert_eq!(
        peer_reject_reason(&offer, &vulkan_engine),
        Some(SessionRejectReason::BackendMismatch)
    );
    let reject = SessionReject {
        session_id: offer.session_id.clone(),
        protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
        reason: SessionRejectReason::BackendMismatch,
    };
    assert_eq!(
        serde_json::to_value(&reject).unwrap(),
        json!({
            "session_id": "sim-mixed-honesty-backend-mismatch-continues",
            "protocol_version": "msp-cooperative-v1",
            "reason": "backend_mismatch"
        })
    );
    let http_accept = SessionAccept {
        session_id: offer.session_id.clone(),
        protocol_version: offer.protocol_version.clone(),
        preset: offer.preset,
        budget: offer.budget.clone(),
        engine: EngineDisclosure {
            backend: Backend::Cpu,
            batch_verify: false,
            batch_window_max: 1,
        },
    };
    assert!(validate_session_accept(&offer, &http_accept).is_ok());
    assert!(
        !eligible_batch_verifier(&offer, &http_accept),
        "batch_verify: false is never a batch verifier"
    );
    let adapter_accept = SessionAccept {
        session_id: offer.session_id.clone(),
        protocol_version: offer.protocol_version.clone(),
        preset: offer.preset,
        budget: offer.budget.clone(),
        engine: EngineDisclosure {
            backend: Backend::Cpu,
            batch_verify: true,
            batch_window_max: 8,
        },
    };
    assert!(validate_session_accept(&offer, &adapter_accept).is_ok());
    assert!(eligible_batch_verifier(&offer, &adapter_accept));
}

/// The pinned matrix holds end-to-end, the CLI summary reports every
/// verdict green, decisions are deterministic, and the record round-trips
/// through JSON (machine-readable gate evidence).
#[tokio::test]
async fn pinned_matrix_verdicts_hold_and_run_mixed_is_ok() {
    let cases = pinned_mixed_cases();
    assert_eq!(cases.len(), 8, "the eight pinned ADR-032 item-4b cases");
    for (spec, expected) in &cases {
        let decision = run_mixed_swarm_decision(spec).expect("case runs");
        assert_eq!(
            decision.case, spec.case,
            "the record carries the case label"
        );
        assert_eq!(decision.engaged, expected.engaged, "case {}", spec.case);
        assert_eq!(
            decision.fallback_peer, expected.fallback_peer,
            "case {}",
            spec.case
        );
        assert_eq!(
            decision.verify_term, expected.verify_term,
            "case {}",
            spec.case
        );
    }

    // Determinism: the same spec decides identically.
    let (spec, _) = &cases[0];
    let a = run_mixed_swarm_decision(spec).unwrap();
    let b = run_mixed_swarm_decision(spec).unwrap();
    assert_eq!(a, b);

    // The record serializes (finite numbers only) and round-trips.
    let value = serde_json::to_value(&a).expect("serializes");
    let text = value.to_string();
    assert!(!text.contains("infinity") && !text.contains("NaN"));
    let back: MixedSwarmDecision = serde_json::from_value(value).expect("round-trips");
    assert_eq!(back, a);

    // The CLI summary is green across the whole matrix.
    let summary = run_mixed().await.expect("mixed summary");
    assert_eq!(summary["scenario"], "mixed");
    assert_eq!(summary["ok"], true, "every pinned verdict must hold");
    assert_eq!(
        summary["cases"].as_array().map(Vec::len),
        Some(8),
        "one row per pinned case"
    );
}

/// The sim's single-prediction mirror is the scheduler's own frozen
/// formula: the ranked entries equal `predicted_single_ms` over the
/// mapped candidates exactly (the sim never invents a comparator).
#[test]
fn single_ranking_is_the_frozen_scheduler_formula() {
    let decision = decision("mixed-http-fastest-single-wins");
    let profile = ModelProfileId::new(MIXED_PROFILE).unwrap();
    for entry in &decision.ranked_singles {
        let decode = if entry.peer_id == "http-fast" {
            0.16
        } else {
            0.10
        };
        let candidate = Candidate {
            peer_id: entry.peer_id.clone(),
            profile_id: profile.clone(),
            measured_rtt_ms: 5.0,
            advertised_queue_ms: 0.0,
            prefill_tokens_per_ms: 3.0,
            decode_tokens_per_ms: decode,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: 1,
            capacity_class: modelswarm_scheduler::CapacityClass::GpuMid,
            nat_path: modelswarm_scheduler::NatPath::Direct,
            batch_verify: entry.batch_verify,
        };
        assert_eq!(
            entry.predicted_single_ms,
            predicted_single_ms(&candidate, 153, 16),
            "{}",
            entry.peer_id
        );
    }
    // Ranking order is fastest-first with the deterministic tie-break.
    let mut sorted = decision.ranked_singles.clone();
    sorted.sort_by(|a, b| {
        a.predicted_single_ms
            .total_cmp(&b.predicted_single_ms)
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    assert_eq!(decision.ranked_singles, sorted);
}
