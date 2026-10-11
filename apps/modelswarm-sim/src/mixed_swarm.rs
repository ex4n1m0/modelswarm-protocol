//! Mixed-swarm fallback scenarios (ADR-032 §7 execution-venue matrix item
//! 4b — per-push, DEFAULT features, `apps/modelswarm-sim`, T/R-owned; NOT
//! bench quic-runner-only modules).
//!
//! This module models the REQUESTER-SIDE decision over a mixed swarm of
//! HTTP-adapter peers (`batch_verify: false` — the pinned llama.cpp HTTP
//! server, today's only production adapter) and C-API-adapter peers
//! (`batch_verify: true` with an honest `batch_window_max`), all hosting
//! the same exact profile P under the project rule (ADR-032 §4). It is a
//! DECISION-level scenario — no sockets, no engine, fully deterministic —
//! because the venue ruling pins the DECISION OUTCOME, not a measured run:
//!
//! - the contract surface for negotiation is the landed session crate
//!   (`SessionOffer`/`SessionAccept`/`SessionReject`, `EngineDisclosure`,
//!   [`validate_session_accept`], [`eligible_batch_verifier`],
//!   [`peer_reject_reason`]);
//! - the decision semantics REUSE the landed scheduler surface
//!   ([`select_microswarm`], [`predicted_single_ms`], [`VerifyTerm`],
//!   [`should_engage_cooperative`]) — the option-A wire-true term itself
//!   is imported, not re-implemented, so this sim cannot drift from the
//!   term that guards production;
//! - the engagement-pool arithmetic that turns the term into a predicted
//!   completion time ([`predicted_swarm_mirror_ms`]) is the sim's thin
//!   MIRROR of the landed scheduler-test shape (bench coefficients, the
//!   pass-2 measured wire costs) and is labeled as a mirror everywhere it
//!   is used. No claim-bearing performance number is produced here.
//!
//! Pinned verdicts (the four scenario families of
//! `docs/implementation/adr-032-verifydrafts-plan.md` §5.2 row 5):
//!
//! 1. **HTTP-only cohorts NEVER engage** — the wire-true term blocks (the
//!    pass-2 never-engage conclusion, enforced rather than advised).
//! 2. **Adapter cohorts engage ONLY when `predicted × (1 + margin)` beats
//!    the fastest eligible single**; an honest `batch_window_max` shrinks
//!    the effective window and can flip engage → fallback, never the
//!    reverse.
//! 3. **Mixed swarms are class-blind** — when the fastest eligible single
//!    (EITHER class) wins the prediction, the swarm falls back to it;
//!    batch capability never overrides the fastest-single comparator
//!    (ADR-032 §4).
//! 4. **Honesty edge** — a `batch_verify: false` peer is a first-class
//!    plain single; a backend-mismatched peer answers
//!    `SessionReject(backend_mismatch)` and the requester CONTINUES with
//!    the remaining eligible peers (no silent drops).

use anyhow::{ensure, Result};
use modelswarm_scheduler::{
    predicted_single_ms, select_microswarm, should_engage_cooperative, Candidate, CapacityClass,
    NatPath, VerifyTerm, DEFAULT_CONFIDENCE_MARGIN, MAX_STALE_ADVERTISEMENT_PENALTY_MS,
    MICROSWARM_CAP, MIN_CAPACITY_CLASS,
};
use modelswarm_session::negotiation::{
    Backend, EngineDisclosure, Preset, SessionAccept, SessionBudget, SessionOffer, SessionReject,
    SessionRejectReason, COOPERATIVE_PROTOCOL_VERSION,
};
use modelswarm_session::verifydrafts::{
    eligible_batch_verifier, peer_reject_reason, validate_session_accept, WINDOW_CAP,
};
use modelswarm_types::ModelProfileId;
use serde::{Deserialize, Serialize};

/// Profile id of the mixed-swarm scenarios. The frozen scheduler
/// [`Candidate`] carries the CATALOG-format [`ModelProfileId`]
/// (`msp:<family>:<quant>:<vN>`), while the transport-level sim scenarios
/// use the free-form [`crate::SIM_PROFILE`] string — the recorded
/// shadow-module type mismatch applies (see `shadow.rs`'s
/// "Recorded type mismatch" note); every mixed-swarm candidate maps to
/// this one id so the exact-profile filter stays structurally satisfied.
pub const MIXED_PROFILE: &str = "msp:sim-mock:q4_k_m:v1";

