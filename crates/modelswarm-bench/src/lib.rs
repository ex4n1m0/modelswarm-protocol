//! Benchmark harness (spec: `docs/research/bench-harness-spec.md`; frozen
//! by ADR-013): runs the fastest-single comparator and cooperative modes
//! under the frozen network matrix, emits machine-readable records
//! conforming to `experiments/schemas/`, and computes statistics with
//! deterministic seeded RNG. Negative results (fallbacks, failures) are
//! recorded with the same fidelity as wins.
//!
//! # Phase C honesty boundary (ADR-019)
//!
//! This harness executes **synthetic decode rounds against the MockRuntime**
//! with artificial per-round latency derived from the network cell — pure
//! computation, no wall-clock measurement of real inference. When the
//! runtime is the mock ([`BenchEngine::is_mock`]):
//!
//! - every record's manifest pins `runtime.name = "mock"` (the frozen
//!   schemas are closed, so the machine-readable label lives in the
//!   manifest the record links to via `run_manifest_id`);
//! - every human-facing report line embeds [`TEST_ONLY_MOCK_LABEL`];
//! - all numbers are structural evidence only — no performance claim.

pub mod acceptance;
pub mod corpus;
pub mod divergence;
pub mod engage_gate;
#[cfg(feature = "quic-runner")]
pub mod harness;
pub mod params;
pub mod records;
#[cfg(feature = "quic-runner")]
pub mod runner;
pub mod shadow_join;
pub mod stats;

pub use corpus::{Pass1Prompt, PASS1_CORPUS, PASS1_CORPUS_ID};
pub use params::Pass1Params;
pub use records::{
    CellPath, Exactness, JitterClass, ManifestNetworkCell, ManifestRuntime, Mode, ModeMetrics,
    ModeResultRecord, RunManifest, RunOutcome, RunStatus, MODE_RESULT_RECORD_TYPE,
    RUN_MANIFEST_RECORD_TYPE,
};
pub use stats::{aggregate, bootstrap_ci, percentile, CellSummary, Xorshift};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use modelswarm_runtime::{
    InferenceRuntime, SamplingParams, MOCK_DECODE_TOKENS_PER_MS, MOCK_PREFILL_TOKENS_PER_MS,
    MOCK_RUNTIME_NAME,
};
use modelswarm_scheduler::{
    predicted_single_ms, predicted_swarm_ms, select_microswarm, should_engage_cooperative,
    Candidate, CapacityClass, NatPath, SwarmInputs, DEFAULT_CONFIDENCE_MARGIN,
};
use modelswarm_speculation::{
    branch_assignment, verify_tree_greedy, BranchBlock, CandidateTrie, TrieLimits,
};
use modelswarm_types::ModelProfileId;
use serde_json::json;
use sha2::{Digest as ShaDigest, Sha256};

/// Literal label required on every mock-backend report line (ADR-019).
pub const TEST_ONLY_MOCK_LABEL: &str = "TEST-ONLY mock backend";
/// Warm-up runs per cell, unrecorded (bench-harness-spec).
pub const WARMUP_RUNS: u16 = 5;
/// Synthetic prompt corpus id.
pub const PROMPT_CORPUS_ID: &str = "synthetic-mock-v1";
/// Synthetic per-run output budget.
pub const OUTPUT_TOKENS_TARGET: u32 = 64;
/// Speculative proposal window (bench-harness-spec window sweep member).
pub const PROPOSAL_WINDOW: u32 = 8;
/// Candidate pool size simulated per run.
pub const SYNTHETIC_PEERS: usize = 4;
/// Planner-side acceptance estimate feeding the expected-rollback term
/// (synthetic TEST-ONLY coefficient; real planners use measured EWMA).
pub const ESTIMATED_ACCEPTANCE: f64 = 0.8;
/// Synthetic coefficient: batch verification of one proposal window costs
/// this many sequential decode steps on the verifier (one forward pass over
/// the window plus overhead). TEST-ONLY model constant — the ADR-013
/// ENGINE term. Per the ADR-032 §4 companion correction this term is
/// charged ONLY for cohorts whose verifier declared batch-verify
/// ([`modelswarm_scheduler::Candidate::batch_verify`]); every other
/// cohort is charged the wire-true sequential term (window+1 verifier
/// tokens per round plus the per-round prefix re-post — see
/// `runner::swarm_inputs` / `runner::VerifyMode` behind the `quic-runner`
/// feature). The Phase C mock engine below models the batch-verify
/// engine directly, so its synthetic candidates declare the capability
/// and its inline `SwarmInputs` derivations keep this constant.
pub const VERIFY_BATCH_STEPS: f64 = 1.5;
/// Synthetic pool asymmetry (bench-harness-spec "asymmetric hardware"):
/// one fast-but-busy host (drafts for slower verifiers).
pub const DRAFTER_SPEEDUP: f64 = 3.0;
/// The busy fast host's advertised queue backlog (ms). Large enough that
/// its own single-stream prediction stays above the verifier-class hosts:
/// it is drafted *from*, not picked as the fastest single comparator.
pub const DRAFTER_QUEUE_MS: f64 = 500.0;
/// Synthetic failure-risk penalty (ms) for the v2 cost model.
pub const FAILURE_RISK_PENALTY_MS: f64 = 5.0;
/// Fixed failure penalty (ms) applied to synthetic candidates.
pub const PEER_FAILURE_PENALTY_MS: f64 = 5.0;
/// Guard: maximum speculative rounds before declaring no-progress.
pub const MAX_SPECULATIVE_ROUNDS: u32 = 512;
/// Phase E trie bounds of [`BenchEngine::run_cell_multi`] (E2): 7 proposer
/// slots at window 8 ⇒ ≤ 7 branches / ≤ 56 nodes well inside the bounds, so
/// pruning only fires for degenerate windows — and is then counted.
pub const MULTI_TRIE_LIMITS: TrieLimits = TrieLimits {
    max_branches: 8,
    max_nodes: 128,
};
/// Wire-envelope overhead per candidate block in the
/// `bytes_per_accepted_token` model (JSON framing, hashes, ids —
/// TEST-ONLY synthetic constant).
pub const CANDIDATE_BLOCK_OVERHEAD_BYTES: usize = 64;
/// Synthetic prompt text (deterministic, no real corpus in Phase C).
const SYNTHETIC_PROMPT: &str = "modelswarm bench synthetic prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; ";

/// Public alias: jitter dimension of the network cell.
pub type Jitter = JitterClass;

/// One cell of the frozen network matrix (bench-harness-spec).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetworkCell {
    /// Nominal RTT in ms; must be one of 0/5/10/20/40/80/150.
    pub nominal_rtt_ms: u32,
    /// Jitter class.
    pub jitter: Jitter,
    /// Packet loss ratio; must be one of 0/0.001/0.01/0.03.
    pub loss: f32,
    /// Path type.
    pub path: CellPath,
}

