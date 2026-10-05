//! Record types mirroring `experiments/schemas/` field-for-field
//! (ADR-013 frozen telemetry vocabulary). These serialize exactly to the
//! schema shapes: no extra fields, optional fields omitted when absent.
//!
//! Honesty labeling (ADR-019): a record produced against the mock runtime
//! carries its mock identity through `run_manifest_id` → the run manifest's
//! `runtime.name = "mock"`, because the frozen schema is closed
//! (`additionalProperties: false`) and admits no label field. Human-facing
//! report lines embed [`crate::TEST_ONLY_MOCK_LABEL`] instead.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `record_type` of a mode result (`mode-result/v1`).
pub const MODE_RESULT_RECORD_TYPE: &str = "mode-result/v1";
/// `record_type` of a run manifest (`run-manifest/v1`).
pub const RUN_MANIFEST_RECORD_TYPE: &str = "run-manifest/v1";

/// Execution mode under test (ADR-013 registry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Ordinary single-host generation (default-on).
    Single,
    /// 2–3 parallel complete streams; one kept, others cancelled.
    Hedged,
    /// Speculative decoding with exact acceptance (2–8 peers).
    SpeculativeExact,
    /// Verifier-guided search (approximate by design; Phase E).
    SearchVerified,
    /// Application-specific map-reduce aggregation.
    MapReduce,
}

/// Per-run status (schema enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The mode produced its output.
    Completed,
    /// The cost model or a runtime condition forced the single fallback.
    FellBackToSingle,
    /// The run failed.
    Failed,
    /// The cell did not match the frozen network matrix.
    InvalidCell,
}

/// NAT path of the run (schema enum; ADR-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NatPath {
    /// Direct QUIC path.
    Direct,
    /// Hole-punched path.
    HolePunched,
    /// Relayed path (Phase F+).
    Relayed,
}

/// Exactness result of the run (schema enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Exactness {
    /// Greedy token-equal output.
    #[serde(rename = "greedy_equal")]
    GreedyEqual,
    /// Sampled distribution-tested output.
    #[serde(rename = "distribution_tested")]
    DistributionTested,
    /// Approximate, declared as such.
    #[serde(rename = "approximate_declared")]
    ApproximateDeclared,
    /// Not applicable (non-cooperative modes).
    #[serde(rename = "n/a")]
    Na,
}

/// Per-run metrics (ADR-13 telemetry vocabulary; required fields always
/// set, optional fields `skip_serializing_if = None`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ModeMetrics {
    /// Time to first token in milliseconds.
    pub ttft_ms: f64,
    /// Mean per-output-token latency in milliseconds.
    pub per_output_token_ms: f64,
    /// Wall-clock completion in milliseconds.
    pub completion_ms: f64,
    /// Prompt token count.
    pub prompt_tokens: u32,
    /// Output token count.
    pub output_tokens: u32,
    /// Predicted completion of the fastest eligible single host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fastest_single_prediction_ms: Option<f64>,
    /// Measured completion of the fastest single host in this cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fastest_single_actual_ms: Option<f64>,
    /// Cost-model-v2 prediction of the cooperative completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooperative_prediction_ms: Option<f64>,
    /// Speculative proposal window size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_window_tokens: Option<u32>,
    /// Mean accepted tokens per round.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_tokens_per_round: Option<f64>,
    /// Accepted/proposed token ratio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance_rate: Option<f64>,
    /// Mean accepted run length before a mismatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_acceptance_length: Option<f64>,
    /// Verification passes executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_steps: Option<u32>,
    /// Rounds whose speculative KV had to be discarded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_count: Option<u32>,
    /// Wire bytes per accepted token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_per_accepted_token: Option<f64>,
    /// Total model tokens computed across all peers (waste included).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_model_tokens: Option<u32>,
    /// Compute time in milliseconds by role (coordinator/proposer/verifier/standby).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_ms_by_role: Option<std::collections::BTreeMap<String, f64>>,
    /// Median measured RTT in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms_p50: Option<f64>,
    /// Jitter amplitude in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    /// Packet loss ratio in `[0, 1]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_ratio: Option<f64>,
    /// Network path of the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nat_path: Option<NatPath>,
    /// Exactness contract result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exactness: Option<Exactness>,
}

/// Outcome block of a mode result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunOutcome {
    /// Terminal status of the run.
    pub status: RunStatus,
    /// Why the run fell back to single mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    /// Why the run failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl Default for RunOutcome {
    fn default() -> Self {
        Self {
            status: RunStatus::Failed,
            fallback_reason: None,
            failure_reason: None,
        }
    }
}

