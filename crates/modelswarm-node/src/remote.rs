//! F1: the requesting side — [`RemoteExecutor`] turns one serving peer
//! into an [`InferenceExecutor`] (docs/reviews/phase-f-research).
//!
//! Each request dials the peer with a FRESH client transport (one live
//! session per client instance is the transport's tested pattern), sends
//! one `InferenceRequest`, and re-emits the peer's
//! `TokenDelta`/`Usage`/`Completed` frames as executor events. Failures
//! map to ADR-007 semantics: a dial/send failure before any token is
//! [`ExecutorError::Retryable`] (the gateway may fail over or fall back
//! to local); a failure after streaming began becomes an
//! `ExecutorEvent::Error` — never a fabricated continuation, never a
//! silent regeneration.
//!
//! Selection (fastest eligible peer, EWMA probing) is deliberately NOT
//! here: this executor serves ONE peer; the choosing/swarm layer stacks
//! on top once live measurements exist (F1.5).
//!
//! F15 measurement plumbing: when a [`PeerMetrics`] handle is attached
//! ( [`RemoteExecutor::with_metrics`] ), every completed request records
//! its requester-side distribution (TTFT / inter-token latency / total)
//! and, after a clean completion, an idle ping/pong RTT probe runs on
//! the pooled session BEFORE it returns to the pool — off the request
//! critical path. Shadow data only: nothing here selects peers.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures_util::stream::{self, Stream};
use modelswarm_gateway::{
    ExecutorError, ExecutorEvent, ExecutorStream, FinishReason, InferenceExecutor,
    NormalizedMessage, NormalizedRequest,
};
use modelswarm_identity::InstallationIdentity;
use modelswarm_transport::libp2p_backend::{Libp2pSession, Libp2pTransport, PeerId};
use modelswarm_transport::message::{
    ChatMessage as WireChat, InferenceRequest as WireRequest, Sampling as WireSampling,
    StreamError as WireStreamError, WireMessage,
};
use modelswarm_transport::observe::{CompletionObservation, CompletionOutcome, UsageDigest};

use crate::measure::PeerMetrics;

const DIAL_DEADLINE: Duration = Duration::from_secs(10);
const IO_DEADLINE: Duration = Duration::from_secs(30);
/// F15: ping/pong probes per post-completion RTT batch.
const RTT_PROBE_SAMPLES: usize = 3;
/// F15: bound for one full probe batch (all samples).
const RTT_PROBE_DEADLINE: Duration = Duration::from_secs(2);

/// One remote serving peer.
#[derive(Debug, Clone)]
pub struct RemotePeer {
    /// Multiaddr, e.g. `/ip4/192.168.1.5/udp/4001/quic-v1`.
    pub addr: String,
    /// The ADR-020 PeerId the QUIC handshake must verify.
    pub peer_id: String,
}

/// Executes requests against a single remote serving peer, REUSING one
/// QUIC session across sequential requests (connection reuse): the
/// per-request handshake (~1 RTT + crypto) is paid once, not per chat
/// message. The pool self-heals — a broken session is dropped and the
/// next request dials fresh.
pub struct RemoteExecutor {
    peer: RemotePeer,
    identity: InstallationIdentity,
    pool: Arc<tokio::sync::Mutex<Option<Libp2pSession>>>,
    /// F15 shadow measurements; `None` keeps the executor measurement-free
    /// (all pre-F15 callers unchanged).
    metrics: Option<Arc<PeerMetrics>>,
    /// The peer's advertised `queueMs` as last known to the requester
    /// (refreshed per roster lookup via
    /// [`RemoteExecutor::set_advertised_queue_ms`]); recorded alongside
    /// measured admission-to-first-token. Advertised values are untrusted
    /// inputs, never measurements.
    advertised_queue_ms: StdMutex<Option<u64>>,
}

impl RemoteExecutor {
    pub fn new(peer: RemotePeer, identity: InstallationIdentity) -> Self {
        Self {
            peer,
            identity,
            pool: Arc::new(tokio::sync::Mutex::new(None)),
            metrics: None,
            advertised_queue_ms: StdMutex::new(None),
        }
    }

    /// Attaches the F15 measurement recorder (shadow data only).
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<PeerMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Refreshes the peer's advertised `queueMs` from the latest roster
    /// (call per lookup; pooled executors outlive single rosters).
    pub fn set_advertised_queue_ms(&self, queue_ms: Option<u64>) {
        *self
            .advertised_queue_ms
            .lock()
            .expect("advertised queue lock") = queue_ms;
    }

    fn record_failure(&self, stage: &'static str) {
        if let Some(metrics) = &self.metrics {
            metrics.record_failure(&self.peer.peer_id, stage);
        }
    }
}

