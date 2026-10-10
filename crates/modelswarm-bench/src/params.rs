//! Sweep parameterization for the 9.6 pass-1 harness (audit gap S6:
//! "Bench cannot sweep the proposal window; the acceptance estimate is a
//! constant" — `PROPOSAL_WINDOW` and `ESTIMATED_ACCEPTANCE` are
//! compile-time constants in the Phase C mock engine).
//!
//! [`Pass1Params`] makes the cohort window, acceptance prior, cohort cap,
//! marginal-benefit threshold τ, confidence margin, and repetition counts
//! runtime-configurable (env vars `MSP_BENCH_*`) so the pass-1 sweep can
//! vary them without rebuilds. Defaults equal the frozen constants
//! wherever one exists; every override is clamped to a safe range with a
//! visible warning (the shadow module's `EnvPolicy` pattern).
//!
//! Honesty rules baked in:
//! - `cohort_cap` defaults to **3** — k=4 is SPEND-GATED (dependency-graph
//!   hard edge 9: "9.6 pass 1 BEFORE the k=4 harness spend decision D4").
//!   The clamp allows larger caps only by explicit env override for the
//!   later, approved runs.
//! - `tau_ms` defaults to 0.0 and is a RECORDED PLACEHOLDER: expanded-
//!   mission §5 requires τ to be measured, not invented; pass 1 is the
//!   measurement vehicle.

use modelswarm_scheduler::{DEFAULT_CONFIDENCE_MARGIN, MIN_CONFIDENCE_MARGIN};

use crate::{ESTIMATED_ACCEPTANCE, MAX_SPECULATIVE_ROUNDS, OUTPUT_TOKENS_TARGET, PROPOSAL_WINDOW};

/// Env var: speculative proposal window (tokens). Default 8
/// (frozen `PROPOSAL_WINDOW`).
pub const ENV_WINDOW: &str = "MSP_BENCH_WINDOW";
/// Env var: output token target per run. Default 64 (frozen
/// `OUTPUT_TOKENS_TARGET`).
pub const ENV_OUTPUT_TOKENS: &str = "MSP_BENCH_OUTPUT_TOKENS";
/// Env var: maximum cohort size (k). Default 3 (k=4 is spend-gated, D4).
pub const ENV_COHORT_CAP: &str = "MSP_BENCH_COHORT_CAP";
/// Env var: acceptance prior for cold (peer, profile) pairs. Default 0.8
/// (frozen `ESTIMATED_ACCEPTANCE`, TEST-ONLY prior — measured acceptance
/// EWMAs take precedence once recorded).
pub const ENV_ACCEPTANCE: &str = "MSP_BENCH_ACCEPTANCE";
/// Env var: marginal-benefit threshold τ in ms (recorded placeholder
/// until 9.6 measures it). Default 0.0.
pub const ENV_TAU_MS: &str = "MSP_BENCH_TAU_MS";
/// Env var: confidence margin for the engage rule. Default
/// `DEFAULT_CONFIDENCE_MARGIN`.
pub const ENV_MARGIN: &str = "MSP_BENCH_MARGIN";
/// Env var: recorded repetitions per arm per cell. Default 30
/// (9.6 design: ≥30 reps/cell).
pub const ENV_RUNS: &str = "MSP_BENCH_RUNS";
/// Env var: warm-up completions per peer before the candidate set is
/// measured. Default 2.
pub const ENV_WARMUP: &str = "MSP_BENCH_WARMUP";
/// Env var: rule-6 observed-loss guard multiplier. Default
/// [`DEFAULT_LOSS_MULTIPLIER`] (the production value); a float ≥ 2.0
/// relaxes it; the literal `off` disables the abort (pass-2 engaged-curve
/// instrument — every relaxed run records the multiplier AND the
/// default-production counterfactual per row).
pub const ENV_LOSS_MULTIPLIER: &str = "MSP_BENCH_LOSS_MULTIPLIER";
/// Production rule-6 guard multiplier (round-1 wall beyond
/// `multiplier × (1 + margin) × predicted-per-round` aborts to single).
pub const DEFAULT_LOSS_MULTIPLIER: f64 = 2.0;
/// Relaxation floor: values below the production default are refused
/// (kept at the default with a warning) — the guard may only be loosened
/// for measurement, never quietly tightened below production.
pub const LOSS_MULTIPLIER_MIN: f64 = DEFAULT_LOSS_MULTIPLIER;