impl NetworkCell {
    /// A loopback-shaped cell (RTT 0, no jitter, no loss).
    pub const LOOPBACK: Self = Self {
        nominal_rtt_ms: 0,
        jitter: Jitter::None,
        loss: 0.0,
        path: CellPath::Direct,
    };

    /// True when the cell is inside the frozen matrix.
    pub fn is_in_matrix(&self) -> bool {
        const RTTS: [u32; 7] = [0, 5, 10, 20, 40, 80, 150];
        const LOSSES: [f32; 4] = [0.0, 0.001, 0.01, 0.03];
        RTTS.contains(&self.nominal_rtt_ms) && LOSSES.iter().any(|l| (*l - self.loss).abs() < 1e-7)
    }
}

/// Errors produced by the harness.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// The runtime failed during a recorded run.
    #[error("runtime error during bench run: {0}")]
    Runtime(String),
    /// Record/manifest writing failed.
    #[error("io error writing bench artifacts: {0}")]
    Io(#[from] std::io::Error),
}

/// Per-run shared simulation context (bundle keeps helper signatures
/// under the argument-count lint).
struct RunContext<'a> {
    candidates: &'a [Candidate],
    cell: &'a NetworkCell,
    prompt_tokens: u32,
    handle: &'a modelswarm_runtime::Handle,
    sampling: &'a SamplingParams,
}

/// The benchmark engine: one runtime + harness version, many cells.
pub struct BenchEngine {
    runtime: Arc<dyn InferenceRuntime>,
    harness_version: String,
}

impl BenchEngine {
    /// Creates an engine over any [`InferenceRuntime`] (the bench crate
    /// itself enables the runtime's `mock-runtime` feature and is the only
    /// intended consumer of it in Phase C).
    pub fn new(runtime: Arc<dyn InferenceRuntime>, harness_version: impl Into<String>) -> Self {
        Self {
            runtime,
            harness_version: harness_version.into(),
        }
    }

    /// The runtime under test.
    pub fn runtime(&self) -> &Arc<dyn InferenceRuntime> {
        &self.runtime
    }

    /// Harness version string (recorded in every manifest).
    pub fn harness_version(&self) -> &str {
        &self.harness_version
    }

    /// True when the runtime identifies itself as the mock backend.
    pub fn is_mock(&self) -> bool {
        self.runtime.id().name() == MOCK_RUNTIME_NAME
    }

