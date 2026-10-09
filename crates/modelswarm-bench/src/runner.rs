//! 9.6 pass-1 harness primitives: the deterministic delay shim, the
//! TEST-ONLY synthetic serving-side executor, loopback bridge spawning
//! over the REAL serving path, the arm/planner vocabulary, and the
//! client-side wire driver. Cell orchestration lives in
//! [`crate::harness`]; see that module for the honesty labeling.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use modelswarm_gateway::{
    ExecutorError, ExecutorEvent, ExecutorStream, FinishReason, InferenceExecutor,
    NormalizedMessage, NormalizedRequest, Sampling,
};
use modelswarm_identity::InstallationIdentity;
use modelswarm_node::measure::PeerMetrics;
use modelswarm_node::remote::{RemoteExecutor, RemotePeer};
use modelswarm_node::serving::serve_sessions;
use modelswarm_scheduler::shadow::RosterFacts;
use modelswarm_scheduler::{
    predicted_single_ms, predicted_swarm_ms, should_engage_cooperative, Candidate, SwarmInputs,
};
use modelswarm_transport::libp2p_backend::Libp2pTransport;
use serde::{Deserialize, Serialize};

use crate::params::Pass1Params;
use crate::records::Mode;
use crate::{FAILURE_RISK_PENALTY_MS, VERIFY_BATCH_STEPS};

/// Serving-side executor name recorded in pass-1 loopback manifests
/// (TEST-ONLY synthetic token source behind the REAL serving bridge).
pub const SYNTHETIC_EXECUTOR_NAME: &str = "synthetic-token-executor";
/// Environment label stamped into every loopback artifact.
pub const ENV_LABEL_LOOPBACK_INJECTED: &str = "loopback+injected-delay";
/// Deadline for every harness request (generous; injection-aware).
pub const REQUEST_DEADLINE_MS: u32 = 20_000;
/// Bound for waiting on F15 post-completion RTT probes.
pub const RTT_WAIT_BOUND: Duration = Duration::from_secs(10);
/// Deterministic verifier sampling seed. Acceptance regimes derive from
/// per-round proposer seeds relative to this (see [`AcceptanceRegime`]).
pub const VERIFIER_SEED: u64 = 0x5EED_0F1C;
/// Build hash for the synthetic executor manifest block (64-hex digest
/// of the executor name — stable, honest about being synthetic).
pub fn synthetic_executor_build_hash() -> String {
    use sha2::{Digest as ShaDigest, Sha256};
    hex::encode(Sha256::digest(SYNTHETIC_EXECUTOR_NAME.as_bytes()))
}

// ---------------------------------------------------------------------------
// Delay shim (RTT injection at the transport test seam)
// ---------------------------------------------------------------------------

/// Deterministic delay injection at the serving-executor seam (the F15/
/// shadow test seam): wraps any [`InferenceExecutor`] and delays a fixed
/// `request_delay` before the first stream event (the injected "RTT")
/// plus an optional fixed `per_event_delay` between subsequent events.
/// Fixed durations, no randomness — deterministic by construction.
///
/// Delays use [`precise_delay`]: `tokio::time::sleep` alone rounds up to
/// the OS timer tick (~15.6 ms on Windows), which would blur a 5 ms
/// injection into ~16 ms and invalidate the manifest's 10% calibration
/// tolerance. Callers must run on a multi-thread runtime (a serving-side
/// sub-tick spin must not block the measuring client).
///
/// Loopback runs carrying this shim are labeled
/// [`ENV_LABEL_LOOPBACK_INJECTED`] in every manifest and report and are
/// NEVER LAN claims; the per-cell calibration record measures the
/// injected delay end-to-end (baseline-subtracted 1-token round-trip
/// p50, 10% manifest tolerance).
pub struct DelayShim {
    inner: Arc<dyn InferenceExecutor>,
    request_delay: Duration,
    per_event_delay: Duration,
}

impl DelayShim {
    /// Wraps `inner` with the injected delays.
    #[must_use]
    pub fn new(
        inner: Arc<dyn InferenceExecutor>,
        request_delay: Duration,
        per_event_delay: Duration,
    ) -> Self {
        Self {
            inner,
            request_delay,
            per_event_delay,
        }
    }

