//! `modelswarm-sim` — deterministic loopback scenario runner for the Phase C
//! transport gate (ADR-018) and the Phase D/E speculative gates.
//!
//! Every scenario runs against real TCP sockets on `127.0.0.1:0` and prints
//! exactly one JSON object per line (machine-readable; timings vary, the
//! schema does not). Identities are derived from fixed seeds so peer ids and
//! request ids are stable across runs.
//!
//! Scenarios (see `main.rs` for the CLI):
//!
//! - [`pair`]: two peers handshake, one inference round-trip against a
//!   canned response (`Hel`+`lo` deltas, usage, `stop`), plus an RTT probe.
//! - [`mesh`]: `n` peers on distinct loopback ports; every pair handshakes;
//!   adjacency matrix + RTT matrix.
//! - [`kill`]: a pair where the responder dies at a deadline; the client
//!   reports a structured failure event — no silent hang, ever.
//! - [`spec`]: in-process proposer + verifier servers (TEST-ONLY
//!   [`MockRuntime`], ADR-019) plus a coordinator over real loopback
//!   `SignedFrameTransport` sessions; 64 speculative tokens and, for the same
//!   prompt, plain single decoding against the verifier runtime; reports the
//!   greedy-equality verdict plus acceptance telemetry (Phase D).
//! - [`spec_multi`]: Phase E — one verifier + N (2..=7) seeded proposer
//!   servers + coordinator over real loopback TCP; the multi-proposer
//!   candidate-trie session plus, for the same prompt, plain single decoding;
//!   reports the greedy-equality verdict plus trie telemetry (duplicate work,
//!   pruning, stragglers).
//! - [`relay`]: Phase F — three nodes A (relay) B (coordinator) C
//!   (verifier+proposer); BOTH of B's speculative sessions dial A, which
//!   forwards every length-prefixed frame verbatim to C (B→A→C and
//!   C→A→B). The end-to-end handshake/commit signatures still authenticate
//!   B↔C through the relay; the session must stay token-exact vs plain
//!   single decoding (F13; the local stand-in for libp2p Circuit-Relay-v2).
//!
//! [`pair`]: run_pair
//! [`mesh`]: run_mesh
//! [`kill`]: run_kill
//! [`spec`]: run_spec
//! [`spec_multi`]: run_spec_multi
//! [`relay`]: run_relay

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use modelswarm_identity::InstallationIdentity;
use modelswarm_runtime::{
    Handle, InferenceRuntime, KvCommitment, MockRuntime, RuntimeDescriptor, RuntimeError,
    RuntimeMetrics, SamplingParams, TaskId,
};
use modelswarm_session::spec::{
    serve_proposer, serve_proposer_multi, serve_speculative, speculate, speculate_multi,
    FallbackPolicy, ProposerRuntimeFactory, SpecMode, SpecOutcome,
};
use modelswarm_speculation::TrieLimits;
use modelswarm_transport::ed25519_dalek::VerifyingKey;
use modelswarm_transport::frame::{read_frame_raw, write_frame_raw};
use modelswarm_transport::{
    ChatMessage, Completed, Handshake, InferenceRequest, Listener, Sampling, Session,
    SignedFrameTransport, Stats, TokenDelta, TransportError, Usage, WireMessage,
};
use serde_json::{json, Value};

/// Canned profile id used by all sim scenarios.
pub const SIM_PROFILE: &str = "msp:sim-mock:v1";
const DEADLINE: Duration = Duration::from_secs(10);

/// Deterministic identity from a fixed seed.
pub fn sim_identity(seed: u8) -> InstallationIdentity {
    InstallationIdentity::from_bytes(&[seed; 32])
}

fn handshake_for(identity: &InstallationIdentity) -> Handshake {
    Handshake::build(
        identity,
        SIM_PROFILE,
        "modelswarm-sim",
        env!("CARGO_PKG_VERSION"),
    )
}

fn sample_request(request_id: &str) -> InferenceRequest {
    InferenceRequest {
        request_id: request_id.to_string(),
        profile_id: SIM_PROFILE.to_string(),
        capability_token: "sim.capability.token".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "Say hello".to_string(),
        }],
        sampling: Sampling {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            seed: Some(42),
        },
        max_tokens: 32,
        deadline_ms: 10_000,
        stream: true,
    }
}

// ---------------------------------------------------------------------------
// pair
// ---------------------------------------------------------------------------

