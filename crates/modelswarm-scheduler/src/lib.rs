//! Swarm scheduling: filtering ineligible candidates, scoring peers by
//! predicted completion time (`docs/architecture.md` §10 v1 formula), the
//! ADR-013 cost model v2 for cooperative modes, the frozen engage rule with
//! its margin floor, bounded micro-swarm selection, and the EWMA estimator
//! that keeps requester-measured values dominant over self-reported ones.
//!
//! # Frozen rules (ADR-013)
//!
//! - `confidence_margin` defaults to [`DEFAULT_CONFIDENCE_MARGIN`] (0.15) and
//!   may never be below [`MIN_CONFIDENCE_MARGIN`] (0.05) — enforced with
//!   [`SchedulerError::MarginTooLow`], not by silently raising the value.
//! - Cooperative modes engage only when
//!   `predicted_swarm × (1 + margin) < predicted_fastest_single`.
//! - The comparator is always the **fastest eligible single host**, never an
//!   average (revision invariant).

use modelswarm_types::ModelProfileId;
use serde::{Deserialize, Serialize};

/// Default confidence margin for the engage rule (ADR-013: 0.15).
pub const DEFAULT_CONFIDENCE_MARGIN: f64 = 0.15;
/// Hard floor for any confidence margin (ADR-013: ≥ 0.05, frozen rule).
pub const MIN_CONFIDENCE_MARGIN: f64 = 0.05;
/// Maximum micro-swarm size (ADR-013 `speculative_exact`: 2–8 peers).
pub const MICROSWARM_CAP: usize = 8;
/// Eligibility filter: peers whose stale-advertisement penalty exceeds this
/// are excluded from selection (tunable default; measured staleness feeds
/// the penalty, the filter only refuses clearly stale data).
pub const MAX_STALE_ADVERTISEMENT_PENALTY_MS: f64 = 1_000.0;
/// Minimum hardware capacity class eligible for cooperative rounds.
pub const MIN_CAPACITY_CLASS: CapacityClass = CapacityClass::Cpu;
/// EWMA smoothing factor for requester-measured observations.
pub const EWMA_ALPHA: f64 = 0.3;

/// Hardware capacity classes, ordered weakest → strongest (matches the
/// `hardware_class_peers` vocabulary of the run-manifest schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityClass {
    /// CPU-only host.
    Cpu,
    /// Entry-level GPU.
    GpuEntry,
    /// Mid-range GPU.
    GpuMid,
    /// High-end GPU.
    GpuHigh,
}

/// NAT situation of a candidate (matches the `nat_path` vocabulary of the
/// mode-result schema; ADR-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NatPath {
    /// Direct QUIC connection.
    Direct,
    /// Hole-punched path.
    HolePunched,
    /// Relayed path (Phase F+).
    Relayed,
}

/// One eligible-or-not serving candidate for a profile, mixing
/// requester-measured fields (RTT, prefill/decode rates, penalties) with
/// peer-advertised ones (`advertised_queue_ms`, `slots`). Measured fields
/// dominate scoring by construction: they carry the token-rate terms.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Peer id (libp2p `PeerId` string); also the deterministic tie-break.
    pub peer_id: String,
    /// The exact profile this peer hosts (project rule).
    pub profile_id: ModelProfileId,
    /// Requester-measured round-trip time in milliseconds (EWMA).
    pub measured_rtt_ms: f64,
    /// Peer-advertised queue backlog in milliseconds.
    pub advertised_queue_ms: f64,
    /// Measured prefill throughput (tokens/ms).
    pub prefill_tokens_per_ms: f64,
    /// Measured decode throughput (tokens/ms).
    pub decode_tokens_per_ms: f64,
    /// Historical failure penalty in milliseconds.
    pub failure_penalty_ms: f64,
    /// Penalty for staleness of the advertisement in milliseconds.
    pub stale_advertisement_penalty_ms: f64,
    /// Currently advertised free serving slots.
    pub slots: u32,
    /// Hardware class of the host.
    pub capacity_class: CapacityClass,
    /// Network path type to this peer.
    pub nat_path: NatPath,
}

