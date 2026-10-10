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
/// 9.6 PASS-2 prefix-match seed family base: a proposer request whose
/// sampling seed lies in `[PREFIX_MATCH_SEED_BASE,
/// PREFIX_MATCH_SEED_BASE + PREFIX_MATCH_MAX_MATCH]` encodes a MATCH
/// LENGTH `m = seed - base` — a non-divergent synthetic executor emits the
/// reference continuation (the [`VERIFIER_SEED`] stream over the SAME
/// re-posted conversation) for its first `m` tokens, then its own
/// deterministically different stream. The driver picks `m`; acceptance
/// stays MEASURED on the wire (real propose request → real verifier
/// request → client-side leading-match). TEST-ONLY synthetic machinery,
/// honestly labeled in every artifact it feeds.
pub const PREFIX_MATCH_SEED_BASE: u64 = 0x5EED_A000_0000_0000;
/// Maximum encodable prefix-match length (tokens; far above the window
/// bound of 16 — the family must cover any sweepable window).
pub const PREFIX_MATCH_MAX_MATCH: u64 = 4095;

/// Encodes a prefix-match request seed for match length `m`.
#[must_use]
pub fn prefix_match_seed(match_len: u32) -> u64 {
    PREFIX_MATCH_SEED_BASE
        + u64::from(match_len.min(u32::try_from(PREFIX_MATCH_MAX_MATCH).unwrap_or(u32::MAX)))
}