/// ADR-013 cost-model v2 ENGINE constant `VERIFY_BATCH_STEPS` (1.5 decode
/// steps per batched verify round). The constant lives with the
/// cost-model coefficients in the bench crate (Scheduler-owned); the sim
/// MIRRORS its value here because [`VerifyTerm::decode_steps_per_round`]
/// takes the coefficient as a caller parameter — this is a mirror, and a
/// bench-side change to the coefficient must be mirrored by hand (the
/// pinned verdicts below would fail loudly if the two ever disagreed
/// enough to flip a decision).
pub const VERIFY_BATCH_STEPS: f64 = 1.5;

// ---------------------------------------------------------------------------
// Scenario inputs
// ---------------------------------------------------------------------------

/// One roster peer of a mixed-swarm scenario: the engine disclosure it
/// would put in its `SessionAccept` plus the requester-measured facts the
/// frozen cost model consumes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SimPeerSpec {
    /// Deterministic label (also the scheduler tie-break).
    pub peer_id: String,
    /// The peer's honest engine disclosure (ADR-032 §3, RC-8).
    pub engine: EngineDisclosure,
    /// Requester-measured RTT EWMA (ms).
    pub measured_rtt_ms: f64,
    /// Peer-advertised queue backlog (ms; untrusted input).
    pub advertised_queue_ms: f64,
    /// Measured prefill throughput (tokens/ms).
    pub prefill_tokens_per_ms: f64,
    /// Measured decode throughput (tokens/ms).
    pub decode_tokens_per_ms: f64,
    /// Advertised free serving slots.
    pub free_slots: u32,
}

impl SimPeerSpec {
    /// An HTTP-adapter peer (`batch_verify: false`) — the pinned
    /// llama.cpp HTTP server cannot verify a window in one batched pass.
    /// `batch_window_max: 1` is the ADR-032 §3 guidance for incapable
    /// peers (any in-range value is schema-valid; the meaning attaches
    /// only to `batch_verify: true`). Such a peer keeps full single
    /// eligibility and may act as proposer.
    pub fn http(peer_id: &str, rtt: f64, queue: f64, prefill: f64, decode: f64) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            engine: EngineDisclosure {
                backend: Backend::Cpu,
                batch_verify: false,
                batch_window_max: 1,
            },
            measured_rtt_ms: rtt,
            advertised_queue_ms: queue,
            prefill_tokens_per_ms: prefill,
            decode_tokens_per_ms: decode,
            free_slots: 1,
        }
    }

    /// A C-API-adapter peer (`batch_verify: true`) with an honest
    /// `batch_window_max` — the ADR-031 in-process adapter class (or an
    /// equivalent batched-decode surface). No production peer may
    /// declare this yet; the sim models the cohort for the decision
    /// outcome only.
    pub fn adapter(
        peer_id: &str,
        backend: Backend,
        batch_window_max: u32,
        rtt: f64,
        queue: f64,
        prefill: f64,
        decode: f64,
    ) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            engine: EngineDisclosure {
                backend,
                batch_verify: true,
                batch_window_max,
            },
            measured_rtt_ms: rtt,
            advertised_queue_ms: queue,
            prefill_tokens_per_ms: prefill,
            decode_tokens_per_ms: decode,
            free_slots: 1,
        }
    }
}

/// One mixed-swarm decision scenario.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MixedSwarmSpec {
    /// Case label (recorded on the decision).
    pub case: String,
    /// The requester's own backend — the offer's `verifier_backend` (the
    /// project rule: the requester hosts exact profile P too).
    pub requester_backend: Backend,
    /// The roster, in tracker-lookup order.
    pub peers: Vec<SimPeerSpec>,
    /// Desired speculative window (adaptive sizing input).
    pub window: u32,
    /// Estimated prompt tokens.
    pub prompt_tokens: u32,
    /// Target output tokens.
    pub target_tokens: u32,
    /// Expected draft acceptance rate (0..=1) — the rollback model input.
    pub acceptance: f64,
    /// The proposer-class decode rate (tokens/ms) — the bench
    /// engagement-pool convention (the drafter is a rate, not a roster
    /// slot, matching the landed scheduler-test arithmetic).
    pub drafter_decode_tokens_per_ms: f64,
    /// Per-session failure-risk penalty (ms; bench coefficient).
    pub failure_risk_ms: f64,
    /// Engage-rule margin (must be ≥ the ADR-013 floor 0.05).
    pub margin: f64,
    /// Cohort size to select (`speculative_exact` minimum 2).
    pub cohort_size: usize,
}