/// Hard bounds for every parameter (clamped with a visible warning).
pub mod bounds {
    pub const WINDOW_MIN: u32 = 1;
    pub const WINDOW_MAX: u32 = 16;
    pub const OUTPUT_TOKENS_MIN: u32 = 1;
    pub const OUTPUT_TOKENS_MAX: u32 = 1024;
    /// Spend-gate default; the ceiling exists for the approved k≥4 runs.
    pub const COHORT_CAP_DEFAULT: u32 = 3;
    pub const COHORT_CAP_MAX: u32 = 8;
    pub const ACCEPTANCE_MIN: f64 = 0.05;
    pub const ACCEPTANCE_MAX: f64 = 0.99;
    pub const TAU_MS_MAX: f64 = 1_000.0;
    pub const RUNS_MIN: u32 = 1;
    pub const RUNS_MAX: u32 = 200;
    pub const WARMUP_MIN: u32 = 1;
    pub const WARMUP_MAX: u32 = 10;
}

/// Resolved sweep parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Pass1Params {
    /// Speculative proposal window (tokens per round).
    pub proposal_window: u32,
    /// Output token target per recorded run.
    pub output_tokens_target: u32,
    /// Maximum cohort size k (spend-gated at 3 by default; D4).
    pub cohort_cap: u32,
    /// Acceptance prior for cold (peer, profile) pairs.
    pub acceptance_default: f64,
    /// Marginal-benefit threshold τ (ms) — recorded placeholder.
    pub tau_ms: f64,
    /// Confidence margin for the engage rule.
    pub margin: f64,
    /// Recorded repetitions per arm per cell.
    pub runs_per_arm: u32,
    /// Warm-up completions per peer before measuring the candidate set.
    pub warmup_completions: u32,
    /// Guard: maximum speculative rounds per run.
    pub max_rounds: u32,
    /// Rule-6 observed-loss guard multiplier. `None` = disabled (the
    /// pass-2 engaged-curve instrument); default
    /// [`DEFAULT_LOSS_MULTIPLIER`]. Relaxations are recorded per row with
    /// the default-production counterfactual.
    pub loss_multiplier: Option<f64>,
}

impl Default for Pass1Params {
    fn default() -> Self {
        Self {
            proposal_window: PROPOSAL_WINDOW,
            output_tokens_target: OUTPUT_TOKENS_TARGET,
            cohort_cap: bounds::COHORT_CAP_DEFAULT,
            acceptance_default: ESTIMATED_ACCEPTANCE,
            tau_ms: 0.0,
            margin: DEFAULT_CONFIDENCE_MARGIN,
            runs_per_arm: 30,
            warmup_completions: 2,
            max_rounds: MAX_SPECULATIVE_ROUNDS,
            loss_multiplier: Some(DEFAULT_LOSS_MULTIPLIER),
        }
    }
}

/// Resolved env overrides + warnings (the shadow `EnvPolicy` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvParams {
    pub params: Pass1Params,
    /// How many env vars actually overrode a default.
    pub env_overrides: usize,
    /// Visible warnings for unparsable/clamped values.
    pub warnings: Vec<String>,
}