fn wire_request(request: &NormalizedRequest) -> WireRequest {
    WireRequest {
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        capability_token: request
            .capability_token
            .clone()
            .unwrap_or_else(|| "f1-unverified".into()),
        messages: request
            .messages
            .iter()
            .map(|m: &NormalizedMessage| WireChat {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        sampling: WireSampling {
            temperature: f64::from(request.sampling.temperature),
            top_p: f64::from(request.sampling.top_p),
            top_k: request.sampling.top_k,
            seed: request.sampling.seed,
        },
        max_tokens: request.max_tokens,
        deadline_ms: request.deadline_ms,
        stream: true,
    }
}

/// Maps one inbound frame to an executor event (None = end of stream).
fn frame_to_event(frame: WireMessage) -> Option<Result<ExecutorEvent, WireStreamError>> {
    match frame {
        WireMessage::TokenDelta(d) => Some(Ok(ExecutorEvent::TokenDelta {
            delta: d.delta,
            index: d.index,
        })),
        WireMessage::Usage(u) => Some(Ok(ExecutorEvent::Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            prefill_ms: f64::from(u.prefill_ms),
            decode_ms: f64::from(u.decode_ms),
        })),
        WireMessage::Completed(c) => Some(Ok(ExecutorEvent::Completed {
            finish_reason: match c.finish_reason.as_str() {
                "length" => FinishReason::Length,
                "cancelled" => FinishReason::Cancelled,
                _ => FinishReason::Stop,
            },
        })),
        WireMessage::Cancelled(_) => Some(Ok(ExecutorEvent::Completed {
            finish_reason: FinishReason::Cancelled,
        })),
        WireMessage::StreamError(e) => Some(Err(e)),
        // Handshake frames never appear mid-request on this path.
        _ => None,
    }
}

/// Requester-side wall-clock record for one in-flight request (F15).
#[derive(Default)]
struct Timeline {
    first_token_at: Option<Instant>,
    last_token_at: Option<Instant>,
    itl_sum_ms: f64,
    itl_max_ms: f64,
    token_deltas: u32,
    usage: Option<UsageDigest>,
}

impl Timeline {
    fn on_token(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_token_at {
            let ms = now.duration_since(last).as_secs_f64() * 1_000.0;
            self.itl_sum_ms += ms;
            self.itl_max_ms = self.itl_max_ms.max(ms);
        }
        self.first_token_at.get_or_insert(now);
        self.last_token_at = Some(now);
        self.token_deltas += 1;
    }
}

/// Everything the stream needs to finalize one observation.
struct StreamContext {
    peer_id: String,
    profile_id: String,
    advertised_queue_ms: Option<u64>,
    admitted_at: Instant,
    metrics: Option<Arc<PeerMetrics>>,
}

fn finalize_observation(ctx: &StreamContext, timeline: &Timeline, outcome: CompletionOutcome) {
    let Some(metrics) = &ctx.metrics else {
        return;
    };
    let ttft_ms = timeline
        .first_token_at
        .map(|t| t.duration_since(ctx.admitted_at).as_secs_f64() * 1_000.0)
        .unwrap_or(0.0);
    let total_ms = Instant::now().duration_since(ctx.admitted_at).as_secs_f64() * 1_000.0;
    let itl_count = timeline.token_deltas.saturating_sub(1);
    metrics.record_completion(&CompletionObservation {
        peer_id: ctx.peer_id.clone(),
        profile_id: ctx.profile_id.clone(),
        outcome,
        ttft_ms,
        total_ms,
        itl_mean_ms: if itl_count > 0 {
            timeline.itl_sum_ms / f64::from(itl_count)
        } else {
            0.0
        },
        itl_max_ms: timeline.itl_max_ms,
        token_deltas: timeline.token_deltas,
        usage: timeline.usage,
        advertised_queue_ms: ctx.advertised_queue_ms,
    });
}

/// Deposits a healthy session back into the executor pool (no-op when the
/// pool is already holding one or is in use).
fn return_to_pool(session: Libp2pSession, pool: &Arc<tokio::sync::Mutex<Option<Libp2pSession>>>) {
    if let Ok(mut guard) = pool.try_lock() {
        if guard.is_none() {
            *guard = Some(session);
        }
    }
}

/// F15: after a clean completion, probe RTT on the now-idle session and
/// only then return it to the pool. The probe rides the serving side's
/// between-requests recv (which answers pings transparently). A failed
/// probe drops the session — the next request self-heals with a fresh
/// dial — and records a failure observation.
async fn probe_and_return(
    session: Libp2pSession,
    pool: Arc<tokio::sync::Mutex<Option<Libp2pSession>>>,
    metrics: Arc<PeerMetrics>,
    peer_id: String,
) {
    let mut session = session;
    match session
        .measure_rtt(RTT_PROBE_SAMPLES, RTT_PROBE_DEADLINE)
        .await
    {
        Ok(stats) => {
            metrics.record_rtt(&peer_id, &stats);
            return_to_pool(session, &pool);
        }
        Err(_) => metrics.record_failure(&peer_id, "rtt_probe"),
    }
}

fn stream_events(
    session: Libp2pSession,
    pool: Arc<tokio::sync::Mutex<Option<Libp2pSession>>>,
    ctx: StreamContext,
) -> impl Stream<Item = ExecutorEvent> {
    // Terminal frames (Completed/Cancelled) END the exchange per the
    // protocol. A CLEAN terminal runs the F15 RTT probe (when metrics are
    // attached) and then returns the session to the pool for the next
    // request (connection reuse); errors drop it so the next request
    // dials fresh.
    stream::unfold(
        (Some(session), pool, false, Timeline::default(), ctx),
        |(slot, pool, done, mut timeline, ctx)| async move {
            if done {
                return None;
            }
            let mut session = match slot {
                Some(s) => s,
                None => return None,
            };
            let frame = match session.recv(IO_DEADLINE).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("modelswarm-node: remote stream read failed: {e}");
                    finalize_observation(
                        &ctx,
                        &timeline,
                        CompletionOutcome::Errored {
                            code: "transport".into(),
                        },
                    );
                    if let Some(metrics) = &ctx.metrics {
                        metrics.record_failure(&ctx.peer_id, "stream_read");
                    }
                    return Some((
                        ExecutorEvent::Error {
                            code: "transport".into(),
                            retryable: false,
                            interrupted_after_tokens: None,
                        },
                        (None, pool, true, timeline, ctx),
                    ));
                }
            };
            match frame_to_event(frame) {
                // Server closed the session: end cleanly, drop it — but
                // the observation still records what was measured.
                None => {
                    finalize_observation(
                        &ctx,
                        &timeline,
                        CompletionOutcome::Errored {
                            code: "connection_closed".into(),
                        },
                    );
                    None
                }
                Some(Ok(event)) => match &event {
                    ExecutorEvent::TokenDelta { .. } => {
                        timeline.on_token();
                        Some((event, (Some(session), pool, false, timeline, ctx)))
                    }
                    ExecutorEvent::Usage {
                        prompt_tokens,
                        completion_tokens,
                        prefill_ms,
                        decode_ms,
                    } => {
                        timeline.usage = Some(UsageDigest {
                            prompt_tokens: *prompt_tokens,
                            completion_tokens: *completion_tokens,
                            prefill_ms: *prefill_ms,
                            decode_ms: *decode_ms,
                        });
                        Some((event, (Some(session), pool, false, timeline, ctx)))
                    }
                    ExecutorEvent::Completed { finish_reason } => {
                        let finish_reason = format!("{finish_reason:?}").to_lowercase();
                        finalize_observation(
                            &ctx,
                            &timeline,
                            CompletionOutcome::Completed { finish_reason },
                        );
                        match &ctx.metrics {
                            Some(metrics) => {
                                let metrics = Arc::clone(metrics);
                                let peer_id = ctx.peer_id.clone();
                                tokio::spawn(probe_and_return(
                                    session,
                                    pool.clone(),
                                    metrics,
                                    peer_id,
                                ));
                            }
                            None => return_to_pool(session, &pool),
                        }
                        Some((event, (None, pool, true, timeline, ctx)))
                    }
                    // Accepted never appears on this wire path today.
                    ExecutorEvent::Accepted { .. } | ExecutorEvent::Error { .. } => {
                        Some((event, (Some(session), pool, false, timeline, ctx)))
                    }
                },
                Some(Err(wire_error)) => {
                    eprintln!(
                        "modelswarm-node: remote peer refused: {} {}",
                        wire_error.code, wire_error.message
                    );
                    let code = wire_error.code.clone();
                    finalize_observation(
                        &ctx,
                        &timeline,
                        CompletionOutcome::Errored { code: code.clone() },
                    );
                    Some((
                        ExecutorEvent::Error {
                            code: wire_error.code,
                            retryable: wire_error.retryable_peer_hint,
                            interrupted_after_tokens: None,
                        },
                        (None, pool, true, timeline, ctx),
                    ))
                }
            }
        },
    )
}