impl MixedSwarmSpec {
    /// The landed pass-2 engagement-pool shape (the scheduler 4a test's
    /// arithmetic made the sim default): window 8, prompt 153, target 16,
    /// acceptance 1.0, drafter 4.0 tokens/ms, failure risk 5 ms, default
    /// margin, cohort of 2 (verifier + proposer).
    pub fn new(case: &str, requester_backend: Backend) -> Self {
        Self {
            case: case.to_string(),
            requester_backend,
            peers: Vec::new(),
            window: 8,
            prompt_tokens: 153,
            target_tokens: 16,
            acceptance: 1.0,
            drafter_decode_tokens_per_ms: 4.0,
            failure_risk_ms: 5.0,
            margin: DEFAULT_CONFIDENCE_MARGIN,
            cohort_size: 2,
        }
    }
}

// ---------------------------------------------------------------------------
// Decision record
// ---------------------------------------------------------------------------

/// How one roster peer answered the `SessionOffer` (the requester's
/// negotiation ledger — completeness here is the "no silent drops"
/// guarantee of scenario family 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PeerNegotiation {
    /// The peer accepted; the disclosure fields are recorded for the
    /// decision audit.
    Accepted {
        peer_id: String,
        batch_verify: bool,
        batch_window_max: u32,
        /// Whether this peer may serve as the family's BATCH verifier
        /// (valid disclosure + declared capability + backend pairing).
        batch_verifier_eligible: bool,
    },
    /// The peer honestly rejected; the requester continues with the rest.
    Rejected {
        peer_id: String,
        reason: SessionRejectReason,
    },
}

/// One row of the class-blind single ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SingleRankEntry {
    pub peer_id: String,
    pub predicted_single_ms: f64,
    /// The peer's declared batch-verify capability (class label only —
    /// it never influences this ranking).
    pub batch_verify: bool,
    /// The frozen selector's eligibility predicate.
    pub eligible: bool,
}

/// The full mixed-swarm decision record: negotiation ledger, single
/// ranking, cohort, engine-true verify term, prediction, verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedSwarmDecision {
    pub case: String,
    /// Every roster peer's negotiation outcome, in roster order.
    pub negotiation: Vec<PeerNegotiation>,
    /// All ACCEPTED peers ranked fastest-single first (deterministic
    /// `peer_id` tie-break).
    pub ranked_singles: Vec<SingleRankEntry>,
    /// The fastest ELIGIBLE single (class-blind comparator), if any.
    pub fastest_single_peer: Option<String>,
    pub fastest_single_ms: Option<f64>,
    /// The selected cohort, fastest first (cohort[0] = the verifier).
    pub cohort: Vec<String>,
    /// The engine-true verification term charged to the cohort (the
    /// landed option-A surface).
    pub verify_term: VerifyTerm,
    /// The window the coordinator may actually send: the desired window,
    /// capped by the verifier's honest `batch_window_max` and
    /// [`WINDOW_CAP`] when the batch family is in play.
    pub effective_window: u32,
    /// Expected verify rounds for the target under the acceptance model.
    pub rounds: u32,
    /// Predicted cooperative completion (ms) under the term — `None`
    /// when no cohort formed.
    pub predicted_swarm_ms: Option<f64>,
    /// The engage-rule verdict (ADR-013 frozen rule, engine-true term).
    pub engaged: bool,
    /// When not engaged: the fastest eligible single the swarm falls
    /// back to (the class-blind guarantee).
    pub fallback_peer: Option<String>,
}

// ---------------------------------------------------------------------------
// The mirror arithmetic
// ---------------------------------------------------------------------------

/// Predicted cooperative completion under the engine-true verify term —
/// the sim's MIRROR of the acting planner's engagement-pool arithmetic
/// (the landed scheduler-test shape; bench coefficients, pass-2 measured
/// wire costs). Pure arithmetic over labeled inputs:
///
/// ```text
/// rounds   = ceil(target / (1 + acceptance × window))
/// per-round: prefix re-post / prefill + window / drafter + rtt (sync)
/// verify   = rounds × term.decode_steps_per_round(window, 1.5) / decode
/// rollback = (1 − acceptance) × rounds × (window + 1) / decode
/// total    = rounds × per-round + verify + rollback + failure_risk
/// ```
///
/// The two term-dependent differences mirror the landed shape exactly:
/// the msp-v1 whole-request wire re-posts the prompt PLUS the expected
/// committed prefix (`window / 2`) every round, while the batch family
/// re-posts the prompt only (KV persists in-session); and the wire
/// charges `window + 1` sequential verifier decode steps per round where
/// a declared batch verifier charges the 1.5-step batch coefficient.
///
/// Cost inputs come from the spec (`drafter_decode_tokens_per_ms`,
/// `prompt_tokens`, `target_tokens`, `acceptance`, `failure_risk_ms`);
/// `window` is passed explicitly because it is the decision's
/// term-capped EFFECTIVE window, not the spec's desired one.
#[must_use]
pub fn predicted_swarm_mirror_ms(
    spec: &MixedSwarmSpec,
    verifier: &Candidate,
    term: VerifyTerm,
    window: u32,
) -> f64 {
    let per_round_commit = 1.0 + spec.acceptance * f64::from(window);
    let rounds = (f64::from(spec.target_tokens) / per_round_commit.max(1.0)).ceil();
    let repost_tokens = f64::from(spec.prompt_tokens)
        + if term == VerifyTerm::SequentialWire {
            f64::from(window) / 2.0
        } else {
            0.0
        };
    let verify_ms = rounds * term.decode_steps_per_round(window, VERIFY_BATCH_STEPS)
        / verifier.decode_tokens_per_ms;
    let rollback_ms =
        (1.0 - spec.acceptance) * rounds * f64::from(window + 1) / verifier.decode_tokens_per_ms;
    rounds
        * (repost_tokens / verifier.prefill_tokens_per_ms
            + f64::from(window) / spec.drafter_decode_tokens_per_ms
            + verifier.measured_rtt_ms)
        + verify_ms
        + rollback_ms
        + spec.failure_risk_ms
}

