//! Local single-peer executor: the gateway's [`InferenceExecutor`] over one
//! local [`InferenceRuntime`] (Phase G scope).
//!
//! This mirrors `modelswarm_session::spec::SpeculativeExecutor`'s event
//! batching — one [`ExecutorEvent::TokenDelta`] per batch of committed tokens
//! (round batch there, [`LOCAL_DELTA_BATCH_TOKENS`] chunk here), then
//! `Usage`, then `Completed` — so the gateway sees the same event shape today
//! and when the full swarm executor replaces this wiring. The full
//! cooperative/swarm executor (`SpeculativeExecutor` + peer rotation over
//! `modelswarm_transport`) is a documented later wiring; see the crate docs.
//!
//! Privacy: the request's message content is tokenized and passed to the
//! local runtime only. It is never logged: the executor's only logging is
//! a closed error-code table through the node telemetry sink (code, HTTP
//! status, byte lengths — never RuntimeError display text, which carries
//! external-process-controlled bodies), and the gateway's audit channel
//! records counts and ids only.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::stream;
use modelswarm_gateway::{
    ExecutorError, ExecutorEvent, ExecutorStream, FinishReason, InferenceExecutor,
    NormalizedMessage, NormalizedRequest,
};
use modelswarm_runtime::{InferenceRuntime, RuntimeError, SamplingParams};

/// Tokens per [`ExecutorEvent::TokenDelta`] (batching parity with the session
/// executor's per-round batches).
pub const LOCAL_DELTA_BATCH_TOKENS: usize = 16;

/// Executes requests against one local runtime, or none.
pub struct SingleLocalExecutor {
    runtime: Option<Arc<dyn InferenceRuntime>>,
    profile_id: Option<String>,
    /// Node telemetry for redacted failure diagnostics (None in tests).
    telemetry: Option<Arc<modelswarm_telemetry::Telemetry>>,
}

impl SingleLocalExecutor {
    /// Wraps an optional local runtime serving `profile_id`. `None` in either
    /// position makes [`InferenceExecutor::execute`] return
    /// [`ExecutorError::NoPeer`] — an honest 503, never a fake answer.
    pub fn new(runtime: Option<Arc<dyn InferenceRuntime>>, profile_id: Option<String>) -> Self {
        Self {
            runtime,
            profile_id,
            telemetry: None,
        }
    }

    /// Routes decode-failure diagnostics to the node telemetry sink.
    pub fn with_telemetry(mut self, telemetry: Arc<modelswarm_telemetry::Telemetry>) -> Self {
        self.telemetry = Some(telemetry);
        self
    }
}

/// Closed error-code table for runtime failures (AGENTS.md rule 5): the
/// wire and audit surfaces see a stable code only — RuntimeError display
/// text never crosses them (engine error bodies can echo request
/// fragments). Returns `(code, http_status_or_0)`.
fn runtime_error_code(e: &RuntimeError) -> (&'static str, u16) {
    match e {
        RuntimeError::Api { status, .. } => ("engine_api_error", *status),
        RuntimeError::Http(_) => ("engine_transport", 0),
        RuntimeError::MalformedResponse(_) => ("engine_malformed_response", 0),
        RuntimeError::InvalidProfile(_) => ("engine_invalid_profile", 0),
        RuntimeError::TokenOutOfRange(_) => ("engine_token_out_of_range", 0),
        RuntimeError::NotLoaded(_) => ("engine_not_loaded", 0),
        RuntimeError::InvalidDescriptor(_) => ("engine_invalid_descriptor", 0),
        RuntimeError::Unsupported(_) => ("engine_unsupported", 0),
        RuntimeError::Timeout { .. } => ("deadline_exceeded", 0),
        RuntimeError::Cancelled(_) => ("cancelled", 0),
    }
}

fn fatal(code: &str) -> ExecutorError {
    ExecutorError::Fatal {
        code: code.to_string(),
    }
}