    /// The honesty label applied to every report line when [`Self::is_mock`].
    pub fn mock_label(&self) -> Option<&'static str> {
        match self.is_mock() {
            true => Some(TEST_ONLY_MOCK_LABEL),
            false => None,
        }
    }

    /// Deterministic run id: `run-` + first 16 hex of sha256(seed).
    pub fn run_id(seed: u64) -> String {
        run_id_from_seed(seed)
    }

    /// Mandatory comparator (ADR-013 / bench-harness-spec): `single` mode
    /// executed in the same cell, with the fastest-of-N selection recorded
    /// as `fastest_single_prediction_ms` + `fastest_single_actual_ms`.
    pub async fn single_mode_baseline(
        &self,
        cell: &NetworkCell,
        runs: u16,
        seed: u64,
    ) -> Vec<ModeResultRecord> {
        self.run_cell(cell, Mode::Single, 1, runs, seed).await
    }

    /// Executes `runs` recorded runs (after [`WARMUP_RUNS`] unrecorded
    /// warm-ups) of `mode` with `peers` in `cell`, deterministically
    /// derived from `seed`. Returns schema-shaped records.
    pub async fn run_cell(
        &self,
        cell: &NetworkCell,
        mode: Mode,
        peers: u8,
        runs: u16,
        seed: u64,
    ) -> Vec<ModeResultRecord> {
        self.run_cell_common(cell, mode, peers, runs, seed, RunKind::Mode)
            .await
    }

    /// Phase E multi-proposer candidate-tree mode: [`Self::run_cell`] with
    /// `mode = speculative_exact`, but every run executes REAL
    /// propose/assemble/verify tree rounds across `peers` proposer slots —
    /// acceptance, duplicate work, pruning and verification cost are
    /// measured from those rounds, not estimated. Trie telemetry maps onto
    /// the frozen schema's existing vocabulary (`mode-result/v1` is closed):
    /// duplicate drafts are included in `aggregate_model_tokens` ("waste
    /// included") and raise `bytes_per_accepted_token` — duplicate
    /// proposals still cost wire bytes (the documented adjustment);
    /// `verification_steps` counts tree-verification rounds.
    pub async fn run_cell_multi(
        &self,
        cell: &NetworkCell,
        peers: u8,
        runs: u16,
        seed: u64,
    ) -> Vec<ModeResultRecord> {
        self.run_cell_common(
            cell,
            Mode::SpeculativeExact,
            peers,
            runs,
            seed,
            RunKind::Multi,
        )
        .await
    }

    /// Shared driver of [`Self::run_cell`] (linear modes) and
    /// [`Self::run_cell_multi`] (multi-proposer trees).
    async fn run_cell_common(
        &self,
        cell: &NetworkCell,
        mode: Mode,
        peers: u8,
        runs: u16,
        seed: u64,
        kind: RunKind,
    ) -> Vec<ModeResultRecord> {
        if !cell.is_in_matrix() {
            return vec![ModeResultRecord::new(
                run_id_from_seed(seed),
                mode,
                peers,
                ModeMetrics::default(),
                RunOutcome {
                    status: RunStatus::InvalidCell,
                    fallback_reason: None,
                    failure_reason: Some(format!(
                        "cell outside frozen matrix: rtt={}ms loss={} (bench-harness-spec)",
                        cell.nominal_rtt_ms, cell.loss
                    )),
                },
            )];
        }
        let run_id = run_id_from_seed(seed);
        // Warm-up runs are executed and discarded (spec: 5 per cell).
        const WARMUP_SALT: u64 = 0x5A_00;
        for warmup in 0..u32::from(WARMUP_RUNS) {
            let warm_seed = mix(seed, WARMUP_SALT + u64::from(warmup));
            let _ = self
                .execute_run_with(cell, mode, peers, warm_seed, kind)
                .await;
        }
        let mut records = Vec::with_capacity(usize::from(runs));
        for run in 0..u32::from(runs) {
            let run_seed = mix(seed, u64::from(run) + 1);
            match self
                .execute_run_with(cell, mode, peers, run_seed, kind)
                .await
            {
                Ok((metrics, outcome)) => {
                    records.push(ModeResultRecord::new(
                        &run_id, mode, peers, metrics, outcome,
                    ));
                }
                Err(error) => {
                    records.push(ModeResultRecord::new(
                        &run_id,
                        mode,
                        peers,
                        ModeMetrics::default(),
                        RunOutcome {
                            status: RunStatus::Failed,
                            fallback_reason: None,
                            failure_reason: Some(error.to_string()),
                        },
                    ));
                }
            }
        }
        records
    }

    /// Builds the run manifest for a cell (deterministic calibration draws;
    /// `created_at` is wall-clock and not part of record determinism).
    pub fn manifest_for(&self, cell: &NetworkCell, runs: u16, seed: u64) -> RunManifest {
        let mut rng = Xorshift::new(mix(seed, 0xCA_1F_00));
        let draws: Vec<f64> = (0..256).map(|_| draw_rtt(&mut rng, cell)).collect();
        let measured_p50 = stats::percentile(&draws, 50.0);
        let tolerance = 0.10 * f64::from(cell.nominal_rtt_ms);
        let calibration_valid = if cell.nominal_rtt_ms == 0 {
            measured_p50 <= 0.0
        } else {
            (measured_p50 - f64::from(cell.nominal_rtt_ms)).abs() <= tolerance
        };
        let descriptor = self.runtime.id();
        let mut manifest = RunManifest::new(
            run_id_from_seed(seed),
            rfc3339_now(),
            self.harness_version.clone(),
            mock_profile_id(),
            ManifestRuntime {
                name: descriptor.name().to_string(),
                version: descriptor.version().to_string(),
                build_hash: descriptor.build_hash().to_string(),
            },
            ManifestNetworkCell {
                nominal_rtt_ms: cell.nominal_rtt_ms,
                jitter: cell.jitter,
                loss_ratio: canonical_loss(cell.loss),
                path: cell.path,
                measured_rtt_ms_p50: Some(round3(measured_p50)),
                calibration_valid: Some(calibration_valid),
            },
            vec![seed],
        );
        manifest.hardware_class_peers = Some(vec!["cpu".to_string(); SYNTHETIC_PEERS]);
        manifest.os = Some(std::env::consts::OS.to_string());
        manifest.generation_params_digest = Some(generation_params_digest());
        manifest.prompt_corpus_id = Some(PROMPT_CORPUS_ID.to_string());
        manifest.warmup_runs = Some(u32::from(WARMUP_RUNS));
        manifest.recorded_runs = Some(u32::from(runs));
        manifest.confidence_margin = Some(DEFAULT_CONFIDENCE_MARGIN);
        manifest
    }

    /// Report lines for one cell: median/p10/p90/IQR of completion times,
    /// fallback/failure counts, and — when the runtime is mock — the
    /// TEST-ONLY label on every line.
    pub fn cell_report(&self, cell: &NetworkCell, records: &[ModeResultRecord]) -> Vec<String> {
        let label = self.mock_label().unwrap_or("");
        let mut lines = Vec::new();
        let completions: Vec<f64> = records.iter().map(|r| r.metrics.completion_ms).collect();
        if completions.is_empty() {
            lines.push(format!(
                "[{label}] cell rtt={}ms jitter={:?} loss={} n=0 no records",
                cell.nominal_rtt_ms, cell.jitter, cell.loss
            ));
            return lines;
        }
        let summary = aggregate(&completions);
        let fallbacks = records
            .iter()
            .filter(|r| r.outcome.status == RunStatus::FellBackToSingle)
            .count();
        let failures = records
            .iter()
            .filter(|r| r.outcome.status == RunStatus::Failed)
            .count();
        lines.push(format!(
            "[{label}] cell rtt={}ms jitter={:?} loss={} mode={:?} peers=… n={} median={:.3}ms p10={:.3}ms p90={:.3}ms iqr={:.3}ms fallbacks={failures} failures={failures}",
            cell.nominal_rtt_ms,
            cell.jitter,
            cell.loss,
            records[0].mode,
            summary.n,
            summary.median,
            summary.p10,
            summary.p90,
            summary.iqr,
        ));
        if fallbacks > 0 {
            lines.push(format!(
                "[{label}] negative result: {fallbacks}/{} runs fell back to single (visible, not hidden)",
                records.len()
            ));
        }
        lines
    }

    /// Dispatch for the Phase C linear modes vs the Phase E multi-proposer
    /// tree runner.
    async fn execute_run_with(
        &self,
        cell: &NetworkCell,
        mode: Mode,
        peers: u8,
        run_seed: u64,
        kind: RunKind,
    ) -> Result<(ModeMetrics, RunOutcome), BenchError> {
        match kind {
            RunKind::Mode => self.execute_run_linear(cell, mode, peers, run_seed).await,
            RunKind::Multi => self.execute_run_multi(cell, peers, run_seed).await,
        }
    }

    /// One recorded run: synthetic candidates → cost model → execution.
    async fn execute_run_linear(
        &self,
        cell: &NetworkCell,
        mode: Mode,
        peers: u8,
        run_seed: u64,
    ) -> Result<(ModeMetrics, RunOutcome), BenchError> {
        let mut rng = Xorshift::new(run_seed);
        let candidates = synthetic_candidates(&mut rng, cell);
        let prompt_tokens = self
            .runtime
            .tokenize(SYNTHETIC_PROMPT)
            .await
            .map_err(runtime_err)?
            .len() as u32;
        let handle = self
            .runtime
            .load(&mock_profile_id())
            .await
            .map_err(runtime_err)?;
        let sampling = SamplingParams {
            seed: Some(run_seed),
            ..SamplingParams::default()
        };
        let ctx = RunContext {
            candidates: &candidates,
            cell,
            prompt_tokens,
            handle: &handle,
            sampling: &sampling,
        };

        match mode {
            Mode::Single => Ok(self.run_single(&ctx, &mut rng).await),
            Mode::Hedged => Ok(self.run_hedged(&ctx, peers, &mut rng).await),
            Mode::SpeculativeExact => self.run_speculative(&ctx, peers, &mut rng).await,
            Mode::SearchVerified | Mode::MapReduce => {
                let (mut metrics, _) = self.run_single(&ctx, &mut rng).await;
                metrics.exactness = Some(Exactness::Na);
                Ok((
                    metrics,
                    RunOutcome {
                        status: RunStatus::FellBackToSingle,
                        fallback_reason: Some(
                            "mode not exercised by the Phase C harness (matrix: single, hedged, speculative_exact)"
                                .to_string(),
                        ),
                        failure_reason: None,
                    },
                ))
            }
        }
    }

    /// Single mode on the fastest predicted peer (comparator fields set).
    async fn run_single(
        &self,
        ctx: &RunContext<'_>,
        rng: &mut Xorshift,
    ) -> (ModeMetrics, RunOutcome) {
        let RunContext {
            candidates,
            cell,
            prompt_tokens,
            handle,
            sampling,
        } = *ctx;
        let fastest = select_microswarm(
            candidates,
            &candidates[0].profile_id,
            prompt_tokens,
            OUTPUT_TOKENS_TARGET,
            1,
        );
        let peer = fastest.first().expect("at least one synthetic peer");
        let predicted = predicted_single_ms(peer, prompt_tokens, OUTPUT_TOKENS_TARGET);

        let rtt = draw_rtt(rng, cell);
        let ttft =
            rtt + peer.advertised_queue_ms + f64::from(prompt_tokens) / peer.prefill_tokens_per_ms;
        let step_ms = 1.0 / peer.decode_tokens_per_ms;
        let output_tokens = self
            .runtime
            .decode_stream(
                handle,
                &prompt_token_ids(),
                sampling,
                OUTPUT_TOKENS_TARGET,
                DEADLINE,
            )
            .await
            .map(|tokens| tokens.len() as u32)
            .unwrap_or(OUTPUT_TOKENS_TARGET);
        let completion = ttft + f64::from(output_tokens) * step_ms;
        let actual = completion * (1.0 + rng.next_symmetric(0.05));
        (
            ModeMetrics {
                ttft_ms: round3(ttft),
                per_output_token_ms: round3(step_ms),
                completion_ms: round3(actual),
                prompt_tokens,
                output_tokens,
                fastest_single_prediction_ms: Some(round3(predicted)),
                fastest_single_actual_ms: Some(round3(actual)),
                cooperative_prediction_ms: None,
                proposal_window_tokens: None,
                accepted_tokens_per_round: None,
                acceptance_rate: None,
                mean_acceptance_length: None,
                verification_steps: None,
                rollback_count: None,
                bytes_per_accepted_token: None,
                aggregate_model_tokens: None,
                compute_ms_by_role: Some(role_map(&[("coordinator", 1.0)])),
                rtt_ms_p50: Some(round3(rtt)),
                jitter_ms: Some(round3(
                    f64::from(cell.nominal_rtt_ms) * cell.jitter.amplitude(),
                )),
                packet_loss_ratio: Some(f64::from(cell.loss)),
                nat_path: Some(records::NatPath::Direct),
                exactness: Some(Exactness::Na),
            },
            RunOutcome {
                status: RunStatus::Completed,
                fallback_reason: None,
                failure_reason: None,
            },
        )
    }

    /// Hedged mode: n parallel simulated streams, winner's completion kept,
    /// wasted peer tokens recorded (`aggregate_model_tokens`).
    async fn run_hedged(
        &self,
        ctx: &RunContext<'_>,
        peers: u8,
        rng: &mut Xorshift,
    ) -> (ModeMetrics, RunOutcome) {
        let RunContext {
            candidates,
            cell,
            prompt_tokens,
            handle,
            sampling,
        } = *ctx;
        let selected = select_microswarm(
            candidates,
            &candidates[0].profile_id,
            prompt_tokens,
            OUTPUT_TOKENS_TARGET,
            usize::from(peers.max(2)),
        );
        // (completion, ttft) per hedged peer, each with run noise.
        let mut simulated: Vec<(f64, f64)> = Vec::with_capacity(selected.len());
        for peer in &selected {
            let rtt = draw_rtt(rng, cell);
            let ttft = rtt
                + peer.advertised_queue_ms
                + f64::from(prompt_tokens) / peer.prefill_tokens_per_ms;
            let step_ms = 1.0 / peer.decode_tokens_per_ms;
            let completion = (ttft + f64::from(OUTPUT_TOKENS_TARGET) * step_ms)
                * (1.0 + rng.next_symmetric(0.05));
            simulated.push((completion, ttft));
        }
        let winner_index = simulated
            .iter()
            .enumerate()
            .min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(i, _)| i)
            .expect("hedged swarm is non-empty");
        let (winner_completion, winner_ttft) = simulated[winner_index];
        let aggregate_tokens: u32 = simulated
            .iter()
            .enumerate()
            .map(|(i, (completion, _))| {
                if i == winner_index {
                    OUTPUT_TOKENS_TARGET
                } else {
                    // A cancelled peer generates tokens until the winner
                    // finishes (ratio capped at a full generation).
                    let ratio = (completion / winner_completion).min(1.0);
                    (f64::from(OUTPUT_TOKENS_TARGET) * ratio) as u32
                }
            })
            .sum();
        let standby_compute: f64 = simulated
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != winner_index)
            .map(|(_, (completion, ttft))| (completion - ttft).max(0.0))
            .sum();
        let output_tokens = self
            .runtime
            .decode_stream(
                handle,
                &prompt_token_ids(),
                sampling,
                OUTPUT_TOKENS_TARGET,
                DEADLINE,
            )
            .await
            .map(|tokens| tokens.len() as u32)
            .unwrap_or(OUTPUT_TOKENS_TARGET);
        (
            ModeMetrics {
                ttft_ms: round3(winner_ttft),
                per_output_token_ms: round3(
                    (winner_completion - winner_ttft) / f64::from(output_tokens.max(1)),
                ),
                completion_ms: round3(winner_completion),
                prompt_tokens,
                output_tokens,
                fastest_single_prediction_ms: Some(round3(predicted_single_ms(
                    selected[0],
                    prompt_tokens,
                    OUTPUT_TOKENS_TARGET,
                ))),
                fastest_single_actual_ms: Some(round3(simulated[0].0)),
                cooperative_prediction_ms: None,
                proposal_window_tokens: None,
                accepted_tokens_per_round: None,
                acceptance_rate: None,
                mean_acceptance_length: None,
                verification_steps: None,
                rollback_count: None,
                bytes_per_accepted_token: None,
                aggregate_model_tokens: Some(aggregate_tokens),
                compute_ms_by_role: Some(role_map(&[
                    ("coordinator", 1.0),
                    ("standby", round3(standby_compute)),
                ])),
                rtt_ms_p50: Some(round3(draw_rtt(rng, cell))),
                jitter_ms: Some(round3(
                    f64::from(cell.nominal_rtt_ms) * cell.jitter.amplitude(),
                )),
                packet_loss_ratio: Some(f64::from(cell.loss)),
                nat_path: Some(records::NatPath::Direct),
                exactness: Some(Exactness::Na),
            },
            RunOutcome {
                status: RunStatus::Completed,
                fallback_reason: None,
                failure_reason: None,
            },
        )
    }

    /// `speculative_exact`: v2 cost model decides engagement; the engaged
    /// path runs real propose/verify rounds against the runtime with
    /// synthetic per-round latency. Honest fallback when not engaged.
    async fn run_speculative(
        &self,
        ctx: &RunContext<'_>,
        peers: u8,
        rng: &mut Xorshift,
    ) -> Result<(ModeMetrics, RunOutcome), BenchError> {
        let RunContext {
            candidates,
            cell,
            prompt_tokens,
            handle,
            sampling: _,
        } = *ctx;
        let swarm = select_microswarm(
            candidates,
            &candidates[0].profile_id,
            prompt_tokens,
            OUTPUT_TOKENS_TARGET,
            usize::from(peers.max(2)),
        );
        // Fastest peer verifies; second fastest drafts (classic setup).
        let verifier = (*swarm[0]).clone();
        let proposer = match swarm.get(1) {
            Some(second) => (**second).clone(),
            None => (*swarm[0]).clone(),
        };
        let fastest_single = predicted_single_ms(swarm[0], prompt_tokens, OUTPUT_TOKENS_TARGET);

        let rtt = draw_rtt(rng, cell);
        let prefill_cost = f64::from(prompt_tokens) / verifier.prefill_tokens_per_ms;
        let window = PROPOSAL_WINDOW;
        // Expected rounds to produce the output budget at the planner's
        // acceptance estimate; every per-round term scales with it.
        let rounds_expected =
            f64::from(OUTPUT_TOKENS_TARGET) / (f64::from(window) * ESTIMATED_ACCEPTANCE);
        let draft_ms = f64::from(window) / proposer.decode_tokens_per_ms;
        let verify_ms = VERIFY_BATCH_STEPS / verifier.decode_tokens_per_ms;
        let inputs = SwarmInputs {
            prefill_cost_ms: prefill_cost,
            proposal_cost_ms: rounds_expected * draft_ms,
            verification_cost_ms: rounds_expected * verify_ms,
            synchronization_cost_ms: rounds_expected * rtt,
            expected_rollback_cost_ms: rounds_expected * (1.0 - ESTIMATED_ACCEPTANCE)
                / verifier.decode_tokens_per_ms,
            failure_risk_penalty_ms: FAILURE_RISK_PENALTY_MS,
        };
        let cooperative_prediction = predicted_swarm_ms(&inputs);
        let engage = should_engage_cooperative(
            cooperative_prediction,
            fastest_single,
            DEFAULT_CONFIDENCE_MARGIN,
        )
        .expect("frozen default margin is legal");

        if !engage {
            // Honest negative result: record the fallback, not a fake win.
            let (mut metrics, _) = self.run_single(ctx, rng).await;
            metrics.cooperative_prediction_ms = Some(round3(cooperative_prediction));
            metrics.proposal_window_tokens = Some(window);
            return Ok((
                metrics,
                RunOutcome {
                    status: RunStatus::FellBackToSingle,
                    fallback_reason: Some(format!(
                        "cost model v2: cooperative {cooperative_prediction:.3}ms × (1+{DEFAULT_CONFIDENCE_MARGIN}) did not beat fastest single {fastest_single:.3}ms"
                    )),
                    failure_reason: None,
                },
            ));
        }

        // Engaged: real proposal/verification rounds on the runtime. The
        // verifier decodes with the default sampling — the reference
        // continuation MockRuntime proposals are drawn against, so measured
        // acceptance is governed by the draft-accuracy knob.
        let reference = SamplingParams::default();
        let mut prefix = prompt_token_ids();
        let mut accepted_total: u32 = 0;
        let mut proposed_total: u32 = 0;
        let mut accepted_lengths: Vec<f64> = Vec::new();
        let mut rollbacks: u32 = 0;
        let mut rounds: u32 = 0;
        let mut round_ms_total = 0.0;
        let mut proposer_ms = 0.0;
        let mut verifier_ms = 0.0;
        let mut first_round_ms = 0.0;
        while accepted_total < OUTPUT_TOKENS_TARGET && rounds < MAX_SPECULATIVE_ROUNDS {
            let draft = self
                .runtime
                .propose(handle, &prefix, window)
                .await
                .map_err(runtime_err)?;
            rounds += 1;
            proposed_total += window;
            let mut accepted_run: u32 = 0;
            let mut mismatch = false;
            for drafted in &draft {
                if accepted_total + accepted_run >= OUTPUT_TOKENS_TARGET {
                    break;
                }
                let truth = self
                    .runtime
                    .decode_step(handle, &prefix, &reference)
                    .await
                    .map_err(runtime_err)?;
                if drafted == &truth {
                    accepted_run += 1;
                    prefix.push(truth);
                } else {
                    mismatch = true;
                    break;
                }
            }
            if mismatch {
                rollbacks += 1;
            }
            accepted_total += accepted_run;
            accepted_lengths.push(f64::from(accepted_run));
            let round_rtt = draw_rtt(rng, cell);
            let draft_ms = f64::from(window) / proposer.decode_tokens_per_ms;
            // One batched verification pass costs VERIFY_BATCH_STEPS
            // sequential decode steps; a rejected round additionally
            // re-decodes the mismatching token (rollback compute).
            let verify_ms = VERIFY_BATCH_STEPS / verifier.decode_tokens_per_ms
                + f64::from(u32::from(mismatch)) / verifier.decode_tokens_per_ms;
            let round_ms = round_rtt + draft_ms + verify_ms;
            proposer_ms += draft_ms;
            verifier_ms += verify_ms;
            round_ms_total += round_ms;
            if rounds == 1 {
                first_round_ms = round_ms + prefill_cost;
            }
        }
        if accepted_total == 0 {
            return Ok((
                ModeMetrics::default(),
                RunOutcome {
                    status: RunStatus::Failed,
                    fallback_reason: None,
                    failure_reason: Some(
                        "zero speculative acceptance: cooperative mode cannot progress (negative result)"
                            .to_string(),
                    ),
                },
            ));
        }
        let completion = prefill_cost + round_ms_total;
        let acceptance_rate = f64::from(accepted_total) / f64::from(proposed_total);
        let mean_acceptance = accepted_lengths.iter().sum::<f64>() / accepted_lengths.len() as f64;
        let output_text = self.runtime.detokenize(&prefix).await.unwrap_or_default();
        let bytes_per_token = output_text.len() as f64 / f64::from(accepted_total);
        Ok((
            ModeMetrics {
                ttft_ms: round3(first_round_ms),
                per_output_token_ms: round3(round_ms_total / f64::from(accepted_total.max(1))),
                completion_ms: round3(completion * (1.0 + rng.next_symmetric(0.05))),
                prompt_tokens,
                output_tokens: accepted_total,
                fastest_single_prediction_ms: Some(round3(fastest_single)),
                fastest_single_actual_ms: None,
                cooperative_prediction_ms: Some(round3(cooperative_prediction)),
                proposal_window_tokens: Some(window),
                accepted_tokens_per_round: Some(round3(
                    f64::from(accepted_total) / f64::from(rounds),
                )),
                acceptance_rate: Some(round6(acceptance_rate)),
                mean_acceptance_length: Some(round3(mean_acceptance)),
                verification_steps: Some(rounds),
                rollback_count: Some(rollbacks),
                bytes_per_accepted_token: Some(round3(bytes_per_token)),
                aggregate_model_tokens: Some(proposed_total + rounds * window),
                compute_ms_by_role: Some(role_map(&[
                    ("coordinator", round3(f64::from(rounds) * rtt)),
                    ("proposer", round3(proposer_ms)),
                    ("verifier", round3(verifier_ms)),
                ])),
                rtt_ms_p50: Some(round3(rtt)),
                jitter_ms: Some(round3(
                    f64::from(cell.nominal_rtt_ms) * cell.jitter.amplitude(),
                )),
                packet_loss_ratio: Some(f64::from(cell.loss)),
                nat_path: Some(records::NatPath::Direct),
                exactness: Some(Exactness::GreedyEqual),
            },
            RunOutcome {
                status: RunStatus::Completed,
                fallback_reason: None,
                failure_reason: None,
            },
        ))
    }

    /// Phase E `speculative_exact` with N proposer slots: real in-process
    /// propose/assemble/verify tree rounds against the engine runtime
    /// (ADR-019 honesty: TEST-ONLY mock backend, structural evidence only).
    ///
    /// Per round, every proposer slot draws the deterministic
    /// [`branch_assignment`] seed for `(session, round, roster, index)` and
    /// drafts via a real [`InferenceRuntime::propose`] call; the coordinator
    /// assembles the [`CandidateTrie`] under [`MULTI_TRIE_LIMITS`]
    /// (duplicate drafts counted, bounds enforced) and the verifier
    /// tree-verifies against a real `max_depth + 1` decode of the target —
    /// acceptance, duplicate work, pruning and rollback counts all come
    /// from these measured rounds.
    ///
    /// Determinism note (documented deviation): a deterministic runtime
    /// proposes IDENTICAL branches for identical prefixes, so identical
    /// proposer slots mostly duplicate each other — the duplicate-work
    /// telemetry is exactly that measured redundancy. Branch DIVERSITY is
    /// exercised in `modelswarm-sim`/session e2e, where per-proposer seeded
    /// runtimes run. The per-proposer branch seeds still key the round's
    /// synthetic latency draws, keeping records reproducible from
    /// `(seed, peers)`.
    async fn execute_run_multi(
        &self,
        cell: &NetworkCell,
        peers: u8,
        run_seed: u64,
    ) -> Result<(ModeMetrics, RunOutcome), BenchError> {
        let peers = peers.clamp(2, 7);
        let mut rng = Xorshift::new(run_seed);
        let candidates = synthetic_candidates(&mut rng, cell);
        let session_id = format!("bench-multi-{run_seed:016x}");
        let prompt_tokens = self
            .runtime
            .tokenize(SYNTHETIC_PROMPT)
            .await
            .map_err(runtime_err)?
            .len() as u32;
        let handle = self
            .runtime
            .load(&mock_profile_id())
            .await
            .map_err(runtime_err)?;
        let swarm = select_microswarm(
            &candidates,
            &candidates[0].profile_id,
            prompt_tokens,
            OUTPUT_TOKENS_TARGET,
            usize::from(peers),
        );
        // Fastest peer verifies; the rest draft (asymmetric pool).
        let verifier = (*swarm[0]).clone();
        let proposer = match swarm.get(1) {
            Some(second) => (**second).clone(),
            None => (*swarm[0]).clone(),
        };
        let fastest_single = predicted_single_ms(swarm[0], prompt_tokens, OUTPUT_TOKENS_TARGET);

        let rtt = draw_rtt(&mut rng, cell);
        let prefill_cost = f64::from(prompt_tokens) / verifier.prefill_tokens_per_ms;
        let window = PROPOSAL_WINDOW;
        let rounds_expected =
            f64::from(OUTPUT_TOKENS_TARGET) / (f64::from(window) * ESTIMATED_ACCEPTANCE);
        let draft_ms = f64::from(window) / proposer.decode_tokens_per_ms;
        let verify_ms = VERIFY_BATCH_STEPS / verifier.decode_tokens_per_ms;
        let inputs = SwarmInputs {
            prefill_cost_ms: prefill_cost,
            proposal_cost_ms: rounds_expected * draft_ms,
            verification_cost_ms: rounds_expected * verify_ms,
            synchronization_cost_ms: rounds_expected * rtt,
            expected_rollback_cost_ms: rounds_expected * (1.0 - ESTIMATED_ACCEPTANCE)
                / verifier.decode_tokens_per_ms,
            failure_risk_penalty_ms: FAILURE_RISK_PENALTY_MS,
        };
        let cooperative_prediction = predicted_swarm_ms(&inputs);
        let engage = should_engage_cooperative(
            cooperative_prediction,
            fastest_single,
            DEFAULT_CONFIDENCE_MARGIN,
        )
        .expect("frozen default margin is legal");

        if !engage {
            // Honest negative result: the record shows the fallback.
            let Ok((mut metrics, _)) = self
                .execute_run_linear(cell, Mode::Single, 1, run_seed ^ 0x5EED_0001)
                .await
            else {
                unreachable!("single-mode runs never fail before metrics");
            };
            metrics.cooperative_prediction_ms = Some(round3(cooperative_prediction));
            metrics.proposal_window_tokens = Some(window);
            return Ok((
                metrics,
                RunOutcome {
                    status: RunStatus::FellBackToSingle,
                    fallback_reason: Some(format!(
                        "cost model v2: cooperative {cooperative_prediction:.3}ms × (1+{DEFAULT_CONFIDENCE_MARGIN}) did not beat fastest single {fastest_single:.3}ms"
                    )),
                    failure_reason: None,
                },
            ));
        }

        let reference = SamplingParams::default();
        let mut prefix = prompt_token_ids();
        let mut accepted_total: u32 = 0;
        let mut accepted_draft_total: u32 = 0;
        let mut proposed_total: u32 = 0;
        let mut accepted_lengths: Vec<f64> = Vec::new();
        let mut rollbacks: u32 = 0;
        let mut rounds: u32 = 0;
        let mut duplicate_work: u64 = 0;
        let mut pruned: u64 = 0;
        // Wire-bytes model for bytes_per_accepted_token: 4 bytes per drafted
        // token id + per-block envelope overhead (documented adjustment:
        // duplicate blocks are still sent, so they still cost wire bytes).
        let mut wire_bytes: f64 = 0.0;
        let mut verify_decode_steps: u32 = 0;
        let mut round_ms_total = 0.0;
        let mut proposer_ms = 0.0;
        let mut verifier_ms = 0.0;
        let mut first_round_ms = 0.0;
        while accepted_total < OUTPUT_TOKENS_TARGET && rounds < MAX_SPECULATIVE_ROUNDS {
            let round = u64::from(rounds) + 1;
            // N seeded proposer slots draft (E1: deterministic assignment).
            let mut seeds_folded: u64 = 0;
            let mut blocks: Vec<BranchBlock> = Vec::with_capacity(usize::from(peers));
            for index in 0..usize::from(peers) {
                let branch_seed = branch_assignment(&session_id, round, usize::from(peers), index);
                seeds_folded ^= branch_seed.rotate_left(17);
                let draft = self
                    .runtime
                    .propose(&handle, &prefix, window)
                    .await
                    .map_err(runtime_err)?;
                proposed_total += window;
                wire_bytes += f64::from(window) * 4.0 + CANDIDATE_BLOCK_OVERHEAD_BYTES as f64;
                blocks.push(BranchBlock {
                    proposer: index,
                    tokens: draft,
                    // Uniform scores: with one deterministic runtime no
                    // proposer is a priori better (diversity lives in the
                    // seeded-runtime e2e, not here).
                    score: 1.0,
                });
            }
            // Assemble the bounded trie (E2/E3)...
            let trie = CandidateTrie::assemble(blocks, MULTI_TRIE_LIMITS);
            duplicate_work += trie.duplicate_work() as u64;
            pruned += trie.pruned() as u64;
            rounds += 1;
            // ...and verify against a real target decode (deepest + 1).
            let max_depth = trie
                .branches()
                .iter()
                .map(|b| b.tokens.len())
                .max()
                .unwrap_or(0);
            let mut target: Vec<u32> = Vec::with_capacity(max_depth + 1);
            let mut work = prefix.clone();
            for _ in 0..=max_depth {
                let token = self
                    .runtime
                    .decode_step(&handle, &work, &reference)
                    .await
                    .map_err(runtime_err)?;
                target.push(token);
                work.push(token);
            }
            let outcome = verify_tree_greedy(&trie, &target);
            let winning_len = outcome
                .winning_branch
                .map(|index| trie.branches()[index].tokens.len());
            let rolled_back = winning_len.is_some_and(|len| outcome.accepted_depth < len);
            if rolled_back {
                rollbacks += 1;
            }
            let committed = outcome.committed;
            accepted_lengths.push(outcome.accepted_depth as f64);
            accepted_draft_total += outcome.accepted_depth as u32;
            accepted_total += committed.len() as u32;
            prefix.extend_from_slice(&committed);
            verify_decode_steps += u32::try_from(max_depth + 1).unwrap_or(u32::MAX);
            // Synthetic latency: per-round RTT keyed to the branch schedule
            // (keeps the multi records a pure function of (seed, peers)).
            // ALL proposer slots pay the draft cost each round (their blocks
            // — duplicates included — are computed and transferred).
            let round_rtt = draw_rtt(&mut Xorshift::new(seeds_folded), cell);
            let draft_ms = f64::from(window) / proposer.decode_tokens_per_ms;
            // One batched tree verification costs VERIFY_BATCH_STEPS
            // sequential decode steps; a rolled-back round additionally
            // re-decodes the mismatching token (rollback compute).
            let verify_ms = VERIFY_BATCH_STEPS / verifier.decode_tokens_per_ms
                + f64::from(u32::from(rolled_back)) / verifier.decode_tokens_per_ms;
            let round_ms = round_rtt + f64::from(peers) * draft_ms + verify_ms;
            proposer_ms += f64::from(peers) * draft_ms;
            verifier_ms += verify_ms;
            round_ms_total += round_ms;
            if rounds == 1 {
                first_round_ms = round_ms + prefill_cost;
            }
        }
        if accepted_total == 0 {
            return Ok((
                ModeMetrics::default(),
                RunOutcome {
                    status: RunStatus::Failed,
                    fallback_reason: None,
                    failure_reason: Some(
                        "zero tree acceptance: cooperative mode cannot progress (negative result)"
                            .to_string(),
                    ),
                },
            ));
        }
        // Trie-telemetry invariants (E2/E3): duplicates are a subset of all
        // drafts, and the bounds cannot bind at window 8 with ≤ 7 proposers
        // (56 nodes ≤ 128). The frozen schema has no dedicated duplicate/
        // prune fields — duplicate drafts surface in `aggregate_model_tokens`
        // ("waste included") and in `bytes_per_accepted_token` (duplicate
        // blocks are still wire bytes: the documented adjustment).
        debug_assert!(duplicate_work <= u64::from(proposed_total));
        debug_assert_eq!(pruned, 0, "MULTI_TRIE_LIMITS never bind at window 8");
        let completion = prefill_cost + round_ms_total;
        let acceptance_rate = f64::from(accepted_draft_total) / f64::from(proposed_total.max(1));
        let mean_acceptance = accepted_lengths.iter().sum::<f64>() / accepted_lengths.len() as f64;
        let bytes_per_token = wire_bytes / f64::from(accepted_total);
        // Model-token accounting: every drafted token (duplicates included —
        // "waste included" per the schema) plus the verifier's real decode
        // steps (deepest branch + 1 per round).
        Ok((
            ModeMetrics {
                ttft_ms: round3(first_round_ms),
                per_output_token_ms: round3(round_ms_total / f64::from(accepted_total.max(1))),
                completion_ms: round3(completion * (1.0 + rng.next_symmetric(0.05))),
                prompt_tokens,
                output_tokens: accepted_total,
                fastest_single_prediction_ms: Some(round3(fastest_single)),
                fastest_single_actual_ms: None,
                cooperative_prediction_ms: Some(round3(cooperative_prediction)),
                proposal_window_tokens: Some(window),
                accepted_tokens_per_round: Some(round3(
                    f64::from(accepted_total) / f64::from(rounds),
                )),
                acceptance_rate: Some(round6(acceptance_rate)),
                mean_acceptance_length: Some(round3(mean_acceptance)),
                verification_steps: Some(rounds),
                rollback_count: Some(rollbacks),
                bytes_per_accepted_token: Some(round3(bytes_per_token)),
                aggregate_model_tokens: Some(proposed_total + verify_decode_steps),
                compute_ms_by_role: Some(role_map(&[
                    ("coordinator", round3(f64::from(rounds) * rtt)),
                    ("proposer", round3(proposer_ms)),
                    ("verifier", round3(verifier_ms)),
                ])),
                rtt_ms_p50: Some(round3(rtt)),
                jitter_ms: Some(round3(
                    f64::from(cell.nominal_rtt_ms) * cell.jitter.amplitude(),
                )),
                packet_loss_ratio: Some(f64::from(cell.loss)),
                nat_path: Some(records::NatPath::Direct),
                exactness: Some(Exactness::GreedyEqual),
            },
            RunOutcome {
                status: RunStatus::Completed,
                fallback_reason: None,
                failure_reason: None,
            },
        ))
    }
}