    /// The injected pre-first-event delay (recorded in calibration).
    #[must_use]
    pub fn request_delay(&self) -> Duration {
        self.request_delay
    }
}

#[async_trait::async_trait]
impl InferenceExecutor for DelayShim {
    async fn execute(&self, request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
        let inner = self.inner.execute(request).await?;
        let first = self.request_delay;
        let per_event = self.per_event_delay;
        Ok(Box::pin(futures_util::stream::unfold(
            (inner, first, per_event, false),
            |(mut inner, first, per_event, started)| async move {
                if !started {
                    precise_delay(first).await;
                } else if per_event > Duration::ZERO {
                    precise_delay(per_event).await;
                }
                inner
                    .next()
                    .await
                    .map(|event| (event, (inner, first, per_event, true)))
            },
        )))
    }
}

// ---------------------------------------------------------------------------
// TEST-ONLY synthetic serving-side executor
// ---------------------------------------------------------------------------

/// TEST-ONLY deterministic token-aligned executor: ONE
/// [`ExecutorEvent::TokenDelta`] per token; token text a pure function of
/// `(executor seed, request sampling seed, prompt, position)`. Real
/// per-token sleeps match the configured rates, so the Usage frames AND
/// the realized timings both reflect the peer's synthetic speed — the
/// F15-measured inputs get genuine heterogeneous signal. Two requests
/// with the same folded (seed, prompt) continue identically; different
/// seeds diverge — the dry run's acceptance-regime knob.
///
/// Token-alignment matters: the pass-1 round protocol compares proposals
/// against the verifier's continuation delta-by-delta, which is EXACT
/// here. (The production local executor batches up to 16 tokens per
/// delta; there a partial in-delta mismatch conservatively rejects the
/// whole delta — documented pass-1 limitation.)
pub struct SyntheticTokenExecutor {
    seed: u64,
    divergent: bool,
    decode_tokens_per_ms: f64,
    prefill_tokens_per_ms: f64,
}

impl SyntheticTokenExecutor {
    /// Configured synthetic peer (exact-profile continuation).
    #[must_use]
    pub fn new(seed: u64, decode_tokens_per_ms: f64, prefill_tokens_per_ms: f64) -> Self {
        Self {
            seed,
            divergent: false,
            decode_tokens_per_ms: decode_tokens_per_ms.max(1e-6),
            prefill_tokens_per_ms: prefill_tokens_per_ms.max(1e-6),
        }
    }

    /// Makes the continuation fold this executor's private seed — the
    /// determinism-divergent peer model (cross-microarch CPU kernels).
    #[must_use]
    pub fn with_divergence(mut self) -> Self {
        self.divergent = true;
        self
    }

    /// Deterministic token text for (folded seed, prompt, position).
    fn token_text(folded_seed: u64, prompt: &str, position: u32) -> String {
        // FNV-1a over (seed, prompt bytes, position), splitmix-finalized.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in folded_seed.to_le_bytes().iter().chain(prompt.as_bytes()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        hash ^= u64::from(position);
        hash = hash.wrapping_mul(0x100_0000_01b3);
        format!("w{:06x}", splitmix_finalize(hash) % 0x10_0000)
    }
}

/// Sub-tick-accurate delay for the benchmark harness: sleeps the coarse
/// part (anything above one OS timer tick), then busy-waits the
/// remainder (bounded by one tick, ~16 ms) so a 5 ms injection lands at
/// ~5 ms instead of the Windows timer tick. BENCHMARK-ONLY technique —
/// never for production request paths. Callers must run on a
/// multi-thread runtime so the bounded spin cannot stall the measuring
/// client task.
pub async fn precise_delay(total: Duration) {
    const TICK: Duration = Duration::from_millis(16);
    let target = Instant::now() + total;
    if total > TICK {
        tokio::time::sleep(total - TICK).await;
    }
    while Instant::now() < target {
        std::hint::spin_loop();
    }
}