/// Renders the conversation in ChatML — the template family of every
/// catalog profile (Qwen, SmolLM2; evidenced by their chat_template_hash
/// inputs). The serving side owns template application (msp-v1 §6.2):
/// the raw `/v1/completions` endpoint applies nothing, and the previous
/// newline flattening made role-less text that tiny models answer with
/// immediate EOS (0-token replies). Non-ChatML profiles need template
/// metadata before they may join the catalog (ADR-025).
pub fn render_chatml(messages: &[NormalizedMessage]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str("<|im_start|>");
        out.push_str(&message.role);
        out.push('\n');
        out.push_str(&message.content);
        out.push_str("<|im_end|>\n");
    }
    out.push_str("<|im_start|>assistant\n");
    out
}

#[async_trait]
impl InferenceExecutor for SingleLocalExecutor {
    async fn execute(&self, request: NormalizedRequest) -> Result<ExecutorStream, ExecutorError> {
        let runtime = self.runtime.as_ref().ok_or(ExecutorError::NoPeer)?;
        let profile = self.profile_id.as_deref().ok_or(ExecutorError::NoPeer)?;
        if request.profile_id != profile {
            return Err(fatal("profile_mismatch"));
        }

        // The serving executor applies the chat template (ADR-025): clients
        // send plain {role, content} messages; ChatML is rendered exactly
        // once, here.
        let conversation = render_chatml(&request.messages);
        let prompt = runtime
            .tokenize(&conversation)
            .await
            .map_err(|_| fatal("tokenize_failed"))?;
        let handle = runtime
            .load(profile)
            .await
            .map_err(|_| fatal("load_failed"))?;

        let sampling = SamplingParams {
            temperature: request.sampling.temperature,
            top_p: request.sampling.top_p,
            top_k: request.sampling.top_k,
            seed: request.sampling.seed,
        };
        let budget = Duration::from_millis(u64::from(request.deadline_ms.max(1)));
        let started = Instant::now();
        let tokens = match runtime
            .decode_stream(&handle, &prompt, &sampling, request.max_tokens, budget)
            .await
        {
            Ok(tokens) => tokens,
            Err(modelswarm_runtime::RuntimeError::Timeout { .. }) => {
                return Err(fatal("deadline_exceeded"));
            }
            Err(modelswarm_runtime::RuntimeError::Cancelled(_)) => {
                return Err(fatal("cancelled"));
            }
            Err(e) => {
                // The cause must not vanish (a hidden engine failure is
                // indistinguishable from "no peer" at the gateway) — but it
                // must not leak either: a stable code on the error, redacted
                // diagnostics (code/status/length only) in telemetry.
                let (code, status) = runtime_error_code(&e);
                if let Some(t) = &self.telemetry {
                    t.warn(
                        "executor.decode_failed",
                        &[
                            ("code", code),
                            ("status", &status.to_string()),
                            ("detail_bytes", &e.to_string().len().to_string()),
                        ],
                    );
                }
                return Err(fatal(code));
            }
        };

        // Batched event stream (Accepted → TokenDelta×n → Usage → Completed).
        let mut events = Vec::with_capacity(tokens.len() / LOCAL_DELTA_BATCH_TOKENS + 3);
        events.push(ExecutorEvent::Accepted {
            queue_position: 0,
            eta_ms: 0,
        });
        for (index, chunk) in tokens.chunks(LOCAL_DELTA_BATCH_TOKENS).enumerate() {
            let delta = runtime
                .detokenize(chunk)
                .await
                .map_err(|_| fatal("detokenize_failed"))?;
            events.push(ExecutorEvent::TokenDelta {
                delta,
                index: index as u32,
            });
        }
        events.push(ExecutorEvent::Usage {
            prompt_tokens: prompt.len() as u32,
            completion_tokens: tokens.len() as u32,
            prefill_ms: 0.0,
            decode_ms: started.elapsed().as_secs_f64() * 1_000.0,
        });
        let finish_reason = if tokens.len() >= request.max_tokens as usize {
            FinishReason::Length
        } else {
            FinishReason::Stop
        };
        events.push(ExecutorEvent::Completed { finish_reason });
        Ok(Box::pin(stream::iter(events)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelswarm_gateway::{NormalizedMessage, Sampling};
    use modelswarm_runtime::MockRuntime;

    #[test]
    fn chatml_rendering_marks_roles_and_opens_assistant_turn() {
        let messages = vec![
            NormalizedMessage {
                role: "system".to_string(),
                content: "You are terse.".to_string(),
            },
            NormalizedMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
            },
            NormalizedMessage {
                role: "assistant".to_string(),
                content: "Hello.".to_string(),
            },
            NormalizedMessage {
                role: "user".to_string(),
                content: "Bye".to_string(),
            },
        ];
        assert_eq!(
            render_chatml(&messages),
            "<|im_start|>system\nYou are terse.<|im_end|>\n\
             <|im_start|>user\nHi<|im_end|>\n\
             <|im_start|>assistant\nHello.<|im_end|>\n\
             <|im_start|>user\nBye<|im_end|>\n\
             <|im_start|>assistant\n"
        );
    }

    fn request(profile: &str, max_tokens: u32) -> NormalizedRequest {
        NormalizedRequest {
            request_id: "selftest-req".to_string(),
            profile_id: profile.to_string(),
            capability_token: None,
            messages: vec![NormalizedMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            sampling: Sampling {
                temperature: 1.0,
                top_p: 1.0,
                top_k: 40,
                seed: None,
            },
            max_tokens,
            deadline_ms: 30_000,
            stream: true,
        }
    }

    #[tokio::test]
    async fn no_runtime_is_honest_no_peer() {
        let executor = SingleLocalExecutor::new(None, Some("msp1:x".to_string()));
        assert!(matches!(
            executor.execute(request("msp1:x", 4)).await,
            Err(ExecutorError::NoPeer)
        ));
    }

    #[tokio::test]
    async fn wrong_profile_is_fatal_mismatch() {
        let executor = SingleLocalExecutor::new(
            Some(Arc::new(MockRuntime::new(1, 1.0))),
            Some("msp1:x".to_string()),
        );
        match executor.execute(request("msp1:other", 4)).await {
            Err(ExecutorError::Fatal { code }) => assert_eq!(code, "profile_mismatch"),
            Err(other) => panic!("expected fatal profile_mismatch, got {other:?}"),
            Ok(_) => panic!("expected fatal profile_mismatch, got a stream"),
        }
    }

    #[tokio::test]
    async fn event_stream_shape_and_batching() {
        use futures_util::StreamExt;
        let executor = SingleLocalExecutor::new(
            Some(Arc::new(MockRuntime::new(7, 1.0))),
            Some("msp1:x".to_string()),
        );
        let stream = executor
            .execute(request("msp1:x", 40))
            .await
            .map_err(|e| format!("{e:?}"))
            .unwrap();
        let events: Vec<ExecutorEvent> = stream.collect().await;
        assert!(matches!(
            events.first(),
            Some(ExecutorEvent::Accepted { .. })
        ));
        assert!(matches!(
            events.last(),
            Some(ExecutorEvent::Completed { .. })
        ));
        let usage = events
            .iter()
            .rev()
            .find(|e| matches!(e, ExecutorEvent::Usage { .. }));
        let Some(ExecutorEvent::Usage {
            prompt_tokens,
            completion_tokens,
            ..
        }) = usage
        else {
            panic!("usage event missing: {events:?}");
        };
        assert_eq!(*completion_tokens, 40);
        assert!(*prompt_tokens > 0);
        // 40 tokens in batches of 16 → exactly 3 token deltas.
        let deltas = events
            .iter()
            .filter(|e| matches!(e, ExecutorEvent::TokenDelta { .. }))
            .count();
        assert_eq!(deltas, 3);
        let finish = events.last().unwrap();
        assert_eq!(
            *finish,
            ExecutorEvent::Completed {
                finish_reason: FinishReason::Length
            }
        );
    }
}
