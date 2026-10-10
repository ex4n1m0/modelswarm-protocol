//! Per-profile engaged-loss EWMA engage gate — the structural complement
//! to the reactive rule-6 guard, per the guard-calibration review
//! (`docs/reviews/review-guard-calibration-2026-10-10.md` §2,
//! recommendation iii) and the ADR-032 §5 option-A posture.
//!
//! # Why this gate exists (review finding, in one paragraph)
//!
//! The rule-6 observed-loss guard compares realized round-1 wall to the
//! *cooperative prediction* — its key is uncorrelated with the release
//! promise ("never meaningfully worse than the fastest eligible
//! single"): on the committed pass-2 LAN dataset it would have aborted
//! every engaged row-level WIN (guard ratios 2.87–4.15) while passing
//! 125 engaged rows that all LOST (1.179–3.278× vs the prompt-best
//! single). The only mechanism that held the promise in that dataset
//! was the ENGAGE GATE (fallback cells realized 0.95–1.02×), and its
//! blind spot was exactly high-acceptance cells where engagement loses
//! structurally. A loss signal keyed on the promise metric closes that
//! blind spot BEFORE a round is paid.
//!
//! # Specification (explicit, per the review's requested proposal)
//!
//! - **Key** — `realized_total_ms / fastest_single_prediction_ms` of
//!   ENGAGED attempts only (completed cooperative runs; the fallback
//!   discriminator is `loss_guard` presence + `Completed`, exactly the
//!   aggregate script's). Both quantities already exist in every
//!   decision-join row: no extra single execution is needed
//!   (production-feasible).
//! - **α = [`ENGAGE_LOSS_EWMA_ALPHA`] = 0.3** — the frozen scheduler
//!   [`modelswarm_scheduler::EWMA_ALPHA`], the same smoothing the
//!   requester-measured RTT/acceptance estimators use (one estimator
//!   family, no new tunable).
//! - **Warm-up N = [`ENGAGE_LOSS_WARMUP_N`] = 3** — the review derived
//!   "a clean monotone band well separated from 1.0 after a handful of
//!   attempts"; on the committed LAN rows N = 3 suffices for every
//!   cell that ever reached four engaged attempts (replay-pinned in
//!   `tests/harness.rs`).
//! - **Block threshold [`ENGAGE_LOSS_BLOCK_THRESHOLD`] = 1.0** — strict
//!   beat-or-fall-back per the competitive revision plan: at ≥ 1.0 the
//!   profile's engaged attempts do not beat the fastest-single
//!   PREDICTION, so engagement is refused and the run takes the single
//!   path.
//! - **Scope: PER PROFILE** (profile-id key). The pass-2 drivers create
//!   a fresh harness per cell, which scopes the gate per cell in those
//!   runs (the same isolation choice as the acceptance store — a
//!   cross-profile carryover would be a confound); one long-lived
//!   harness (the production shape) carries one gate per profile across
//!   requests.
//! - **Cold start (samples < N): MARGIN-GATED ALLOW** — the frozen
//!   `should_engage_cooperative` rule alone governs until N engaged
//!   completions exist. Default-block-on-cold would deadlock: blocked ⇒
//!   no engaged attempts ⇒ no samples ⇒ blocked forever, i.e. a
//!   permanent kill switch, not a gate.
//! - **Recovery: none yet (honest limitation)** — once blocked, a
//!   profile stays blocked until its loss ratio distribution changes,
//!   which the gate cannot observe while blocking (no probe cadence, no
//!   decay). A reviewed probe/decay policy — or keying by
//!   (profile, verify term) so a future batch-verify cohort starts a
//!   fresh loss history — belongs to a follow-up decision, not here.
//!
//! # Relation to the other two guards (all three are live together)
//!
//! 1. The **engage gate** (margin rule over the wire-true cost model,
//!    ADR-032 §4 companion correction) blocks structurally losing
//!    cohorts before anything is paid — on HTTP-only cohorts it blocks
//!    every engagement, so this EWMA stays cold there (0 samples: the
//!    honest expected state in the gated pass-2 re-run).
//! 2. THIS gate blocks profiles whose predictions pass the margin rule
//!    but whose realized engaged attempts still lose ≥ 1.0× (a
//!    mis-modeled cost, an optimistic acceptance EWMA, or a future
//!    batch cohort whose engine term is wrong).
//! 3. The **rule-6 observed-loss guard** stays at the production
//!    multiplier 2.0 as the within-request round-1 backstop.

use std::collections::BTreeMap;

use modelswarm_scheduler::EWMA_ALPHA;
use serde::{Deserialize, Serialize};