/// Decodes a seed into a prefix-match length, `None` outside the family.
#[must_use]
pub fn prefix_match_len(seed: u64) -> Option<u32> {
    (PREFIX_MATCH_SEED_BASE..=PREFIX_MATCH_SEED_BASE + PREFIX_MATCH_MAX_MATCH)
        .contains(&seed)
        .then(|| u32::try_from(seed - PREFIX_MATCH_SEED_BASE).ok())
        .flatten()
}
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
        // PASS-2 prefix-match family: a non-divergent executor asked (via
        // its request seed) for an m-token match emits the REFERENCE
        // stream (the verifier's) for its first m tokens, then its own
        // stream. Divergent executors (the determinism-failure model)
        // never match — their streams fold their private seed, family or
        // not.
        let match_len = if self.divergent {
            0
        } else {
            prefix_match_len(request.sampling.seed.unwrap_or(0)).unwrap_or(0)
        };
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
                        let stream_seed = if step < match_len {
                            // Prefix-match position: the reference
                            // (verifier) continuation over this exact
                            // conversation — both sides re-post the same
                            // text, so positions align token-for-token.
                            VERIFIER_SEED
                        } else {
                            folded
                        };
                        let event = ExecutorEvent::TokenDelta {
                            delta: SyntheticTokenExecutor::token_text(stream_seed, &prompt, step),
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
    /// Socket bind address for the serving listener. Defaults to
    /// `127.0.0.1` (loopback harness); the LAN serve side passes the
    /// machine's real LAN IP so the advertised multiaddr is dialable
    /// cross-machine (off-loopback requires `MSP_LISTENER=1`).
    pub bind_addr: String,
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
            bind_addr: "127.0.0.1".to_string(),
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

/// Environment label for the two-machine LAN cells (real QUIC, real RTT,
/// synthetic executor on the serve side — never a WAN claim).
pub const ENV_LABEL_LAN: &str = "lan-2machine-quic";

/// Parses one `MSP_BENCH_PEER_N` line as printed by `pass1_serve`:
/// `addr|peer_id|capability_token|advertised_queue_ms|capacity_class`.
pub fn parse_peer_line(line: &str) -> Result<RemoteBridge, String> {
    let parts: Vec<&str> = line.trim().split('|').collect();
    if parts.len() != 5 {
        return Err(format!(
            "peer line needs 5 '|'-separated fields, got {}: {line:?}",
            parts.len()
        ));
    }
    let capacity_class = match parts[4] {
        "gpu_high" => "gpu_high",
        "gpu_low" => "gpu_low",
        "cpu" => "cpu",
        _ => "gpu_mid",
    };
    Ok(RemoteBridge {
        addr: parts[0].to_string(),
        peer_id: parts[1].to_string(),
        capability_token: parts[2].to_string(),
        advertised_queue_ms: parts[3]
            .parse()
            .map_err(|_| format!("queue ms not a number: {:?}", parts[3]))?,
        capacity_class,
    })
}

/// Connects the driver to a LAN serve-side bridge (`pass1_serve` on the
/// other machine) and returns a `LiveBridge` handle whose pooled executor
/// dials the real remote. No local serving happens on this side.
pub async fn connect_remote(
    remote: &RemoteBridge,
    client_identity: &InstallationIdentity,
    metrics: Arc<PeerMetrics>,
) -> Result<LiveBridge, String> {
    let executor = RemoteExecutor::new(
        RemotePeer {
            addr: remote.addr.clone(),
            peer_id: remote.peer_id.clone(),
        },
        client_identity.clone(),
    )
    .with_metrics(metrics);
    executor.set_advertised_queue_ms(Some(remote.advertised_queue_ms));
    let roster = RosterFacts {
        peer_id: remote.peer_id.clone(),
        free_slots: 1,
        advertised_queue_ms: Some(remote.advertised_queue_ms),
        capacity_class: Some(remote.capacity_class.to_string()),
        has_direct_quic_addr: true,
    };
    // No local serving: the sender is only held to satisfy the handle
    // shape (never fired; the remote outlives this cell).
    let (shutdown, _hold) = tokio::sync::watch::channel(false);
    Ok(LiveBridge {
        addr: remote.addr.clone(),
        peer_id: remote.peer_id.clone(),
        token: remote.capability_token.clone(),
        roster,
        executor,
        shutdown,
    })
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
        .listen(&(config.bind_addr.clone() + ":0"))
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

/// Client-side acceptance profile: how proposer drafts relate to the
/// verifier continuation. Acceptance is still MEASURED from real wire
/// rounds (leading token match over the wire); the profile only decides
/// which drafts the proposers actually produce (the ground-truth knob).
/// Pass-1's High/Zero/Mixed full-window profiles are preserved with
/// identical acceptance outcomes; pass-2 adds the geometric
/// per-token-match model so acceptance becomes a DIALABLE curve
/// (window trade-offs, k-vs-acceptance, engaged win/loss vs acceptance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceRegime {
    /// Every draft matches the whole window (expected ~1.0).
    High,
    /// No draft ever matches (expected ~0.0).
    Zero,
    /// No match on rounds ≡ 2 (mod 3), full match otherwise (~2/3 at the
    /// acceptance-rate level).
    Mixed,
    /// Deterministic geometric per-token match: each round's draft
    /// matches the reference continuation for `m` tokens where `m` is
    /// drawn by successive `p = p_permille/1000` coin flips (capped at
    /// the window) — the classic speculative-decoding acceptance model
    /// (per-token acceptance p ⇒ expected accepted per round
    /// `p(1-p^w)/(1-p)`, so the per-round acceptance RATE is NOT p:
    /// ~0.64 at p=0.9 w8, ~0.42 at p=0.8 w8, ~0.27 at p=0.7 w8; larger
    /// windows LOWER the rate for the same p (~0.46 at p=0.9 w16) — the
    /// ANALYSIS tables state implied rates). Draws are keyed by
    /// `(round, draw_key)` — reproducible for a fixed driver seed.
    Geometric { p_permille: u32 },
}

impl AcceptanceRegime {
    /// Stable cell-name fragment (`high`/`zero`/`mixed`/`geo70`...).
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            AcceptanceRegime::High => "high",
            AcceptanceRegime::Zero => "zero",
            AcceptanceRegime::Mixed => "mixed",
            AcceptanceRegime::Geometric { .. } => "geo",
        }
    }

    /// The cell-name spelling of this regime (`geo` carries its permille).
    #[must_use]
    pub fn cell_tag(self) -> String {
        match self {
            AcceptanceRegime::Geometric { p_permille } => format!("geo{p_permille}"),
            other => other.tag().to_string(),
        }
    }

    /// The proposer request seed for one round: a prefix-match family
    /// seed encoding the intended match length. `window` is the
    /// effective window for the round; `draw_key` mixes rep + prompt so
    /// geometric draws vary per recorded run while staying reproducible.
    #[must_use]
    pub fn proposer_seed(self, round: u32, window: u32, draw_key: u64) -> u64 {
        let match_len = match self {
            AcceptanceRegime::High => window,
            AcceptanceRegime::Zero => 0,
            AcceptanceRegime::Mixed => {
                if round % 3 == 2 {
                    0
                } else {
                    window
                }
            }
            AcceptanceRegime::Geometric { p_permille } => {
                geometric_match_len(p_permille, window, round, draw_key)
            }
        };
        prefix_match_seed(match_len.min(window))
    }
}

