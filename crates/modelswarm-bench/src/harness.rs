//! 9.6 pass-1 cell orchestration: warm-up + candidate-set measurement,
//! injected-delay calibration, the three arms over the same candidate
//! set, decision↔completion joining at request-id level, and artifact
//! emission into `experiments/`-shaped directories (frozen schemas; the
//! environment label rides the manifest `harness_version`, the
//! calibration record, and every report line).
//!
//! # Honesty contract for every artifact this module writes
//!
//! - Loopback cells carry [`crate::runner::ENV_LABEL_LOOPBACK_INJECTED`]
//!   ("loopback+injected-delay") and are NEVER LAN claims (testbed
//!   ladder: sim → 2-machine LAN → k=4 → WAN).
//! - The comparator is the fastest eligible single host at that moment:
//!   the single arm runs INDEPENDENTLY in every cell, and cooperative
//!   records fill `fastest_single_actual_ms` with the min over those
//!   real completions in the same cell + prompt (audit S4 fixed for
//!   real-runtime records — never a model draw).
//! - Fallbacks and failures are recorded with the same fidelity as wins
//!   (`RunStatus::FellBackToSingle` + mandatory reason; zero-acceptance
//!   stalls are failures, not wins).
//! - Every number is environment-labeled; negative results are
//!   first-class.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use modelswarm_identity::InstallationIdentity;
use modelswarm_node::measure::PeerMetrics;
use modelswarm_scheduler::shadow::{candidate_from_observations, EnvPolicy, ShadowCandidateInput};
use modelswarm_scheduler::{select_microswarm, Candidate};
use modelswarm_transport::observe::QUINN_STATS_UNAVAILABLE;
use serde::{Deserialize, Serialize};

use crate::acceptance::AcceptanceStore;
use crate::corpus::{Pass1Prompt, PASS1_CORPUS, PASS1_CORPUS_ID};
use crate::engage_gate::{EngagedLossGate, GateDecision};
use crate::params::Pass1Params;
use crate::records::{
    CellPath, Exactness, JitterClass, ManifestNetworkCell, ManifestRuntime, ModeMetrics,
    ModeResultRecord, NatPath, RunManifest, RunOutcome, RunStatus,
};
use crate::runner::{
    execute_on, plan_cohort, spawn_bridge, swarm_inputs, synthetic_executor_build_hash,
    verify_mode, AcceptanceRegime, Arm, BridgeConfig, LiveBridge, WireCompletion, WireRequest,
    ENV_LABEL_LOOPBACK_INJECTED, RTT_WAIT_BOUND, SYNTHETIC_EXECUTOR_NAME, VERIFIER_SEED,
};
use crate::stats;
use crate::{mix, run_id_from_seed, CANDIDATE_BLOCK_OVERHEAD_BYTES};

/// Calibration method string recorded per cell.
const CALIBRATION_METHOD: &str =
    "executor-seam 1-token round-trip p50, baseline-subtracted (zero-delay calibration bridge)";
/// Calibration probes per endpoint.
pub const CALIBRATION_PROBES: usize = 24;
/// Warm-up completion budget in tokens (unrecorded).
const WARMUP_TOKENS: u32 = 4;

/// One loopback cell specification.
#[derive(Debug, Clone)]
pub struct LoopbackCellSpec {
    /// Injected per-request delay (ms). Must be a frozen-matrix RTT member
    /// (5/10/20 for meaningful 10% calibration; 0 records no
    /// `calibration_valid` because 10% of 0 is undefined).
    pub injected_delay_ms: u32,
    /// Client-side acceptance profile (ground-truth knob).
    pub regime: AcceptanceRegime,
    /// Bridge configurations (the candidate pool).
    pub bridges: Vec<BridgeConfig>,
    /// Optional window note for the cell name (`Some(16)` → `...-w16`).
    /// Pass-1 cells predate window sweeps and leave `None` (their
    /// committed artifact names stay reproducible); pass-2 sweeps, where
    /// two cells can differ only by window, must set it.
    pub window_tag: Option<u32>,
}

impl LoopbackCellSpec {
    /// Directory name for artifacts.
    #[must_use]
    pub fn name(&self) -> String {
        let window = self.window_tag.map_or(String::new(), |w| format!("-w{w}"));
        format!(
            "inj{}ms-{}{window}",
            self.injected_delay_ms,
            self.regime.cell_tag()
        )
    }
}

/// The request-id-level join row: what the planner predicted vs what the
/// wire realized, per recorded run (ids and numbers only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinRow {
    pub request_root: String,
    pub arm: String,
    pub k: Option<u32>,
    pub prompt_id: String,
    pub rep: u32,
    pub predicted_ms: Option<f64>,
    pub fastest_single_prediction_ms: Option<f64>,
    pub realized_ttft_ms: f64,
    pub realized_total_ms: f64,
    pub accepted_tokens: u32,
    pub proposed_tokens: u32,
    pub rounds: u32,
    pub status: String,
    /// Fallback/failure reason (visible negatives).
    pub reason: Option<String>,
    pub run_manifest_id: String,
    /// Planner-arm sweep telemetry `(k, prediction_ms)`.
    pub planner_sweep: Option<Vec<(u32, f64)>>,
    /// Environment label (loopback+injected-delay / lan).
    pub label: String,
    /// Observed-loss rule-6 guard telemetry for engaged runs (round 1);
    /// `None` for fallback rows (no round ran) and single rows.
    pub loss_guard: Option<LossGuardRecord>,
    /// Per-profile engaged-loss EWMA gate state at DECISION time (present
    /// on cooperative rows whose finite prediction made the gate
    /// consultable; `None` on single rows, degenerate-planner rows, and
    /// pre-gate pass-1/pass-2 artifacts — `#[serde(default)]` keeps
    /// committed join rows parsing).
    #[serde(default)]
    pub loss_gate: Option<GateDecision>,
}

/// The rule-6 observed-loss guard's verdict for one engaged run's first
/// round: what the wire realized vs what the frozen model predicted per
/// round, whether the guard fired, and the counterfactual at the default
/// production multiplier (pass 2 relaxes the multiplier via a RECORDED
/// env override to measure full engaged curves; the default-production
/// verdict stays visible per row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LossGuardRecord {
    /// Guard multiplier in effect (the production default is
    /// [`DEFAULT_LOSS_MULTIPLIER`]; `"off"` disables the abort — recorded,
    /// never silent).
    pub multiplier: String,
    /// Realized round-1 wall (ms).
    pub round1_wall_ms: f64,
    /// Predicted per-round budget (ms).
    pub predicted_round_ms: f64,
    /// Whether the guard aborted this run pre-first-token.
    pub fired: bool,
    /// Whether the DEFAULT production guard would have aborted (the
    /// counterfactual that keeps pass-2 relaxed runs honest).
    pub would_fire_at_default: bool,
}

/// Per-cell injected-delay calibration record (loopback honesty).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRecord {
    pub label: String,
    pub method: &'static str,
    pub nominal_injected_ms: u32,
    pub measured_injected_ms_p50: f64,
    pub calibration_valid: Option<bool>,
    pub baseline_p50_ms: f64,
    pub cell_p50_ms: f64,
    pub probes: usize,
    pub transport_note: String,
}

/// Aggregated per-arm summary (computed by this harness — never by hand).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmSummary {
    pub arm: String,
    pub k: Option<u32>,
    pub n: usize,
    pub completion_ms: stats::CellSummary,
    pub fallbacks: usize,
    pub failures: usize,
    /// Median arm completion ÷ median fastest-single completion in the
    /// same cell (the 9.6 headline metric; < 1.0 is a win, ≥ 1.0 a
    /// visible negative).
    pub median_vs_fastest_single: Option<f64>,
    pub accepted_tokens_per_round_median: Option<f64>,
    pub acceptance_rate_mean: Option<f64>,
}

/// Processed cell summary (experiments/processed/ shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellArtifactsSummary {
    pub cell: String,
    pub label: String,
    pub regime: String,
    pub injected_delay_ms: u32,
    pub arms: Vec<ArmSummary>,
    pub fastest_single_median_ms: Option<f64>,
}

