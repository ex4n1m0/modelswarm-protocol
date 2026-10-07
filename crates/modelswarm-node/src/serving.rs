//! F0(3) serving bridge: inbound framed sessions → local executor →
//! outbound token frames (Phase F0; see docs/reviews/phase-f-research).
//!
//! The listener hands us authenticated sessions (the QUIC handshake
//! verifies the remote's libp2p PeerId = the ADR-020 derivation). For
//! each session we read one `InferenceRequest`, drive the node's
//! [`InferenceExecutor`] (the same executor the local gateway uses —
//! ADR-025 ChatML templating applies), and stream
//! `TokenDelta`/`Usage`/`Completed` frames back.
//!
//! Honest F0 scope: the capability/lease token is NOT yet verified here
//! (TODO F3 — the tracker-signed lease check lands with hostile
//! hardening); a session is admitted if it speaks the protocol and asks
//! for the profile this node serves.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use modelswarm_gateway::{ExecutorEvent, InferenceExecutor, NormalizedMessage, NormalizedRequest};
use modelswarm_transport::libp2p_backend::{Libp2pListener, Libp2pSession};
use modelswarm_transport::message::{
    ChatMessage, Completed, StreamError, TokenDelta, Usage, WireMessage,
};

const IO_DEADLINE: Duration = Duration::from_secs(30);

/// Serves accepted sessions until `shutdown` flips. Never panics on a bad
/// session: malformed input becomes a `StreamError` frame, then the session
/// closes (one request per session in F0).
pub async fn serve_sessions(
    mut listener: Libp2pListener,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let session = tokio::select! {
            _ = shutdown.changed() => return,
            accepted = listener.accept(std::time::Duration::from_secs(1)) => match accepted {
                Ok(s) => s,
                Err(_) => continue, // timeout/accept miss: keep serving
            },
        };
        let executor = Arc::clone(&executor);
        let profile = profile_id.clone();
        tokio::spawn(async move {
            let _ = serve_one(session, executor, profile).await;
        });
    }
}

async fn serve_one(
    mut session: Libp2pSession,
    executor: Arc<dyn InferenceExecutor>,
    profile_id: String,
) -> Result<(), String> {
    let request = match session.recv(IO_DEADLINE).await {
        Ok(WireMessage::InferenceRequest(r)) => r,
        Ok(other) => {
            let code = format!("expected InferenceRequest, got {other:?}");
            send_error(&mut session, "bad_frame", &code).await;
            return Err(code);
        }
        Err(e) => {
            send_error(&mut session, "bad_frame", &e.to_string()).await;
            return Err(e.to_string());
        }
    };
    if request.profile_id != profile_id {
        send_error(
            &mut session,
            "profile_mismatch",
            "this node serves another profile",
        )
        .await;
        return Err("profile_mismatch".into());
    }

    let normalized = NormalizedRequest {
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        capability_token: Some(request.capability_token.clone()),
        messages: request
            .messages
            .into_iter()
            .map(|m: ChatMessage| NormalizedMessage {
                role: m.role,
                content: m.content,
            })
            .collect(),
        sampling: modelswarm_gateway::Sampling {
            temperature: request.sampling.temperature as f32,
            top_p: request.sampling.top_p as f32,
            top_k: request.sampling.top_k,
            seed: request.sampling.seed,
        },
        max_tokens: request.max_tokens,
        deadline_ms: request.deadline_ms,
        stream: true,
    };

    let mut events = match executor.execute(normalized).await {
        Ok(stream) => stream,
        Err(e) => {
            let code = format!("{e:?}");
            send_error(&mut session, "executor_error", &code).await;
            return Err(code);
        }
    };
    while let Some(event) = events.next().await {
        let frame = match event {
            ExecutorEvent::Accepted { .. } => continue,
            ExecutorEvent::TokenDelta { delta, index } => WireMessage::TokenDelta(TokenDelta {
                request_id: request.request_id.clone(),
                delta,
                index,
            }),
            ExecutorEvent::Usage {
                prompt_tokens,
                completion_tokens,
                prefill_ms,
                decode_ms,
            } => WireMessage::Usage(Usage {
                prompt_tokens,
                completion_tokens,
                prefill_ms: prefill_ms as u32,
                decode_ms: decode_ms as u32,
            }),
            ExecutorEvent::Completed { finish_reason } => WireMessage::Completed(Completed {
                request_id: request.request_id.clone(),
                finish_reason: format!("{finish_reason:?}").to_lowercase(),
            }),
            ExecutorEvent::Error {
                code, retryable, ..
            } => {
                send_error(&mut session, &code, &format!("retryable={retryable}")).await;
                return Err(code);
            }
        };
        if session.send(&frame, IO_DEADLINE).await.is_err() {
            return Err("client vanished mid-stream".into());
        }
    }
    Ok(())
}

async fn send_error(session: &mut Libp2pSession, code: &str, detail: &str) {
    let _ = session
        .send(
            &WireMessage::StreamError(StreamError {
                request_id: String::new(),
                code: code.to_string(),
                message: detail.chars().take(200).collect(),
                retryable_peer_hint: false,
            }),
            IO_DEADLINE,
        )
        .await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn serving_module_wires_wire_shapes() {
        // The full accept→request→execute→frame loop is exercised by the
        // F0(4) same-host proof; this pins the frame-shape compile contract.
        use modelswarm_transport::message::Completed;
        assert_eq!(
            Completed {
                request_id: "r".into(),
                finish_reason: "stop".into()
            }
            .finish_reason,
            "stop"
        );
    }
}