#[async_trait::async_trait]
impl InferenceExecutor for RemoteExecutor {
    async fn execute(&self, request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
        let expected_peer: PeerId =
            self.peer
                .peer_id
                .parse()
                .map_err(|_| ExecutorError::Fatal {
                    code: format!("bad_peer_id: {}", self.peer.peer_id),
                })?;
        // Pooled connection reuse: take a warm session if the pool holds
        // one, otherwise dial fresh.
        let mut guard = self.pool.lock().await;
        let session = match guard.take() {
            Some(s) => s,
            None => {
                let client =
                    Libp2pTransport::new(&self.identity).map_err(|e| ExecutorError::Fatal {
                        code: format!("transport: {e}"),
                    })?;
                match client
                    .dial(self.peer.addr.as_str(), &expected_peer, DIAL_DEADLINE)
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("modelswarm-node: remote dial failed: {e}");
                        self.record_failure("dial");
                        return Err(ExecutorError::Retryable {
                            peer_hint: Some(self.peer.peer_id.clone()),
                        });
                    }
                }
            }
        };
        drop(guard);
        let mut session = session;
        let ctx = StreamContext {
            peer_id: self.peer.peer_id.clone(),
            profile_id: request.profile_id.clone(),
            advertised_queue_ms: *self
                .advertised_queue_ms
                .lock()
                .expect("advertised queue lock"),
            // Admission time: the request leaves right now — measured
            // admission-to-first-token starts at this instant (F15).
            admitted_at: Instant::now(),
            metrics: self.metrics.clone(),
        };
        if let Err(e) = session
            .send(
                &WireMessage::InferenceRequest(wire_request(&request)),
                IO_DEADLINE,
            )
            .await
        {
            eprintln!("modelswarm-node: remote send failed: {e}");
            self.record_failure("send");
            return Err(ExecutorError::Retryable {
                peer_hint: Some(self.peer.peer_id.clone()),
            });
        }
        Ok(Box::pin(stream_events(
            session,
            std::sync::Arc::clone(&self.pool),
            ctx,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serving::serve_sessions;
    use futures_util::StreamExt;
    use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedRequest};
    use modelswarm_identity::InstallationIdentity;
    use modelswarm_transport::libp2p_backend::Libp2pTransport;
    use std::sync::Arc;

    /// F1 same-host proof: RemoteExecutor dials a serving peer over real
    /// QUIC and re-emits its frames as gateway executor events — the
    /// exact path a requesting gateway will drive.
    #[tokio::test]
    async fn remote_executor_streams_from_serving_peer() {
        struct FixedExecutor;
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for FixedExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    modelswarm_gateway::ExecutorEvent::TokenDelta {
                        delta: "remote ".into(),
                        index: 0,
                    },
                    modelswarm_gateway::ExecutorEvent::TokenDelta {
                        delta: "answer".into(),
                        index: 1,
                    },
                    modelswarm_gateway::ExecutorEvent::Completed {
                        finish_reason: modelswarm_gateway::FinishReason::Stop,
                    },
                ])))
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[27u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[28u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();

        let peer = RemotePeer {
            addr: listener.bound_addr().to_string(),
            peer_id: server.peer_id().to_string(),
        };
        let client_peer_id = Libp2pTransport::new(&client_identity)
            .unwrap()
            .peer_id()
            .to_string();
        let (policy, token) = crate::serving::lease_helpers::policy_and_lease(
            crate::serving::lease_helpers::TEST_PROFILE,
            &client_peer_id,
        );
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener,
            Arc::new(FixedExecutor),
            crate::serving::lease_helpers::TEST_PROFILE.into(),
            policy,
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            shutdown_rx,
        ));

        let executor = RemoteExecutor::new(peer, client_identity);
        let stream = executor
            .execute(NormalizedRequest {
                request_id: "req-f1".into(),
                profile_id: crate::serving::lease_helpers::TEST_PROFILE.into(),
                capability_token: Some(token),
                messages: vec![modelswarm_gateway::NormalizedMessage {
                    role: "user".into(),
                    content: "hi".into(),
                }],
                sampling: modelswarm_gateway::Sampling {
                    temperature: 0.0,
                    top_p: 1.0,
                    top_k: 40,
                    seed: None,
                },
                max_tokens: 8,
                deadline_ms: 10_000,
                stream: true,
            })
            .await
            .expect("remote execute starts");

        let mut text = String::new();
        let mut finish = String::new();
        futures_util::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            match event {
                ExecutorEvent::TokenDelta { delta, .. } => text.push_str(&delta),
                ExecutorEvent::Completed { finish_reason } => {
                    finish = format!("{finish_reason:?}");
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(text, "remote answer");
        assert_eq!(finish, "Stop");
    }
}

/// F1.5 chooser: remote-first with honest local fallback. If a serving
/// peer is known for the profile, requests go there; ANY pre-first-token
/// failure (dial, send, retryable error) falls back to the LOCAL
/// executor — the requester never notices unless the remote was slower.
/// After the first streamed token, remote failures are surfaced as
/// honest interrupted errors (ADR-007: no cross-peer retry mid-stream).
///
/// v1 policy is deliberately this simple: one remote peer, no EWMA, no
/// hedging. Live latency measurement (fastest-peer selection) stacks
/// above this once real WAN numbers exist (phase-f-research, F1.5).
pub struct FailoverExecutor {
    remote: std::sync::Arc<RemoteExecutor>,
    local: std::sync::Arc<dyn InferenceExecutor>,
}

impl FailoverExecutor {
    pub fn new(
        remote: std::sync::Arc<RemoteExecutor>,
        local: std::sync::Arc<dyn InferenceExecutor>,
    ) -> Self {
        Self { remote, local }
    }
}