fn splitmix_finalize(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[async_trait::async_trait]
impl InferenceExecutor for SyntheticTokenExecutor {
    async fn execute(&self, request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
        let mut prompt = String::new();
        for message in &request.messages {
            prompt.push_str(&message.content);
            prompt.push('\n');
        }
        let folded =
            (u64::from(u32::from(self.divergent)) * self.seed) ^ request.sampling.seed.unwrap_or(0);
        let max_tokens = request.max_tokens;
        let per_token = Duration::from_secs_f64(1.0 / self.decode_tokens_per_ms / 1000.0);
        let prefill = Duration::from_secs_f64(
            f64::from(u32::try_from(prompt.len().div_ceil(4)).unwrap_or(u32::MAX))
                / self.prefill_tokens_per_ms
                / 1000.0,
        );
        let decode_tokens_per_ms = self.decode_tokens_per_ms;
        let prefill_tokens_per_ms = self.prefill_tokens_per_ms;
        let prompt_tokens = u32::try_from(prompt.len().div_ceil(4)).unwrap_or(u32::MAX);

        // Step schedule: 0..max_tokens emit TokenDelta; max_tokens emits
        // Usage; max_tokens + 1 emits Completed; then the stream ends.
        Ok(Box::pin(futures_util::stream::unfold(
            (0u32, false),
            move |(step, _)| {
                let prompt = prompt.clone();
                let per_token = per_token;
                let prefill = prefill;
                let decode_tokens_per_ms = decode_tokens_per_ms;
                let prefill_tokens_per_ms = prefill_tokens_per_ms;
                let prompt_tokens = prompt_tokens;
                async move {
                    // Steps: 0..max_tokens → deltas, max_tokens → Usage,
                    // max_tokens + 1 → Completed, beyond → stream end.
                    if step > max_tokens + 1 {
                        return None;
                    }
                    if step < max_tokens {
                        if step == 0 {
                            precise_delay(prefill).await;
                        }
                        precise_delay(per_token).await;
                        let event = ExecutorEvent::TokenDelta {
                            delta: SyntheticTokenExecutor::token_text(folded, &prompt, step),
                            index: step,
                        };
                        Some((event, (step + 1, false)))
                    } else if step == max_tokens {
                        let event = ExecutorEvent::Usage {
                            prompt_tokens,
                            completion_tokens: max_tokens,
                            prefill_ms: f64::from(prompt_tokens) / prefill_tokens_per_ms,
                            decode_ms: f64::from(max_tokens) / decode_tokens_per_ms,
                        };
                        Some((event, (step + 1, false)))
                    } else {
                        let event = ExecutorEvent::Completed {
                            finish_reason: FinishReason::Stop,
                        };
                        Some((event, (step + 1, false)))
                    }
                }
            },
        )))
    }
}

// ---------------------------------------------------------------------------
// Topology: bridges over the REAL serving path
// ---------------------------------------------------------------------------

/// Loopback bridge configuration (a synthetic peer behind the real
/// serving bridge).
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Discriminator folded into the executor seed (the peer identity
    /// seed derives from it too).
    pub seed: u64,
    /// Synthetic decode rate (tokens/ms) — drives real per-token delays
    /// and the Usage frames.
    pub decode_tokens_per_ms: f64,
    /// Synthetic prefill rate (tokens/ms).
    pub prefill_tokens_per_ms: f64,
    /// Advertised `queueMs` (untrusted roster input for the discount).
    pub advertised_queue_ms: u64,
    /// Roster `capacityClass` vocabulary member.
    pub capacity_class: &'static str,
    /// Free serving slots advertised.
    pub free_slots: u32,
    /// Determinism-divergent peer (cross-machine determinism-failure
    /// model; off by default — exact-profile peers continue identically).
    pub divergent: bool,
}

impl BridgeConfig {
    /// A gpu_mid-shaped peer at the given speeds.
    #[must_use]
    pub fn new(seed: u64, decode: f64, prefill: f64) -> Self {
        Self {
            seed,
            decode_tokens_per_ms: decode,
            prefill_tokens_per_ms: prefill,
            advertised_queue_ms: 0,
            capacity_class: "gpu_mid",
            free_slots: 1,
            divergent: false,
        }
    }
}

/// One live serving bridge the harness dials: roster facts, lease
/// token, the pooled metrics-attached executor, and the shutdown
/// handle. The handle is held (NOT fired) for the bridge's lifetime:
/// `serve_sessions` starves its accept loop if the watch sender drops,
/// so the bridge keeps it alive.
pub struct LiveBridge {
    /// Bound QUIC multiaddr.
    pub addr: String,
    /// Serving peer id (QUIC-verified).
    pub peer_id: String,
    /// Capability token for requests (experiment lease).
    pub token: String,
    /// Roster facts for the candidate set.
    pub roster: RosterFacts,
    /// The pooled, F15-metrics-attached executor for this peer.
    pub executor: RemoteExecutor,
    /// Held alive for the bridge's lifetime; send `true` to stop serving.
    pub shutdown: tokio::sync::watch::Sender<bool>,
}

/// A remote (LAN) bridge for the owner-gated pass: address + lease
/// material supplied by the serve side (the `pass1_serve` example).
#[derive(Debug, Clone)]
pub struct RemoteBridge {
    /// Bound QUIC multiaddr on the serving machine.
    pub addr: String,
    /// Serving peer id.
    pub peer_id: String,
    /// Experiment capability token (minted by the serve side).
    pub capability_token: String,
    /// Advertised `queueMs` (untrusted roster input).
    pub advertised_queue_ms: u64,
    /// Roster `capacityClass` vocabulary member.
    pub capacity_class: &'static str,
}

/// Mints a (policy, wire token) pair binding `client_peer_id` to
/// `profile` under a fresh throwaway hub key. Loopback/experiment only —
/// production leases come from the tracker (ADR-026).
pub fn mint_lease(
    profile: &str,
    client_peer_id: &str,
) -> (modelswarm_node::serving::LeasePolicy, String) {
    use ed25519_dalek::SigningKey;
    let signing = SigningKey::generate(&mut rand_core::OsRng);
    let rfc3339 = |plus_secs: i64| {
        time::OffsetDateTime::now_utc()
            .saturating_add(time::Duration::seconds(plus_secs))
            .format(&time::format_description::well_known::Rfc3339)
            .expect("rfc3339 formats")
    };
    let lease = modelswarm_eligibility::EligibilityLease::issue(
        client_peer_id,
        "pass1-harness",
        profile,
        rfc3339(-60),
        rfc3339(3600),
        rfc3339(3700),
        true,
        true,
        1,
        modelswarm_eligibility::CapacityClass::GpuMid,
        1,
        "pass1-challenge",
        "pass1-nonce",
        &signing,
    );
    let token = lease.to_wire();
    let policy = modelswarm_node::serving::LeasePolicy::from_hex(&hex::encode(
        signing.verifying_key().to_bytes(),
    ))
    .expect("fresh key hex parses");
    (policy, token)
}

/// Deterministic 32-byte identity seed from a config seed.
fn bridge_identity_seed(config_seed: u64) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    let mixed = splitmix_finalize(config_seed | 1);
    bytes[..8].copy_from_slice(&mixed.to_le_bytes());
    bytes
}