/// Everything one cell run produced (also written to disk).
pub struct CellArtifacts {
    pub dir: PathBuf,
    pub manifests: Vec<(String, RunManifest)>,
    pub records: Vec<ModeResultRecord>,
    pub join_rows: Vec<JoinRow>,
    pub calibration: CalibrationRecord,
    pub summary: CellArtifactsSummary,
}

/// One recorded run's joined result: realized wire measurements plus the
/// decision-side numbers that produced them.
struct RunJoined {
    root: String,
    arm: Arm,
    k: Option<u32>,
    ttft_ms: f64,
    total_ms: f64,
    output_tokens: u32,
    accepted_tokens: u32,
    proposed_tokens: u32,
    rounds: u32,
    aggregate_model_tokens: u32,
    wire_bytes: f64,
    proposer_ms: f64,
    verifier_ms: f64,
    coordinator_ms: f64,
    acceptance_rate: Option<f64>,
    rollbacks: u32,
    status: RunStatus,
    reason: Option<String>,
    predicted_ms: Option<f64>,
    fastest_single_prediction_ms: Option<f64>,
    planner_sweep: Option<Vec<(u32, f64)>>,
    loss_guard: Option<LossGuardRecord>,
    loss_gate: Option<GateDecision>,
}

/// What one cooperative arm run targets: the corpus prompt, the unique
/// request root, and the resolved parameters.
struct RunTarget<'a> {
    prompt: &'a Pass1Prompt,
    root: String,
    params: &'a Pass1Params,
}

/// The cooperative arm's plan inputs (fixed-k and planner-k differ only
/// in how these are derived).
struct CoopSpec {
    /// Cohort size the arm executes.
    k: u32,
    /// Frozen cooperative prediction at that k (None = no finite one).
    prediction_ms: Option<f64>,
    /// The engage-gate verdict (margin rule AND the engaged-loss EWMA
    /// gate — see `run_loopback_cell`).
    engaged: bool,
    /// Planner sweep telemetry.
    sweep: Option<Vec<(u32, f64)>>,
    /// Whether this is the planner arm (affects record attribution).
    is_planner: bool,
    /// The engaged-loss EWMA gate state at decision time (`Some` whenever
    /// a finite cooperative prediction made the gate consultable).
    loss_gate: Option<GateDecision>,
}

/// The pass-1 harness: shared measurement state across cells.
pub struct Pass1Harness {
    profile_id: String,
    identity: InstallationIdentity,
    telemetry: Arc<modelswarm_telemetry::Telemetry>,
    metrics: Arc<PeerMetrics>,
    acceptance: AcceptanceStore,
    /// Per-profile engaged-loss EWMA engage gate (review recommendation
    /// iii; see [`crate::engage_gate`] for the specification). Fed from
    /// COMPLETED engaged attempts; consulted by every cooperative arm
    /// alongside the frozen margin rule. Scoped per profile within this
    /// harness's lifetime (the pass-2 drivers create one harness per
    /// cell, which scopes it per cell — the recorded isolation choice).
    engage_loss_gate: EngagedLossGate,
    /// Harness version recorded in manifests (e.g. "9.6-pass1").
    harness_version: String,
    /// Artifact root directory.
    out_root: PathBuf,
    /// Environment label stamped on every record/manifest/join.
    env_label: &'static str,
    /// Pre-connected LAN bridges (owner-gated pass); when set, cells run
    /// against them instead of spawning local loopback bridges.
    lan_bridges: Option<Vec<LiveBridge>>,
}

impl Pass1Harness {
    /// Creates the harness (shared metrics + acceptance store).
    #[must_use]
    pub fn new(
        profile_id: impl Into<String>,
        identity: InstallationIdentity,
        acceptance: AcceptanceStore,
        harness_version: impl Into<String>,
        out_root: impl Into<PathBuf>,
    ) -> Self {
        let telemetry = Arc::new(modelswarm_telemetry::Telemetry::memory().0);
        Self {
            profile_id: profile_id.into(),
            identity,
            telemetry: Arc::clone(&telemetry),
            metrics: Arc::new(PeerMetrics::memory(Arc::clone(&telemetry))),
            acceptance,
            engage_loss_gate: EngagedLossGate::new(),
            harness_version: harness_version.into(),
            out_root: out_root.into(),
            env_label: ENV_LABEL_LOOPBACK_INJECTED,
            lan_bridges: None,
        }
    }

    /// Owner-gated LAN mode: run cells against pre-connected remote
    /// bridges (`pass1_serve` on the other machine) with LAN labels.
    #[must_use]
    pub fn lan_mode(mut self, bridges: Vec<LiveBridge>) -> Self {
        self.env_label = crate::runner::ENV_LABEL_LAN;
        self.lan_bridges = Some(bridges);
        self
    }

    /// Re-arms LAN bridges for the next cell (cells consume the pool;
    /// reconnect per cell for fresh warm-up, matching the dry run's
    /// per-cell bridge lifetimes). Also (re)asserts the LAN label: the
    /// first cell must be labeled LAN too, not the loopback default —
    /// the join/summary rows carry this and must never claim loopback
    /// for a real two-machine run.
    pub fn set_lan_bridges(&mut self, bridges: Vec<LiveBridge>) {
        self.env_label = crate::runner::ENV_LABEL_LAN;
        self.lan_bridges = Some(bridges);
    }

    /// The F15 recorder (observations for the candidate set).
    #[must_use]
    pub fn metrics(&self) -> &Arc<PeerMetrics> {
        &self.metrics
    }

    /// The acceptance-EWMA store.
    #[must_use]
    pub fn acceptance(&self) -> &AcceptanceStore {
        &self.acceptance
    }