/// Scheduler-rule violations (frozen rules, not heuristics).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SchedulerError {
    /// A caller-supplied margin was below [`MIN_CONFIDENCE_MARGIN`]
    /// (ADR-013 frozen floor; the caller must raise it explicitly).
    #[error(
        "confidence margin {provided} is below the frozen floor {MIN_CONFIDENCE_MARGIN} \
         (ADR-013); pass an explicit, reviewed margin"
    )]
    MarginTooLow {
        /// The rejected margin value.
        provided: f64,
    },
}

/// `predicted_ms` for a single host, v1 formula
/// (`docs/architecture.md` §10):
///
/// ```text
/// measured_rtt_ms
/// + advertised_queue_ms
/// + prompt_tokens / measured_prefill_tokens_per_ms
/// + output_tokens / measured_decode_tokens_per_ms
/// + failure_penalty_ms
/// + stale_advertisement_penalty_ms
/// ```
///
/// A non-positive measured rate yields `INFINITY` (an unusable peer ranks
/// last instead of dividing by zero).
pub fn predicted_single_ms(candidate: &Candidate, prompt_tokens: u32, output_tokens: u32) -> f64 {
    candidate.measured_rtt_ms
        + candidate.advertised_queue_ms
        + tokens_over_rate(prompt_tokens, candidate.prefill_tokens_per_ms)
        + tokens_over_rate(output_tokens, candidate.decode_tokens_per_ms)
        + candidate.failure_penalty_ms
        + candidate.stale_advertisement_penalty_ms
}

fn tokens_over_rate(tokens: u32, rate_per_ms: f64) -> f64 {
    if rate_per_ms > 0.0 {
        f64::from(tokens) / rate_per_ms
    } else {
        f64::INFINITY
    }
}

/// Named inputs of the ADR-013 cost model v2. The function
/// [`predicted_swarm_ms`] is pure arithmetic over these terms — coefficients
/// are measured by the bench harness, never invented here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwarmInputs {
    /// Target-model prefill of the prompt on the verifier (ms).
    pub prefill_cost_ms: f64,
    /// Expected wall time for one proposal window on the proposer:
    /// network round trips plus drafter compute (ms).
    pub proposal_cost_ms: f64,
    /// Target-model batched verification of the window: per-token verify
    /// cost × window size (ms).
    pub verification_cost_ms: f64,
    /// Coordination overhead per bounded round window: RTT + jitter + relay
    /// effects on the commit path (ms).
    pub synchronization_cost_ms: f64,
    /// Expected rollback loss: P(window mismatch) × (discard + re-decode)
    /// cost, derived from the measured acceptance rate (ms).
    pub expected_rollback_cost_ms: f64,
    /// Expected loss from peer failure mid-round, from session failure
    /// history (ms).
    pub failure_risk_penalty_ms: f64,
}

impl SwarmInputs {
    /// Zero inputs (a base to overwrite field-by-field in tests).
    pub const ZERO: Self = Self {
        prefill_cost_ms: 0.0,
        proposal_cost_ms: 0.0,
        verification_cost_ms: 0.0,
        synchronization_cost_ms: 0.0,
        expected_rollback_cost_ms: 0.0,
        failure_risk_penalty_ms: 0.0,
    };
}

/// `predicted_swarm_completion` — the frozen ADR-013 v2 sum:
/// prefill + proposal + verification + synchronization +
/// expected rollback + failure risk. Pure arithmetic over [`SwarmInputs`].
pub fn predicted_swarm_ms(inputs: &SwarmInputs) -> f64 {
    inputs.prefill_cost_ms
        + inputs.proposal_cost_ms
        + inputs.verification_cost_ms
        + inputs.synchronization_cost_ms
        + inputs.expected_rollback_cost_ms
        + inputs.failure_risk_penalty_ms
}