impl Pass1Params {
    /// Reads the `MSP_BENCH_*` overrides. Unset vars keep defaults;
    /// unparsable values keep the default for THAT var with a warning;
    /// out-of-range values clamp with a warning.
    pub fn from_env() -> EnvParams {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Testable core of [`Pass1Params::from_env`] (no process env).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> EnvParams {
        let mut params = Self::default();
        let mut warnings = Vec::new();
        let mut env_overrides = 0usize;

        let mut apply_u32 = |name: &str, target: &mut u32, min: u32, max: u32| {
            let Some(raw) = lookup(name) else { return };
            match raw.trim().parse::<u32>() {
                Ok(value) => {
                    if !(min..=max).contains(&value) {
                        warnings.push(format!("{name}={value} outside [{min},{max}] — clamped"));
                        *target = value.clamp(min, max);
                    } else {
                        *target = value;
                    }
                    env_overrides += 1;
                }
                Err(_) => warnings.push(format!("{name}={raw:?} is not a u32 — default kept")),
            }
        };
        apply_u32(
            ENV_WINDOW,
            &mut params.proposal_window,
            bounds::WINDOW_MIN,
            bounds::WINDOW_MAX,
        );
        apply_u32(
            ENV_OUTPUT_TOKENS,
            &mut params.output_tokens_target,
            bounds::OUTPUT_TOKENS_MIN,
            bounds::OUTPUT_TOKENS_MAX,
        );
        apply_u32(
            ENV_COHORT_CAP,
            &mut params.cohort_cap,
            2,
            bounds::COHORT_CAP_MAX,
        );
        apply_u32(
            ENV_RUNS,
            &mut params.runs_per_arm,
            bounds::RUNS_MIN,
            bounds::RUNS_MAX,
        );
        apply_u32(
            ENV_WARMUP,
            &mut params.warmup_completions,
            bounds::WARMUP_MIN,
            bounds::WARMUP_MAX,
        );

        let mut apply_f64 = |name: &str, target: &mut f64, min: f64, max: f64| {
            let Some(raw) = lookup(name) else { return };
            match raw.trim().parse::<f64>() {
                Ok(value) if value.is_finite() => {
                    if !(min..=max).contains(&value) {
                        warnings.push(format!("{name}={value} outside [{min},{max}] — clamped"));
                        *target = value.clamp(min, max);
                    } else {
                        *target = value;
                    }
                    env_overrides += 1;
                }
                Ok(_) | Err(_) => warnings.push(format!(
                    "{name}={raw:?} is not a finite number — default kept"
                )),
            }
        };
        apply_f64(
            ENV_ACCEPTANCE,
            &mut params.acceptance_default,
            bounds::ACCEPTANCE_MIN,
            bounds::ACCEPTANCE_MAX,
        );
        apply_f64(ENV_TAU_MS, &mut params.tau_ms, 0.0, bounds::TAU_MS_MAX);
        apply_f64(ENV_MARGIN, &mut params.margin, MIN_CONFIDENCE_MARGIN, 10.0);
        // Loss-guard multiplier: float ≥ the production default, or the
        // literal "off" (recorded per row; never silently below default).
        if let Some(raw) = lookup(ENV_LOSS_MULTIPLIER) {
            let trimmed = raw.trim();
            if trimmed.eq_ignore_ascii_case("off") {
                params.loss_multiplier = None;
                env_overrides += 1;
            } else {
                match trimmed.parse::<f64>() {
                    Ok(value) if value.is_finite() => {
                        if value < LOSS_MULTIPLIER_MIN {
                            warnings.push(format!(
                                "{ENV_LOSS_MULTIPLIER}={value} below production default \
                                 {LOSS_MULTIPLIER_MIN} — kept at default"
                            ));
                            params.loss_multiplier = Some(DEFAULT_LOSS_MULTIPLIER);
                        } else {
                            params.loss_multiplier = Some(value);
                        }
                        env_overrides += 1;
                    }
                    Ok(_) | Err(_) => warnings.push(format!(
                        "{ENV_LOSS_MULTIPLIER}={raw:?} is neither a number nor 'off' — default kept"
                    )),
                }
            }
        }

        EnvParams {
            params,
            env_overrides,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_match_frozen_constants_and_spend_gate() {
        let env = Pass1Params::from_lookup(lookup_none);
        assert_eq!(env.env_overrides, 0);
        assert!(env.warnings.is_empty());
        let p = env.params;
        assert_eq!(p.proposal_window, PROPOSAL_WINDOW);
        assert_eq!(p.output_tokens_target, OUTPUT_TOKENS_TARGET);
        assert_eq!(p.acceptance_default, ESTIMATED_ACCEPTANCE);
        assert_eq!(p.margin, DEFAULT_CONFIDENCE_MARGIN);
        assert_eq!(p.max_rounds, MAX_SPECULATIVE_ROUNDS);
        // k=4 is spend-gated (D4): the default cap must stay below it.
        assert_eq!(p.cohort_cap, 3, "default cohort cap must be 3 (D4 gate)");
        assert_eq!(p.tau_ms, 0.0, "tau placeholder until measured");
        assert_eq!(p.runs_per_arm, 30, "9.6 design: >=30 reps/cell");
        assert_eq!(
            p.loss_multiplier,
            Some(DEFAULT_LOSS_MULTIPLIER),
            "production rule-6 guard default"
        );
    }

    #[test]
    fn loss_multiplier_relaxes_or_disables_but_never_tightens() {
        // "off" disables the guard abort.
        let env = Pass1Params::from_lookup(|name| {
            (name == ENV_LOSS_MULTIPLIER).then(|| "off".to_string())
        });
        assert_eq!(env.params.loss_multiplier, None);
        assert_eq!(env.env_overrides, 1);
        assert!(env.warnings.is_empty());
        // A float ≥ 2.0 relaxes it.
        let env = Pass1Params::from_lookup(|name| {
            (name == ENV_LOSS_MULTIPLIER).then(|| "12.5".to_string())
        });
        assert_eq!(env.params.loss_multiplier, Some(12.5));
        // Below the production default is refused (kept at default,
        // visibly) — the guard may only be loosened for measurement.
        let env = Pass1Params::from_lookup(|name| {
            (name == ENV_LOSS_MULTIPLIER).then(|| "1.0".to_string())
        });
        assert_eq!(env.params.loss_multiplier, Some(DEFAULT_LOSS_MULTIPLIER));
        assert_eq!(env.warnings.len(), 1);
        // Garbage keeps the default with a warning.
        let env = Pass1Params::from_lookup(|name| {
            (name == ENV_LOSS_MULTIPLIER).then(|| "sometimes".to_string())
        });
        assert_eq!(env.params.loss_multiplier, Some(DEFAULT_LOSS_MULTIPLIER));
        assert_eq!(env.warnings.len(), 1);
    }

    #[test]
    fn overrides_parse_and_are_recorded() {
        let vars = [
            (ENV_WINDOW, "4".to_string()),
            (ENV_OUTPUT_TOKENS, "32".to_string()),
            (ENV_COHORT_CAP, "2".to_string()),
            (ENV_ACCEPTANCE, "0.5".to_string()),
            (ENV_TAU_MS, "1.5".to_string()),
            (ENV_MARGIN, "0.2".to_string()),
            (ENV_RUNS, "5".to_string()),
            (ENV_WARMUP, "1".to_string()),
        ];
        let env = Pass1Params::from_lookup(|name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        });
        assert_eq!(env.env_overrides, 8);
        assert!(env.warnings.is_empty());
        let p = env.params;
        assert_eq!(p.proposal_window, 4);
        assert_eq!(p.output_tokens_target, 32);
        assert_eq!(p.cohort_cap, 2);
        assert!((p.acceptance_default - 0.5).abs() < 1e-9);
        assert!((p.tau_ms - 1.5).abs() < 1e-9);
        assert!((p.margin - 0.2).abs() < 1e-9);
        assert_eq!(p.runs_per_arm, 5);
        assert_eq!(p.warmup_completions, 1);
    }

    #[test]
    fn garbage_and_out_of_range_clamp_with_warnings() {
        let vars = [
            (ENV_WINDOW, "999".to_string()),              // clamps to 16
            (ENV_COHORT_CAP, "1".to_string()),            // clamps to 2
            (ENV_ACCEPTANCE, "not-a-number".to_string()), // default kept
            (ENV_MARGIN, "0.01".to_string()),             // clamps to MIN margin
            (ENV_RUNS, "0".to_string()),                  // clamps to 1
        ];
        let env = Pass1Params::from_lookup(|name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        });
        let p = env.params;
        assert_eq!(p.proposal_window, bounds::WINDOW_MAX);
        assert_eq!(p.cohort_cap, 2);
        assert!((p.acceptance_default - ESTIMATED_ACCEPTANCE).abs() < 1e-9);
        assert!((p.margin - MIN_CONFIDENCE_MARGIN).abs() < 1e-9);
        assert_eq!(p.runs_per_arm, 1);
        assert_eq!(
            env.warnings.len(),
            5,
            "every bad value warns: {:?}",
            env.warnings
        );
        assert!(env.warnings.iter().any(|w| w.contains("default kept")));
        assert!(env.warnings.iter().any(|w| w.contains("clamped")));
    }
}