    /// Runs one loopback cell end-to-end and writes artifacts.
    pub async fn run_loopback_cell(
        &mut self,
        spec: &LoopbackCellSpec,
        params: &Pass1Params,
        env: &EnvPolicy,
        seed: u64,
    ) -> Result<CellArtifacts, String> {
        let injected = Duration::from_millis(u64::from(spec.injected_delay_ms));
        if !matches!(spec.injected_delay_ms, 0 | 5 | 10 | 20 | 40 | 80 | 150) {
            return Err(format!(
                "injected delay {}ms is outside the frozen network matrix",
                spec.injected_delay_ms
            ));
        }
        // Bridge source: LAN mode uses the pre-connected remote bridges
        // (owner-gated; real QUIC + real RTT); loopback spawns locally
        // with the injected-delay shim.
        let lan_mode = self.lan_bridges.is_some();
        let bridges = match self.lan_bridges.take() {
            Some(lan) => {
                if lan.len() < 2 {
                    return Err("lan cell needs >= 2 bridges".into());
                }
                lan
            }
            None => {
                let mut bridges = Vec::with_capacity(spec.bridges.len());
                for config in &spec.bridges {
                    bridges.push(
                        spawn_bridge(
                            &self.profile_id,
                            &self.identity,
                            config,
                            injected,
                            Arc::clone(&self.telemetry),
                            Arc::clone(&self.metrics),
                        )
                        .await?,
                    );
                }
                if bridges.len() < 2 {
                    return Err("loopback cell needs >= 2 bridges".into());
                }
                bridges
            }
        };
        // Calibration bridge: the SAME engine speeds as the probed cell
        // bridge (the first config) but ZERO injected delay, so the p50
        // subtraction isolates exactly the injected term. Distinct seed ⇒
        // distinct peer id, same workload shape. LAN cells have no
        // injected delay to isolate (the RTT is real), so calibration is
        // recorded as skipped with the LAN label.
        let calibration_bridge = if lan_mode {
            None
        } else {
            let mut calibration_config = BridgeConfig::new(0xCA1, 0.30, 1.5);
            if let Some(first) = spec.bridges.first() {
                calibration_config.decode_tokens_per_ms = first.decode_tokens_per_ms;
                calibration_config.prefill_tokens_per_ms = first.prefill_tokens_per_ms;
                calibration_config.advertised_queue_ms = first.advertised_queue_ms;
                calibration_config.capacity_class = first.capacity_class;
            }
            Some(
                spawn_bridge(
                    &self.profile_id,
                    &self.identity,
                    &calibration_config,
                    Duration::ZERO,
                    Arc::clone(&self.telemetry),
                    Arc::clone(&self.metrics),
                )
                .await?,
            )
        };

        // Warm-up: real completions + idle RTT probes per bridge.
        for bridge in &bridges {
            self.warm_up(bridge, params, &spec.name()).await?;
        }

        // The candidate set: roster facts + F15 observations, shared by
        // every arm; the mapping is the shadow planner's.
        let inputs: Vec<ShadowCandidateInput> = bridges
            .iter()
            .map(|bridge| ShadowCandidateInput {
                roster: bridge.roster.clone(),
                observed: self.metrics.observe(&bridge.peer_id),
            })
            .collect();
        let ordered = self.ordered_candidates(&inputs, env, params);
        if ordered.len() < 2 {
            return Err("loopback cell needs >= 2 measured candidates".into());
        }
        let candidate_set = self.candidate_set_json(&inputs, env, &ordered);
        let rtt_ms = inputs
            .iter()
            .filter_map(|i| i.observed.rtt_ewma_ms)
            .fold(f64::EPSILON, f64::max);

        // Calibration probes (after warm-up so pools are hot). LAN cells
        // skip them: the RTT is real, there is no injected term to
        // isolate, and the local baseline would contaminate the diff.
        let calibration = match &calibration_bridge {
            Some(cb) => {
                self.calibrate(&bridges[0], cb, spec.injected_delay_ms)
                    .await
            }
            None => CalibrationRecord {
                label: self.env_label.to_string(),
                method: "skipped-lan-real-rtt",
                nominal_injected_ms: spec.injected_delay_ms,
                measured_injected_ms_p50: 0.0,
                calibration_valid: None,
                baseline_p50_ms: 0.0,
                cell_p50_ms: 0.0,
                probes: 0,
                transport_note: QUINN_STATS_UNAVAILABLE.to_string(),
            },
        };

        // Arms × corpus × reps; single arm FIRST per prompt so later arms
        // cite its independently executed completions (S4).
        let mut arms: Vec<Arm> = vec![Arm::FastestSingle];
        for k in 2..=params
            .cohort_cap
            .min(u32::try_from(ordered.len()).unwrap_or(2))
        {
            arms.push(Arm::FixedK { k });
        }
        arms.push(Arm::PlannerK);

        let mut joined: Vec<RunJoined> = Vec::new();
        let mut single_totals_by_prompt: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for prompt in PASS1_CORPUS {
            for arm in &arms {
                for rep in 0..params.runs_per_arm {
                    let root = format!("p1-{}/{}/rep{}/{}", spec.name(), arm.tag(), rep, prompt.id);
                    let run = match arm {
                        Arm::FastestSingle => {
                            self.run_single(&ordered, &bridges, prompt, &root, params)
                                .await?
                        }
                        Arm::FixedK { k } => {
                            let acceptance_of = |peer: &str| {
                                self.acceptance.acceptance_ewma(peer, &self.profile_id)
                            };
                            let cohort: Vec<Candidate> =
                                ordered.iter().take(*k as usize).cloned().collect();
                            let acceptance = cohort_acceptance(
                                &cohort,
                                &acceptance_of,
                                params.acceptance_default,
                            );
                            let inputs_at_k = swarm_inputs(
                                &cohort,
                                rtt_ms,
                                acceptance,
                                params.proposal_window,
                                params.output_tokens_target,
                                prompt.documented_prompt_tokens,
                            );
                            let prediction_ms =
                                modelswarm_scheduler::predicted_swarm_ms(&inputs_at_k);
                            let fastest_single = modelswarm_scheduler::predicted_single_ms(
                                &ordered[0],
                                prompt.documented_prompt_tokens,
                                params.output_tokens_target,
                            );
                            // Engage = frozen margin rule AND the per-profile
                            // engaged-loss EWMA gate (cold start allows — the
                            // margin rule alone governs until warm).
                            let margin_engaged = prediction_ms.is_finite()
                                && should_engage(prediction_ms, fastest_single, params.margin);
                            let gate = self.engage_loss_gate.decision(&self.profile_id);
                            let coop_spec = CoopSpec {
                                k: *k,
                                prediction_ms: prediction_ms.is_finite().then_some(prediction_ms),
                                engaged: margin_engaged && !gate.blocked,
                                sweep: None,
                                is_planner: false,
                                loss_gate: prediction_ms.is_finite().then_some(gate),
                            };
                            self.run_cooperative(
                                &ordered,
                                &bridges,
                                spec.regime,
                                coop_spec,
                                &RunTarget {
                                    prompt,
                                    root,
                                    params,
                                },
                            )
                            .await?
                        }
                        Arm::PlannerK => {
                            let acceptance_of = |peer: &str| {
                                self.acceptance.acceptance_ewma(peer, &self.profile_id)
                            };
                            let plan = plan_cohort(
                                &ordered,
                                params,
                                acceptance_of,
                                rtt_ms,
                                prompt.documented_prompt_tokens,
                            );
                            let gate = self.engage_loss_gate.decision(&self.profile_id);
                            let coop_spec = CoopSpec {
                                k: plan.k.unwrap_or(1),
                                prediction_ms: plan.prediction_ms,
                                engaged: plan.engaged && !gate.blocked,
                                sweep: Some(plan.sweep),
                                is_planner: true,
                                loss_gate: plan.prediction_ms.is_some().then_some(gate),
                            };
                            self.run_cooperative(
                                &ordered,
                                &bridges,
                                spec.regime,
                                coop_spec,
                                &RunTarget {
                                    prompt,
                                    root,
                                    params,
                                },
                            )
                            .await?
                        }
                    };
                    if matches!(arm, Arm::FastestSingle) && run.status == RunStatus::Completed {
                        single_totals_by_prompt
                            .entry(prompt.id)
                            .or_default()
                            .push(run.total_ms);
                    }
                    // Feed the per-profile engaged-loss EWMA from COMPLETED
                    // engaged attempts only (`loss_guard` presence == a
                    // cooperative round ran; fallback and aborted rows carry
                    // no clean realized-vs-prediction ratio). The key is
                    // exactly the review's: realized_total_ms /
                    // fastest_single_prediction_ms.
                    if run.status == RunStatus::Completed && run.loss_guard.is_some() {
                        if let Some(prediction) = run
                            .fastest_single_prediction_ms
                            .filter(|p| p.is_finite() && *p > 0.0)
                        {
                            self.engage_loss_gate
                                .record_engaged(&self.profile_id, run.total_ms / prediction);
                        }
                    }
                    joined.push(run);
                }
            }
        }

        // Fill fastest_single_actual from the INDEPENDENT single runs of
        // the same prompt (min over real completions — S4).
        let single_actual = |prompt_id: &str| -> Option<f64> {
            single_totals_by_prompt
                .get(prompt_id)
                .map(|totals| totals.iter().copied().fold(f64::INFINITY, f64::min))
                .filter(|v| v.is_finite())
        };

        let manifests = self.build_manifests(spec, params, seed, &arms);
        let manifest_id_for = |arm: Arm| run_id_from_seed(mix(seed, arm_discriminant(arm)));
        let records: Vec<ModeResultRecord> = joined
            .iter()
            .map(|run| build_record(run, prompt_of(run), &single_actual, &manifest_id_for))
            .collect();
        let join_rows: Vec<JoinRow> = joined
            .iter()
            .map(|run| build_join_row(run, &manifest_id_for, self.env_label))
            .collect();
        let summary = build_summary(spec, &join_rows, self.env_label);

        let dir = self.write_artifacts(
            spec,
            &manifests,
            &records,
            &join_rows,
            &calibration,
            &summary,
        )?;
        std::fs::write(
            dir.join("candidate-set.json"),
            serde_json::to_string_pretty(&candidate_set).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("write candidate set: {e}"))?;
        Ok(CellArtifacts {
            dir,
            manifests,
            records,
            join_rows,
            calibration,
            summary,
        })
    }