/// Spawns one REAL serving bridge on loopback and returns the dialable
/// handle. `injected_request_delay` is the shim's pre-first-event delay.
pub async fn spawn_bridge(
    profile_id: &str,
    client_identity: &InstallationIdentity,
    config: &BridgeConfig,
    injected_request_delay: Duration,
    telemetry: Arc<modelswarm_telemetry::Telemetry>,
    metrics: Arc<PeerMetrics>,
) -> Result<LiveBridge, String> {
    let transport = Libp2pTransport::new(&InstallationIdentity::from_bytes(&bridge_identity_seed(
        config.seed,
    )))
    .map_err(|e| format!("transport: {e}"))?;
    let listener = transport
        .listen("127.0.0.1:0")
        .await
        .map_err(|e| format!("listen: {e}"))?;
    let peer_id = transport.peer_id().to_string();
    let addr = listener.bound_addr().to_string();

    let client_peer_id = Libp2pTransport::new(client_identity)
        .map_err(|e| format!("client transport: {e}"))?
        .peer_id()
        .to_string();
    let (policy, token) = mint_lease(profile_id, &client_peer_id);

    let mut executor = SyntheticTokenExecutor::new(
        config.seed,
        config.decode_tokens_per_ms,
        config.prefill_tokens_per_ms,
    );
    if config.divergent {
        executor = executor.with_divergence();
    }
    let executor = Arc::new(executor);
    let shimmed = Arc::new(DelayShim::new(
        executor,
        injected_request_delay,
        Duration::ZERO,
    ));
    let (shutdown, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(serve_sessions(
        listener,
        shimmed,
        profile_id.to_string(),
        policy,
        telemetry,
        rx,
    ));

    let executor = RemoteExecutor::new(
        RemotePeer {
            addr: addr.clone(),
            peer_id: peer_id.clone(),
        },
        client_identity.clone(),
    )
    .with_metrics(metrics);
    executor.set_advertised_queue_ms(Some(config.advertised_queue_ms));
    let roster = RosterFacts {
        peer_id: peer_id.clone(),
        free_slots: config.free_slots,
        advertised_queue_ms: Some(config.advertised_queue_ms),
        capacity_class: Some(config.capacity_class.to_string()),
        has_direct_quic_addr: true,
    };
    Ok(LiveBridge {
        addr,
        peer_id,
        token,
        roster,
        executor,
        shutdown,
    })
}

// ---------------------------------------------------------------------------
// Arms, planning, wire driving
// ---------------------------------------------------------------------------

/// The pass-1 arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    /// Fastest-predicted single peer, independently executed (the
    /// inviolable comparator).
    FastestSingle,
    /// Fixed cohort size k.
    FixedK { k: u32 },
    /// Planner-chosen cohort size k (deterministic heuristic).
    PlannerK,
}