#[async_trait::async_trait]
impl InferenceExecutor for FailoverExecutor {
    async fn execute(&self, request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
        match self.remote.execute(request.clone()).await {
            Ok(stream) => Ok(stream),
            Err(ExecutorError::Retryable { .. }) => {
                eprintln!("modelswarm-node: remote peer failed pre-token; falling back to local");
                self.local.execute(request).await
            }
            // Fatal remote errors (bad peer id, transport build) also
            // degrade to local — the request itself is still servable.
            Err(ExecutorError::Fatal { code }) => {
                eprintln!("modelswarm-node: remote executor fatal ({code}); falling back to local");
                self.local.execute(request).await
            }
            Err(ExecutorError::NoPeer) => self.local.execute(request).await,
        }
    }
}

#[cfg(test)]
mod failover_tests {
    use super::*;
    use crate::serving::serve_sessions;
    use futures_util::StreamExt;
    use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedRequest};
    use modelswarm_identity::InstallationIdentity;
    use modelswarm_transport::libp2p_backend::Libp2pTransport;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn request(profile: &str) -> NormalizedRequest {
        NormalizedRequest {
            request_id: "req-fo".into(),
            profile_id: profile.into(),
            capability_token: None,
            messages: vec![modelswarm_gateway::NormalizedMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            sampling: modelswarm_gateway::Sampling {
                temperature: 0.0,
                top_p: 1.0,
                top_k: 40,
                seed: None,
            },
            max_tokens: 8,
            deadline_ms: 10_000,
            stream: true,
        }
    }

    struct CountingExecutor {
        label: &'static str,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl modelswarm_gateway::InferenceExecutor for CountingExecutor {
        async fn execute(
            &self,
            _request: NormalizedRequest,
        ) -> Result<ExecutorStream, ExecutorError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(futures_util::stream::iter(vec![
                ExecutorEvent::TokenDelta {
                    delta: self.label.into(),
                    index: 0,
                },
                ExecutorEvent::Completed {
                    finish_reason: modelswarm_gateway::FinishReason::Stop,
                },
            ])))
        }
    }

    async fn collect(stream: ExecutorStream) -> String {
        let mut text = String::new();
        futures_util::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            if let ExecutorEvent::TokenDelta { delta, .. } = event {
                text.push_str(&delta);
            }
        }
        text
    }

    #[tokio::test]
    async fn remote_up_answers_come_from_the_peer() {
        let server_identity = InstallationIdentity::from_bytes(&[37u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[38u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let peer = RemotePeer {
            addr: listener.bound_addr().to_string(),
            peer_id: server.peer_id().to_string(),
        };
        let client_peer_id = Libp2pTransport::new(&client_identity)
            .unwrap()
            .peer_id()
            .to_string();
        let profile = crate::serving::lease_helpers::TEST_PROFILE;
        let (policy, token) =
            crate::serving::lease_helpers::policy_and_lease(profile, &client_peer_id);
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let remote_executor = Arc::new(CountingExecutor {
            label: "REMOTE",
            calls: AtomicUsize::new(0),
        });
        tokio::spawn(serve_sessions(
            listener,
            remote_executor.clone(),
            profile.into(),
            policy,
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx,
        ));

        let local = Arc::new(CountingExecutor {
            label: "LOCAL",
            calls: AtomicUsize::new(0),
        });
        let mut with_lease = request(profile);
        with_lease.capability_token = Some(token);
        let fo = FailoverExecutor::new(
            Arc::new(RemoteExecutor::new(peer, client_identity)),
            local.clone(),
        );

        let text = collect(fo.execute(with_lease).await.unwrap()).await;
        assert_eq!(text, "REMOTE");
        assert_eq!(
            local.calls.load(Ordering::SeqCst),
            0,
            "local untouched when remote is up"
        );
        assert_eq!(remote_executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn remote_down_falls_back_to_local_honestly() {
        // No listener at that address: the dial must fail and the local
        // executor answers instead.
        let client_identity = InstallationIdentity::from_bytes(&[39u8; 32]);
        // A REAL PeerId derivation with an unreachable address: the DIAL
        // must fail (a placeholder-style base58 string would fail
        // `PeerId::parse` first and never reach the dial at all).
        let peer = RemotePeer {
            addr: "/ip4/127.0.0.1/udp/9/udt".into(), // nothing listens here
            peer_id: Libp2pTransport::new(&InstallationIdentity::from_bytes(&[83u8; 32]))
                .unwrap()
                .peer_id()
                .to_string(),
        };
        let local = Arc::new(CountingExecutor {
            label: "LOCAL",
            calls: AtomicUsize::new(0),
        });
        let fo = FailoverExecutor::new(
            Arc::new(RemoteExecutor::new(peer, client_identity)),
            local.clone(),
        );
        let text = collect(fo.execute(request("msp1:fo")).await.unwrap()).await;
        assert_eq!(text, "LOCAL");
        assert_eq!(local.calls.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod lan_proof {
    use super::*;
    use crate::load_or_create_identity;
    use futures_util::StreamExt;
    use modelswarm_gateway::{
        ExecutorEvent, InferenceExecutor, NormalizedMessage, NormalizedRequest,
    };
    use modelswarm_telemetry::Telemetry;
    use modelswarm_tracker_api::TrackerClient;
    use std::time::Duration;

    /// F0(5)/F1 LAN CROSS-MACHINE PROOF (ignored; env-gated):
    /// MSP_LAN_PROFILE=<profile id> MSP_DATA_DIR=<ModelSwarm data dir>
    /// MSP_EXCLUDE_ADDR=<own advertised multiaddr> — looks the profile up
    /// on the PRODUCTION tracker with the real installation identity,
    /// dials the first OTHER peer advertising a real QUIC multiaddr, and
    /// streams one real completion from that machine.
    #[tokio::test]
    #[ignore = "set MSP_LAN_PROFILE, MSP_DATA_DIR and MSP_EXCLUDE_ADDR"]
    async fn lan_cross_machine_completion() {
        let profile = std::env::var("MSP_LAN_PROFILE").expect("MSP_LAN_PROFILE");
        let data_dir = std::env::var("MSP_DATA_DIR").expect("MSP_DATA_DIR");
        let exclude = std::env::var("MSP_EXCLUDE_ADDR").unwrap_or_default();
        let tracker_url = std::env::var("MSP_TRACKER")
            .unwrap_or_else(|_| "https://modelswarm.deepflux.space".into());

        let dir = std::path::PathBuf::from(&data_dir);
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let sink = modelswarm_telemetry::FileSink::open(&logs.join("node.jsonl")).unwrap();
        let telemetry = Telemetry::with_sink(Box::new(sink));
        let identity = load_or_create_identity(&dir, &telemetry).unwrap();

        let tracker = TrackerClient::new(
            &tracker_url,
            std::sync::Arc::new(load_or_create_identity(&dir, &telemetry).unwrap()),
        );
        // The roster endpoint needs an enrolled session (X-MSP-Session);
        // prefer the app's persisted session, else enroll on the fly
        // (production auto-approves devices).
        if let Ok(token) = std::fs::read_to_string(dir.join("session.token")) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                tracker.set_session(token);
            }
        }
        if !tracker.has_session() {
            let start = tracker.device_start().await.unwrap();
            let device_code = start["deviceCode"].as_str().expect("deviceCode");
            let mut session = None;
            for _ in 0..10 {
                match tracker.device_complete(device_code).await {
                    Ok(v) => {
                        session = v["token"].as_str().map(str::to_string);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                }
            }
            tracker.set_session(session.expect("device enrollment completes (auto-approve)"));
        }
        let peers = tracker.lookup(&profile, 10).await.unwrap();
        println!("roster: {} peers for this profile", peers.len());
        for p in &peers {
            println!(
                "  peer {} addr {:?} slots {}",
                p.peer_id, p.addresses, p.free_slots
            );
        }
        // Structural self-exclusion: MSP_EXCLUDE_ADDR is treated as a
        // substring (any peer advertising it — e.g. our own multiaddr — is
        // skipped), so the proof always dials the OTHER machine.
        let candidate = peers
            .iter()
            .find(|p| {
                p.free_slots > 0
                    && p.addresses.iter().any(|a| {
                        a.ends_with("quic-v1")
                            && !a.contains("0.0.0.0")
                            && (exclude.is_empty() || !a.contains(&exclude))
                    })
            })
            .expect("no other serving peer with a real address on the roster");
        let addr = candidate
            .addresses
            .iter()
            .find(|a| {
                a.ends_with("quic-v1")
                    && !a.contains("0.0.0.0")
                    && (exclude.is_empty() || !a.contains(&exclude))
            })
            .unwrap()
            .clone();
        println!(
            "dialing {addr} (peer {}…)",
            &candidate.peer_id[..16.min(candidate.peer_id.len())]
        );

        let executor = RemoteExecutor::new(
            RemotePeer {
                addr,
                peer_id: candidate.peer_id.clone(),
            },
            identity,
        );
        let started = std::time::Instant::now();
        let mut stream = executor
            .execute(NormalizedRequest {
                request_id: format!("lan-proof-{}", modelswarm_identity::new_nonce()),
                profile_id: profile.clone(),
                capability_token: (async {
                    // F3 gated path: earn a consume lease the honest way —
                    // hosting challenge timed against THIS machine's real
                    // engine (the app is hosting this profile).
                    let lease_id = std::fs::read_to_string(dir.join("lease.id"))
                        .ok()?
                        .trim()
                        .to_string();
                    let started = std::time::Instant::now();
                    let probe: serde_json::Value = reqwest::Client::new()
                        .post("http://127.0.0.1:11435/v1/chat/completions")
                        .timeout(Duration::from_secs(60))
                        .json(&serde_json::json!({
                            "model": profile,
                            "messages": [{ "role": "user", "content": "Say ok." }],
                            "max_tokens": 8,
                            "temperature": 0.0,
                            "stream": false,
                        }))
                        .send()
                        .await
                        .ok()?
                        .json()
                        .await
                        .ok()?;
                    probe["choices"][0]["message"]["content"].as_str()?;
                    let total_ms = started.elapsed().as_millis() as u64;
                    let ch = tracker
                        .challenge_start(&lease_id, profile.as_str())
                        .await
                        .ok()?;
                    let ch_id = ch["challengeId"].as_str()?.to_string();
                    tracker
                        .challenge_complete(&lease_id, profile.as_str(), &ch_id, total_ms, total_ms)
                        .await
                        .ok()?;
                    let issued = tracker
                        .request_lease(&lease_id, profile.as_str())
                        .await
                        .ok()?;
                    println!("lease earned (challenge {} ms)", total_ms);
                    Some(issued.lease)
                })
                .await,
                messages: vec![NormalizedMessage {
                    role: "user".into(),
                    content: "Say exactly: cross-machine swarm serving works.".into(),
                }],
                sampling: modelswarm_gateway::Sampling {
                    temperature: 0.0,
                    top_p: 1.0,
                    top_k: 40,
                    seed: None,
                },
                max_tokens: 24,
                deadline_ms: 60_000,
                stream: true,
            })
            .await
            .expect("remote execute starts");

        let mut text = String::new();
        let mut completion_tokens = 0u32;
        while let Some(event) = stream.next().await {
            match event {
                ExecutorEvent::TokenDelta { delta, .. } => text.push_str(&delta),
                ExecutorEvent::Usage {
                    completion_tokens: t,
                    ..
                } => completion_tokens = t,
                ExecutorEvent::Completed { finish_reason } => {
                    println!("finish: {finish_reason:?}");
                    break;
                }
                ExecutorEvent::Error { code, .. } => panic!("stream error: {code}"),
                ExecutorEvent::Accepted { .. } => {}
            }
        }
        let secs = started.elapsed().as_secs_f64();
        println!("REMOTE REPLY ({completion_tokens} tokens, {secs:.1}s): {text}");
        assert!(!text.trim().is_empty(), "remote peer must produce tokens");
    }
}

#[cfg(test)]
mod reuse_tests {
    use super::*;
    use crate::serving::serve_sessions;
    use futures_util::StreamExt;
    use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedMessage, NormalizedRequest};
    use modelswarm_identity::InstallationIdentity;
    use modelswarm_transport::libp2p_backend::Libp2pTransport;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Connection reuse: TWO sequential requests through ONE
    /// RemoteExecutor reuse the same QUIC session (one dial, one serving
    /// session, both requests served) — the per-request handshake
    /// disappears.
    #[tokio::test]
    async fn two_requests_reuse_one_session() {
        struct CountingExecutor {
            served: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for CountingExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                let n = self.served.fetch_add(1, Ordering::SeqCst);
                Ok(Box::pin(futures_util::stream::iter(vec![
                    ExecutorEvent::TokenDelta {
                        delta: format!("answer-{n}"),
                        index: 0,
                    },
                    ExecutorEvent::Completed {
                        finish_reason: modelswarm_gateway::FinishReason::Stop,
                    },
                ])))
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[67u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[68u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let addr = listener.bound_addr().to_string();
        let profile = crate::serving::lease_helpers::TEST_PROFILE;
        let client_peer_id = Libp2pTransport::new(&client_identity)
            .unwrap()
            .peer_id()
            .to_string();
        let (policy, token) =
            crate::serving::lease_helpers::policy_and_lease(profile, &client_peer_id);
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let served = Arc::new(CountingExecutor {
            served: AtomicUsize::new(0),
        });
        tokio::spawn(serve_sessions(
            listener,
            served.clone(),
            profile.into(),
            policy,
            std::sync::Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx,
        ));

        let executor = RemoteExecutor::new(
            RemotePeer {
                addr,
                peer_id: server.peer_id().to_string(),
            },
            client_identity,
        );

        async fn run(executor: &RemoteExecutor, profile: &str, token: &str, id: &str) -> String {
            let stream = executor
                .execute(NormalizedRequest {
                    request_id: id.into(),
                    profile_id: profile.into(),
                    capability_token: Some(token.to_string()),
                    messages: vec![NormalizedMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                    sampling: modelswarm_gateway::Sampling {
                        temperature: 0.0,
                        top_p: 1.0,
                        top_k: 40,
                        seed: None,
                    },
                    max_tokens: 8,
                    deadline_ms: 10_000,
                    stream: true,
                })
                .await
                .expect("execute starts");
            let mut text = String::new();
            futures_util::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                if let ExecutorEvent::TokenDelta { delta, .. } = event {
                    text.push_str(&delta);
                }
            }
            text
        }

        let first = run(&executor, profile, &token, "r1").await;
        // Give the pool a beat to take the returned session.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let pooled = executor.pool.lock().await.is_some();
        let second = run(&executor, profile, &token, "r2").await;

        assert_eq!(first, "answer-0");
        assert_eq!(second, "answer-1");
        assert!(
            pooled,
            "the session returned to the pool after the clean terminal"
        );
        assert_eq!(served.served.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod measure_tests {
    use super::*;
    use crate::serving::serve_sessions;
    use futures_util::StreamExt;
    use modelswarm_gateway::{ExecutorError, ExecutorStream, NormalizedMessage, NormalizedRequest};
    use modelswarm_identity::InstallationIdentity;
    use modelswarm_transport::libp2p_backend::Libp2pTransport;
    use std::sync::Arc;
    use std::time::Duration;

    /// F15 end-to-end over REAL loopback QUIC against the real serving
    /// bridge (environment label: loopback-proven): a metrics-attached
    /// RemoteExecutor records the completion distribution
    /// (TTFT/ITL/total, advertised queue alongside the measured
    /// admission-to-first-token), the post-completion idle RTT probe
    /// lands via the serving side's transparent ping answering, and the
    /// pooled session returns to the pool afterwards (connection reuse
    /// preserved).
    #[tokio::test]
    async fn remote_executor_measures_completions_and_rtt_over_quic() {
        struct TwoTokenExecutor;
        #[async_trait::async_trait]
        impl modelswarm_gateway::InferenceExecutor for TwoTokenExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    ExecutorEvent::TokenDelta {
                        delta: "a".into(),
                        index: 0,
                    },
                    ExecutorEvent::TokenDelta {
                        delta: "b".into(),
                        index: 1,
                    },
                    ExecutorEvent::Usage {
                        prompt_tokens: 3,
                        completion_tokens: 2,
                        prefill_ms: 1.0,
                        decode_ms: 2.0,
                    },
                    ExecutorEvent::Completed {
                        finish_reason: modelswarm_gateway::FinishReason::Stop,
                    },
                ])))
            }
        }

        let server_identity = InstallationIdentity::from_bytes(&[77u8; 32]);
        let client_identity = InstallationIdentity::from_bytes(&[78u8; 32]);
        let server = Libp2pTransport::new(&server_identity).unwrap();
        let listener = server.listen("127.0.0.1:0").await.unwrap();
        let server_peer = server.peer_id().to_string();
        let peer = RemotePeer {
            addr: listener.bound_addr().to_string(),
            peer_id: server_peer.clone(),
        };
        let client_peer_id = Libp2pTransport::new(&client_identity)
            .unwrap()
            .peer_id()
            .to_string();
        let profile_id = crate::serving::lease_helpers::TEST_PROFILE;
        let (policy, token) =
            crate::serving::lease_helpers::policy_and_lease(profile_id, &client_peer_id);
        let (_tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener,
            Arc::new(TwoTokenExecutor),
            profile_id.into(),
            policy,
            Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx,
        ));

        let metrics = Arc::new(PeerMetrics::memory(Arc::new(
            modelswarm_telemetry::Telemetry::memory().0,
        )));
        let executor =
            RemoteExecutor::new(peer, client_identity).with_metrics(Arc::clone(&metrics));
        executor.set_advertised_queue_ms(Some(120));

        let stream = executor
            .execute(NormalizedRequest {
                request_id: "req-f15".into(),
                profile_id: profile_id.into(),
                capability_token: Some(token),
                messages: vec![NormalizedMessage {
                    role: "user".into(),
                    content: "hi".into(),
                }],
                sampling: modelswarm_gateway::Sampling {
                    temperature: 0.0,
                    top_p: 1.0,
                    top_k: 40,
                    seed: None,
                },
                max_tokens: 8,
                deadline_ms: 10_000,
                stream: true,
            })
            .await
            .expect("execute starts");
        let mut text = String::new();
        futures_util::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            if let ExecutorEvent::TokenDelta { delta, .. } = event {
                text.push_str(&delta);
            }
        }
        assert_eq!(text, "ab");

        // The completion distribution and the (asynchronous) RTT probe
        // both land; then the session is back in the pool.
        let wait_until = std::time::Instant::now() + Duration::from_secs(5);
        let obs = loop {
            let obs = metrics.observe(&server_peer);
            if obs.rtt_ewma_ms.is_some() {
                break obs;
            }
            assert!(
                std::time::Instant::now() < wait_until,
                "post-completion RTT probe did not land within 5 s"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        assert!(obs.rtt_ewma_ms.unwrap() > 0.0, "loopback rtt positive");
        assert!(obs.rtt_jitter_ewma_ms.unwrap() >= 0.0);
        assert_eq!(obs.rtt_probe_count, 1);
        assert_eq!(obs.failure_count, 0);

        let profile = obs.profile(profile_id).expect("completion recorded");
        assert!(profile.ttft_ewma_ms.unwrap() > 0.0, "measured ttft");
        assert!(profile.itl_mean_ewma_ms.unwrap() >= 0.0);
        assert!(profile.total_ewma_ms.unwrap() >= profile.ttft_ewma_ms.unwrap());
        assert_eq!(profile.advertised_queue_ms_last, Some(120));
        assert_eq!(profile.completion_count, 1);
        assert_eq!(profile.decode_tokens_per_ms_ewma, Some(1.0)); // 2 tokens / 2 ms
        assert_eq!(profile.prefill_tokens_per_ms_ewma, Some(3.0)); // 3 tokens / 1 ms

        // Connection reuse survives measurement: the probed session is
        // pooled again.
        let pooled = loop {
            if executor.pool.lock().await.is_some() {
                break true;
            }
            assert!(
                std::time::Instant::now() < wait_until,
                "session never returned to the pool after the probe"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert!(pooled);
    }

    /// F15: dial failures are counted (the shadow failure signal), and a
    /// metrics-free executor stays measurement-silent.
    #[tokio::test]
    async fn dial_failures_count_and_unmeasured_executors_stay_silent() {
        let metrics = Arc::new(PeerMetrics::memory(Arc::new(
            modelswarm_telemetry::Telemetry::memory().0,
        )));
        // A REAL PeerId derivation (the placeholder-style base58 string
        // fails PeerId::parse before any dial) with an unreachable
        // address: the dial itself must fail and be counted.
        let dead_peer_id = Libp2pTransport::new(&InstallationIdentity::from_bytes(&[81u8; 32]))
            .unwrap()
            .peer_id()
            .to_string();
        let dead_peer = RemotePeer {
            addr: "/ip4/127.0.0.1/udp/9/udt".into(), // nothing listens here
            peer_id: dead_peer_id,
        };
        let peer_id = dead_peer.peer_id.clone();
        let executor =
            RemoteExecutor::new(dead_peer, InstallationIdentity::from_bytes(&[79u8; 32]))
                .with_metrics(Arc::clone(&metrics));
        let request = NormalizedRequest {
            request_id: "req-dead".into(),
            profile_id: "msp1:dead".into(),
            capability_token: None,
            messages: vec![NormalizedMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            sampling: modelswarm_gateway::Sampling {
                temperature: 0.0,
                top_p: 1.0,
                top_k: 40,
                seed: None,
            },
            max_tokens: 8,
            deadline_ms: 10_000,
            stream: true,
        };
        assert!(executor.execute(request).await.is_err());
        assert_eq!(metrics.observe(&peer_id).failure_count, 1);

        // Without metrics attached, nothing is recorded anywhere.
        let silent = Arc::new(PeerMetrics::memory(Arc::new(
            modelswarm_telemetry::Telemetry::memory().0,
        )));
        let silent_exec = RemoteExecutor::new(
            RemotePeer {
                addr: "/ip4/127.0.0.1/udp/9/udt".into(),
                peer_id: Libp2pTransport::new(&InstallationIdentity::from_bytes(&[82u8; 32]))
                    .unwrap()
                    .peer_id()
                    .to_string(),
            },
            InstallationIdentity::from_bytes(&[80u8; 32]),
        );
        let request = NormalizedRequest {
            request_id: "req-silent".into(),
            profile_id: "msp1:dead".into(),
            capability_token: None,
            messages: vec![NormalizedMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            sampling: modelswarm_gateway::Sampling {
                temperature: 0.0,
                top_p: 1.0,
                top_k: 40,
                seed: None,
            },
            max_tokens: 8,
            deadline_ms: 10_000,
            stream: true,
        };
        assert!(silent_exec.execute(request).await.is_err());
        assert!(silent.observe_all().is_empty());
    }
}

#[cfg(test)]
mod shadow_tests {
    use super::*;
    use crate::serving::serve_sessions;
    use futures_util::StreamExt;
    use modelswarm_gateway::{
        ExecutorError, ExecutorStream, NormalizedMessage, NormalizedRequest, Sampling,
    };
    use modelswarm_identity::InstallationIdentity;
    use modelswarm_scheduler::shadow::{
        self, QueueDiscountPolicy, RosterFacts, ShadowCandidateInput,
    };
    use modelswarm_transport::libp2p_backend::Libp2pTransport;
    use std::sync::Arc;
    use std::time::Duration;

    /// Two tokens + a Usage frame (deterministic rates: prefill 3 tok/ms,
    /// decode 1 tok/ms), counting invocations so the test can prove the
    /// SHADOW never caused a dial.
    struct CountingTwoTokenExecutor {
        served: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl modelswarm_gateway::InferenceExecutor for CountingTwoTokenExecutor {
        async fn execute(
            &self,
            _request: NormalizedRequest,
        ) -> Result<ExecutorStream, ExecutorError> {
            self.served
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::pin(futures_util::stream::iter(vec![
                ExecutorEvent::TokenDelta {
                    delta: "a".into(),
                    index: 0,
                },
                ExecutorEvent::TokenDelta {
                    delta: "b".into(),
                    index: 1,
                },
                ExecutorEvent::Usage {
                    prompt_tokens: 3,
                    completion_tokens: 2,
                    prefill_ms: 1.0,
                    decode_ms: 2.0,
                },
                ExecutorEvent::Completed {
                    finish_reason: modelswarm_gateway::FinishReason::Stop,
                },
            ])))
        }
    }

    fn request(profile_id: &str, token: &str) -> NormalizedRequest {
        NormalizedRequest {
            request_id: "req-shadow".into(),
            profile_id: profile_id.into(),
            capability_token: Some(token.to_string()),
            messages: vec![NormalizedMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            sampling: Sampling {
                temperature: 0.0,
                top_p: 1.0,
                top_k: 40,
                seed: None,
            },
            max_tokens: 8,
            deadline_ms: 10_000,
            stream: true,
        }
    }

    /// Shadow-planner wiring over the REAL serving bridge (F15 pattern;
    /// environment label: loopback-proven). Two serving peers come up in
    /// PRODUCTION roster order — `first` (never measured) then `fast`
    /// (measured via one real completion + the idle RTT probe). Production
    /// semantics pick first-found; the shadow planner must pick the
    /// measured-fast peer, record the divergence, and NEVER dial anything
    /// on its own behalf (`first`'s served counter must stay 0).
    #[tokio::test]
    async fn shadow_planner_diverges_from_first_found_over_quic() {
        // The PRODUCTION profile shape end-to-end: ADR-011 derived id
        // (valid for the lease gate; the shadow's profile-id adapter
        // carries it into the frozen selector).
        let profile_id = crate::serving::lease_helpers::TEST_PROFILE;

        let transport_first =
            Libp2pTransport::new(&InstallationIdentity::from_bytes(&[91u8; 32])).unwrap();
        let transport_fast =
            Libp2pTransport::new(&InstallationIdentity::from_bytes(&[92u8; 32])).unwrap();
        let listener_first = transport_first.listen("127.0.0.1:0").await.unwrap();
        let listener_fast = transport_fast.listen("127.0.0.1:0").await.unwrap();
        let first_peer = transport_first.peer_id().to_string();
        let fast_peer = transport_fast.peer_id().to_string();
        let first_addr = listener_first.bound_addr().to_string();
        let fast_addr = listener_fast.bound_addr().to_string();
        // The roster dialability filter (app.rs) matches on this suffix —
        // pin the contract the shadow inputs rely on.
        assert!(first_addr.ends_with("quic-v1"));
        assert!(fast_addr.ends_with("quic-v1"));

        let client_identity = InstallationIdentity::from_bytes(&[93u8; 32]);
        let client_peer_id = Libp2pTransport::new(&client_identity)
            .unwrap()
            .peer_id()
            .to_string();
        // One independent (policy, token) pair per serving side (the
        // first-found side is never dialed — its token goes unused).
        let (policy_first, _token_first) =
            crate::serving::lease_helpers::policy_and_lease(profile_id, &client_peer_id);
        let (policy_fast, token_fast) =
            crate::serving::lease_helpers::policy_and_lease(profile_id, &client_peer_id);
        let first_executor = Arc::new(CountingTwoTokenExecutor {
            served: std::sync::atomic::AtomicUsize::new(0),
        });
        let (_tx_a, rx_a) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener_first,
            first_executor.clone(),
            profile_id.into(),
            policy_first,
            Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx_a,
        ));
        let (_tx_b, rx_b) = tokio::sync::watch::channel(false);
        tokio::spawn(serve_sessions(
            listener_fast,
            Arc::new(CountingTwoTokenExecutor {
                served: std::sync::atomic::AtomicUsize::new(0),
            }),
            profile_id.into(),
            policy_fast,
            Arc::new(modelswarm_telemetry::Telemetry::memory().0),
            rx_b,
        ));

        // ONE real completion against the fast peer (production would
        // have picked FIRST here — roster order; we dial fast directly to
        // create the measurement asymmetry the shadow must detect).
        let metrics = Arc::new(PeerMetrics::memory(Arc::new(
            modelswarm_telemetry::Telemetry::memory().0,
        )));
        let executor = RemoteExecutor::new(
            RemotePeer {
                addr: fast_addr,
                peer_id: fast_peer.clone(),
            },
            client_identity,
        )
        .with_metrics(Arc::clone(&metrics));
        executor.set_advertised_queue_ms(Some(120));
        let stream = executor
            .execute(request(profile_id, &token_fast))
            .await
            .expect("execute starts");
        let mut text = String::new();
        futures_util::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            if let ExecutorEvent::TokenDelta { delta, .. } = event {
                text.push_str(&delta);
            }
        }
        assert_eq!(text, "ab");

        // The F15 idle RTT probe lands after the clean completion.
        let wait_until = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if metrics.observe(&fast_peer).rtt_ewma_ms.is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < wait_until,
                "post-completion RTT probe did not land within 5 s"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Shadow inputs in production roster order: FIRST is first-found
        // and advertises no queue; FAST advertises 120 ms.
        let roster = |peer: &str, queue: u64| ShadowCandidateInput {
            roster: RosterFacts {
                peer_id: peer.to_string(),
                free_slots: 1,
                advertised_queue_ms: Some(queue),
                capacity_class: Some("gpu_mid".into()),
                has_direct_quic_addr: true,
            },
            observed: metrics.observe(peer),
        };
        let inputs = vec![roster(&first_peer, 0), roster(&fast_peer, 120)];
        let env = shadow::EnvPolicy {
            policy: QueueDiscountPolicy::DEFAULT,
            env_overrides: 0,
            warnings: Vec::new(),
        };
        let decision = shadow::shadow_decision(
            &inputs,
            &client_peer_id,
            profile_id,
            Some(&first_peer), // what first-found production picks
            2,
            8,
            &env,
        );

        // The divergence the shadow exists to surface.
        assert_eq!(decision.pick.as_deref(), Some(fast_peer.as_str()));
        assert_eq!(
            decision.production_pick.as_deref(),
            Some(first_peer.as_str())
        );
        assert_eq!(decision.agree, Some(false));
        assert_eq!(decision.candidate_count, 2);
        assert_eq!(decision.measured_count, 1);
        assert_eq!(decision.label, "shadow-no-action");

        // Ranking: the measured peer first with a finite, queue-charged
        // prediction; the cold peer last with NO prediction (not a fake
        // alphabetical one).
        assert_eq!(decision.ranked[0].peer_id, fast_peer);
        let predicted = decision.ranked[0].predicted_ms.expect("finite prediction");
        assert!(
            predicted > 120.0 && predicted < 400.0,
            "queue must be charged and loopback rates must keep it small: {predicted}"
        );
        // Discount record: advertised 120 dominates the small measured
        // estimate (advertised only ever penalizes), and the measured
        // estimate exists (measured dominates when it would be larger).
        assert!((decision.ranked[0].effective_queue_ms - 120.0).abs() < 1e-6);
        assert!(decision.ranked[0].measured_queue_estimate_ms.is_some());
        assert!(decision.ranked[0].measured);
        assert_eq!(decision.ranked[1].peer_id, first_peer);
        assert_eq!(decision.ranked[1].predicted_ms, None);
        assert!(!decision.ranked[1].measured);

        // The shadow NEVER acts: the first-found peer was never dialed by
        // the planner (only the explicit completion above moved bytes).
        assert_eq!(
            first_executor
                .served
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "shadow planning must not dial anyone"
        );
        // And the serialized record is telemetry-safe (finite numbers).
        let json = serde_json::to_string(&decision).expect("serializes");
        assert!(json.contains("\"predicted_ms\":null"));
        assert!(!json.contains("infinity") && !json.contains("NaN"));
    }
}