/// Which engine path a recorded run takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunKind {
    /// Phase C linear modes (single/hedged/speculative two-peer).
    Mode,
    /// Phase E multi-proposer candidate trees.
    Multi,
}

/// Deadline given to runtime decode calls (never hit; mock is instant).
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

fn prompt_token_ids() -> Vec<u32> {
    SYNTHETIC_PROMPT
        .as_bytes()
        .iter()
        .map(|b| u32::from(*b))
        .collect()
}

fn runtime_err(error: modelswarm_runtime::RuntimeError) -> BenchError {
    BenchError::Runtime(error.to_string())
}

fn role_map(entries: &[(&str, f64)]) -> std::collections::BTreeMap<String, f64> {
    entries
        .iter()
        .map(|(role, ms)| ((*role).to_string(), *ms))
        .collect()
}

/// Deterministic candidate pool for one run (pure synthetic data).
/// Pool: one verifier-class host, one fast-but-busy host (the asymmetric
/// "hardware" dimension of the frozen matrix), two mid/low hosts.
fn synthetic_candidates(rng: &mut Xorshift, cell: &NetworkCell) -> Vec<Candidate> {
    let profile = ModelProfileId::new("msp:mock-4b:q4_k_m:v1").expect("static profile id");
    (0..SYNTHETIC_PEERS)
        .map(|index| {
            let (speed, queue) = if index == 1 {
                (DRAFTER_SPEEDUP, DRAFTER_QUEUE_MS)
            } else {
                (1.0 - 0.2 * (index as f64 - 1.0).max(0.0), 5.0)
            };
            let speed = (speed + rng.next_symmetric(0.05)).max(0.1);
            Candidate {
                peer_id: format!("mock-peer-{index}"),
                profile_id: profile.clone(),
                measured_rtt_ms: draw_rtt(rng, cell).max(0.0),
                advertised_queue_ms: (queue + rng.next_symmetric(5.0)).max(0.0),
                prefill_tokens_per_ms: MOCK_PREFILL_TOKENS_PER_MS * speed,
                decode_tokens_per_ms: MOCK_DECODE_TOKENS_PER_MS * speed,
                failure_penalty_ms: PEER_FAILURE_PENALTY_MS,
                stale_advertisement_penalty_ms: 0.0,
                slots: 1,
                capacity_class: CapacityClass::Cpu,
                nat_path: NatPath::Direct,
                // The mock runtime models the BATCH-VERIFY ENGINE (that is
                // exactly what VERIFY_BATCH_STEPS charges — the ADR-013
                // engine term, ADR-032 §4). The Phase C simulation is not
                // the msp-v1 request/reply wire; the wire-true sequential
                // term governs the pass-1/2 harness path, whose candidates
                // never declare the capability.
                batch_verify: true,
            }
        })
        .collect()
}