impl Arm {
    /// Stable identifier for request roots, manifests, and joins.
    #[must_use]
    pub fn tag(self) -> String {
        match self {
            Arm::FastestSingle => "fastest-single".to_string(),
            Arm::FixedK { k } => format!("fixed-k{k}"),
            Arm::PlannerK => "planner-k".to_string(),
        }
    }

    /// The schema `mode` this arm records as.
    #[must_use]
    pub fn mode(self) -> Mode {
        match self {
            Arm::FastestSingle => Mode::Single,
            Arm::FixedK { .. } | Arm::PlannerK => Mode::SpeculativeExact,
        }
    }
}

/// Client-side acceptance regime: how proposer request seeds relate to
/// the verifier's. Acceptance is still MEASURED from real wire rounds;
/// the regime only decides which proposals the proposers actually
/// produce (the dry run's ground-truth knob).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceRegime {
    /// Proposer seed equals the verifier's every round (expected ~1.0).
    High,
    /// Proposer seed differs every round (expected ~0.0).
    Zero,
    /// Differs on rounds ≡ 2 (mod 3) (expected ~2/3).
    Mixed,
}

impl AcceptanceRegime {
    /// The proposer request seed for one round.
    #[must_use]
    pub fn proposer_seed(self, round: u32) -> u64 {
        match self {
            AcceptanceRegime::High => VERIFIER_SEED,
            AcceptanceRegime::Zero => VERIFIER_SEED ^ 0xDEAD_BEEF,
            AcceptanceRegime::Mixed => {
                if round % 3 == 2 {
                    VERIFIER_SEED ^ 0xDEAD_BEEF
                } else {
                    VERIFIER_SEED
                }
            }
        }
    }
}

/// The deterministic §5-shaped cohort plan (no learned components).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohortPlan {
    /// Chosen cohort size (2..=cap); `None` when no finite prediction
    /// existed (insufficient measurement).
    pub k: Option<u32>,
    /// Frozen `predicted_swarm_ms` at the chosen k (finite only).
    pub prediction_ms: Option<f64>,
    /// `predicted_single_ms` of the fastest candidate (always computed;
    /// infinite when the candidate set is cold).
    pub fastest_single_prediction_ms: Option<f64>,
    /// Whether the margin rule engages the cooperative mode.
    pub engaged: bool,
    /// Every (k, prediction) the sweep evaluated (planner telemetry).
    pub sweep: Vec<(u32, f64)>,
}