/// EWMA smoothing factor for the engaged-loss estimate (0.3 — the frozen
/// scheduler default, shared with the RTT/acceptance estimators).
pub const ENGAGE_LOSS_EWMA_ALPHA: f64 = EWMA_ALPHA;
/// Warm-up: the gate blocks only after this many engaged attempts have
/// been recorded for the profile (cold start is margin-gated allow).
pub const ENGAGE_LOSS_WARMUP_N: u32 = 3;
/// Block threshold: an engaged-loss EWMA at or above 1.0 means the
/// profile's engaged attempts do not beat the fastest-single prediction
/// — engagement is refused (strict beat-or-fall-back).
pub const ENGAGE_LOSS_BLOCK_THRESHOLD: f64 = 1.0;

/// The gate's verdict for one profile at decision time — also the
/// per-row artifact record (serialized onto cooperative join rows
/// whenever the gate was consulted).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GateDecision {
    /// Whether engagement is blocked for this profile.
    pub blocked: bool,
    /// The current engaged-loss EWMA (`None` while cold).
    pub ewma: Option<f64>,
    /// Engaged attempts recorded so far for this profile.
    pub samples: u32,
    /// The smoothing factor in force (recorded per decision).
    pub alpha: f64,
    /// The warm-up bound in force (recorded per decision).
    pub warmup_n: u32,
    /// The block threshold in force (recorded per decision).
    pub threshold: f64,
}

impl GateDecision {
    /// The never-consulted placeholder (no cooperative prediction
    /// existed; the gate was not asked).
    pub const NOT_CONSULTED: Self = Self {
        blocked: false,
        ewma: None,
        samples: 0,
        alpha: ENGAGE_LOSS_EWMA_ALPHA,
        warmup_n: ENGAGE_LOSS_WARMUP_N,
        threshold: ENGAGE_LOSS_BLOCK_THRESHOLD,
    };
}

/// The per-profile engaged-loss EWMA gate (specification in the module
/// docs). In-memory by design: one instance per harness/requester
/// lifetime; persistence (if ever wanted) follows the acceptance-store
/// pattern under a reviewed migration.
#[derive(Debug, Default)]
pub struct EngagedLossGate {
    state: BTreeMap<String, (modelswarm_scheduler::Ewma, u32)>,
}

impl EngagedLossGate {
    /// A gate with the frozen constants (α 0.3, N 3, threshold 1.0).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The decision for one profile: blocked iff the warm-up has passed
    /// AND the EWMA sits at or above the block threshold.
    #[must_use]
    pub fn decision(&self, profile_id: &str) -> GateDecision {
        let Some((ewma, samples)) = self.state.get(profile_id) else {
            return GateDecision {
                blocked: false,
                ewma: None,
                samples: 0,
                alpha: ENGAGE_LOSS_EWMA_ALPHA,
                warmup_n: ENGAGE_LOSS_WARMUP_N,
                threshold: ENGAGE_LOSS_BLOCK_THRESHOLD,
            };
        };
        let value = ewma.value();
        GateDecision {
            blocked: *samples >= ENGAGE_LOSS_WARMUP_N
                && value.is_some_and(|v| v >= ENGAGE_LOSS_BLOCK_THRESHOLD),
            ewma: value,
            samples: *samples,
            alpha: ENGAGE_LOSS_EWMA_ALPHA,
            warmup_n: ENGAGE_LOSS_WARMUP_N,
            threshold: ENGAGE_LOSS_BLOCK_THRESHOLD,
        }
    }

    /// Feeds one engaged attempt's loss ratio
    /// (`realized_total_ms / fastest_single_prediction_ms` of a
    /// COMPLETED engaged run — the caller's discriminator; failed or
    /// aborted attempts carry no clean ratio and are skipped). The
    /// first observation seeds the estimate (no zero bias). A
    /// non-finite or negative ratio is a CALLER BUG — asserted in
    /// debug builds, and in release builds skipped (never fed into the
    /// estimate) rather than silently corrupting the gate.
    pub fn record_engaged(&mut self, profile_id: &str, loss_ratio: f64) {
        debug_assert!(
            loss_ratio.is_finite() && loss_ratio >= 0.0,
            "engaged-loss ratio must be a finite non-negative number, got {loss_ratio}"
        );
        if !loss_ratio.is_finite() || loss_ratio < 0.0 {
            return;
        }
        let entry = self
            .state
            .entry(profile_id.to_string())
            .or_insert_with(|| (modelswarm_scheduler::Ewma::new(ENGAGE_LOSS_EWMA_ALPHA), 0));
        entry.0.update(loss_ratio);
        entry.1 += 1;
    }