/// Deterministic geometric match-length draw: successive p-coin flips
/// from an LCG seeded by `(round, draw_key, p)`; stops at the first
/// failure or the window bound. Distribution check pinned by test.
fn geometric_match_len(p_permille: u32, window: u32, round: u32, draw_key: u64) -> u32 {
    let mut state =
        splitmix_finalize(draw_key ^ (u64::from(round) << 32) ^ (u64::from(p_permille) << 48));
    let mut matched = 0u32;
    while matched < window {
        state = state
            .wrapping_mul(0x5851_F42D_4C95_7F2D)
            .wrapping_add(0x1405_7B7E_F767_814F);
        let coin = (state >> 33) % 1000;
        if coin < u64::from(p_permille) {
            matched += 1;
        } else {
            break;
        }
    }
    matched
}

/// Parses a driver-side profile spelling: `high`/`zero`/`mixed` or
/// `geo<permilles>` (`geo700` = p=0.7). The pass-2 sweep vocabulary.
pub fn parse_regime(tag: &str) -> Result<AcceptanceRegime, String> {
    match tag.trim() {
        "high" => Ok(AcceptanceRegime::High),
        "zero" => Ok(AcceptanceRegime::Zero),
        "mixed" => Ok(AcceptanceRegime::Mixed),
        other => other
            .strip_prefix("geo")
            .and_then(|p| p.parse::<u32>().ok())
            .filter(|p| (1..=999).contains(p))
            .map(|p_permille| AcceptanceRegime::Geometric { p_permille })
            .ok_or_else(|| format!("profile {tag:?} is not high|zero|mixed|geo<1..999 permilles>")),
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

/// How a cohort's verification cost is charged: re-export of the frozen
/// scheduler selection (ADR-032 §4 companion correction — see
/// [`modelswarm_scheduler::VerifyTerm`] for the semantics; the term and
/// its decision logic live in the scheduler crate so the item-4a
/// engage-gate tests run per-push under default features).
pub use modelswarm_scheduler::VerifyTerm;

/// The verification cost term of a cohort — the scheduler's frozen
/// selection: the VERIFIER decides (`cohort[0]`, the fastest-ranked
/// peer, is the role that executes verification). An HTTP peer may
/// still act as proposer (ADR-032 §4: first-class single, never a batch
/// verifier), so only the verifier's declaration selects the term; a
/// mixed cohort whose verifier has not declared the capability is
/// charged the sequential wire term.
#[must_use]
pub fn verify_mode(cohort: &[Candidate]) -> VerifyTerm {
    VerifyTerm::of_cohort(cohort)
}

/// Frozen `SwarmInputs` derivation from measured candidates (audit S2:
/// "no function derives them from candidates/measurements"), with the
/// PASS-1 WIRE adjustments recorded openly:
///
/// - **Per-round prefix re-post (S3):** over the request/reply wire a
///   speculative round RE-POSTS the whole conversation (there is no
///   propose/verify protocol — msp-v1 carries whole requests), so the
///   prefill term is charged PER ROUND, not once (the S3 structural cost
///   the pass-1 experiment exists to measure).
/// - **Capability-aware verification term (ADR-032 §4 companion
///   correction):** verification is charged as
///   [`crate::VERIFY_BATCH_STEPS`] forwards on the verifier ONLY when
///   the verifier declared batch-verify ([`verify_mode`] =
///   [`VerifyTerm::Batch`]) — the batch-verify ENGINE model (ADR-013).
///   Every other cohort (the default; no production adapter can declare
///   the capability) is charged the WIRE-TRUE sequential term:
///   `window+1` sequential verifier tokens per round, plus the per-round
///   re-post of the GROWING committed prefix (the committed prefix grows
///   by `window × acceptance` per round, so the per-round average over
///   `rounds_expected` rounds is `output × (R−1)/(2R)` tokens on top of
///   the prompt). Pass 2 measured this reality at 2.8–8.3× the batch
///   term's prediction — with this term the pass-2 never-engage
///   conclusion is enforced by the engage gate, not merely advised.
/// - Parallel proposers draft concurrently (wall time = fastest
///   proposer).
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
    let term = verify_mode(cohort);
    // Wire-true sequential term: the committed prefix the verifier
    // re-prefills grows by window × acceptance per round — average
    // output × (R−1)/(2R) committed tokens per round on top of the
    // prompt. The batch cohort keeps the ADR-013 ENGINE model
    // unchanged (a VerifyDrafts-class wire — ADR-032 §1, Proposed —
    // would carry token ids + hashes and drop the re-post entirely;
    // until that wire exists the engine term is the declared-
    // capability model).
    let committed_avg_tokens = match term {
        VerifyTerm::Batch => 0.0,
        VerifyTerm::SequentialWire => {
            f64::from(output_tokens) * (rounds_expected - 1.0) / (2.0 * rounds_expected)
        }
    };
    let verification_cost_ms = rounds_expected
        * term.decode_steps_per_round(window, VERIFY_BATCH_STEPS)
        / verifier.decode_tokens_per_ms.max(1e-6);
    SwarmInputs {
        // Wire rounds re-post the prefix every round (S3): per-round
        // prefill charged into the aggregate term.
        prefill_cost_ms: rounds_expected * (f64::from(prompt_tokens) + committed_avg_tokens)
            / verifier.prefill_tokens_per_ms.max(1e-6),
        proposal_cost_ms: rounds_expected * f64::from(window) / best_proposer_rate,
        verification_cost_ms,
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
    fn regimes_map_rounds_to_match_lengths() {
        // High: full-window match every round.
        assert_eq!(
            AcceptanceRegime::High.proposer_seed(5, 8, 0),
            prefix_match_seed(8),
            "high: always a full-window match"
        );
        // Zero: never matches (match length 0 still yields a draft — it
        // just agrees on nothing).
        assert_eq!(
            AcceptanceRegime::Zero.proposer_seed(5, 8, 0),
            prefix_match_seed(0),
            "zero: never matches"
        );
        // Mixed: no match on rounds ≡ 2 mod 3, full match otherwise —
        // the pass-1 outcome shape preserved through the family encoding.
        let mixed: Vec<bool> = (0..6)
            .map(|r| AcceptanceRegime::Mixed.proposer_seed(r, 8, 0) == prefix_match_seed(8))
            .collect();
        assert_eq!(
            mixed,
            vec![true, true, false, true, true, false],
            "mixed: diverges on rounds ≡ 2 mod 3"
        );
        // Geometric degenerate ends: p=1000 always matches, p=0 never.
        assert_eq!(
            AcceptanceRegime::Geometric { p_permille: 1000 }.proposer_seed(3, 8, 77),
            prefix_match_seed(8)
        );
        assert_eq!(
            AcceptanceRegime::Geometric { p_permille: 0 }.proposer_seed(3, 8, 77),
            prefix_match_seed(0)
        );
    }

    /// The geometric draw must be deterministic, key-sensitive, and
    /// statistically sane: over many draws the mean match fraction must
    /// sit near the CLASSIC-model expectation `p(1-p^w)/((1-p)w)` — at
    /// p=0.7, w=8 that is ~0.275, NOT 0.7 (per-token acceptance p means
    /// the run stops at the first mismatched token).
    #[test]
    fn geometric_draws_are_deterministic_and_centered() {
        let draw = |round: u32, key: u64| {
            prefix_match_len(
                AcceptanceRegime::Geometric { p_permille: 700 }.proposer_seed(round, 8, key),
            )
            .unwrap_or(0)
        };
        assert_eq!(draw(4, 99), draw(4, 99), "deterministic");
        let mut differing_keys = 0;
        for key in 0..50u64 {
            if draw(4, key) != draw(4, key.wrapping_add(1)) {
                differing_keys += 1;
            }
        }
        assert!(differing_keys > 10, "draw key varies the draw");
        let mean: f64 = (0..400u64)
            .map(|i| f64::from(draw(u32::try_from(i % 7).unwrap_or(0), i * 31 + 5)))
            .sum::<f64>()
            / (400.0 * 8.0);
        let p = 0.7_f64;
        let expected = p * (1.0 - p.powi(8)) / ((1.0 - p) * 8.0);
        assert!(
            (mean - expected).abs() < 0.05,
            "p=700 permille: mean match fraction {mean} near classic-model {expected:.3}"
        );
    }

    /// Family encode/decode round-trip and non-collision with the
    /// verifier seed.
    #[test]
    fn prefix_match_family_round_trips() {
        for m in [0u32, 1, 8, 16, 64, 4095] {
            assert_eq!(prefix_match_len(prefix_match_seed(m)), Some(m));
        }
        assert_eq!(prefix_match_len(VERIFIER_SEED), None);
        assert_eq!(prefix_match_len(0), None);
        // The family's own stream must differ from the reference stream
        // at every position (else "own" tokens could accidentally match).
        for m in [0u32, 1, 8] {
            assert_ne!(
                SyntheticTokenExecutor::token_text(prefix_match_seed(m), "p", 0),
                SyntheticTokenExecutor::token_text(VERIFIER_SEED, "p", 0),
                "family seed {m}: own stream differs at position 0"
            );
        }
    }

    /// PASS-1 RECORD CORRECTION, pinned: non-divergent synthetic peers
    /// with DIFFERENT bridge seeds produce IDENTICAL continuations for
    /// the same request seed (the executor fold keys on the request seed
    /// only; the bridge seed keys identity + speeds). Drafts match
    /// whenever the driver's seeds match — the pass-1 LAN all-fallback
    /// was the ENGAGE GATE (its reason strings say so), not per-bridge
    /// seed divergence, and the two engaged 4-bridge runs matched drafts
    /// through exactly this property.
    #[tokio::test]
    async fn non_divergent_peers_agree_per_request_seed() {
        use futures_util::StreamExt;
        use modelswarm_gateway::{NormalizedMessage, NormalizedRequest, Sampling};
        async fn deltas(executor_seed: u64, request_seed: u64) -> Vec<String> {
            let executor = SyntheticTokenExecutor::new(executor_seed, 1000.0, 1000.0);
            let request = NormalizedRequest {
                request_id: format!("t-{executor_seed}-{request_seed}"),
                profile_id: "msp1:aa".into(),
                capability_token: None,
                messages: vec![NormalizedMessage {
                    role: "user".into(),
                    content: "same conversation".into(),
                }],
                sampling: Sampling {
                    temperature: 0.0,
                    top_p: 1.0,
                    top_k: 40,
                    seed: Some(request_seed),
                },
                max_tokens: 4,
                deadline_ms: 1000,
                stream: true,
            };
            let mut stream = executor.execute(request).await.expect("stream");
            let mut out = Vec::new();
            while let Some(event) = stream.next().await {
                if let ExecutorEvent::TokenDelta { delta, .. } = event {
                    out.push(delta);
                }
            }
            out
        }
        let a = deltas(0x11, VERIFIER_SEED).await;
        let b = deltas(0x33, VERIFIER_SEED).await;
        assert_eq!(a, b, "different bridge seeds, same request seed: identical");
        let c = deltas(0x11, VERIFIER_SEED ^ 1).await;
        assert_ne!(a, c, "different request seeds diverge");
        // And the pass-2 family: a partial-match draft agrees on exactly
        // the encoded prefix of the verifier stream, then diverges.
        let reference = deltas(0x11, VERIFIER_SEED).await;
        let draft = deltas(0x33, prefix_match_seed(2)).await;
        assert_eq!(&draft[..2], &reference[..2], "first 2 tokens match");
        assert_ne!(draft[2], reference[2], "then the draft diverges");
    }

    #[test]
    fn regime_tags_round_trip_through_the_parser() {
        for regime in [
            AcceptanceRegime::High,
            AcceptanceRegime::Zero,
            AcceptanceRegime::Mixed,
            AcceptanceRegime::Geometric { p_permille: 700 },
        ] {
            assert_eq!(parse_regime(&regime.cell_tag()).unwrap(), regime);
        }
        assert_eq!(
            parse_regime("geo500").unwrap(),
            AcceptanceRegime::Geometric { p_permille: 500 }
        );
        // The degenerate geometric ends are spelled high/zero, not
        // geo1000/geo0 — the parser refuses them so cell names stay
        // canonical.
        assert!(parse_regime("geo1000").is_err());
        assert!(parse_regime("geo0").is_err());
        assert!(parse_regime("fast").is_err());
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
        wire_candidate(peer, decode, prefill, 0.0, false)
    }

    /// Candidate with an explicit advertised queue and verify capability
    /// (the two fields the ADR-032 §4 tests vary).
    fn wire_candidate(
        peer: &str,
        decode: f64,
        prefill: f64,
        queue: f64,
        batch_verify: bool,
    ) -> Candidate {
        use modelswarm_scheduler::{CapacityClass, NatPath};
        use modelswarm_types::ModelProfileId;
        Candidate {
            peer_id: peer.to_string(),
            profile_id: ModelProfileId::new("msp:pass1:q4_k_m:v1").unwrap(),
            measured_rtt_ms: 5.0,
            advertised_queue_ms: queue,
            prefill_tokens_per_ms: prefill,
            decode_tokens_per_ms: decode,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: 1,
            capacity_class: CapacityClass::Cpu,
            nat_path: NatPath::Direct,
            batch_verify,
        }
    }

    /// The pass-2 ENGAGEMENT pool shape (`pass2_pool()` in
    /// `tests/pass1_loopback.rs` and `pass1_serve --pool pass2`): V =
    /// fastest single (prefill 3.0, decode 0.10, free), D = 40×-decode
    /// drafter (queue 300 — drafted from, never the comparator), M/M2 =
    /// busy mids. `verifier_batch`/`proposer_batch` set the ADR-032 §4
    /// capability declaration per role.
    fn pass2_shaped_pool(verifier_batch: bool, proposer_batch: bool) -> Vec<Candidate> {
        vec![
            wire_candidate("v", 0.10, 3.0, 0.0, verifier_batch),
            wire_candidate("d", 4.00, 6.0, 300.0, proposer_batch),
            wire_candidate("m", 0.25, 1.2, 400.0, proposer_batch),
            wire_candidate("m2", 0.20, 1.0, 500.0, proposer_batch),
        ]
    }

    fn pass2_params(window: u32) -> Pass1Params {
        Pass1Params {
            proposal_window: window,
            output_tokens_target: 16,
            cohort_cap: 4,
            tau_ms: 0.0,
            ..Pass1Params::default()
        }
    }

    /// ADR-032 §4 companion correction, test (a): an HTTP-only cohort
    /// (NO batch-verify declaration — every production cohort today)
    /// NEVER engages, even at acceptance 1.0 in the pool shape that the
    /// frozen 1.5-step batch term engaged on. The wire-true term does
    /// not merely miss the (1+margin) bar: the cooperative prediction
    /// loses OUTRIGHT to the fastest eligible single — the pass-2
    /// never-engage conclusion enforced, not advised.
    #[test]
    fn http_only_cohorts_never_engage_on_the_wire_true_term() {
        let pool = pass2_shaped_pool(false, false);
        assert_eq!(verify_mode(&pool), VerifyTerm::SequentialWire);
        let perfect = |_: &str| Some(1.0_f64);
        for window in [8, 16] {
            let plan = plan_cohort(&pool, &pass2_params(window), perfect, 5.0, 153);
            assert!(
                !plan.engaged,
                "window {window}: HTTP-only cohort must never engage"
            );
            let prediction = plan.prediction_ms.expect("finite wire-true prediction");
            let fastest = plan
                .fastest_single_prediction_ms
                .expect("measured fastest single");
            assert!(
                prediction > fastest,
                "window {window}: wire-true prediction {prediction:.1}ms must exceed \
                 the fastest single {fastest:.1}ms outright"
            );
        }
    }

    /// ADR-032 §4 companion correction, test (b): a batch-DECLARED cohort
    /// uses the 1.5-step engine term and engages exactly when
    /// `predicted × (1+margin)` beats the fastest eligible single — it
    /// engages in the pass-2 corners, and falls back the moment the
    /// fastest single wins or the margin refuses.
    #[test]
    fn batch_declared_cohorts_engage_only_when_margin_beats_the_fastest_single() {
        let pool = pass2_shaped_pool(true, true);
        assert_eq!(verify_mode(&pool), VerifyTerm::Batch);
        let perfect = |_: &str| Some(1.0_f64);
        for window in [8, 16] {
            let plan = plan_cohort(&pool, &pass2_params(window), perfect, 5.0, 153);
            assert!(
                plan.engaged,
                "window {window}: the declared-capability cohort engages in the \
                 pass-2 corner (the engine term is legitimately cheaper there)"
            );
            let prediction = plan.prediction_ms.unwrap();
            let fastest = plan.fastest_single_prediction_ms.unwrap();
            assert!(prediction * (1.0 + plan_margin()) < fastest);
        }
        // A faster fastest-single flips it off: V at decode 0.25 completes
        // in ~120ms; the w8 batch prediction 151ms × 1.15 ≈ 174 loses.
        let faster_v = {
            let mut pool = pass2_shaped_pool(true, true);
            pool[0] = wire_candidate("v", 0.25, 3.0, 0.0, true);
            pool
        };
        let plan = plan_cohort(&faster_v, &pass2_params(8), perfect, 5.0, 153);
        assert!(
            !plan.engaged,
            "the fastest single winning must force fallback"
        );

        // And a refusing margin must block even a cheap prediction.
        let refusing = Pass1Params {
            margin: 10.0,
            ..pass2_params(8)
        };
        let plan = plan_cohort(&pool, &refusing, perfect, 5.0, 153);
        assert!(!plan.engaged, "margin rule still governs batch cohorts");
    }

    /// ADR-032 §4 companion correction, test (c): MIXED cohorts. The
    /// VERIFIER decides the term — an HTTP verifier is charged the
    /// sequential wire term regardless of the proposers' declarations,
    /// and a batch verifier with HTTP proposers still falls back when
    /// the fastest eligible single (either class) wins.
    #[test]
    fn mixed_cohorts_fall_back_when_the_fastest_single_wins() {
        let perfect = |_: &str| Some(1.0_f64);
        // HTTP verifier + batch-declared proposers: sequential term, the
        // never-engage conclusion applies (w8 AND the w16 parity corner).
        let http_verifier = pass2_shaped_pool(false, true);
        assert_eq!(verify_mode(&http_verifier), VerifyTerm::SequentialWire);
        for window in [8, 16] {
            let plan = plan_cohort(&http_verifier, &pass2_params(window), perfect, 5.0, 153);
            assert!(
                !plan.engaged,
                "window {window}: an HTTP verifier can never batch-verify"
            );
        }
        // Batch verifier + HTTP proposers: the proposer class is
        // irrelevant to the verify term, so this engages on the pass-2
        // pool shape — but falls back when the fastest single wins.
        let batch_verifier = pass2_shaped_pool(true, false);
        assert_eq!(verify_mode(&batch_verifier), VerifyTerm::Batch);
        let plan = plan_cohort(&batch_verifier, &pass2_params(8), perfect, 5.0, 153);
        assert!(plan.engaged, "batch verifier + HTTP proposers engages");
        let faster_single_wins = {
            let mut pool = pass2_shaped_pool(true, false);
            pool[0] = wire_candidate("v", 0.25, 3.0, 0.0, true);
            pool
        };
        let plan = plan_cohort(&faster_single_wins, &pass2_params(8), perfect, 5.0, 153);
        assert!(
            !plan.engaged,
            "mixed cohort falls back when the fastest single (either class) wins"
        );
    }

    /// Exact arithmetic pin of the capability-aware `swarm_inputs` terms
    /// (window 8, target 16, acceptance 1.0 ⇒ 2 expected rounds; prompt
    /// 128 tokens; verifier decode 0.10 / prefill 3.0; drafter 4.0;
    /// rtt 5 ms).
    #[test]
    fn swarm_inputs_charges_the_exact_capability_aware_terms() {
        let http = vec![
            wire_candidate("v", 0.10, 3.0, 0.0, false),
            wire_candidate("d", 4.00, 6.0, 300.0, false),
        ];
        let s = swarm_inputs(&http, 5.0, 1.0, 8, 16, 128);
        // Sequential wire term: 2 rounds × (window+1 = 9) verifier tokens
        // at 0.10 tok/ms = 180 ms — vs 30 ms under the 1.5-step engine
        // term, exactly the (window+1)/VERIFY_BATCH_STEPS = 6× pass-2 gap.
        assert!((s.verification_cost_ms - 180.0).abs() < 1e-9);
        // Per-round prefix re-post, wire-true: committed grows 0→16 over
        // 2 rounds, average 4 extra tokens per round ⇒ 2×(128+4)/3.0.
        assert!((s.prefill_cost_ms - 88.0).abs() < 1e-9);
        assert!((s.proposal_cost_ms - 4.0).abs() < 1e-9);
        assert!((s.synchronization_cost_ms - 10.0).abs() < 1e-9);
        assert!((s.expected_rollback_cost_ms - 0.0).abs() < 1e-9);
        assert_eq!(s.failure_risk_penalty_ms, FAILURE_RISK_PENALTY_MS);

        let batch: Vec<Candidate> = http
            .iter()
            .map(|c| Candidate {
                batch_verify: true,
                ..c.clone()
            })
            .collect();
        let b = swarm_inputs(&batch, 5.0, 1.0, 8, 16, 128);
        assert!((b.verification_cost_ms - 30.0).abs() < 1e-9);
        assert!((b.prefill_cost_ms - 2.0 * 128.0 / 3.0).abs() < 1e-9);
        assert!(
            (s.verification_cost_ms / b.verification_cost_ms - (8.0 + 1.0) / 1.5).abs() < 1e-9,
            "sequential/batch verify ratio is exactly (window+1)/VERIFY_BATCH_STEPS"
        );
    }

    fn plan_margin() -> f64 {
        modelswarm_scheduler::DEFAULT_CONFIDENCE_MARGIN
    }
}