/// Deterministic marginal-benefit planner (expanded-mission §5 shape):
/// sweeps k = 2..=cap ascending over the frozen
/// [`predicted_swarm_ms`] with MEASURED inputs, keeps the best
/// prediction, stops once the marginal benefit (previous − current)
/// drops to `tau_ms`, engages only when the frozen margin rule beats the
/// fastest single. Deterministic; no learned components; τ is a recorded
/// placeholder until 9.6 measures it.
#[must_use]
pub fn plan_cohort(
    candidates: &[Candidate],
    params: &Pass1Params,
    acceptance_of: impl Fn(&str) -> Option<f64>,
    rtt_ms: f64,
    prompt_tokens: u32,
) -> CohortPlan {
    let fastest_single = candidates
        .first()
        .map(|c| predicted_single_ms(c, prompt_tokens, params.output_tokens_target))
        .filter(|p| p.is_finite());
    let cap = params
        .cohort_cap
        .min(u32::try_from(candidates.len()).unwrap_or(params.cohort_cap))
        .max(2);
    let mut sweep: Vec<(u32, f64)> = Vec::new();
    let mut best: Option<(u32, f64)> = None;
    let mut previous = f64::INFINITY;
    for k in 2..=cap {
        let cohort: Vec<Candidate> = candidates
            .iter()
            .take(usize::try_from(k).unwrap_or(usize::MAX))
            .cloned()
            .collect();
        let acceptance = cohort_acceptance(&cohort, &acceptance_of, params.acceptance_default);
        let inputs = swarm_inputs(
            &cohort,
            rtt_ms,
            acceptance,
            params.proposal_window,
            params.output_tokens_target,
            prompt_tokens,
        );
        let prediction = predicted_swarm_ms(&inputs);
        if prediction.is_finite() {
            let marginal = previous - prediction;
            sweep.push((k, prediction));
            if best.is_none() || prediction < best.map(|(_, p)| p).unwrap_or(f64::INFINITY) {
                best = Some((k, prediction));
            }
            if marginal <= params.tau_ms {
                break;
            }
            previous = prediction;
        }
    }
    let (k, prediction) = best.unzip();
    let engaged = k.is_some()
        && prediction.is_some_and(|p| p.is_finite())
        && fastest_single.is_some_and(|f| {
            should_engage_cooperative(prediction.unwrap_or(f64::INFINITY), f, params.margin)
                .unwrap_or(false)
        });
    CohortPlan {
        k,
        prediction_ms: prediction,
        fastest_single_prediction_ms: fastest_single,
        engaged,
        sweep,
    }
}