/// Frozen engage rule (ADR-013): engage a cooperative mode only when
/// `predicted_swarm × (1 + margin) < predicted_fastest_single`.
///
/// `margin` below [`MIN_CONFIDENCE_MARGIN`] (0.05) is rejected with
/// [`SchedulerError::MarginTooLow`] — never silently clamped. Use
/// [`DEFAULT_CONFIDENCE_MARGIN`] (0.15) unless a reviewed ADR-backed value
/// exists; the margin actually used must be recorded in every run record.
pub fn should_engage_cooperative(
    predicted_swarm_ms: f64,
    predicted_fastest_single_ms: f64,
    margin: f64,
) -> Result<bool, SchedulerError> {
    if margin < MIN_CONFIDENCE_MARGIN {
        return Err(SchedulerError::MarginTooLow { provided: margin });
    }
    Ok(predicted_swarm_ms * (1.0 + margin) < predicted_fastest_single_ms)
}

/// Selects a micro-swarm: filter to exact-profile peers with a free slot,
/// non-stale advertisements, and at least [`MIN_CAPACITY_CLASS`] hardware;
/// order by [`predicted_single_ms`] (fastest first) with a deterministic
/// `peer_id` tie-break; take at most `want`, never more than
/// [`MICROSWARM_CAP`].
pub fn select_microswarm<'a>(
    candidates: &'a [Candidate],
    profile: &ModelProfileId,
    prompt_tokens: u32,
    output_tokens: u32,
    want: usize,
) -> Vec<&'a Candidate> {
    let mut eligible: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| {
            &c.profile_id == profile
                && c.slots > 0
                && c.stale_advertisement_penalty_ms <= MAX_STALE_ADVERTISEMENT_PENALTY_MS
                && c.capacity_class >= MIN_CAPACITY_CLASS
        })
        .collect();
    eligible.sort_by(|a, b| {
        predicted_single_ms(a, prompt_tokens, output_tokens)
            .total_cmp(&predicted_single_ms(b, prompt_tokens, output_tokens))
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    let take = want.min(MICROSWARM_CAP);
    eligible.truncate(take);
    eligible
}

/// Exponentially weighted moving average for requester-measured
/// observations (RTT, throughput). Persistence-agnostic: the store crate
/// serializes [`Ewma::value`] later; nothing here touches disk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ewma {
    alpha: f64,
    value: Option<f64>,
}