    /// The decision-input evidence: every considered candidate with its
    /// measured inputs and the frozen-model ranking the arms shared.
    fn candidate_set_json(
        &self,
        inputs: &[ShadowCandidateInput],
        env: &EnvPolicy,
        ordered: &[Candidate],
    ) -> serde_json::Value {
        let tokens = PASS1_CORPUS[0].documented_prompt_tokens;
        let mapped: Vec<serde_json::Value> = inputs
            .iter()
            .filter_map(|input| {
                let mapped =
                    candidate_from_observations(input, &self.profile_id, tokens, &env.policy)?;
                let predicted = modelswarm_scheduler::predicted_single_ms(
                    &mapped.candidate,
                    tokens,
                    Pass1Params::default().output_tokens_target,
                );
                Some(serde_json::json!({
                    "peer_id": input.roster.peer_id,
                    "roster": {
                        "free_slots": input.roster.free_slots,
                        "advertised_queue_ms": input.roster.advertised_queue_ms,
                        "capacity_class": input.roster.capacity_class,
                    },
                    "measured": {
                        "rtt_ewma_ms": input.observed.rtt_ewma_ms,
                        "ttft_ewma_ms": input.observed.profile(&self.profile_id)
                            .and_then(|p| p.ttft_ewma_ms),
                        "prefill_tokens_per_ms_ewma": input.observed.profile(&self.profile_id)
                            .and_then(|p| p.prefill_tokens_per_ms_ewma),
                        "decode_tokens_per_ms_ewma": input.observed.profile(&self.profile_id)
                            .and_then(|p| p.decode_tokens_per_ms_ewma),
                        "failure_count": input.observed.failure_count,
                    },
                    "effective_queue_ms": round3(mapped.candidate.advertised_queue_ms),
                    "predicted_single_ms": predicted.is_finite().then_some(round3(predicted)),
                }))
            })
            .collect();
        serde_json::json!({
            "label": self.env_label,
            "prompt_tokens_basis": tokens,
            "output_tokens_basis": Pass1Params::default().output_tokens_target,
            "ranking": ordered.iter().map(|c| c.peer_id.clone()).collect::<Vec<_>>(),
            "candidates": mapped,
        })
    }