    /// Serializable snapshot of every profile's state (artifact shape).
    #[must_use]
    pub fn snapshot(&self) -> Vec<(String, Option<f64>, u32)> {
        self.state
            .iter()
            .map(|(profile, (ewma, samples))| (profile.clone(), ewma.value(), *samples))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_is_margin_gated_allow_regardless_of_ratio() {
        let mut gate = EngagedLossGate::new();
        // No samples at all: never blocked (default-block would deadlock —
        // blocked means no engaged attempts means no samples).
        assert!(!gate.decision("msp1:a").blocked);
        // Even after warm-up-1 samples of catastrophic loss ratios the
        // gate has not warmed: the frozen margin rule alone governs.
        for _ in 0..ENGAGE_LOSS_WARMUP_N - 1 {
            gate.record_engaged("msp1:a", 4.0);
        }
        let d = gate.decision("msp1:a");
        assert!(!d.blocked, "samples {} < warm-up {}", d.samples, d.warmup_n);
        assert_eq!(d.samples, ENGAGE_LOSS_WARMUP_N - 1);
        assert_eq!(d.alpha, 0.3);
        assert_eq!(d.threshold, 1.0);
    }

    #[test]
    fn seeds_then_smooths_with_alpha_03() {
        let mut gate = EngagedLossGate::new();
        gate.record_engaged("p", 1.2);
        assert!((gate.decision("p").ewma.unwrap() - 1.2).abs() < 1e-12);
        gate.record_engaged("p", 1.4);
        // 0.3 × 1.4 + 0.7 × 1.2 = 1.26
        assert!((gate.decision("p").ewma.unwrap() - 1.26).abs() < 1e-12);
    }

    #[test]
    fn blocks_at_or_above_one_after_warmup() {
        let mut gate = EngagedLossGate::new();
        for _ in 0..ENGAGE_LOSS_WARMUP_N {
            gate.record_engaged("p", 1.3);
        }
        let d = gate.decision("p");
        assert_eq!(d.samples, ENGAGE_LOSS_WARMUP_N);
        assert!((d.ewma.unwrap() - 1.3).abs() < 1e-12);
        assert!(d.blocked, "1.3 ≥ 1.0 after warm-up blocks");
        // Exactly at the threshold also blocks (strict beat-or-fall-back:
        // 1.0 is NOT a win over the fastest single).
        let mut edge = EngagedLossGate::new();
        for _ in 0..ENGAGE_LOSS_WARMUP_N {
            edge.record_engaged("q", 1.0);
        }
        assert!(edge.decision("q").blocked, "an EWMA of exactly 1.0 blocks");
    }

    #[test]
    fn below_threshold_still_allows() {
        let mut gate = EngagedLossGate::new();
        for _ in 0..10 {
            gate.record_engaged("p", 0.8);
        }
        let d = gate.decision("p");
        assert!(
            !d.blocked,
            "a profile whose engaged attempts win keeps engaging"
        );
        assert!((d.ewma.unwrap() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn scope_is_per_profile() {
        let mut gate = EngagedLossGate::new();
        for _ in 0..ENGAGE_LOSS_WARMUP_N {
            gate.record_engaged("msp1:losing", 2.0);
        }
        gate.record_engaged("msp1:winning", 0.7);
        assert!(gate.decision("msp1:losing").blocked);
        let fresh = gate.decision("msp1:winning");
        assert!(!fresh.blocked);
        assert_eq!(fresh.samples, 1, "profiles do not share samples");
        assert_eq!(gate.snapshot().len(), 2);
    }

    /// A garbage ratio is a caller bug: the debug contract panics (the
    /// call sites filter to finite positive predictions, so it cannot
    /// happen — this pins the contract, and release builds skip rather
    /// than corrupt the estimate).
    #[test]
    #[should_panic(expected = "finite non-negative")]
    fn garbage_ratio_is_a_contract_violation() {
        let mut gate = EngagedLossGate::new();
        gate.record_engaged("p", f64::NAN);
    }

    /// The committed pass-2 LAN dataset in miniature: three catastrophic
    /// attempts (the geo-cell shape, engaged loss ~3×) block the profile
    /// from the fourth attempt on — the review's "the gate says don't
    /// engage for free; the guard says it only after paying round 1".
    #[test]
    fn three_losses_block_the_fourth_attempt() {
        let mut gate = EngagedLossGate::new();
        for ratio in [3.9, 3.8, 3.7] {
            assert!(
                !gate.decision("msp1:geo").blocked,
                "cold/warm-up attempts run"
            );
            gate.record_engaged("msp1:geo", ratio);
        }
        assert!(
            gate.decision("msp1:geo").blocked,
            "attempt 4 is refused pre-round"
        );
    }
}