impl Ewma {
    /// Creates an estimator with the given smoothing factor (0..1).
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.0, 1.0),
            value: None,
        }
    }

    /// Creates an estimator with the default [`EWMA_ALPHA`] (0.3).
    #[must_use]
    pub fn default_alpha() -> Self {
        Self::new(EWMA_ALPHA)
    }

    /// Feeds one observation and returns the updated estimate. The first
    /// observation seeds the estimate directly.
    pub fn update(&mut self, observation: f64) -> f64 {
        let next = match self.value {
            None => observation,
            Some(previous) => self.alpha * observation + (1.0 - self.alpha) * previous,
        };
        self.value = Some(next);
        next
    }

    /// The current estimate, if any observation has been fed.
    pub fn value(&self) -> Option<f64> {
        self.value
    }

    /// The configured smoothing factor.
    pub fn alpha(&self) -> f64 {
        self.alpha
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ModelProfileId {
        ModelProfileId::new("msp:qwen3-4b:q4_k_m:v1").unwrap()
    }

    fn candidate(peer: &str, rtt: f64, queue: f64, prefill: f64, decode: f64) -> Candidate {
        Candidate {
            peer_id: peer.to_string(),
            profile_id: profile(),
            measured_rtt_ms: rtt,
            advertised_queue_ms: queue,
            prefill_tokens_per_ms: prefill,
            decode_tokens_per_ms: decode,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: 2,
            capacity_class: CapacityClass::GpuMid,
            nat_path: NatPath::Direct,
        }
    }

    #[test]
    fn v1_formula_matches_architecture_section_10() {
        let c = Candidate {
            failure_penalty_ms: 30.0,
            stale_advertisement_penalty_ms: 10.0,
            ..candidate("p1", 20.0, 50.0, 2.0, 0.5)
        };
        // 20 + 50 + 512/2 + 256/0.5 + 30 + 10 = 878
        let predicted = predicted_single_ms(&c, 512, 256);
        assert!((predicted - 878.0).abs() < 1e-9, "got {predicted}");
    }

    #[test]
    fn zero_measured_rate_ranks_as_infinite_time() {
        let c = candidate("p1", 10.0, 0.0, 0.0, 0.0);
        assert!(predicted_single_ms(&c, 10, 10).is_infinite());
    }

    #[test]
    fn swarm_model_is_the_frozen_sum_of_terms() {
        let inputs = SwarmInputs {
            prefill_cost_ms: 100.0,
            proposal_cost_ms: 20.0,
            verification_cost_ms: 40.0,
            synchronization_cost_ms: 15.0,
            expected_rollback_cost_ms: 25.0,
            failure_risk_penalty_ms: 10.0,
        };
        assert!((predicted_swarm_ms(&inputs) - 210.0).abs() < 1e-9);
        assert!((predicted_swarm_ms(&SwarmInputs::ZERO) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn engage_rule_flips_at_break_even_with_default_margin() {
        // Margin 0.5 is exact in binary, so the break-even product is exact:
        // 100 * 1.5 == 150 → not strictly faster → disengage.
        assert!(!should_engage_cooperative(100.0, 150.0, 0.5).unwrap());
        // One millisecond of headroom → engage.
        assert!(should_engage_cooperative(100.0, 151.0, 0.5).unwrap());
        // With the default 0.15 margin: clearly faster engages, slower not.
        assert!(should_engage_cooperative(100.0, 120.0, DEFAULT_CONFIDENCE_MARGIN).unwrap());
        assert!(!should_engage_cooperative(100.0, 110.0, DEFAULT_CONFIDENCE_MARGIN).unwrap());
    }

    #[test]
    fn margin_below_floor_is_rejected_not_clamped() {
        let err = should_engage_cooperative(1.0, 100.0, 0.049).unwrap_err();
        assert_eq!(err, SchedulerError::MarginTooLow { provided: 0.049 });
        assert!(should_engage_cooperative(1.0, 100.0, MIN_CONFIDENCE_MARGIN).unwrap());
        assert!(matches!(
            should_engage_cooperative(1.0, 100.0, 0.05 - f64::EPSILON * 8.0),
            Err(SchedulerError::MarginTooLow { .. })
        ));
    }

    #[test]
    fn three_distinct_peers_select_lowest_predicted_first() {
        let peers = [
            candidate("slow", 80.0, 200.0, 1.0, 0.2),
            candidate("middle", 40.0, 100.0, 2.0, 0.4),
            candidate("fast", 10.0, 20.0, 4.0, 1.0),
        ];
        let picked = select_microswarm(&peers, &profile(), 512, 256, 2);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].peer_id, "fast");
        assert_eq!(picked[1].peer_id, "middle");
    }

    #[test]
    fn measured_beats_advertised_when_they_contradict() {
        // Fake-fast advertiser: claims zero queue but is slow where it
        // matters (measured RTT + measured rates).
        let fake_fast = candidate("fake-fast", 200.0, 0.0, 0.5, 0.1);
        // Honest peer: advertises a real queue but measures fast everywhere.
        let honest = candidate("honest", 5.0, 100.0, 4.0, 1.0);
        let peers = [fake_fast.clone(), honest.clone()];
        let picked = select_microswarm(&peers, &profile(), 1024, 512, 8);
        assert_eq!(picked[0].peer_id, "honest");
        // The measured terms must dominate: honest wins despite the 100 ms
        // advertised queue versus the fake 0 ms.
        assert!(
            predicted_single_ms(&honest, 1024, 512) * 3.0
                < predicted_single_ms(&fake_fast, 1024, 512),
            "measured fields should dominate by a wide margin"
        );
    }

    #[test]
    fn selection_filters_wrong_profile_slots_staleness_and_capacity() {
        let other_profile = ModelProfileId::new("msp:qwen3-4b:q8_0:v1").unwrap();
        let wrong_profile = Candidate {
            profile_id: other_profile,
            ..candidate("wrong-profile", 1.0, 0.0, 9.0, 9.0)
        };
        let no_slots = Candidate {
            slots: 0,
            ..candidate("no-slots", 1.0, 0.0, 9.0, 9.0)
        };
        let stale = Candidate {
            stale_advertisement_penalty_ms: MAX_STALE_ADVERTISEMENT_PENALTY_MS + 0.1,
            ..candidate("stale", 1.0, 0.0, 9.0, 9.0)
        };
        let good = candidate("good", 5.0, 0.0, 9.0, 9.0);
        let peers = [wrong_profile, no_slots, stale, good];
        let picked = select_microswarm(&peers, &profile(), 16, 16, 8);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].peer_id, "good");
    }

    #[test]
    fn selection_is_capped_at_eight_and_want() {
        let peers: Vec<Candidate> = (0..20)
            .map(|i| candidate(&format!("peer-{i:02}"), f64::from(i), 0.0, 1.0, 1.0))
            .collect();
        assert_eq!(select_microswarm(&peers, &profile(), 0, 0, 20).len(), 8);
        assert_eq!(select_microswarm(&peers, &profile(), 0, 0, 3).len(), 3);
        assert_eq!(select_microswarm(&peers, &profile(), 0, 0, 0).len(), 0);
    }

    #[test]
    fn tie_break_is_peer_id_and_selection_is_deterministic() {
        let peers = vec![
            candidate("delta", 10.0, 10.0, 2.0, 0.5),
            candidate("alpha", 10.0, 10.0, 2.0, 0.5),
            candidate("charlie", 10.0, 10.0, 2.0, 0.5),
            candidate("bravo", 10.0, 10.0, 2.0, 0.5),
        ];
        let first: Vec<&str> = select_microswarm(&peers, &profile(), 64, 64, 3)
            .iter()
            .map(|c| c.peer_id.as_str())
            .collect();
        let second: Vec<&str> = select_microswarm(&peers, &profile(), 64, 64, 3)
            .iter()
            .map(|c| c.peer_id.as_str())
            .collect();
        assert_eq!(first, vec!["alpha", "bravo", "charlie"]);
        assert_eq!(first, second, "identical inputs must select identically");
    }

    #[test]
    fn ewma_seeds_then_smooths_with_alpha_03() {
        let mut ewma = Ewma::default_alpha();
        assert_eq!(ewma.alpha(), EWMA_ALPHA);
        assert_eq!(ewma.value(), None);
        assert!((ewma.update(100.0) - 100.0).abs() < 1e-12);
        // 0.3 * 200 + 0.7 * 100 = 130
        assert!((ewma.update(200.0) - 130.0).abs() < 1e-12);
        // 0.3 * 0 + 0.7 * 130 = 91
        assert!((ewma.update(0.0) - 91.0).abs() < 1e-12);
        assert_eq!(ewma.value(), Some(91.0));
        // Repeated large observations converge toward the new level.
        for _ in 0..60 {
            ewma.update(1000.0);
        }
        assert!(ewma.value().unwrap() > 999.0);
    }

    #[test]
    fn ewma_alpha_is_bounded() {
        assert_eq!(Ewma::new(7.0).alpha(), 1.0);
        assert_eq!(Ewma::new(-1.0).alpha(), 0.0);
    }

    #[test]
    fn capacity_classes_order_weakest_to_strongest() {
        assert!(CapacityClass::Cpu < CapacityClass::GpuEntry);
        assert!(CapacityClass::GpuEntry < CapacityClass::GpuMid);
        assert!(CapacityClass::GpuMid < CapacityClass::GpuHigh);
    }
}