/// One recorded run (`mode-result.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeResultRecord {
    /// Always [`MODE_RESULT_RECORD_TYPE`].
    pub record_type: &'static str,
    /// Link to the run manifest (`run-` + 16 hex).
    pub run_manifest_id: String,
    /// Mode under test.
    pub mode: Mode,
    /// Peer count.
    pub peers: u8,
    /// Metrics per ADR-13.
    pub metrics: ModeMetrics,
    /// Outcome block.
    pub outcome: RunOutcome,
}

impl ModeResultRecord {
    /// Builds a record with the const `record_type`.
    #[must_use]
    pub fn new(
        run_manifest_id: impl Into<String>,
        mode: Mode,
        peers: u8,
        metrics: ModeMetrics,
        outcome: RunOutcome,
    ) -> Self {
        Self {
            record_type: MODE_RESULT_RECORD_TYPE,
            run_manifest_id: run_manifest_id.into(),
            mode,
            peers,
            metrics,
            outcome,
        }
    }
}

/// Runtime identity block of a manifest (`name`, `version`, `build_hash`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestRuntime {
    /// e.g. `llama.cpp` or `mock`.
    pub name: String,
    /// Runtime version label.
    pub version: String,
    /// 64-hex build identity digest.
    pub build_hash: String,
}

/// Network cell block of a manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestNetworkCell {
    /// Nominal RTT (frozen matrix: 0/5/10/20/40/80/150 ms).
    pub nominal_rtt_ms: u32,
    /// Jitter class.
    pub jitter: JitterClass,
    /// Loss ratio serialized as the exact frozen-matrix f64 value
    /// (an f32 0.001 would not round-trip to the schema's enum literal).
    pub loss_ratio: f64,
    /// Path type.
    pub path: CellPath,
    /// Harness-measured RTT p50 (calibration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_rtt_ms_p50: Option<f64>,
    /// Whether the calibration stayed within 10% of nominal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_valid: Option<bool>,
}

/// Jitter classes of the frozen network matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JitterClass {
    /// No jitter.
    None,
    /// ±10% of nominal RTT.
    Low,
    /// ±30% of nominal RTT.
    High,
}

impl JitterClass {
    /// Fractional amplitude of the class (0 / 0.1 / 0.3).
    pub fn amplitude(self) -> f64 {
        match self {
            JitterClass::None => 0.0,
            JitterClass::Low => 0.10,
            JitterClass::High => 0.30,
        }
    }
}

/// Path dimension of the frozen network matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellPath {
    /// Direct connection.
    Direct,
    /// Relayed connection (Phase F+).
    Relayed,
}

/// Run manifest (`run-manifest.schema.json`): environment pinning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    /// Always [`RUN_MANIFEST_RECORD_TYPE`].
    pub record_type: &'static str,
    /// `run-` + 16 hex.
    pub run_manifest_id: String,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
    /// Harness version string.
    pub harness_version: String,
    /// Profile id (`msp1:` + 64 hex).
    pub profile_id: String,
    /// Runtime identity (name `mock` ⇒ every record is TEST-ONLY, ADR-019).
    pub runtime: ManifestRuntime,
    /// Hardware classes of the peer set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_class_peers: Option<Vec<String>>,
    /// Operating system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// 64-hex digest of the generation parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_params_digest: Option<String>,
    /// The network cell under test.
    pub network_cell: ManifestNetworkCell,
    /// Prompt corpus identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_corpus_id: Option<String>,
    /// Warm-up runs (unrecorded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_runs: Option<u32>,
    /// Recorded runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_runs: Option<u32>,
    /// Seed list.
    pub seeds: Vec<u64>,
    /// Confidence margin used by the engage rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_margin: Option<f64>,
}

impl RunManifest {
    /// Builds a manifest with the const `record_type`.
    #[must_use]
    pub fn new(
        run_manifest_id: impl Into<String>,
        created_at: impl Into<String>,
        harness_version: impl Into<String>,
        profile_id: impl Into<String>,
        runtime: ManifestRuntime,
        network_cell: ManifestNetworkCell,
        seeds: Vec<u64>,
    ) -> Self {
        Self {
            record_type: RUN_MANIFEST_RECORD_TYPE,
            run_manifest_id: run_manifest_id.into(),
            created_at: created_at.into(),
            harness_version: harness_version.into(),
            profile_id: profile_id.into(),
            runtime,
            hardware_class_peers: None,
            os: None,
            generation_params_digest: None,
            network_cell,
            prompt_corpus_id: None,
            warmup_runs: None,
            recorded_runs: None,
            seeds,
            confidence_margin: None,
        }
    }

    /// Serializes to a JSON value (for tests/inspection).
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("manifest serializes")
    }
}