/// Mean measured acceptance over the cohort's proposers (the verifier is
/// `cohort[0]`; cold pairs fall back to the recorded prior).
fn cohort_acceptance(
    cohort: &[Candidate],
    acceptance_of: &impl Fn(&str) -> Option<f64>,
    prior: f64,
) -> f64 {
    let proposers = cohort.iter().skip(1);
    let mut sum = 0.0;
    let mut n = 0usize;
    for proposer in proposers {
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

/// Frozen `SwarmInputs` derivation from measured candidates (audit S2:
/// "no function derives them from candidates/measurements"), with the
/// PASS-1 WIRE adjustment recorded openly: over the request/reply wire a
/// speculative round RE-POSTS the whole prefix (there is no propose/
/// verify protocol — msp-v1 carries whole requests), so the prefill term
/// is charged PER ROUND, not once (the S3 structural cost the pass-1
/// experiment exists to measure). Parallel proposers draft concurrently
/// (wall time = fastest proposer); verification is charged as
/// `VERIFY_BATCH_STEPS` forwards on the verifier — the batch-verify
/// ENGINE model (ADR-013). The wire's sequential reality then shows up
/// as realized-vs-predicted divergence in the join rows, which is
/// exactly the pass-1 evidence.
#[must_use]
pub fn swarm_inputs(
    cohort: &[Candidate],
    rtt_ms: f64,
    acceptance: f64,
    window: u32,
    output_tokens: u32,
    prompt_tokens: u32,
) -> SwarmInputs {
    let verifier = &cohort[0];
    let best_proposer_rate = cohort
        .iter()
        .skip(1)
        .map(|c| c.decode_tokens_per_ms)
        .fold(0.0_f64, f64::max)
        .max(1e-6);
    let rounds_expected = (f64::from(output_tokens) / (f64::from(window) * acceptance)).max(1.0);
    SwarmInputs {
        // Wire rounds re-post the prefix every round (S3): per-round
        // prefill charged into the aggregate term.
        prefill_cost_ms: rounds_expected * f64::from(prompt_tokens)
            / verifier.prefill_tokens_per_ms.max(1e-6),
        proposal_cost_ms: rounds_expected * f64::from(window) / best_proposer_rate,
        verification_cost_ms: rounds_expected * VERIFY_BATCH_STEPS
            / verifier.decode_tokens_per_ms.max(1e-6),
        synchronization_cost_ms: rounds_expected * rtt_ms.max(0.0),
        expected_rollback_cost_ms: rounds_expected * (1.0 - acceptance)
            / verifier.decode_tokens_per_ms.max(1e-6),
        failure_risk_penalty_ms: FAILURE_RISK_PENALTY_MS,
    }
}

/// One measured client-side completion (timings + delta texts + usage).
#[derive(Debug, Clone, Default)]
pub struct WireCompletion {
    /// Token-delta texts in arrival order.
    pub deltas: Vec<String>,
    /// Request-sent → first delta (ms); 0.0 when none arrived.
    pub ttft_ms: f64,
    /// Request-sent → terminal event (ms).
    pub total_ms: f64,
    /// Usage frame `(prompt_tokens, completion_tokens, prefill_ms,
    /// decode_ms)`, when the stream produced one.
    pub usage: Option<(u32, u32, f64, f64)>,
}

impl WireCompletion {
    /// The completion text (deltas concatenated).
    #[must_use]
    pub fn text(&self) -> String {
        self.deltas.concat()
    }
}

/// Arguments of [`execute_on`]: target executor + lease material +
/// conversation + sampling seed (bundle keeps the helper under the
/// argument-count lint).
pub struct WireRequest<'a> {
    /// The bridge's pooled executor.
    pub executor: &'a RemoteExecutor,
    /// Profile the serving bridge hosts.
    pub profile_id: &'a str,
    /// The bridge's capability token.
    pub token: &'a str,
    /// Unique request id (single-use; the serving side rejects replays).
    pub request_id: &'a str,
    /// The corpus prompt.
    pub prompt: &'a str,
    /// The committed speculative prefix so far (re-posted in full — the
    /// honest current-adapter cost model).
    pub committed: &'a str,
    /// Output budget for this request.
    pub max_tokens: u32,
    /// Sampling seed folded into the deterministic continuation.
    pub seed: u64,
}

/// Drives one full request through a bridge's executor and wall-clocks
/// it.
pub async fn execute_on(request: &WireRequest<'_>) -> Result<WireCompletion, ExecutorError> {
    let WireRequest {
        executor,
        profile_id,
        token,
        request_id,
        prompt,
        committed,
        max_tokens,
        seed,
    } = *request;
    let mut conversation = String::with_capacity(prompt.len() + committed.len());
    conversation.push_str(prompt);
    conversation.push_str(committed);
    let request = NormalizedRequest {
        request_id: request_id.to_string(),
        profile_id: profile_id.to_string(),
        capability_token: Some(token.to_string()),
        messages: vec![NormalizedMessage {
            role: "user".into(),
            content: conversation,
        }],
        sampling: Sampling {
            temperature: 0.0,
            top_p: 1.0,
            top_k: 40,
            seed: Some(seed),
        },
        max_tokens,
        deadline_ms: REQUEST_DEADLINE_MS,
        stream: true,
    };
    let started = Instant::now();
    let mut stream = executor.execute(request).await?;
    let mut completion = WireCompletion::default();
    let mut first_token_at: Option<Instant> = None;
    while let Some(event) = stream.next().await {
        match event {
            ExecutorEvent::TokenDelta { delta, .. } => {
                first_token_at.get_or_insert_with(Instant::now);
                completion.deltas.push(delta);
            }
            ExecutorEvent::Usage {
                prompt_tokens,
                completion_tokens,
                prefill_ms,
                decode_ms,
            } => {
                completion.usage = Some((prompt_tokens, completion_tokens, prefill_ms, decode_ms));
            }
            ExecutorEvent::Completed { .. } => break,
            ExecutorEvent::Error { code, .. } => {
                return Err(ExecutorError::Fatal { code });
            }
            ExecutorEvent::Accepted { .. } => {}
        }
    }
    completion.ttft_ms = first_token_at
        .map(|t| t.duration_since(started).as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    completion.total_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(completion)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_token_text_is_deterministic_and_sensitive() {
        let a = SyntheticTokenExecutor::token_text(7, "prompt", 0);
        let b = SyntheticTokenExecutor::token_text(7, "prompt", 0);
        assert_eq!(a, b);
        assert_ne!(
            a,
            SyntheticTokenExecutor::token_text(8, "prompt", 0),
            "seed matters"
        );
        assert_ne!(
            a,
            SyntheticTokenExecutor::token_text(7, "prompt", 1),
            "position matters"
        );
        assert_ne!(
            a,
            SyntheticTokenExecutor::token_text(7, "other", 0),
            "prompt matters"
        );
        assert!(a.starts_with('w') && a.len() == 7, "compact token: {a}");
    }

    #[test]
    fn regimes_map_rounds_to_proposer_seeds() {
        assert_eq!(
            AcceptanceRegime::High.proposer_seed(5),
            VERIFIER_SEED,
            "high: always matches"
        );
        assert_ne!(
            AcceptanceRegime::Zero.proposer_seed(5),
            VERIFIER_SEED,
            "zero: never matches"
        );
        let mixed: Vec<bool> = (0..6)
            .map(|r| AcceptanceRegime::Mixed.proposer_seed(r) == VERIFIER_SEED)
            .collect();
        assert_eq!(
            mixed,
            vec![true, true, false, true, true, false],
            "mixed: diverges on rounds ≡ 2 mod 3"
        );
    }

    #[test]
    fn planner_sweeps_stops_on_tau_and_engages_conservatively() {
        use crate::params::bounds;
        // Two measured candidates (fast verifier + decent proposer).
        let params = Pass1Params {
            cohort_cap: 3,
            tau_ms: 0.0,
            ..Pass1Params::default()
        };
        let candidates = vec![candidate("v", 0.5, 2.0), candidate("p", 0.25, 1.0)];
        let none = |_peer: &str| None::<f64>;
        // k can only be 2 with two candidates.
        let plan = plan_cohort(&candidates, &params, none, 5.0, 128);
        assert_eq!(plan.k, Some(2));
        assert_eq!(plan.sweep.len(), 1, "one sweep entry (k=2)");
        assert!(plan.prediction_ms.unwrap() > 0.0);
        // Engagement must respect the frozen margin rule; with a huge
        // margin nothing engages (honest negative by construction).
        let params = Pass1Params {
            margin: 10.0,
            tau_ms: bounds::TAU_MS_MAX,
            ..params
        };
        let plan = plan_cohort(&candidates, &params, none, 5.0, 128);
        assert!(!plan.engaged, "margin rule blocks engagement");
        // Cold candidates (zero rates): the frozen single formula is
        // INFINITY (no fastest-single prediction at all), and the
        // swarm-input guards keep the cooperative prediction finite but
        // absurd — the plan records the absurdity and does NOT engage
        // (no comparator to beat).
        let cold = vec![candidate("v", 0.0, 0.0), candidate("p", 0.0, 0.0)];
        let plan = plan_cohort(&cold, &params, none, 5.0, 128);
        assert_eq!(plan.fastest_single_prediction_ms, None);
        assert!(plan.prediction_ms.unwrap() > 1.0e6, "absurd, not hidden");
        assert!(!plan.engaged);
    }

    fn candidate(peer: &str, decode: f64, prefill: f64) -> Candidate {
        use modelswarm_scheduler::{CapacityClass, NatPath};
        use modelswarm_types::ModelProfileId;
        Candidate {
            peer_id: peer.to_string(),
            profile_id: ModelProfileId::new("msp:pass1:q4_k_m:v1").unwrap(),
            measured_rtt_ms: 5.0,
            advertised_queue_ms: 0.0,
            prefill_tokens_per_ms: prefill,
            decode_tokens_per_ms: decode,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: 1,
            capacity_class: CapacityClass::Cpu,
            nat_path: NatPath::Direct,
        }
    }
}