/// Serving side of `pair`/`kill`: accept with signature verification, then
/// answer every request with the canned stream. Stays alive until the client
/// goes away or `stop` fires (kill scenario).
async fn serve_canned(
    listener: Listener,
    expected: VerifyingKey,
    stop_after: Option<Duration>,
) -> Result<()> {
    let mut session = listener
        .accept(&expected, DEADLINE)
        .await
        .context("responder handshake")?;
    if let Some(delay) = stop_after {
        // kill scenario: one delta, then die abruptly at the deadline
        // (drop everything without Completed — an injected peer crash).
        let req = loop {
            match session.recv(DEADLINE).await? {
                WireMessage::InferenceRequest(req) => break req,
                _ => continue,
            }
        };
        session
            .send(
                WireMessage::TokenDelta(TokenDelta {
                    request_id: req.request_id.clone(),
                    delta: "Hel".to_string(),
                    index: 0,
                }),
                DEADLINE,
            )
            .await?;
        tokio::time::sleep(delay).await;
        session.close();
        drop(session);
        return Ok(());
    }
    let mut served = false;
    loop {
        let msg = match session.recv(DEADLINE).await {
            Ok(msg) => msg,
            Err(_) if served => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        let WireMessage::InferenceRequest(req) = msg else {
            continue;
        };
        for (index, delta) in ["Hel", "lo"].iter().enumerate() {
            session
                .send(
                    WireMessage::TokenDelta(TokenDelta {
                        request_id: req.request_id.clone(),
                        delta: (*delta).to_string(),
                        index: index as u32,
                    }),
                    DEADLINE,
                )
                .await?;
        }
        session
            .send(
                WireMessage::Usage(Usage {
                    prompt_tokens: 12,
                    completion_tokens: 2,
                    prefill_ms: 3,
                    decode_ms: 4,
                }),
                DEADLINE,
            )
            .await?;
        session
            .send(
                WireMessage::Completed(Completed {
                    request_id: req.request_id.clone(),
                    finish_reason: "stop".to_string(),
                }),
                DEADLINE,
            )
            .await?;
        served = true;
    }
}

/// `pair`: one verified handshake + one inference round-trip + RTT probes.
/// Returns the JSON summary object (printed by the caller).
pub async fn run_pair(rtt_samples: usize) -> Result<Value> {
    let client = sim_identity(101);
    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("pair listener")?;
    let addr = listener
        .local_addr()
        .context("pair local addr")?
        .to_string();
    let responder = tokio::spawn(serve_canned(listener, client.verifying_key(), None));

    let started = Instant::now();
    let mut session = SignedFrameTransport::connect(&addr, handshake_for(&client), DEADLINE)
        .await
        .context("pair connect+handshake")?;
    session
        .send(
            WireMessage::InferenceRequest(sample_request("sim-pair-0001")),
            DEADLINE,
        )
        .await
        .context("pair request send")?;

    let mut text = String::new();
    let mut deltas = 0u32;
    let mut usage: Option<Usage> = None;
    let mut finish_reason = String::new();
    while let Ok(msg) = session.recv(DEADLINE).await {
        match msg {
            WireMessage::TokenDelta(d) => {
                text.push_str(&d.delta);
                deltas += 1;
            }
            WireMessage::Usage(u) => usage = Some(u),
            WireMessage::Completed(c) => {
                finish_reason = c.finish_reason;
                break;
            }
            WireMessage::StreamError(e) => anyhow::bail!("stream error: {} {}", e.code, e.message),
            other => anyhow::bail!("unexpected message: {other:?}"),
        }
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let stats: Stats = session
        .measure_rtt(rtt_samples.max(1), DEADLINE)
        .await
        .context("pair rtt probe")?;
    session.close();
    responder
        .await
        .context("responder task")?
        .context("responder flow")?;

    Ok(json!({
        "scenario": "pair",
        "ok": true,
        "client_peer_id": client.peer_id(),
        "request_id": "sim-pair-0001",
        "profile_id": SIM_PROFILE,
        "text": text,
        "token_deltas": deltas,
        "finish_reason": finish_reason,
        "prompt_tokens": usage.map(|u| u.prompt_tokens),
        "elapsed_ms": round2(elapsed_ms),
        "rtt_p50_ms": round2(stats.p50_ms),
        "rtt_p95_ms": round2(stats.p95_ms),
        "jitter_ms": round2(stats.jitter_ms),
    }))
}

// ---------------------------------------------------------------------------
// mesh
// ---------------------------------------------------------------------------

/// `mesh`: `n` peers (2..=16) on distinct loopback ports; every unordered
/// pair handshakes (direction i→j); prints adjacency + RTT matrices. The RTT
/// for pair (i,j) is measured on that single session and mirrored into both
/// matrix cells.
pub async fn run_mesh(n: usize, rtt_samples: usize) -> Result<Value> {
    anyhow::ensure!((2..=16).contains(&n), "mesh needs 2..=16 peers, got {n}");
    let identities: Vec<InstallationIdentity> = (0..n).map(|i| sim_identity(i as u8 + 1)).collect();
    let peer_ids: Vec<String> = identities.iter().map(|p| p.peer_id()).collect();

    let mut listeners = Vec::with_capacity(n);
    let mut addrs = Vec::with_capacity(n);
    for _ in &identities {
        let listener = SignedFrameTransport::listen("127.0.0.1:0")
            .await
            .context("mesh listener")?;
        addrs.push(
            listener
                .local_addr()
                .context("mesh local addr")?
                .to_string(),
        );
        listeners.push(Some(listener));
    }

    // Trusted peer directory (the tracker's job in production): peer id ->
    // verifying key, shared by every accepting side.
    let directory: HashMap<String, VerifyingKey> = identities
        .iter()
        .map(|p| (p.peer_id(), p.verifying_key()))
        .collect();

    // Peer j accepts exactly j incoming connections (from peers i < j) and
    // holds every session open until the whole mesh finished dialing — the
    // transport answers RTT pings itself, so the sessions must outlive the
    // dialing loop, and dropping them early would EOF the peer mid-probe.
    let (release_tx, release_rx) = tokio::sync::watch::channel(false);
    let mut acceptors = Vec::with_capacity(n);
    for (j, listener_slot) in listeners.into_iter().enumerate() {
        let listener = listener_slot.expect("each peer has a listener");
        let directory = directory.clone();
        let mut release = release_rx.clone();
        acceptors.push(tokio::spawn(async move {
            let mut sessions = Vec::new();
            for _ in 0..j {
                sessions.push(
                    listener
                        .accept_with(|hs| directory.get(&hs.peer_id).copied(), DEADLINE)
                        .await?,
                );
            }
            // Hold the sessions until the dialing phase is complete.
            let _ = release.changed().await;
            Ok::<(), TransportError>(())
        }));
    }

    let mut adjacency = vec![vec![0u8; n]; n];
    let mut rtt = vec![vec![0.0f64; n]; n];
    let mut jitter = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let dialer = identities[i].clone();
            let mut session =
                SignedFrameTransport::connect(&addrs[j], handshake_for(&dialer), DEADLINE)
                    .await
                    .with_context(|| format!("mesh connect {i}->{j}"))?;
            let stats = session
                .measure_rtt(rtt_samples.max(1), DEADLINE)
                .await
                .with_context(|| format!("mesh rtt {i}->{j}"))?;
            adjacency[i][j] = 1;
            adjacency[j][i] = 1;
            rtt[i][j] = stats.p50_ms;
            rtt[j][i] = stats.p50_ms; // same session, measured from i
            jitter[i][j] = stats.jitter_ms;
            jitter[j][i] = stats.jitter_ms;
            session.close();
        }
    }
    let _ = release_tx.send(true);
    for acceptor in acceptors {
        acceptor
            .await
            .context("mesh acceptor task")?
            .context("mesh accept flow")?;
    }

    Ok(json!({
        "scenario": "mesh",
        "ok": true,
        "peers": n,
        "peer_ids": peer_ids,
        "adjacency": adjacency,
        "rtt_p50_ms": rtt.iter().map(|row| row.iter().map(|v| round2(*v)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "jitter_ms": jitter.iter().map(|row| row.iter().map(|v| round2(*v)).collect::<Vec<_>>()).collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------------------
// kill
// ---------------------------------------------------------------------------

/// `kill`: a pair where the responder dies (abrupt session drop) after
/// `at_ms`. The client must observe an explicit failure — `closed` (peer
/// EOF) or `timeout` (deadline, capped at at_ms + 2 s) — and the scenario
/// returns a structured failure event, never a hang.
pub async fn run_kill(at_ms: u64) -> Result<Value> {
    let at = Duration::from_millis(at_ms.max(1));
    let client = sim_identity(201);
    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("kill listener")?;
    let addr = listener
        .local_addr()
        .context("kill local addr")?
        .to_string();
    let responder = tokio::spawn(serve_canned(listener, client.verifying_key(), Some(at)));

    let started = Instant::now();
    let connect_result =
        SignedFrameTransport::connect(&addr, handshake_for(&client), DEADLINE).await;
    let mut session: Session = match connect_result {
        Ok(session) => session,
        // Responder died before the exchange completed: still an explicit,
        // structured failure — not a hang.
        Err(err) => {
            return Ok(kill_event(
                "connect_failed",
                &err.to_string(),
                err.msp_code().map(str::to_string),
                at_ms,
                started,
                0,
            ));
        }
    };
    session
        .send(
            WireMessage::InferenceRequest(sample_request("sim-kill-0001")),
            DEADLINE,
        )
        .await
        .context("kill request send")?;

    // First delta arrives; the responder then dies at `at`. Bound the wait
    // strictly (at + 2 s) so the scenario is hang-free by construction.
    let recv_deadline = at + Duration::from_secs(2);
    let mut deltas = 0u32;
    let failure: TransportError = loop {
        match session.recv(recv_deadline).await {
            Ok(WireMessage::TokenDelta(_)) => deltas += 1,
            Ok(_) => continue,
            Err(err) => break err,
        }
    };
    let observed = match &failure {
        TransportError::Timeout => "timeout",
        _ => "closed",
    };
    let code = failure.msp_code().map(str::to_string);
    let event = kill_event(observed, &failure.to_string(), code, at_ms, started, deltas);
    // The responder already died (that is the point); reap it so no task
    // outlives the scenario.
    let _ = responder.await;
    Ok(event)
}

fn kill_event(
    observed: &str,
    detail: &str,
    code: Option<String>,
    at_ms: u64,
    started: Instant,
    deltas: u32,
) -> Value {
    json!({
        "scenario": "kill",
        "ok": false,
        "event": "peer_failure",
        "observed": observed,
        "msp_code": code,
        "detail": detail,
        "died_after_ms": at_ms,
        "token_deltas_received": deltas,
        "elapsed_ms": round2(started.elapsed().as_secs_f64() * 1_000.0),
        "hang": false,
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------
// spec (Phase D)
// ---------------------------------------------------------------------------

/// Output length of the `spec` scenario.
pub const SPEC_TOKENS: u32 = 64;

/// The measured facts of one `spec` run (kept as a struct so the test matrix
/// asserts on data, not on the printed JSON line).
#[derive(Debug, Clone)]
pub struct SpecCase {
    pub prompt_seed: u64,
    pub window: u32,
    pub draft_accuracy: f32,
    pub tokens_equal_to_single: bool,
    pub rounds: u64,
    pub mean_acceptance_length: f32,
    pub acceptance_rate: f32,
    pub fallback: Option<&'static str>,
    pub receipts_verified: bool,
    pub verifier_reports: usize,
    pub proposer_reports: usize,
    pub verifier_addr: String,
    pub proposer_addr: String,
    pub elapsed_ms: f64,
}

/// Runs one `spec` case: in-process proposer + verifier servers (MockRuntime,
/// `draft_accuracy` knob) + coordinator over real loopback TCP, 64 tokens;
/// then plain single decoding of the same prompt against the verifier
/// runtime for the greedy-equality verdict (D1 contract).
async fn spec_case(prompt_seed: u64, window: u32, draft_accuracy: f32) -> Result<SpecCase> {
    let coordinator = sim_identity(150);
    let verifier_id = sim_identity(151);
    let proposer_id = sim_identity(152);
    let prompt: Vec<u32> = (0..8u64)
        .map(|k| (prompt_seed.wrapping_mul(k + 3) % 256) as u32)
        .collect();

    let verifier_runtime = Arc::new(MockRuntime::new(prompt_seed, draft_accuracy));
    let proposer_runtime = Arc::new(MockRuntime::new(prompt_seed, draft_accuracy));
    let client_runtime: Arc<dyn InferenceRuntime> =
        Arc::new(MockRuntime::new(prompt_seed, draft_accuracy));

    let verifier_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("spec verifier listener")?;
    let verifier_addr = verifier_listener
        .local_addr()
        .context("spec verifier addr")?
        .to_string();
    let verifier_key = coordinator.verifying_key();
    let verifier_task = tokio::spawn(serve_speculative(
        verifier_listener,
        verifier_runtime.clone(),
        SIM_PROFILE,
        verifier_id.clone(),
        verifier_key,
        1,
    ));

    let proposer_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("spec proposer listener")?;
    let proposer_addr = proposer_listener
        .local_addr()
        .context("spec proposer addr")?
        .to_string();
    let proposer_task = tokio::spawn(serve_proposer(
        proposer_listener,
        proposer_runtime,
        SIM_PROFILE,
        proposer_id.clone(),
        verifier_key,
        1,
    ));

    let started = Instant::now();
    let outcome: SpecOutcome = speculate(
        &verifier_addr,
        &proposer_addr,
        client_runtime,
        SIM_PROFILE,
        &prompt,
        window.max(1),
        SPEC_TOKENS,
        &coordinator,
        &verifier_id.verifying_key(),
        FallbackPolicy::default(),
    )
    .await
    .context("spec speculate")?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    // Plain single decode of the same prompt on the verifier's own runtime.
    let handle = verifier_runtime.load(SIM_PROFILE).await?;
    let single = verifier_runtime
        .decode_stream(
            &handle,
            &prompt,
            &SamplingParams::default(),
            SPEC_TOKENS,
            DEADLINE,
        )
        .await
        .context("spec single decode")?;

    let verifier_reports = verifier_task
        .await
        .context("verifier task")?
        .context("verifier served")?
        .len();
    let proposer_reports = proposer_task
        .await
        .context("proposer task")?
        .context("proposer served")?
        .len();

    Ok(SpecCase {
        prompt_seed,
        window,
        draft_accuracy,
        tokens_equal_to_single: outcome.tokens == single,
        rounds: outcome.rounds,
        mean_acceptance_length: outcome.acceptance.mean_acceptance_length(),
        acceptance_rate: outcome.acceptance.acceptance_rate(),
        fallback: outcome.fallback.map(|reason| reason.as_str()),
        receipts_verified: outcome.receipts_verified,
        verifier_reports,
        proposer_reports,
        verifier_addr,
        proposer_addr,
        elapsed_ms,
    })
}

/// `spec <prompt_seed> [window] [draft_accuracy]`: one speculative session vs
/// plain single decoding; prints the equality verdict + acceptance telemetry.
pub async fn run_spec(prompt_seed: u64, window: u32, draft_accuracy: f32) -> Result<Value> {
    let case = spec_case(prompt_seed, window, draft_accuracy).await?;
    Ok(json!({
        "scenario": "spec",
        "ok": true,
        "prompt_seed": case.prompt_seed,
        "window": case.window,
        "draft_accuracy": case.draft_accuracy,
        "tokens": SPEC_TOKENS,
        "tokens_equal_to_single": case.tokens_equal_to_single,
        "rounds": case.rounds,
        "mean_acceptance_length": case.mean_acceptance_length,
        "acceptance_rate": case.acceptance_rate,
        "fallback": case.fallback,
        "receipts_verified": case.receipts_verified,
        "verifier_reports": case.verifier_reports,
        "proposer_reports": case.proposer_reports,
        "elapsed_ms": round2(case.elapsed_ms),
        "verifier_addr": case.verifier_addr,
        "proposer_addr": case.proposer_addr,
    }))
}

// ---------------------------------------------------------------------------
// spec_multi (Phase E)
// ---------------------------------------------------------------------------

/// Default proposer count of the `spec_multi` scenario.
pub const SPEC_MULTI_DEFAULT_PROPOSERS: usize = 4;
/// Proposal window of the `spec_multi` scenario.
pub const SPEC_MULTI_WINDOW: u32 = 4;
/// Coordinator proposal-collection deadline used by `spec_multi`: a proposer
/// whose block misses it is a straggler (E5) and the round proceeds without
/// it.
pub const SPEC_MULTI_COLLECT: Duration = Duration::from_millis(250);
/// Delay injected into a chosen proposer's propose call (the straggler test
/// hook; env `MODELSWARM_SIM_STRAGGLER_MS` overrides via `run_spec_multi`).
pub const SPEC_MULTI_STRAGGLER_DELAY: Duration = Duration::from_millis(600);
/// Env variable that injects a straggler into `spec_multi` (millisecond
/// delay on proposer 0's first proposal; unset/0 = healthy roster).
pub const SPEC_MULTI_STRAGGLER_ENV: &str = "MODELSWARM_SIM_STRAGGLER_MS";

/// The injected straggler of a `spec_multi` case: `proposer`'s `call`-th
/// propose (1-based) sleeps for `delay` before drafting.
#[derive(Debug, Clone, Copy)]
pub struct StragglerSpec {
    pub proposer: usize,
    pub call: usize,
    pub delay: Duration,
}

/// TEST-ONLY wrapper delaying one `propose` call of the wrapped runtime —
/// the sim's straggler injection point (honest: the delay is at the runtime
/// boundary, not faked inside the protocol layer).
struct DelayedRuntime {
    inner: Arc<dyn InferenceRuntime>,
    call: usize,
    delay: Duration,
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl InferenceRuntime for DelayedRuntime {
    fn id(&self) -> RuntimeDescriptor {
        self.inner.id()
    }
    async fn load(&self, profile_id: &str) -> Result<Handle, RuntimeError> {
        self.inner.load(profile_id).await
    }
    async fn tokenize(&self, text: &str) -> Result<Vec<u32>, RuntimeError> {
        self.inner.tokenize(text).await
    }
    async fn detokenize(&self, ids: &[u32]) -> Result<String, RuntimeError> {
        self.inner.detokenize(ids).await
    }
    async fn prefill(&self, handle: &Handle, ids: &[u32]) -> Result<KvCommitment, RuntimeError> {
        self.inner.prefill(handle, ids).await
    }
    async fn decode_step(
        &self,
        handle: &Handle,
        prefix: &[u32],
        sampling: &SamplingParams,
    ) -> Result<u32, RuntimeError> {
        self.inner.decode_step(handle, prefix, sampling).await
    }
    fn metrics(&self) -> RuntimeMetrics {
        self.inner.metrics()
    }
    fn cancel(&self, task: TaskId) -> Result<(), RuntimeError> {
        self.inner.cancel(task)
    }
    async fn propose(
        &self,
        handle: &Handle,
        prefix: &[u32],
        window: u32,
    ) -> Result<Vec<u32>, RuntimeError> {
        let call = {
            let mut calls = self.calls.lock().expect("delay counter");
            *calls += 1;
            *calls
        };
        if call == self.call {
            tokio::time::sleep(self.delay).await;
        }
        self.inner.propose(handle, prefix, window).await
    }
}

/// The measured facts of one `spec_multi` run.
#[derive(Debug, Clone)]
pub struct SpecMultiCase {
    pub prompt_seed: u64,
    pub proposers: usize,
    pub draft_accuracy: f32,
    /// The injected straggler, if any (E5 exercise hook).
    pub straggler: Option<StragglerSpec>,
    pub tokens_equal_to_single: bool,
    /// The committed token stream (determinism checks compare it directly).
    pub tokens: Vec<u32>,
    pub rounds: u64,
    pub mean_acceptance_length: f32,
    pub duplicate_work: u64,
    pub pruned: u64,
    pub stragglers_total: u64,
    pub fallback: Option<&'static str>,
    pub proposer_reports_k: usize,
    pub elapsed_ms: f64,
}

/// Runs one `spec_multi` case: verifier + N seeded proposer servers
/// (MockRuntime drafted from the per-round branch seed) + coordinator over
/// real loopback TCP, 64 tokens; then plain single decoding of the same
/// prompt against the verifier runtime for the greedy-equality verdict.
async fn spec_multi_case(
    prompt_seed: u64,
    proposers: usize,
    draft_accuracy: f32,
    straggler: Option<StragglerSpec>,
) -> Result<SpecMultiCase> {
    anyhow::ensure!(
        (2..=7).contains(&proposers),
        "spec_multi needs 2..=7 proposers, got {proposers}"
    );
    let coordinator = sim_identity(160);
    let verifier_id = sim_identity(161);
    let prompt: Vec<u32> = (0..8u64)
        .map(|k| (prompt_seed.wrapping_mul(k + 3) % 256) as u32)
        .collect();

    let verifier_runtime = Arc::new(MockRuntime::new(prompt_seed, draft_accuracy));
    let client_runtime: Arc<dyn InferenceRuntime> =
        Arc::new(MockRuntime::new(prompt_seed, draft_accuracy));

    let verifier_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("spec_multi verifier listener")?;
    let verifier_addr = verifier_listener
        .local_addr()
        .context("spec_multi verifier addr")?
        .to_string();
    let verifier_key = coordinator.verifying_key();
    let verifier_task = tokio::spawn(serve_speculative(
        verifier_listener,
        verifier_runtime.clone(),
        SIM_PROFILE,
        verifier_id.clone(),
        verifier_key,
        1,
    ));

    let mut proposer_addrs = Vec::with_capacity(proposers);
    let mut proposer_tasks = Vec::with_capacity(proposers);
    for index in 0..proposers {
        let listener = SignedFrameTransport::listen("127.0.0.1:0")
            .await
            .context("spec_multi proposer listener")?;
        let addr = listener
            .local_addr()
            .context("spec_multi proposer addr")?
            .to_string();
        let delay = straggler.filter(|spec| spec.proposer == index);
        let calls = Arc::new(Mutex::new(0usize));
        let factory: ProposerRuntimeFactory = Arc::new(move |seed| {
            let inner: Arc<dyn InferenceRuntime> = Arc::new(MockRuntime::new(seed, draft_accuracy));
            match delay {
                Some(spec) => Arc::new(DelayedRuntime {
                    inner,
                    call: spec.call,
                    delay: spec.delay,
                    calls: Arc::clone(&calls),
                }),
                None => inner,
            }
        });
        let identity = sim_identity(162 + index as u8);
        let task = tokio::spawn(async move {
            serve_proposer_multi(listener, factory, SIM_PROFILE, identity, verifier_key, 1).await
        });
        proposer_addrs.push(addr);
        proposer_tasks.push(task);
    }

    let started = Instant::now();
    let outcome: SpecOutcome = speculate_multi(
        &verifier_addr,
        &proposer_addrs,
        client_runtime,
        SIM_PROFILE,
        &prompt,
        SPEC_MULTI_WINDOW,
        SPEC_TOKENS,
        &coordinator,
        &verifier_id.verifying_key(),
        FallbackPolicy::default(),
        TrieLimits::default(),
        SPEC_MULTI_COLLECT,
    )
    .await
    .context("spec_multi speculate_multi")?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    // Plain single decode of the same prompt on the verifier's own runtime.
    let handle = verifier_runtime.load(SIM_PROFILE).await?;
    let single = verifier_runtime
        .decode_stream(
            &handle,
            &prompt,
            &SamplingParams::default(),
            SPEC_TOKENS,
            DEADLINE,
        )
        .await
        .context("spec_multi single decode")?;

    verifier_task
        .await
        .context("verifier task")?
        .context("verifier served")?;
    let mut proposer_reports_k = 0usize;
    for task in proposer_tasks {
        proposer_reports_k += task
            .await
            .context("proposer task")?
            .context("proposer served")?
            .len();
    }

    debug_assert_eq!(outcome.mode, SpecMode::MultiProposer);
    Ok(SpecMultiCase {
        prompt_seed,
        proposers,
        draft_accuracy,
        straggler,
        tokens_equal_to_single: outcome.tokens == single,
        tokens: outcome.tokens,
        rounds: outcome.rounds,
        mean_acceptance_length: outcome.acceptance.mean_acceptance_length(),
        duplicate_work: outcome.duplicate_work,
        pruned: outcome.pruned,
        stragglers_total: outcome.stragglers,
        fallback: outcome.fallback.map(|reason| reason.as_str()),
        proposer_reports_k,
        elapsed_ms,
    })
}

/// `spec_multi <prompt_seed> [proposers] [draft_accuracy]`: one Phase E
/// multi-proposer candidate-tree session vs plain single decoding; prints
/// the equality verdict plus trie telemetry. Setting
/// `MODELSWARM_SIM_STRAGGLER_MS=<ms>` injects a straggler (proposer 0's
/// first proposal delayed by `<ms>`), demonstrating the E5 round completing
/// without it.
pub async fn run_spec_multi(
    prompt_seed: u64,
    proposers: usize,
    draft_accuracy: f32,
) -> Result<Value> {
    let straggler = match std::env::var(SPEC_MULTI_STRAGGLER_ENV) {
        Ok(raw) => {
            let ms: u64 = raw
                .parse()
                .with_context(|| format!("{SPEC_MULTI_STRAGGLER_ENV} must be milliseconds"))?;
            (ms > 0).then_some(StragglerSpec {
                proposer: 0,
                call: 1,
                delay: Duration::from_millis(ms),
            })
        }
        Err(_) => None,
    };
    let case = spec_multi_case(prompt_seed, proposers, draft_accuracy, straggler).await?;
    Ok(json!({
        "scenario": "spec_multi",
        "ok": true,
        "prompt_seed": case.prompt_seed,
        "proposers": case.proposers,
        "draft_accuracy": case.draft_accuracy,
        "straggler_injected": case.straggler.is_some(),
        "tokens": SPEC_TOKENS,
        "tokens_equal_to_single": case.tokens_equal_to_single,
        "rounds": case.rounds,
        "mean_acceptance_length": case.mean_acceptance_length,
        "duplicate_work": case.duplicate_work,
        "pruned": case.pruned,
        "stragglers_total": case.stragglers_total,
        "fallback": case.fallback,
        "proposer_reports_k": case.proposer_reports_k,
        "elapsed_ms": round2(case.elapsed_ms),
    }))
}

// ---------------------------------------------------------------------------
// relay (Phase F, F13)
// ---------------------------------------------------------------------------

/// Output length of the `relay` scenario (shorter than `spec`: the point is
/// the path, not the volume).
pub const RELAY_TOKENS: u32 = 24;
/// How long the relay node waits for the next inbound relayed connection.
pub const RELAY_ACCEPT_DEADLINE: Duration = Duration::from_secs(10);

/// Node A of the `relay` scenario: accepts RAW (unterminated-handshake)
/// connections on a loopback-only listener and pumps every length-prefixed
/// frame verbatim to `backend` — one TCP session per relayed connection,
/// both directions forwarded. This is the local stand-in for libp2p
/// Circuit-Relay-v2 (ADR-014): the frame layer is opaque to the relay, and
/// the terminal peers' end-to-end Ed25519 handshake + session-layer commit
/// signatures are what authenticate the traffic (the relay cannot forge or
/// silently alter either).
async fn serve_relay(listener: Listener, backend: String) -> Result<u64> {
    let mut forwarded_frames = 0u64;
    loop {
        // Two relayed connections are expected (verifier + proposer session);
        // the third accept timing out ends the relay cleanly.
        let inbound = match listener.accept_raw(RELAY_ACCEPT_DEADLINE).await {
            Ok(stream) => stream,
            Err(_) => return Ok(forwarded_frames),
        };
        let upstream = tokio::net::TcpStream::connect(&backend)
            .await
            .context("relay dial backend")?;
        let (mut client_read, mut client_write) = inbound.into_split();
        let (mut up_read, mut up_write) = upstream.into_split();
        let up_task = tokio::spawn(async move {
            // C -> A -> B (server answers travel back through the relay).
            // Owned halves moved in: the pump lives until either side EOFs.
            while let Ok(bytes) = read_frame_raw(&mut up_read).await {
                if write_frame_raw(&mut client_write, &bytes).await.is_err() {
                    break;
                }
            }
        });
        // B -> A -> C (client frames forwarded verbatim).
        while let Ok(bytes) = read_frame_raw(&mut client_read).await {
            if write_frame_raw(&mut up_write, &bytes).await.is_err() {
                break;
            }
            forwarded_frames += 1;
        }
        let _ = up_task.await;
    }
}

/// The measured facts of one `relay` run.
#[derive(Debug, Clone)]
pub struct RelayCase {
    pub prompt_seed: u64,
    pub tokens_equal_to_single: bool,
    pub rounds: u64,
    pub fallback: Option<&'static str>,
    pub receipts_verified: bool,
    pub relayed_frames: u64,
    pub elapsed_ms: f64,
}

/// `relay <prompt_seed>`: node B runs the full Phase-D speculative flow, but
/// BOTH dial addresses point at node A (relay), which forwards frames to
/// node C's verifier/proposer listeners. The committed output must still be
/// token-identical to plain single decoding of the same prompt (the relay is
/// transport-only; correctness is end-to-end).
async fn relay_case(prompt_seed: u64) -> Result<RelayCase> {
    // Node C: verifier + proposer on distinct loopback listeners.
    let coordinator = sim_identity(170);
    let verifier_id = sim_identity(171);
    let proposer_id = sim_identity(172);
    let prompt: Vec<u32> = (0..8u64)
        .map(|k| (prompt_seed.wrapping_mul(k + 3) % 256) as u32)
        .collect();
    let verifier_runtime = Arc::new(MockRuntime::new(prompt_seed, 0.7));
    let proposer_runtime = Arc::new(MockRuntime::new(prompt_seed, 0.7));
    let client_runtime: Arc<dyn InferenceRuntime> = Arc::new(MockRuntime::new(prompt_seed, 0.7));

    let verifier_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("relay verifier listener")?;
    let verifier_addr = verifier_listener
        .local_addr()
        .context("relay verifier addr")?
        .to_string();
    let verifier_key = coordinator.verifying_key();
    let verifier_task = tokio::spawn(serve_speculative(
        verifier_listener,
        verifier_runtime.clone(),
        SIM_PROFILE,
        verifier_id.clone(),
        verifier_key,
        1,
    ));

    let proposer_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("relay proposer listener")?;
    let proposer_addr = proposer_listener
        .local_addr()
        .context("relay proposer addr")?
        .to_string();
    let proposer_task = tokio::spawn(serve_proposer(
        proposer_listener,
        proposer_runtime,
        SIM_PROFILE,
        proposer_id.clone(),
        verifier_key,
        1,
    ));

    // Node A: one relay listener per backend service. B dials these two
    // addresses instead of C's real ones.
    let relay_v_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("relay-v listener")?;
    let relay_v_addr = relay_v_listener
        .local_addr()
        .context("relay-v addr")?
        .to_string();
    let relay_v_task = tokio::spawn(serve_relay(relay_v_listener, verifier_addr));
    let relay_p_listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("relay-p listener")?;
    let relay_p_addr = relay_p_listener
        .local_addr()
        .context("relay-p addr")?
        .to_string();
    let relay_p_task = tokio::spawn(serve_relay(relay_p_listener, proposer_addr));

    // Node B: the coordinator, dialing A (relay) for both peers.
    let started = Instant::now();
    let outcome: SpecOutcome = speculate(
        &relay_v_addr,
        &relay_p_addr,
        client_runtime,
        SIM_PROFILE,
        &prompt,
        4,
        RELAY_TOKENS,
        &coordinator,
        &verifier_id.verifying_key(),
        FallbackPolicy::default(),
    )
    .await
    .context("relay speculate")?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    // Plain single decode of the same prompt on the verifier's runtime.
    let handle = verifier_runtime.load(SIM_PROFILE).await?;
    let single = verifier_runtime
        .decode_stream(
            &handle,
            &prompt,
            &SamplingParams::default(),
            RELAY_TOKENS,
            DEADLINE,
        )
        .await
        .context("relay single decode")?;

    let verifier_reports = verifier_task
        .await
        .context("verifier task")?
        .context("verifier served")?
        .len();
    let proposer_reports = proposer_task
        .await
        .context("proposer task")?
        .context("proposer served")?
        .len();
    anyhow::ensure!(verifier_reports == 1 && proposer_reports == 1);
    // The relays see the connections close; accept the next inbound until
    // the deadline passes and return the forwarded-frame counts.
    let relayed_v = relay_v_task
        .await
        .context("relay-v task")?
        .context("relay-v flow")?;
    let relayed_p = relay_p_task
        .await
        .context("relay-p task")?
        .context("relay-p flow")?;

    Ok(RelayCase {
        prompt_seed,
        tokens_equal_to_single: outcome.tokens == single,
        rounds: outcome.rounds,
        fallback: outcome.fallback.map(|reason| reason.as_str()),
        receipts_verified: outcome.receipts_verified,
        relayed_frames: relayed_v + relayed_p,
        elapsed_ms,
    })
}

/// `relay <prompt_seed>`: three nodes (A relay, B coordinator, C
/// verifier+proposer); B's whole speculative session is routed B→A→C.
/// Reports the exactness verdict, the round count, and how many frames the
/// relay node forwarded (the observable relay-path cost).
pub async fn run_relay(prompt_seed: u64) -> Result<Value> {
    let case = relay_case(prompt_seed).await?;
    Ok(json!({
        "scenario": "relay",
        "ok": true,
        "prompt_seed": case.prompt_seed,
        "tokens": RELAY_TOKENS,
        "tokens_equal_to_single": case.tokens_equal_to_single,
        "rounds": case.rounds,
        "fallback": case.fallback,
        "receipts_verified": case.receipts_verified,
        "relayed_frames": case.relayed_frames,
        "topology": "B(coordinator)->A(relay)->C(verifier+proposer), 2 TCP sessions",
        "elapsed_ms": round2(case.elapsed_ms),
        "note": "local Circuit-Relay stand-in; libp2p Circuit-Relay-v2 replaces this at the backend swap (ADR-014/018)",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_identities_are_deterministic() {
        assert_eq!(sim_identity(1).peer_id(), sim_identity(1).peer_id());
        assert_ne!(sim_identity(1).peer_id(), sim_identity(2).peer_id());
        // F12: the wire identity is the ADR-020 multihash derivation.
        assert!(sim_identity(1).peer_id().starts_with("12D3Koo"));
        assert_eq!(sim_identity(1).peer_id().len(), 52);
        assert_ne!(sim_identity(1).peer_id(), sim_identity(1).installation_id());
    }

    /// F13: the relayed path (B→A→C, both of B's sessions forwarded by node
    /// A) completes the full speculative flow with output token-exact
    /// against plain single decoding — the relay is transport-only, and both
    /// peers' end-to-end authentication survives it. Bounded by a 30 s
    /// timeout; the relay must have forwarded real traffic (> 0 frames).
    #[tokio::test]
    async fn relay_path_is_end_to_end_exact() {
        const CASE_TIMEOUT: Duration = Duration::from_secs(30);
        for seed in [5u64, 21] {
            let case = tokio::time::timeout(CASE_TIMEOUT, relay_case(seed))
                .await
                .expect("relay case is bounded")
                .expect("relay case runs");
            assert!(
                case.tokens_equal_to_single,
                "seed={seed}: relayed path must stay token-exact"
            );
            assert!(
                case.receipts_verified,
                "seed={seed}: receipt through the relay"
            );
            assert_eq!(case.fallback, None, "seed={seed}: healthy relayed session");
            assert!(
                case.relayed_frames > 0,
                "seed={seed}: the relay must have forwarded frames"
            );
        }
    }

    /// D1/D5/D6 sweep: 20 seeds × windows {2,4,8} × draft accuracy
    /// {0.0, 0.5, 1.0}. The greedy contract is unconditional — speculative
    /// output equals plain single decoding token for token, INCLUDING the
    /// fallback path (which is the plain decoder). Accuracy 0.0 must trip
    /// the acceptance-collapse fallback. Every case is bounded by a 30 s
    /// timeout: no hangs.
    #[tokio::test]
    async fn spec_matrix_greedy_equality_always_and_fallback_at_zero_accuracy() {
        const CASE_TIMEOUT: Duration = Duration::from_secs(30);
        let mut cases = 0usize;
        for seed in 0..20u64 {
            for window in [2u32, 4, 8] {
                for accuracy in [0.0f32, 0.5, 1.0] {
                    let case =
                        tokio::time::timeout(CASE_TIMEOUT, spec_case(seed, window, accuracy))
                            .await
                            .expect("no hangs: every case is bounded")
                            .expect("case runs");
                    assert!(
                        case.tokens_equal_to_single,
                        "seed={seed} window={window} accuracy={accuracy}: greedy contract"
                    );
                    assert!(case.receipts_verified, "receipt must verify");
                    assert_eq!(case.verifier_reports, 1);
                    assert_eq!(case.proposer_reports, 1);
                    if accuracy == 0.0 {
                        assert_eq!(
                            case.fallback,
                            Some("acceptance_collapse"),
                            "seed={seed} window={window}: accuracy 0 must fall back"
                        );
                    } else {
                        assert_eq!(
                            case.fallback,
                            None,
                            "seed={seed} window={window} accuracy={accuracy}: healthy acceptance must not fall back"
                        );
                    }
                    if accuracy == 1.0 {
                        // Every draft accepted: mean acceptance length is
                        // exactly 1 + window (vLLM convention), modulo the
                        // final budget-capped round.
                        assert!(
                            case.mean_acceptance_length >= window as f32,
                            "seed={seed} window={window}: perfect drafts must accept the whole window (got {})",
                            case.mean_acceptance_length
                        );
                    }
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 180, "20 seeds × 3 windows × 3 accuracies");
    }

    /// Phase E matrix: 15 seeds × proposers {2,4,7} × accuracy {0.3, 0.8}.
    /// The greedy contract is unconditional — multi-proposer tree output
    /// equals plain single decoding token for token on every case. Plus one
    /// straggler case (proposer 0's first proposal delayed past the
    /// collection deadline): the round completes without it, exactness
    /// holds, and every proposer server still closes cleanly. Determinism:
    /// the same seed run twice yields the identical token stream. Every
    /// case is bounded by a 30 s timeout: no hangs.
    #[tokio::test]
    async fn spec_multi_matrix_exact_straggler_and_determinism() {
        const CASE_TIMEOUT: Duration = Duration::from_secs(30);
        let mut cases = 0usize;
        for seed in 0..15u64 {
            for proposers in [2usize, 4, 7] {
                for accuracy in [0.3f32, 0.8] {
                    let case = tokio::time::timeout(
                        CASE_TIMEOUT,
                        spec_multi_case(seed, proposers, accuracy, None),
                    )
                    .await
                    .expect("no hangs: every case is bounded")
                    .expect("case runs");
                    assert!(
                        case.tokens_equal_to_single,
                        "seed={seed} proposers={proposers} accuracy={accuracy}: greedy contract"
                    );
                    assert_eq!(case.proposer_reports_k, proposers);
                    assert_eq!(case.stragglers_total, 0, "healthy roster: no stragglers");
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 90, "15 seeds × 3 proposer counts × 2 accuracies");

        // E5 exercise: one proposer misses the round deadline; the round (and
        // the session) completes from the remaining candidates, still exact.
        let straggler_case = tokio::time::timeout(
            CASE_TIMEOUT,
            spec_multi_case(
                3,
                4,
                0.8,
                Some(StragglerSpec {
                    proposer: 0,
                    call: 1,
                    delay: SPEC_MULTI_STRAGGLER_DELAY,
                }),
            ),
        )
        .await
        .expect("straggler case is bounded")
        .expect("straggler case runs");
        assert!(
            straggler_case.tokens_equal_to_single,
            "straggler case exact"
        );
        assert!(
            straggler_case.stragglers_total >= 1,
            "the delayed proposer must be counted"
        );
        assert_eq!(straggler_case.proposer_reports_k, 4);
        assert_eq!(straggler_case.straggler.map(|s| s.proposer), Some(0));

        // Determinism: same seed → identical token stream. The committed
        // stream equals plain single decoding (a pure function of the prompt
        // + default sampling), so two runs must agree token for token even
        // though their minted session ids (and hence branch seeds) differ.
        let a = tokio::time::timeout(CASE_TIMEOUT, spec_multi_case(7, 4, 0.5, None))
            .await
            .expect("determinism A bounded")
            .expect("determinism A runs");
        let b = tokio::time::timeout(CASE_TIMEOUT, spec_multi_case(7, 4, 0.5, None))
            .await
            .expect("determinism B bounded")
            .expect("determinism B runs");
        assert!(a.tokens_equal_to_single && b.tokens_equal_to_single);
        assert_eq!(a.tokens, b.tokens, "same seeds: same token stream");
        assert_eq!(a.tokens.len(), SPEC_TOKENS as usize);
    }
}