    /// Warm-up completions + RTT-probe wait for one bridge (unrecorded).
    /// `cell` rides the request id: LAN cells re-dial the SAME long-lived
    /// serve-side bridges, whose replay dedup persists across cells — a
    /// cell-agnostic id would be refused as `replayed_request` from the
    /// second cell on (loopback cells spawn fresh bridges per cell, so
    /// only LAN can hit this).
    async fn warm_up(
        &self,
        bridge: &LiveBridge,
        params: &Pass1Params,
        cell: &str,
    ) -> Result<(), String> {
        for round in 0..params.warmup_completions {
            let request_id = format!("p1-warmup-{cell}/{}/{}", bridge.peer_id, round);
            execute_on(&WireRequest {
                executor: &bridge.executor,
                profile_id: &self.profile_id,
                token: &bridge.token,
                request_id: &request_id,
                prompt: PASS1_CORPUS[0].text,
                committed: "",
                max_tokens: WARMUP_TOKENS,
                seed: VERIFIER_SEED,
            })
            .await
            .map_err(|e| format!("warm-up completion: {e}"))?;
        }
        // The F15 idle probe lands after the last clean completion.
        let deadline = Instant::now() + RTT_WAIT_BOUND;
        loop {
            if self.metrics.observe(&bridge.peer_id).rtt_ewma_ms.is_some() {
                break;
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "RTT probe never landed for {} within {RTT_WAIT_BOUND:?}",
                    bridge.peer_id
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(())
    }

    /// Full frozen-selector ranking of the mapped candidates
    /// (fastest-predicted first).
    fn ordered_candidates(
        &self,
        inputs: &[ShadowCandidateInput],
        env: &EnvPolicy,
        params: &Pass1Params,
    ) -> Vec<Candidate> {
        let tokens = PASS1_CORPUS[0].documented_prompt_tokens;
        let mapped: Vec<Candidate> = inputs
            .iter()
            .filter_map(|input| {
                candidate_from_observations(input, &self.profile_id, tokens, &env.policy)
                    .map(|m| m.candidate)
            })
            .collect();
        let Some(first) = mapped.first() else {
            return mapped;
        };
        let profile = first.profile_id.clone();
        let mut ranked: Vec<Candidate> = select_microswarm(
            &mapped,
            &profile,
            tokens,
            params.output_tokens_target,
            mapped.len(),
        )
        .into_iter()
        .cloned()
        .collect();
        if ranked.is_empty() {
            ranked = mapped;
        }
        ranked
    }

    /// Baseline-subtracted injected-delay calibration: 1-token round
    /// trips against the zero-delay calibration bridge and a cell bridge.
    async fn calibrate(
        &self,
        cell_bridge: &LiveBridge,
        calibration_bridge: &LiveBridge,
        nominal_ms: u32,
    ) -> CalibrationRecord {
        let baseline = probe_bridge(calibration_bridge, &self.profile_id, CALIBRATION_PROBES).await;
        let cell = probe_bridge(cell_bridge, &self.profile_id, CALIBRATION_PROBES).await;
        let baseline_p50 = stats::percentile(&baseline, 50.0);
        let cell_p50 = stats::percentile(&cell, 50.0);
        let measured = (cell_p50 - baseline_p50).max(0.0);
        let valid = if nominal_ms == 0 {
            None
        } else {
            Some((measured - f64::from(nominal_ms)).abs() <= 0.10 * f64::from(nominal_ms))
        };
        CalibrationRecord {
            label: self.env_label.to_string(),
            method: CALIBRATION_METHOD,
            nominal_injected_ms: nominal_ms,
            measured_injected_ms_p50: round3(measured),
            calibration_valid: valid,
            baseline_p50_ms: round3(baseline_p50),
            cell_p50_ms: round3(cell_p50),
            probes: baseline.len().min(cell.len()),
            transport_note: QUINN_STATS_UNAVAILABLE.to_string(),
        }
    }

    /// The fastest-single arm: one full, independently executed request
    /// against the fastest-predicted candidate.
    async fn run_single(
        &self,
        ordered: &[Candidate],
        bridges: &[LiveBridge],
        prompt: &Pass1Prompt,
        root: &str,
        params: &Pass1Params,
    ) -> Result<RunJoined, String> {
        let fastest = &ordered[0];
        let bridge = bridge_for(bridges, &fastest.peer_id)?;
        let prediction = modelswarm_scheduler::predicted_single_ms(
            fastest,
            prompt.documented_prompt_tokens,
            params.output_tokens_target,
        );
        let completion = execute_on(&WireRequest {
            executor: &bridge.executor,
            profile_id: &self.profile_id,
            token: &bridge.token,
            request_id: root,
            prompt: prompt.text,
            committed: "",
            max_tokens: params.output_tokens_target,
            seed: VERIFIER_SEED,
        })
        .await
        .map_err(|e| format!("single arm: {e}"))?;
        let output_tokens = u32::try_from(completion.deltas.len()).unwrap_or(u32::MAX);
        Ok(RunJoined {
            root: root.to_string(),
            arm: Arm::FastestSingle,
            k: None,
            ttft_ms: completion.ttft_ms,
            total_ms: completion.total_ms,
            output_tokens,
            accepted_tokens: output_tokens,
            proposed_tokens: output_tokens,
            rounds: 1,
            aggregate_model_tokens: output_tokens,
            wire_bytes: completion.text().len() as f64,
            proposer_ms: 0.0,
            verifier_ms: completion.total_ms,
            coordinator_ms: 0.0,
            acceptance_rate: None,
            rollbacks: 0,
            status: RunStatus::Completed,
            reason: None,
            predicted_ms: prediction.is_finite().then_some(prediction),
            fastest_single_prediction_ms: prediction.is_finite().then_some(prediction),
            planner_sweep: None,
            loss_guard: None,
            loss_gate: None,
        })
    }

    /// Fixed-k and planner-k arms: engage gate, then wire-round
    /// speculation with client-side delta comparison.
    async fn run_cooperative(
        &self,
        ordered: &[Candidate],
        bridges: &[LiveBridge],
        regime: AcceptanceRegime,
        spec: CoopSpec,
        target: &RunTarget<'_>,
    ) -> Result<RunJoined, String> {
        let RunTarget {
            prompt,
            root,
            params,
        } = target;
        let prompt_tokens = prompt.documented_prompt_tokens;
        let CoopSpec {
            k,
            prediction_ms,
            engaged,
            sweep: planner_sweep,
            is_planner,
            loss_gate,
        } = spec;
        let k_effective = k;
        let fastest_single_prediction = modelswarm_scheduler::predicted_single_ms(
            &ordered[0],
            prompt_tokens,
            params.output_tokens_target,
        );

        // Degenerate planner outcome: no finite prediction anywhere —
        // honest fallback (recorded, single executed).
        if k_effective < 2 {
            let mut single = self
                .run_single(
                    ordered,
                    bridges,
                    prompt,
                    &format!("{root}/fallback"),
                    params,
                )
                .await?;
            single.arm = Arm::PlannerK;
            single.k = Some(1);
            single.status = RunStatus::FellBackToSingle;
            single.reason = Some(
                "planner: no finite cooperative prediction (insufficient measurement)".to_string(),
            );
            single.planner_sweep = planner_sweep;
            return Ok(single);
        }

        // Honest negative: the engage gate blocks the cooperative mode —
        // record the fallback AND still execute the single path so the
        // row carries realized numbers. The reason names the verification
        // term in force (ADR-032 §4 companion correction: wire-true
        // sequential for any cohort whose verifier has not declared
        // batch-verify — the pass-2 never-engage conclusion, enforced).
        if !engaged {
            let mut single = self
                .run_single(
                    ordered,
                    bridges,
                    prompt,
                    &format!("{root}/fallback"),
                    params,
                )
                .await?;
            single.arm = if is_planner {
                Arm::PlannerK
            } else {
                Arm::FixedK { k: k_effective }
            };
            single.k = Some(k_effective);
            single.status = RunStatus::FellBackToSingle;
            // Which gate blocked: engaged == margin_engaged && !gate.blocked,
            // so a blocking loss-gate record here means the margin rule
            // PASSED and the per-profile engaged-loss EWMA refused.
            let verify_label = verify_mode(&ordered[..k_effective as usize]).label();
            let cost_model_reason = format!(
                "cost model v2 [{verify_label}]: cooperative {} × (1+{:.2}) did not beat fastest single {} (engage gate)",
                prediction_ms.map(|p| format!("{:.1}ms", p)).unwrap_or_else(|| "∞".into()),
                params.margin,
                format_fastest(fastest_single_prediction),
            );
            single.reason = Some(match loss_gate.filter(|gate| gate.blocked) {
                Some(gate) => format!(
                    "engaged-loss EWMA {} ≥ {:.1} over {} engaged attempts (α={:.2}, per-profile engage gate) — the margin rule passed but the profile's engaged history loses; falling back to single",
                    gate.ewma.map_or_else(|| "n/a".to_string(), |v| format!("{v:.3}")),
                    gate.threshold,
                    gate.samples,
                    gate.alpha
                ),
                None => cost_model_reason,
            });
            single.predicted_ms = prediction_ms;
            single.planner_sweep = planner_sweep;
            single.rollbacks = 0;
            single.loss_gate = loss_gate;
            return Ok(single);
        }

        // Rounds over the wire.
        let cohort: Vec<Candidate> = ordered.iter().take(k_effective as usize).cloned().collect();
        let verifier = bridge_for(bridges, &cohort[0].peer_id)?;
        let proposers: Vec<&LiveBridge> = cohort[1..]
            .iter()
            .map(|c| bridge_for(bridges, &c.peer_id))
            .collect::<Result<_, _>>()?;
        let target = params.output_tokens_target;
        let window = params.proposal_window;
        let mut committed_text = String::new();
        let mut committed_tokens: u32 = 0;
        let mut accepted_total: u32 = 0;
        let mut proposed_total: u32 = 0;
        let mut rollbacks: u32 = 0;
        let mut rounds: u32 = 0;
        let mut aggregate_model_tokens: u32 = 0;
        let mut wire_bytes: f64 = 0.0;
        let mut proposer_ms: f64 = 0.0;
        let mut verifier_ms: f64 = 0.0;
        let mut round_walls: Vec<f64> = Vec::new();
        let mut ttft_ms: f64 = 0.0;
        let mut status = RunStatus::Completed;
        let mut reason: Option<String> = None;
        let mut loss_guard: Option<LossGuardRecord> = None;

        while committed_tokens < target && rounds < params.max_rounds {
            let remaining = target - committed_tokens;
            let window_eff = window.min(remaining.max(1));
            let round_index = rounds + 1;
            // Draw key mixes rep + prompt so acceptance-profile draws vary
            // per recorded run yet stay reproducible from the artifacts
            // (rep and prompt id are both embedded in the request root).
            let draw_key = mix(u64::from(rep_of(root)), prompt_discriminant(prompt.id));
            let proposer_seed = regime.proposer_seed(round_index, window_eff, draw_key);
            let round_started = Instant::now();
            let mut request_ids = Vec::with_capacity(proposers.len() + 1);
            for index in 0..proposers.len() {
                request_ids.push(format!("{root}/r{round_index}/p{index}"));
            }
            request_ids.push(format!("{root}/r{round_index}/v"));
            let mut wires = Vec::with_capacity(proposers.len() + 1);
            for (index, proposer) in proposers.iter().enumerate() {
                wires.push(WireRequest {
                    executor: &proposer.executor,
                    profile_id: &self.profile_id,
                    token: &proposer.token,
                    request_id: &request_ids[index],
                    prompt: prompt.text,
                    committed: &committed_text,
                    max_tokens: window_eff,
                    seed: proposer_seed,
                });
            }
            wires.push(WireRequest {
                executor: &verifier.executor,
                profile_id: &self.profile_id,
                token: &verifier.token,
                request_id: &request_ids[request_ids.len() - 1],
                prompt: prompt.text,
                committed: &committed_text,
                max_tokens: window_eff + 1,
                seed: VERIFIER_SEED,
            });
            let results = join_all(wires.iter().map(execute_on)).await;
            let round_wall = round_started.elapsed().as_secs_f64() * 1000.0;
            round_walls.push(round_wall);
            if rounds == 0 {
                ttft_ms = round_wall;
            }
            rounds += 1;

            let mut proposer_completions: Vec<WireCompletion> = Vec::with_capacity(proposers.len());
            for result in &results[..proposers.len()] {
                match result {
                    Ok(completion) => proposer_completions.push(completion.clone()),
                    Err(_) => status = RunStatus::Failed,
                }
            }
            let verifier_completion = match results.last() {
                Some(Ok(completion)) => Some(completion.clone()),
                _ => None,
            };
            let Some(verifier_completion) = verifier_completion else {
                reason = Some("verifier round failed on the wire".to_string());
                status = RunStatus::Failed;
                break;
            };
            let verifier_deltas = &verifier_completion.deltas;
            verifier_ms += verifier_completion.total_ms;
            aggregate_model_tokens += window_eff + 1;
            wire_bytes += f64::from(window_eff + 1) * 4.0;

            // Acceptance per proposer measured on the real deltas and
            // recorded into the acceptance store; the round's branch
            // choice is the deterministic rotation.
            let chosen_index = (round_index as usize - 1) % proposers.len();
            let mut chosen_matched: u32 = 0;
            for (index, completion) in proposer_completions.iter().enumerate() {
                proposer_ms += completion.total_ms;
                aggregate_model_tokens += window_eff;
                wire_bytes += f64::from(window_eff) * 4.0 + CANDIDATE_BLOCK_OVERHEAD_BYTES as f64;
                let matched = leading_match(&completion.deltas, verifier_deltas);
                self.acceptance.record_round(
                    &proposers[index].peer_id,
                    &self.profile_id,
                    matched,
                    window_eff,
                );
                if index == chosen_index {
                    chosen_matched = matched;
                }
            }
            proposed_total += window_eff;
            accepted_total += chosen_matched;

            // Commit: matched prefix + the verifier's corrective token on
            // mismatch (speculative semantics — the committed output IS
            // the verifier's greedy continuation by construction).
            let mismatch =
                chosen_matched < window_eff && (chosen_matched as usize) < verifier_deltas.len();
            let commit_count = chosen_matched + u32::from(mismatch);
            for delta in verifier_deltas.iter().take(commit_count as usize) {
                committed_text.push_str(delta);
            }
            committed_tokens += commit_count;
            if mismatch {
                rollbacks += 1;
            }

            // Observed-loss guard (round 1 only — pre-first-committed-
            // token, honoring AGENTS rule 6): a first round beyond
            // `multiplier × (1 + margin)` the predicted per-round budget
            // aborts to single. The multiplier defaults to the production
            // value 2.0; pass 2 relaxes or disables it via a RECORDED env
            // override to measure full engaged curves — every row then
            // carries the guard record with the default-production
            // counterfactual (`would_fire_at_default`), so a relaxed run
            // can never masquerade as guard-passing.
            if rounds == 1 {
                if let Some(prediction) = prediction_ms {
                    let acceptance = cohort_acceptance(
                        &cohort,
                        &|peer: &str| self.acceptance.acceptance_ewma(peer, &self.profile_id),
                        params.acceptance_default,
                    );
                    let rounds_expected =
                        (f64::from(target) / (f64::from(window) * acceptance)).max(1.0);
                    let predicted_round = prediction / rounds_expected;
                    let threshold_at =
                        |multiplier: f64| multiplier * (1.0 + params.margin) * predicted_round;
                    let would_fire_at_default =
                        round_wall > threshold_at(crate::params::DEFAULT_LOSS_MULTIPLIER);
                    let (fired, multiplier_label) = match params.loss_multiplier {
                        Some(multiplier) => (
                            round_wall > threshold_at(multiplier),
                            format!("{multiplier:.2}"),
                        ),
                        None => (false, "off".to_string()),
                    };
                    loss_guard = Some(LossGuardRecord {
                        multiplier: multiplier_label.clone(),
                        round1_wall_ms: round_wall,
                        predicted_round_ms: predicted_round,
                        fired,
                        would_fire_at_default,
                    });
                    if fired {
                        let mut single = self
                            .run_single(
                                ordered,
                                bridges,
                                prompt,
                                &format!("{root}/observed-loss"),
                                params,
                            )
                            .await?;
                        single.arm = if is_planner {
                            Arm::PlannerK
                        } else {
                            Arm::FixedK { k: k_effective }
                        };
                        single.k = Some(k_effective);
                        single.status = RunStatus::FellBackToSingle;
                        single.reason = Some(format!(
                            "observed loss: round-1 wall {:.1}ms > {}×(1+{:.2})×predicted-per-round {:.1}ms (abort pre-first-token, rule 6)",
                            round_wall, multiplier_label, params.margin, predicted_round
                        ));
                        single.predicted_ms = prediction_ms;
                        single.planner_sweep = planner_sweep;
                        single.loss_guard = loss_guard;
                        single.loss_gate = loss_gate;
                        return Ok(single);
                    }
                }
            }
        }

        let completion_total: f64 = round_walls.iter().sum();
        let arm_of_run = || {
            if is_planner {
                Arm::PlannerK
            } else {
                Arm::FixedK { k: k_effective }
            }
        };
        if committed_tokens == 0 {
            return Ok(RunJoined {
                root: root.to_string(),
                arm: arm_of_run(),
                k: Some(k_effective),
                ttft_ms,
                total_ms: completion_total,
                output_tokens: 0,
                accepted_tokens: 0,
                proposed_tokens: proposed_total,
                rounds,
                aggregate_model_tokens,
                wire_bytes,
                proposer_ms,
                verifier_ms,
                coordinator_ms: 0.0,
                acceptance_rate: Some(0.0),
                rollbacks,
                status: RunStatus::Failed,
                reason: Some(
                    "zero speculative acceptance: cooperative mode cannot progress (negative result)"
                        .to_string(),
                ),
                predicted_ms: prediction_ms,
                fastest_single_prediction_ms: fastest_single_prediction.is_finite()
                    .then_some(fastest_single_prediction),
                planner_sweep,
                loss_guard,
                loss_gate,
            });
        }
        let coordinator_ms = (completion_total - proposer_ms.max(verifier_ms)).max(0.0);
        let acceptance_rate = if proposed_total == 0 {
            None
        } else {
            Some(f64::from(accepted_total) / f64::from(proposed_total))
        };
        Ok(RunJoined {
            root: root.to_string(),
            arm: arm_of_run(),
            k: Some(k_effective),
            ttft_ms,
            total_ms: completion_total,
            output_tokens: committed_tokens,
            accepted_tokens: accepted_total,
            proposed_tokens: proposed_total,
            rounds,
            aggregate_model_tokens,
            wire_bytes,
            proposer_ms,
            verifier_ms,
            coordinator_ms,
            acceptance_rate,
            rollbacks,
            status,
            reason,
            predicted_ms: prediction_ms,
            fastest_single_prediction_ms: fastest_single_prediction
                .is_finite()
                .then_some(fastest_single_prediction),
            planner_sweep,
            loss_guard,
            loss_gate,
        })
    }

    /// One run manifest per arm (frozen schema; the arm tag and the
    /// environment label ride `harness_version`, the runtime block names
    /// the TEST-ONLY synthetic executor).
    fn build_manifests(
        &self,
        spec: &LoopbackCellSpec,
        params: &Pass1Params,
        seed: u64,
        arms: &[Arm],
    ) -> Vec<(String, RunManifest)> {
        arms.iter()
            .map(|arm| {
                let manifest_seed = mix(seed, arm_discriminant(*arm));
                let mut manifest = RunManifest::new(
                    run_id_from_seed(manifest_seed),
                    crate::rfc3339_now(),
                    format!(
                        "{}+arm={}+env={}",
                        self.harness_version,
                        arm.tag(),
                        self.env_label
                    ),
                    self.profile_id.clone(),
                    ManifestRuntime {
                        name: SYNTHETIC_EXECUTOR_NAME.to_string(),
                        version: "v1 (TEST-ONLY, behind the real serving bridge)".to_string(),
                        build_hash: synthetic_executor_build_hash(),
                    },
                    ManifestNetworkCell {
                        nominal_rtt_ms: spec.injected_delay_ms,
                        jitter: JitterClass::None,
                        loss_ratio: 0.0,
                        path: CellPath::Direct,
                        measured_rtt_ms_p50: None,
                        calibration_valid: None,
                    },
                    vec![manifest_seed],
                );
                manifest.prompt_corpus_id = Some(PASS1_CORPUS_ID.to_string());
                manifest.warmup_runs = Some(params.warmup_completions * spec.bridges.len() as u32);
                manifest.recorded_runs = Some(params.runs_per_arm * PASS1_CORPUS.len() as u32);
                manifest.confidence_margin = Some(params.margin);
                manifest.os = Some(std::env::consts::OS.to_string());
                manifest.hardware_class_peers = Some(
                    spec.bridges
                        .iter()
                        .map(|b| b.capacity_class.to_string())
                        .collect(),
                );
                (arm.tag(), manifest)
            })
            .collect()
    }

    /// Writes the cell's artifacts (manifests, mode-results.jsonl,
    /// decision-join.jsonl, calibration.json, acceptance snapshot,
    /// summary.json).
    fn write_artifacts(
        &self,
        spec: &LoopbackCellSpec,
        manifests: &[(String, RunManifest)],
        records: &[ModeResultRecord],
        join_rows: &[JoinRow],
        calibration: &CalibrationRecord,
        summary: &CellArtifactsSummary,
    ) -> Result<PathBuf, String> {
        let dir = self.out_root.join(spec.name());
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
        for (tag, manifest) in manifests {
            std::fs::write(
                dir.join(format!("manifest-{tag}.json")),
                serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("write manifest: {e}"))?;
        }
        write_jsonl(&dir.join("mode-results.jsonl"), records)?;
        write_jsonl(&dir.join("decision-join.jsonl"), join_rows)?;
        std::fs::write(
            dir.join("calibration.json"),
            serde_json::to_string_pretty(calibration).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("write calibration: {e}"))?;
        let snapshot: Vec<serde_json::Value> = self
            .acceptance
            .snapshot()
            .into_iter()
            .map(|(peer, profile, state)| {
                serde_json::json!({
                    "peer_id": peer,
                    "profile_id": profile,
                    "acceptance_ewma": round3(state.ewma),
                    "rounds": state.rounds,
                    "accepted_count": state.accepted_count,
                    "proposed_count": state.proposed_count,
                    "lifetime_rate": round3(state.lifetime_rate()),
                })
            })
            .collect();
        std::fs::write(
            dir.join("acceptance-snapshot.json"),
            serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("write acceptance: {e}"))?;
        std::fs::write(
            dir.join("summary.json"),
            serde_json::to_string_pretty(summary).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("write summary: {e}"))?;
        Ok(dir)
    }
}

// ---------------------------------------------------------------------------
// Record / join / summary construction
// ---------------------------------------------------------------------------

fn prompt_of(run: &RunJoined) -> &'static Pass1Prompt {
    // The root embeds the prompt id as its last path segment.
    let id = run.root.rsplit('/').next().unwrap_or("p1-short");
    PASS1_CORPUS
        .iter()
        .find(|p| p.id == id)
        .unwrap_or(&PASS1_CORPUS[0])
}

fn build_record(
    run: &RunJoined,
    prompt: &Pass1Prompt,
    single_actual: &impl Fn(&str) -> Option<f64>,
    manifest_id_for: &impl Fn(Arm) -> String,
) -> ModeResultRecord {
    let manifest_id = manifest_id_for(run.arm);
    let fastest_single_actual = if matches!(run.arm, Arm::FastestSingle) {
        Some(run.total_ms)
    } else {
        single_actual(prompt.id)
    };
    let cooperative = !matches!(run.arm, Arm::FastestSingle);
    let accepted_per_round =
        cooperative.then(|| f64::from(run.accepted_tokens) / f64::from(run.rounds.max(1)));
    let per_output_token = if run.output_tokens > 0 {
        run.total_ms / f64::from(run.output_tokens)
    } else {
        0.0
    };
    let bytes_per_accepted =
        (run.accepted_tokens > 0).then(|| run.wire_bytes / f64::from(run.accepted_tokens));
    let mut compute_roles = BTreeMap::new();
    compute_roles.insert("coordinator".to_string(), round3(run.coordinator_ms));
    if run.proposer_ms > 0.0 {
        compute_roles.insert("proposer".to_string(), round3(run.proposer_ms));
    }
    compute_roles.insert("verifier".to_string(), round3(run.verifier_ms));
    let metrics = ModeMetrics {
        ttft_ms: round3(run.ttft_ms),
        per_output_token_ms: round3(per_output_token),
        completion_ms: round3(run.total_ms),
        prompt_tokens: prompt.documented_prompt_tokens,
        output_tokens: run.output_tokens,
        fastest_single_prediction_ms: run
            .fastest_single_prediction_ms
            .filter(|v| v.is_finite())
            .map(round3),
        fastest_single_actual_ms: fastest_single_actual.map(round3),
        cooperative_prediction_ms: run.predicted_ms.filter(|v| v.is_finite()).map(round3),
        proposal_window_tokens: cooperative.then_some(crate::PROPOSAL_WINDOW),
        accepted_tokens_per_round: accepted_per_round.map(round3),
        acceptance_rate: run
            .acceptance_rate
            .map(|r| (r.clamp(0.0, 1.0) * 1e6).round() / 1e6),
        mean_acceptance_length: None,
        verification_steps: cooperative.then_some(run.rounds),
        rollback_count: cooperative.then_some(run.rollbacks),
        bytes_per_accepted_token: bytes_per_accepted.map(round3),
        aggregate_model_tokens: Some(run.aggregate_model_tokens),
        compute_ms_by_role: Some(compute_roles),
        rtt_ms_p50: None,
        jitter_ms: Some(0.0),
        packet_loss_ratio: Some(0.0),
        nat_path: Some(NatPath::Direct),
        exactness: Some(if cooperative {
            Exactness::GreedyEqual
        } else {
            Exactness::Na
        }),
    };
    let outcome = RunOutcome {
        status: run.status,
        fallback_reason: run.reason.clone(),
        failure_reason: (run.status == RunStatus::Failed).then(|| {
            run.reason
                .clone()
                .unwrap_or_else(|| "unknown failure".into())
        }),
    };
    let peers: u8 = match run.arm {
        Arm::FastestSingle => 1,
        _ => u8::try_from(run.k.unwrap_or(1).clamp(1, 64)).unwrap_or(1),
    };
    ModeResultRecord::new(&manifest_id, run.arm.mode(), peers, metrics, outcome)
}

fn rep_of(root: &str) -> u32 {
    root.split('/')
        .find_map(|segment| segment.strip_prefix("rep").and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}

/// Stable per-prompt discriminator for acceptance-profile draw keys (the
/// prompt id is part of the request root, so draws are reproducible from
/// the artifacts alone).
fn prompt_discriminant(prompt_id: &str) -> u64 {
    prompt_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

fn build_join_row(
    run: &RunJoined,
    manifest_id_for: &impl Fn(Arm) -> String,
    env_label: &str,
) -> JoinRow {
    JoinRow {
        request_root: run.root.clone(),
        arm: run.arm.tag(),
        k: run.k,
        prompt_id: prompt_of(run).id.to_string(),
        rep: rep_of(&run.root),
        predicted_ms: run.predicted_ms.filter(|v| v.is_finite()).map(round3),
        fastest_single_prediction_ms: run
            .fastest_single_prediction_ms
            .filter(|v| v.is_finite())
            .map(round3),
        realized_ttft_ms: round3(run.ttft_ms),
        realized_total_ms: round3(run.total_ms),
        accepted_tokens: run.accepted_tokens,
        proposed_tokens: run.proposed_tokens,
        rounds: run.rounds,
        status: format!("{:?}", run.status),
        reason: run.reason.clone(),
        run_manifest_id: manifest_id_for(run.arm),
        planner_sweep: run.planner_sweep.clone(),
        label: env_label.to_string(),
        loss_guard: run.loss_guard.clone(),
        loss_gate: run.loss_gate,
    }
}

fn build_summary(
    spec: &LoopbackCellSpec,
    join_rows: &[JoinRow],
    env_label: &str,
) -> CellArtifactsSummary {
    let single_totals: Vec<f64> = join_rows
        .iter()
        .filter(|r| r.arm == "fastest-single" && r.status == "Completed")
        .map(|r| r.realized_total_ms)
        .collect();
    let single_median =
        (!single_totals.is_empty()).then(|| round3(stats::percentile(&single_totals, 50.0)));
    let mut arms = Vec::new();
    let mut tags: Vec<String> = join_rows.iter().map(|r| r.arm.clone()).collect();
    tags.sort();
    tags.dedup();
    for tag in tags {
        let rows: Vec<&JoinRow> = join_rows.iter().filter(|r| r.arm == tag).collect();
        let completions: Vec<f64> = rows.iter().map(|r| r.realized_total_ms).collect();
        let summary = stats::aggregate(&completions);
        let accepted: Vec<f64> = rows
            .iter()
            .filter(|r| r.rounds > 1)
            .map(|r| r.accepted_tokens as f64)
            .collect();
        arms.push(ArmSummary {
            arm: tag.clone(),
            k: rows.iter().find(|r| r.k.is_some()).and_then(|r| r.k),
            n: summary.n,
            completion_ms: summary,
            fallbacks: rows
                .iter()
                .filter(|r| r.status == "FellBackToSingle")
                .count(),
            failures: rows.iter().filter(|r| r.status == "Failed").count(),
            median_vs_fastest_single: single_median
                .map(|single| round3(stats::percentile(&completions, 50.0) / single)),
            accepted_tokens_per_round_median: (!accepted.is_empty())
                .then(|| round3(stats::percentile(&accepted, 50.0))),
            // Acceptance aggregates only ENGAGED runs (rounds > 1):
            // fallback rows executed the single path and carry no
            // speculative acceptance.
            acceptance_rate_mean: {
                let rates: Vec<f64> = rows
                    .iter()
                    .filter(|r| r.rounds > 1 && r.proposed_tokens > 0)
                    .map(|r| f64::from(r.accepted_tokens) / f64::from(r.proposed_tokens))
                    .collect();
                (!rates.is_empty()).then(|| round3(rates.iter().sum::<f64>() / rates.len() as f64))
            },
        });
    }
    CellArtifactsSummary {
        cell: spec.name(),
        label: env_label.to_string(),
        regime: spec.regime.cell_tag(),
        injected_delay_ms: spec.injected_delay_ms,
        arms,
        fastest_single_median_ms: single_median,
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn write_jsonl<T: Serialize>(path: &std::path::Path, rows: &[T]) -> Result<(), String> {
    use std::io::Write;
    let mut file =
        std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    for row in rows {
        writeln!(
            file,
            "{}",
            serde_json::to_string(row).map_err(|e| e.to_string())?
        )
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Deterministic arm discriminator for seed mixing.
fn arm_discriminant(arm: Arm) -> u64 {
    match arm {
        Arm::FastestSingle => 0xA1,
        Arm::FixedK { k } => 0xB2 ^ (u64::from(k) << 8),
        Arm::PlannerK => 0xC3,
    }
}

fn bridge_for<'b>(bridges: &'b [LiveBridge], peer_id: &str) -> Result<&'b LiveBridge, String> {
    bridges
        .iter()
        .find(|b| b.peer_id == peer_id)
        .ok_or_else(|| format!("no bridge for peer {peer_id}"))
}

async fn probe_bridge(bridge: &LiveBridge, profile_id: &str, probes: usize) -> Vec<f64> {
    let mut totals = Vec::with_capacity(probes);
    for i in 0..probes {
        let request_id = format!("p1-cal-{}/{}", bridge.peer_id, i);
        if let Ok(completion) = execute_on(&WireRequest {
            executor: &bridge.executor,
            profile_id,
            token: &bridge.token,
            request_id: &request_id,
            prompt: "cal",
            committed: "",
            max_tokens: 1,
            seed: VERIFIER_SEED,
        })
        .await
        {
            totals.push(completion.total_ms);
        }
    }
    totals
}

fn leading_match(proposer: &[String], verifier: &[String]) -> u32 {
    u32::try_from(
        proposer
            .iter()
            .zip(verifier.iter())
            .take_while(|(a, b)| a == b)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// Mean measured acceptance over the cohort's proposers (verifier is
/// `cohort[0]`; cold pairs fall back to the recorded prior).
fn cohort_acceptance(
    cohort: &[Candidate],
    acceptance_of: &impl Fn(&str) -> Option<f64>,
    prior: f64,
) -> f64 {
    let mut sum = 0.0;
    let mut n = 0usize;
    for proposer in cohort.iter().skip(1) {
        sum += acceptance_of(&proposer.peer_id)
            .unwrap_or(prior)
            .clamp(0.0, 1.0);
        n += 1;
    }
    if n == 0 {
        prior.clamp(0.0, 1.0)
    } else {
        (sum / n as f64).clamp(0.01, 1.0)
    }
}

fn should_engage(prediction: f64, fastest_single: f64, margin: f64) -> bool {
    modelswarm_scheduler::should_engage_cooperative(prediction, fastest_single, margin)
        .unwrap_or(false)
}

fn format_fastest(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.1}ms")
    } else {
        "∞".to_string()
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_names_encode_delay_and_regime() {
        let spec = LoopbackCellSpec {
            injected_delay_ms: 20,
            regime: AcceptanceRegime::Mixed,
            bridges: vec![],
            window_tag: None,
        };
        assert_eq!(spec.name(), "inj20ms-mixed");
    }

    #[test]
    fn leading_match_counts_common_delta_prefix() {
        let p = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let v = vec!["a".to_string(), "b".to_string(), "x".to_string()];
        assert_eq!(leading_match(&p, &v), 2);
        assert_eq!(leading_match(&p, &p), 3);
        assert_eq!(leading_match(&[], &v), 0);
    }

    #[test]
    fn set_lan_bridges_labels_cells_lan_not_loopback() {
        // The LAN run re-arms bridges per cell; if the re-arm doesn't
        // flip the label, every join/summary row ships as
        // loopback+injected-delay for a real two-machine run (found on
        // the 2026-10-10 LAN pass: console said lan-2machine-quic from a
        // hardcoded constant while all 1,080 rows said loopback).
        let dir = std::env::temp_dir().join(format!("msp-lan-label-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let acceptance = crate::acceptance::AcceptanceStore::open(dir.join("acceptance.sqlite"))
            .expect("open acceptance store");
        let mut harness = Pass1Harness::new(
            "msp1:aa",
            InstallationIdentity::from_bytes(&[0xB1; 32]),
            acceptance,
            "label-test",
            dir.clone(),
        );
        assert_eq!(harness.env_label, ENV_LABEL_LOOPBACK_INJECTED);
        harness.set_lan_bridges(Vec::new());
        assert_eq!(harness.env_label, crate::runner::ENV_LABEL_LAN);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn join_rows_drive_the_summary_ratios() {
        let spec = LoopbackCellSpec {
            injected_delay_ms: 10,
            regime: AcceptanceRegime::High,
            bridges: vec![],
            window_tag: None,
        };
        let rows = vec![
            JoinRow {
                request_root: "r1".into(),
                arm: "fastest-single".into(),
                k: None,
                prompt_id: "p1-short".into(),
                rep: 0,
                predicted_ms: Some(100.0),
                fastest_single_prediction_ms: Some(100.0),
                realized_ttft_ms: 10.0,
                realized_total_ms: 100.0,
                accepted_tokens: 8,
                proposed_tokens: 8,
                rounds: 1,
                status: "Completed".into(),
                reason: None,
                run_manifest_id: "run-aaaaaaaaaaaaaaaa".into(),
                planner_sweep: None,
                label: ENV_LABEL_LOOPBACK_INJECTED.into(),
                loss_guard: None,
                loss_gate: None,
            },
            JoinRow {
                request_root: "r2".into(),
                arm: "planner-k".into(),
                k: Some(2),
                prompt_id: "p1-short".into(),
                rep: 0,
                predicted_ms: Some(80.0),
                fastest_single_prediction_ms: Some(100.0),
                realized_ttft_ms: 20.0,
                realized_total_ms: 200.0,
                accepted_tokens: 6,
                proposed_tokens: 8,
                rounds: 4,
                status: "Completed".into(),
                reason: None,
                run_manifest_id: "run-bbbbbbbbbbbbbbbb".into(),
                planner_sweep: Some(vec![(2, 80.0)]),
                label: ENV_LABEL_LOOPBACK_INJECTED.into(),
                loss_guard: None,
                loss_gate: None,
            },
        ];
        let summary = build_summary(&spec, &rows, ENV_LABEL_LOOPBACK_INJECTED);
        assert_eq!(summary.fastest_single_median_ms, Some(100.0));
        let planner = summary.arms.iter().find(|a| a.arm == "planner-k").unwrap();
        // 200ms realized vs 100ms single: a VISIBLE NEGATIVE (ratio 2.0).
        assert_eq!(planner.median_vs_fastest_single, Some(2.0));
        assert_eq!(planner.fallbacks, 0);
        assert!((planner.acceptance_rate_mean.unwrap() - 0.75).abs() < 1e-9);
    }
}