/// Pure latency model: nominal RTT + symmetric jitter draw + one
/// retransmission RTT on a loss draw. Clearly synthetic (documented).
fn draw_rtt(rng: &mut Xorshift, cell: &NetworkCell) -> f64 {
    let nominal = f64::from(cell.nominal_rtt_ms);
    let jitter = rng.next_symmetric(cell.jitter.amplitude()) * nominal;
    let loss_hit = rng.next_f64() < f64::from(cell.loss);
    (nominal + jitter + if loss_hit { nominal } else { 0.0 }).max(0.0)
}

/// Seed mixer: separates warm-up/recorded streams and runs (crate-shared
/// with the pass-1 harness).
fn mix(seed: u64, salt: u64) -> u64 {
    let mut z = seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Canonical f64 loss value of the frozen matrix for a validated cell
/// (serializes exactly as the schema enum literals 0/0.001/0.01/0.03).
fn canonical_loss(loss: f32) -> f64 {
    const MATRIX: [f64; 4] = [0.0, 0.001, 0.01, 0.03];
    let mut best = MATRIX[0];
    let mut best_diff = f64::INFINITY;
    for candidate in MATRIX {
        let diff = (candidate - f64::from(loss)).abs();
        if diff < best_diff {
            best_diff = diff;
            best = candidate;
        }
    }
    best
}

/// `run-` + first 16 hex of sha256(seed).
pub fn run_id_from_seed(seed: u64) -> String {
    let digest = Sha256::digest(seed.to_le_bytes());
    format!("run-{}", &hex::encode(digest)[..16])
}

/// Deterministic mock profile id (`msp1:` + 64 hex, ADR-011 shape).
pub fn mock_profile_id() -> String {
    let digest = Sha256::digest(b"modelswarm-bench-mock-profile-v1");
    format!("msp1:{}", hex::encode(digest))
}

fn generation_params_digest() -> String {
    let params = json!({
        "max_tokens": OUTPUT_TOKENS_TARGET,
        "proposal_window": PROPOSAL_WINDOW,
        "temperature": 1.0,
        "top_p": 1.0,
        "top_k": 40,
    });
    hex::encode(Sha256::digest(params.to_string().as_bytes()))
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

/// RFC 3339 UTC timestamp of now (no time crate; civil-from-days).
pub fn rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Writes records as JSON-lines to `dir/mode-results.jsonl` (one compact
/// record per line). Deterministic: same records ⇒ identical bytes.
pub fn write_records(dir: &Path, records: &[ModeResultRecord]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join("mode-results.jsonl");
    let mut file = std::fs::File::create(&path)?;
    for record in records {
        let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
        writeln!(file, "{line}")?;
    }
    file.flush()?;
    Ok(path)
}

/// Writes the run manifest as pretty JSON.
pub fn write_run_manifest(path: &Path, manifest: &RunManifest) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(manifest).map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_are_schema_shaped_and_seed_deterministic() {
        let id = run_id_from_seed(7);
        assert!(id.starts_with("run-"), "{id}");
        assert_eq!(id.len(), 20);
        assert!(id[4..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(run_id_from_seed(7), id);
        assert_ne!(run_id_from_seed(8), id);
    }

    #[test]
    fn mock_profile_id_matches_manifest_pattern() {
        let id = mock_profile_id();
        assert!(id.starts_with("msp1:"));
        assert_eq!(id.len(), "msp1:".len() + 64);
        assert!(id["msp1:".len()..]
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }

    #[test]
    fn matrix_validation_accepts_only_frozen_cells() {
        assert!(NetworkCell::LOOPBACK.is_in_matrix());
        assert!(NetworkCell {
            nominal_rtt_ms: 20,
            jitter: Jitter::High,
            loss: 0.01,
            path: CellPath::Direct,
        }
        .is_in_matrix());
        assert!(!NetworkCell {
            nominal_rtt_ms: 15,
            ..NetworkCell::LOOPBACK
        }
        .is_in_matrix());
        assert!(!NetworkCell {
            loss: 0.05,
            ..NetworkCell::LOOPBACK
        }
        .is_in_matrix());
    }

    #[test]
    fn rfc3339_now_has_date_time_shape() {
        let stamp = rfc3339_now();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'));
        assert_eq!(stamp.as_bytes()[4], b'-');
        assert_eq!(stamp.as_bytes()[10], b'T');
        assert_eq!(stamp.as_bytes()[13], b':');
        // 2026-10-04-ish sanity (must not be 1970).
        assert!(stamp.starts_with("20"), "{stamp}");
    }

    #[test]
    fn civil_from_days_known_date() {
        // Cross-checked against Python's datetime (epoch day numbers).
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_639), (2026, 7, 5));
        assert_eq!(civil_from_days(20_730), (2026, 10, 4));
    }

    #[test]
    fn mix_separates_streams() {
        assert_ne!(mix(1, 0), mix(1, 1));
        assert_eq!(mix(1, 2), mix(1, 2));
    }
}