// ---------------------------------------------------------------------------
// The decision core
// ---------------------------------------------------------------------------

/// Runs one mixed-swarm decision: negotiate with every roster peer
/// (honest accept, or `SessionReject` per the landed peer-side reason),
/// map accepts to frozen-model candidates, rank the class-blind
/// fastest-single comparator, select the cohort, charge the engine-true
/// verify term, and apply the frozen engage rule — falling back to the
/// fastest eligible single whenever the prediction does not beat it by
/// the margin. Deterministic and pure (no I/O, no clock, no env).
///
/// # Errors
///
/// `anyhow` errors on malformed specs (acceptance outside 0..=1, window
/// 0, an invalid profile id, a dishonest `SessionAccept` that fails
/// [`validate_session_accept`], or a margin below the ADR-013 floor —
/// the scheduler rejects those rather than clamping).
pub fn run_mixed_swarm_decision(spec: &MixedSwarmSpec) -> Result<MixedSwarmDecision> {
    ensure!(
        (0.0..=1.0).contains(&spec.acceptance),
        "mixed[{}]: acceptance must be in 0.0..=1.0, got {}",
        spec.case,
        spec.acceptance
    );
    ensure!(
        spec.window >= 1,
        "mixed[{}]: window must be >= 1, got {}",
        spec.case,
        spec.window
    );
    ensure!(
        spec.cohort_size >= 1,
        "mixed[{}]: cohort_size must be >= 1",
        spec.case
    );
    let profile = ModelProfileId::new(MIXED_PROFILE)?;

    // 1. The offer (msp-cooperative-v1 §2 + the ADR-032 §3 additive
    //    requester backend; budget envelope generous — clamping is not
    //    this scenario's subject).
    let offer = SessionOffer {
        session_id: format!("sim-mixed-{}", spec.case),
        protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
        preset: Preset::Balanced,
        budget: SessionBudget {
            max_endpoints: MICROSWARM_CAP as u32,
            max_endpoint_seconds: 600,
            max_output_tokens: spec.target_tokens.max(1),
        },
        verifier_backend: spec.requester_backend,
    };

    // 2. Negotiate with EVERY roster peer. The peer-side honest-reject
    //    surface is the landed `peer_reject_reason` (backend pairing);
    //    every accept must pass the landed honesty validation. The
    //    ledger keeps rejected peers visible — the requester continues
    //    with the survivors and nothing is dropped silently.
    let mut negotiation = Vec::with_capacity(spec.peers.len());
    let mut accepted: Vec<&SimPeerSpec> = Vec::with_capacity(spec.peers.len());
    for peer in &spec.peers {
        match peer_reject_reason(&offer, &peer.engine) {
            Some(reason) => {
                // The wire frame the peer sends (A-4 shape) — built from
                // the real SessionReject type and consumed by the ledger.
                let reject = SessionReject {
                    session_id: offer.session_id.clone(),
                    protocol_version: offer.protocol_version.clone(),
                    reason,
                };
                negotiation.push(PeerNegotiation::Rejected {
                    peer_id: peer.peer_id.clone(),
                    reason: reject.reason,
                });
            }
            None => {
                let accept = SessionAccept {
                    session_id: offer.session_id.clone(),
                    protocol_version: offer.protocol_version.clone(),
                    preset: offer.preset,
                    budget: offer.budget.clone(),
                    engine: peer.engine.clone(),
                };
                validate_session_accept(&offer, &accept).map_err(|err| {
                    anyhow::anyhow!("mixed[{}]: dishonest accept: {err}", spec.case)
                })?;
                let batch_verifier_eligible = eligible_batch_verifier(&offer, &accept);
                negotiation.push(PeerNegotiation::Accepted {
                    peer_id: peer.peer_id.clone(),
                    batch_verify: peer.engine.batch_verify,
                    batch_window_max: peer.engine.batch_window_max,
                    batch_verifier_eligible,
                });
                accepted.push(peer);
            }
        }
    }

    // 3. Map accepted peers to frozen-model candidates. The ONLY
    //    capability field is the peer's own declared `batch_verify`
    //    (ADR-032 §4: misreporting degrades acceptance economics, never
    //    correctness).
    let candidates: Vec<Candidate> = accepted
        .iter()
        .map(|p| Candidate {
            peer_id: p.peer_id.clone(),
            profile_id: profile.clone(),
            measured_rtt_ms: p.measured_rtt_ms,
            advertised_queue_ms: p.advertised_queue_ms,
            prefill_tokens_per_ms: p.prefill_tokens_per_ms,
            decode_tokens_per_ms: p.decode_tokens_per_ms,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: p.free_slots,
            capacity_class: CapacityClass::GpuMid,
            nat_path: NatPath::Direct,
            batch_verify: p.engine.batch_verify,
        })
        .collect();

    // 4. Class-blind comparator: the fastest eligible SINGLE, regardless
    //    of adapter class (ADR-032 §4 — batch capability never overrides
    //    it). Ranking is the frozen formula with the deterministic
    //    peer-id tie-break.
    let mut ranked_singles: Vec<SingleRankEntry> = candidates
        .iter()
        .map(|c| SingleRankEntry {
            peer_id: c.peer_id.clone(),
            predicted_single_ms: predicted_single_ms(c, spec.prompt_tokens, spec.target_tokens),
            batch_verify: c.batch_verify,
            eligible: c.slots > 0
                && c.stale_advertisement_penalty_ms <= MAX_STALE_ADVERTISEMENT_PENALTY_MS
                && c.capacity_class >= MIN_CAPACITY_CLASS,
        })
        .collect();
    ranked_singles.sort_by(|a, b| {
        a.predicted_single_ms
            .total_cmp(&b.predicted_single_ms)
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    let fastest = ranked_singles.iter().find(|e| e.eligible);

    // 5. Cohort + the engine-true verify term (the landed option-A
    //    surface: cohort[0] — the fastest-ranked peer — is the verifier
    //    role; a mixed cohort whose verifier has not declared the
    //    capability is charged the sequential wire term).
    let cohort: Vec<Candidate> = select_microswarm(
        &candidates,
        &profile,
        spec.prompt_tokens,
        spec.target_tokens,
        spec.cohort_size,
    )
    .into_iter()
    .cloned()
    .collect();
    let verify_term = VerifyTerm::of_cohort(&cohort);

    // 6. Prediction + the frozen engage rule. The window the coordinator
    //    may send is capped by the verifier's honest disclosure when the
    //    batch family is in play (the coordinator MUST NOT send longer
    //    windows; the cap degrades the prediction, never inflates it).
    let Some(fastest) = fastest else {
        return Ok(MixedSwarmDecision {
            case: spec.case.clone(),
            negotiation,
            ranked_singles,
            fastest_single_peer: None,
            fastest_single_ms: None,
            cohort: Vec::new(),
            verify_term,
            effective_window: spec.window.min(WINDOW_CAP),
            rounds: 0,
            predicted_swarm_ms: None,
            engaged: false,
            fallback_peer: None,
        });
    };
    // Clone out of the borrow before `ranked_singles` moves into the
    // decision record.
    let fastest_peer_id = fastest.peer_id.clone();
    let fastest_ms = fastest.predicted_single_ms;
    let Some(verifier) = cohort.first() else {
        // Eligible singles exist but no cohort formed: fall back.
        return Ok(MixedSwarmDecision {
            case: spec.case.clone(),
            negotiation,
            ranked_singles,
            fastest_single_peer: Some(fastest_peer_id.clone()),
            fastest_single_ms: Some(fastest_ms),
            cohort: Vec::new(),
            verify_term,
            effective_window: spec.window.min(WINDOW_CAP),
            rounds: 0,
            predicted_swarm_ms: None,
            engaged: false,
            fallback_peer: Some(fastest_peer_id),
        });
    };
    let verifier_window_max = accepted
        .iter()
        .find(|p| p.peer_id == verifier.peer_id)
        .map_or(1, |p| p.engine.batch_window_max);
    let effective_window = match verify_term {
        VerifyTerm::Batch => spec.window.min(verifier_window_max).min(WINDOW_CAP),
        VerifyTerm::SequentialWire => spec.window.min(WINDOW_CAP),
    };
    let rounds = (f64::from(spec.target_tokens)
        / (1.0 + spec.acceptance * f64::from(effective_window)))
    .ceil() as u32;
    let predicted_swarm_ms =
        predicted_swarm_mirror_ms(spec, verifier, verify_term, effective_window);
    // The frozen engage rule (margin below the floor is an error, never
    // a silent clamp).
    let engaged =
        should_engage_cooperative(predicted_swarm_ms, fastest.predicted_single_ms, spec.margin)?;

    Ok(MixedSwarmDecision {
        case: spec.case.clone(),
        negotiation,
        ranked_singles,
        fastest_single_peer: Some(fastest_peer_id.clone()),
        fastest_single_ms: Some(fastest_ms),
        cohort: cohort.iter().map(|c| c.peer_id.clone()).collect(),
        verify_term,
        effective_window,
        rounds,
        predicted_swarm_ms: Some(predicted_swarm_ms),
        engaged,
        fallback_peer: (!engaged).then_some(fastest_peer_id),
    })
}

// ---------------------------------------------------------------------------
// The pinned matrix (ADR-032 §7 item 4b scenario families)
// ---------------------------------------------------------------------------

/// The pinned expectation for one scenario (part of the scenario
/// definition — the ADR's item-4b scenarios ARE expected outcomes).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MixedExpectation {
    pub engaged: bool,
    pub fallback_peer: Option<String>,
    pub verify_term: VerifyTerm,
}

fn expect(engaged: bool, fallback: Option<&str>, term: VerifyTerm) -> MixedExpectation {
    MixedExpectation {
        engaged,
        fallback_peer: fallback.map(str::to_string),
        verify_term: term,
    }
}

/// The pinned mixed-swarm matrix. Numbers follow the landed scheduler 4a
/// test (verifier-class decode 0.10 / prefill 3.0 / rtt 5 tokens-ms
/// shape; prompt 153, target 16, window 8, acceptance 1.0, drafter 4.0,
/// failure risk 5 ms, margin 0.15): the fastest single over that shape
/// predicts 216 ms; the batch-term prediction is 151 ms (engages,
/// 151 × 1.15 = 173.65 < 216) and the wire-term prediction is
/// ≈ 303.67 ms (never engages). Every case is a variation on that shape,
/// so each verdict is decided by structural arithmetic, not tuned
/// constants.
pub fn pinned_mixed_cases() -> Vec<(MixedSwarmSpec, MixedExpectation)> {
    // The shared peer-measurement shape (rtt 5, queue 0, prefill 3.0;
    // decode varies per peer).
    let peers = |decode: f64| (5.0, 0.0, 3.0, decode);
    let mut cases = Vec::new();

    // Family 1 — HTTP-only cohort (all batch_verify: false): the
    // wire-true term blocks, fallback to the fastest HTTP single.
    let mut spec = MixedSwarmSpec::new("http-only-never-engages", Backend::Cpu);
    let (rtt, queue, prefill, _) = peers(0.10);
    spec.peers = vec![
        SimPeerSpec::http("http-1", rtt, queue, prefill, 0.10),
        SimPeerSpec::http("http-2", rtt, queue, prefill, 0.08),
        SimPeerSpec::http("http-3", rtt, queue, prefill, 0.06),
    ];
    cases.push((
        spec,
        expect(false, Some("http-1"), VerifyTerm::SequentialWire),
    ));

    // Family 2a — adapter cohort, comfortable advantage: engages.
    let mut spec = MixedSwarmSpec::new("adapter-cohort-engages", Backend::Cpu);
    spec.peers = vec![
        SimPeerSpec::adapter("ada-1", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.10),
        SimPeerSpec::adapter("ada-2", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.08),
    ];
    cases.push((spec, expect(true, None, VerifyTerm::Batch)));

    // Family 2b — adapter cohort, but a faster single exists: falls back
    // to it (the comparator wins even though it is adapter-class).
    let mut spec = MixedSwarmSpec::new("adapter-faster-single-wins", Backend::Cpu);
    spec.peers = vec![
        SimPeerSpec::adapter(
            "ada-fast",
            Backend::Cpu,
            WINDOW_CAP,
            rtt,
            queue,
            prefill,
            0.25,
        ),
        SimPeerSpec::adapter("ada-1", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.10),
        SimPeerSpec::adapter("ada-2", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.08),
    ];
    cases.push((spec, expect(false, Some("ada-fast"), VerifyTerm::Batch)));

    // Family 2c — honest batch_window_max shrinks the effective window
    // (8 → 4): more rounds, prediction rises, engage flips to fallback.
    // The twin with window_max 32 is family 2a — same rates, engages.
    let mut spec = MixedSwarmSpec::new("adapter-honest-window-cap-falls-back", Backend::Cpu);
    spec.peers = vec![
        SimPeerSpec::adapter("ada-cap", Backend::Cpu, 4, rtt, queue, prefill, 0.10),
        SimPeerSpec::adapter("ada-2", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.08),
    ];
    cases.push((spec, expect(false, Some("ada-cap"), VerifyTerm::Batch)));

    // Family 3a — mixed swarm where the fastest eligible single is an
    // HTTP peer: the cohort's verifier is that peer (cohort[0]), the
    // cohort is charged the sequential wire term, and the swarm falls
    // back to the HTTP single — the adapter capability present in the
    // roster never overrides the class-blind comparator.
    let mut spec = MixedSwarmSpec::new("mixed-http-fastest-single-wins", Backend::Cpu);
    spec.peers = vec![
        SimPeerSpec::http("http-fast", rtt, queue, prefill, 0.16),
        SimPeerSpec::adapter("ada-1", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.10),
    ];
    cases.push((
        spec,
        expect(false, Some("http-fast"), VerifyTerm::SequentialWire),
    ));

    // Family 3b — mixed swarm where the adapter cohort engages WITH an
    // HTTP peer aboard: the HTTP peer is the slower single, stays out of
    // the verifier role, and may act as proposer (first-class, never a
    // batch verifier).
    let mut spec = MixedSwarmSpec::new(
        "mixed-adapter-cohort-engages-with-http-proposer",
        Backend::Cpu,
    );
    spec.peers = vec![
        SimPeerSpec::adapter("ada-1", Backend::Cpu, WINDOW_CAP, rtt, queue, prefill, 0.10),
        SimPeerSpec::http("http-slow", rtt, queue, prefill, 0.08),
    ];
    cases.push((spec, expect(true, None, VerifyTerm::Batch)));

    // Family 3c — mixed swarm where the fastest single is adapter-class:
    // the batch-term prediction still loses to it; fallback to it.
    let mut spec = MixedSwarmSpec::new("mixed-adapter-fastest-single-wins", Backend::Cpu);
    spec.peers = vec![
        SimPeerSpec::adapter(
            "ada-fast",
            Backend::Cpu,
            WINDOW_CAP,
            rtt,
            queue,
            prefill,
            0.25,
        ),
        SimPeerSpec::http("http-1", rtt, queue, prefill, 0.10),
    ];
    cases.push((spec, expect(false, Some("ada-fast"), VerifyTerm::Batch)));

    // Family 4 — honesty edge: the backend-mismatched peer (vulkan engine
    // vs a cpu offer) answers SessionReject(backend_mismatch) and is
    // EXCLUDED even though its measurements are the best in the roster;
    // the requester continues with the eligible peers; the
    // batch_verify:false peer is the fastest eligible single and is
    // selected as the plain-single fallback.
    let mut spec = MixedSwarmSpec::new("honesty-backend-mismatch-continues", Backend::Cpu);
    spec.peers = vec![
        SimPeerSpec::http("http-best", rtt, queue, prefill, 0.25),
        SimPeerSpec::adapter("ada-cpu", Backend::Cpu, 8, rtt, queue, prefill, 0.10),
        SimPeerSpec::adapter("ada-vulkan", Backend::Vulkan, 8, rtt, queue, prefill, 0.50),
    ];
    cases.push((
        spec,
        expect(false, Some("http-best"), VerifyTerm::SequentialWire),
    ));

    cases
}

/// `mixed`: runs the whole pinned ADR-032 §7 item-4b matrix and prints
/// one JSON summary. `ok` is true only when every pinned verdict holds —
/// the CLI scenario is self-verifying (a gate, not a benchmark: timings
/// are predicted values from labeled coefficients, never measurements).
pub async fn run_mixed() -> Result<serde_json::Value> {
    use serde_json::json;

    let mut case_rows = Vec::new();
    let mut all_ok = true;
    for (spec, expected) in pinned_mixed_cases() {
        let decision = run_mixed_swarm_decision(&spec)?;
        let verdict_ok = decision.engaged == expected.engaged
            && decision.fallback_peer == expected.fallback_peer
            && decision.verify_term == expected.verify_term;
        all_ok &= verdict_ok;
        case_rows.push(json!({
            "case": decision.case,
            "verdict_ok": verdict_ok,
            "expected": {
                "engaged": expected.engaged,
                "fallback_peer": expected.fallback_peer,
                "verify_term": serde_json::to_value(expected.verify_term).unwrap_or_default(),
            },
            "decision": serde_json::to_value(&decision)?,
        }));
    }
    Ok(json!({
        "scenario": "mixed",
        "ok": all_ok,
        "cases": case_rows,
        "note": "decision-level ADR-032 item-4b scenarios: negotiation via modelswarm-session, term + engage rule via modelswarm-scheduler; predictions are labeled mirrors of the bench engagement-pool arithmetic, not measurements",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verifier_shape(decode: f64) -> Candidate {
        Candidate {
            peer_id: "v".to_string(),
            profile_id: ModelProfileId::new(MIXED_PROFILE).unwrap(),
            measured_rtt_ms: 5.0,
            advertised_queue_ms: 0.0,
            prefill_tokens_per_ms: 3.0,
            decode_tokens_per_ms: decode,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: 1,
            capacity_class: CapacityClass::GpuMid,
            nat_path: NatPath::Direct,
            batch_verify: true,
        }
    }

    /// The mirror arithmetic reproduces the landed scheduler 4a test
    /// numbers exactly (window 8, prompt 153, target 16, acceptance 1.0,
    /// verifier decode 0.10 / prefill 3.0 / rtt 5, drafter 4.0, risk 5):
    /// batch 151 ms, wire ≈ 303.67 ms — and the wire-true verify term
    /// alone (180 ms) exceeds the single's whole decode budget (160 ms),
    /// the pass-2 structural finding.
    #[test]
    fn mirror_matches_the_landed_scheduler_test_numbers() {
        // The spec defaults ARE the landed shape (window 8, prompt 153,
        // target 16, acceptance 1.0, drafter 4.0, risk 5).
        let spec = MixedSwarmSpec::new("mirror-numbers", Backend::Cpu);
        let verifier = verifier_shape(0.10);
        let batch = predicted_swarm_mirror_ms(&spec, &verifier, VerifyTerm::Batch, 8);
        assert!((batch - 151.0).abs() < 1e-9, "batch mirror: {batch}");
        let wire = predicted_swarm_mirror_ms(&spec, &verifier, VerifyTerm::SequentialWire, 8);
        assert!(
            (wire - 303.666_666_666_666_7).abs() < 1e-9,
            "wire mirror: {wire}"
        );
        // The structural never-engage fact: wire verification alone
        // exceeds the single's entire decode term.
        let wire_verify = 2.0
            * VerifyTerm::SequentialWire.decode_steps_per_round(8, VERIFY_BATCH_STEPS)
            / verifier.decode_tokens_per_ms;
        let single_decode = 16.0 / verifier.decode_tokens_per_ms;
        assert!(wire_verify > single_decode);
        // Rollback vanishes exactly at acceptance 1.0 and is charged at
        // lower acceptance (never negative).
        let mut partial = MixedSwarmSpec::new("mirror-numbers-partial", Backend::Cpu);
        partial.acceptance = 0.5;
        let risky = predicted_swarm_mirror_ms(&partial, &verifier, VerifyTerm::Batch, 8);
        assert!(
            risky > batch,
            "imperfect acceptance must cost more: {risky} vs {batch}"
        );
    }

    /// A margin below the ADR-013 floor is an error, never a silent
    /// clamp (the frozen scheduler rule surfaces through the sim
    /// unchanged).
    #[test]
    fn margin_below_floor_is_an_error() {
        let mut spec = MixedSwarmSpec::new("margin-floor", Backend::Cpu);
        spec.peers = vec![SimPeerSpec::adapter(
            "ada-1",
            Backend::Cpu,
            WINDOW_CAP,
            5.0,
            0.0,
            3.0,
            0.10,
        )];
        spec.margin = 0.04;
        let err = run_mixed_swarm_decision(&spec).expect_err("floor must reject");
        assert!(err.to_string().contains("below the frozen floor"), "{err}");
    }

    /// A spec whose peer would send a schema-invalid disclosure
    /// (batch_window_max out of range) fails validation — the sim never
    /// lets a dishonest accept through.
    #[test]
    fn dishonest_disclosure_fails_validation() {
        let mut spec = MixedSwarmSpec::new("dishonest", Backend::Cpu);
        spec.peers = vec![SimPeerSpec {
            engine: EngineDisclosure {
                backend: Backend::Cpu,
                batch_verify: true,
                batch_window_max: WINDOW_CAP + 1,
            },
            ..SimPeerSpec::adapter("ada-liar", Backend::Cpu, 1, 5.0, 0.0, 3.0, 0.10)
        }];
        let err = run_mixed_swarm_decision(&spec).expect_err("invalid disclosure must fail");
        assert!(
            err.to_string().contains("engine disclosure invalid"),
            "{err}"
        );
    }
}
