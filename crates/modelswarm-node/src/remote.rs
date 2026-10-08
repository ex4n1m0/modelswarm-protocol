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

use std::time::Duration;

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

const DIAL_DEADLINE: Duration = Duration::from_secs(10);
const IO_DEADLINE: Duration = Duration::from_secs(30);

/// One remote serving peer.
#[derive(Debug, Clone)]
pub struct RemotePeer {
    /// Multiaddr, e.g. `/ip4/192.168.1.5/udp/4001/quic-v1`.
    pub addr: String,
    /// The ADR-020 PeerId the QUIC handshake must verify.
    pub peer_id: String,
}

/// Executes requests against a single remote serving peer.
pub struct RemoteExecutor {
    peer: RemotePeer,
    /// Client identity for the dial (a fresh transport per request).
    identity: InstallationIdentity,
}

impl RemoteExecutor {
    pub fn new(peer: RemotePeer, identity: InstallationIdentity) -> Self {
        Self { peer, identity }
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

fn stream_events(session: Libp2pSession) -> impl Stream<Item = ExecutorEvent> {
    // `completed` latches on the terminal frame: Completed/Cancelled END
    // the exchange per the protocol — reading past them would only see
    // the serving peer's post-stream connection close.
    stream::unfold((session, false), |(mut session, completed)| async move {
        if completed {
            return None;
        }
        let frame = match session.recv(IO_DEADLINE).await {
            Ok(f) => f,
            Err(e) => {
                eprintln!("modelswarm-node: remote stream read failed: {e}");
                return Some((
                    ExecutorEvent::Error {
                        code: "transport".into(),
                        retryable: false,
                        interrupted_after_tokens: None,
                    },
                    (session, true),
                ));
            }
        };
        match frame_to_event(frame) {
            None => None, // clean end
            Some(Ok(event)) => {
                let done = matches!(event, ExecutorEvent::Completed { .. });
                Some((event, (session, done)))
            }
            Some(Err(wire_error)) => Some((
                ExecutorEvent::Error {
                    code: wire_error.code,
                    retryable: wire_error.retryable_peer_hint,
                    interrupted_after_tokens: None,
                },
                (session, true),
            )),
        }
    })
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
        let client = Libp2pTransport::new(&self.identity).map_err(|e| ExecutorError::Fatal {
            code: format!("transport: {e}"),
        })?;
        let mut session = client
            .dial(self.peer.addr.as_str(), &expected_peer, DIAL_DEADLINE)
            .await
            .map_err(|e| {
                eprintln!("modelswarm-node: remote dial failed: {e}");
                ExecutorError::Retryable {
                    peer_hint: Some(self.peer.peer_id.clone()),
                }
            })?;
        session
            .send(
                &WireMessage::InferenceRequest(wire_request(&request)),
                IO_DEADLINE,
            )
            .await
            .map_err(|e| {
                eprintln!("modelswarm-node: remote send failed: {e}");
                ExecutorError::Retryable {
                    peer_hint: Some(self.peer.peer_id.clone()),
                }
            })?;
        Ok(Box::pin(stream_events(session)))
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
    use std::time::Duration;

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
    remote: RemoteExecutor,
    local: std::sync::Arc<dyn InferenceExecutor>,
}

impl FailoverExecutor {
    pub fn new(remote: RemoteExecutor, local: std::sync::Arc<dyn InferenceExecutor>) -> Self {
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
    use std::time::Duration;

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
            rx,
        ));

        let local = Arc::new(CountingExecutor {
            label: "LOCAL",
            calls: AtomicUsize::new(0),
        });
        let mut with_lease = request(profile);
        with_lease.capability_token = Some(token);
        let fo = FailoverExecutor::new(RemoteExecutor::new(peer, client_identity), local.clone());

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
        let peer = RemotePeer {
            addr: "/ip4/127.0.0.1/udp/9/udt".into(), // nothing listens here
            peer_id: "12D3KooWExamplePeerIdThatIsValidBase58ForTheParseCheck1111111111111".into(),
        };
        let local = Arc::new(CountingExecutor {
            label: "LOCAL",
            calls: AtomicUsize::new(0),
        });
        let fo = FailoverExecutor::new(RemoteExecutor::new(peer, client_identity), local.clone());
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
                capability_token: None,
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
