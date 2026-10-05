//! `LlamaCppAdapter`: the production-baseline runtime (ADR-002/019).
//!
//! A reqwest HTTP client against a llama.cpp-server base URL (loopback,
//! random internal bearer per ADR-002). The trait's load/tokenize/decode
//! paths map onto the server's OpenAI-compatible endpoints:
//!
//! | Trait call | Endpoint |
//! |---|---|
//! | `load` | `GET /v1/models` (reachability + auth check) |
//! | `tokenize` | `POST /tokenize` |
//! | `detokenize` | `POST /detokenize` |
//! | `prefill` | `POST /v1/completions` with `max_tokens: 1` (prompt token array) |
//! | `decode_step` | `POST /v1/completions` (1 token) + `POST /tokenize` for the id |
//! | `decode_stream` | default trait impl: repeated `decode_step` under the deadline |
//! | `cancel` | best-effort `POST /cancel`; 404/405 tolerated (the server also cancels on client abort) |
//!
//! Prefill/decode throughput metrics are derived from the response `usage`
//! fields and client-observed elapsed time (documented approximation: the
//! pinned server build does not expose per-phase timings on every route).
//! Not feature-gated: this adapter only ever talks HTTP; binary download and
//! process supervision live elsewhere (ADR-019).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Handle, KvCommitment, RuntimeDescriptor, RuntimeError, RuntimeMetrics, SamplingParams, TaskId,
};

/// Builder-style configuration for [`LlamaCppAdapter`].
#[derive(Debug, Clone)]
pub struct LlamaCppConfig {
    /// Base URL of the llama.cpp server, e.g. `http://127.0.0.1:8123`.
    pub base_url: String,
    /// Random internal bearer secret (ADR-002); sent as `Authorization: Bearer …`.
    pub bearer_token: String,
    /// Per-request timeout for adapter calls. Decode paths compute the
    /// remaining budget from the caller's deadline, bounded by this value.
    pub request_timeout: Duration,
}

impl LlamaCppConfig {
    /// Configuration with defaults for everything but the endpoint identity.
    pub fn new(base_url: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            bearer_token: bearer_token.into(),
            request_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug)]
struct Shared {
    metrics: RwLock<RuntimeMetrics>,
    next_handle: AtomicU64,
}

/// HTTP-backed [`crate::InferenceRuntime`] for the pinned llama.cpp server.
#[derive(Debug)]
pub struct LlamaCppAdapter {
    config: LlamaCppConfig,
    client: reqwest::Client,
    shared: Shared,
}

impl LlamaCppAdapter {
    /// Creates the adapter (loopback base URL + bearer secret).
    pub fn new(config: LlamaCppConfig) -> Result<Self, RuntimeError> {
        let client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|e| RuntimeError::Http(format!("client build failed: {e}")))?;
        Ok(Self {
            config,
            client,
            shared: Shared {
                metrics: RwLock::new(RuntimeMetrics::default()),
                next_handle: AtomicU64::new(1),
            },
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url.trim_end_matches('/'), path)
    }

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &serde_json::Value,
        timeout: Duration,
    ) -> Result<T, RuntimeError> {
        let started = Instant::now();
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.config.bearer_token)
            .timeout(timeout)
            .json(body)
            .send()
            .await;
        let response = map_reqwest(response, started)?;
        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(RuntimeError::Api {
                status: code,
                code: extract_error_code(&text),
                message: text.chars().take(300).collect(),
            });
        }
        let raw = response
            .text()
            .await
            .map_err(|e| RuntimeError::Http(format!("body read failed: {e}")))?;
        serde_json::from_str(&raw)
            .map_err(|e| RuntimeError::MalformedResponse(format!("{path}: {e}")))
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, RuntimeError> {
        let started = Instant::now();
        let response = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.config.bearer_token)
            .send()
            .await;
        let response = map_reqwest(response, started)?;
        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(RuntimeError::Api {
                status: code,
                code: extract_error_code(&text),
                message: text.chars().take(300).collect(),
            });
        }
        let raw = response
            .text()
            .await
            .map_err(|e| RuntimeError::Http(format!("body read failed: {e}")))?;
        serde_json::from_str(&raw)
            .map_err(|e| RuntimeError::MalformedResponse(format!("{path}: {e}")))
    }

    fn record_prefill_rate(&self, prompt_tokens: u64, elapsed: Duration) {
        if prompt_tokens > 0 {
            if let Some(rate) = tokens_per_ms(prompt_tokens, elapsed) {
                if let Ok(mut metrics) = self.shared.metrics.write() {
                    metrics.prefill_tokens_per_ms = rate;
                }
            }
        }
    }

    fn record_decode_rate(&self, tokens: u64, elapsed: Duration) {
        if tokens > 0 {
            if let Some(rate) = tokens_per_ms(tokens, elapsed) {
                if let Ok(mut metrics) = self.shared.metrics.write() {
                    metrics.decode_tokens_per_ms = rate;
                }
            }
        }
    }
}

fn map_reqwest(
    result: Result<reqwest::Response, reqwest::Error>,
    started: Instant,
) -> Result<reqwest::Response, RuntimeError> {
    match result {
        Ok(response) => Ok(response),
        Err(e) => Err(reqwest_error(e, started)),
    }
}

fn reqwest_error(e: reqwest::Error, started: Instant) -> RuntimeError {
    if e.is_timeout() {
        RuntimeError::Timeout {
            after_ms: crate::duration_ms_u64(started.elapsed()),
        }
    } else {
        RuntimeError::Http(e.to_string())
    }
}

/// Tokens per millisecond with sub-millisecond resolution; `None` when the
/// elapsed time is indistinguishable from zero.
fn tokens_per_ms(tokens: u64, elapsed: Duration) -> Option<f64> {
    let micros = elapsed.as_micros();
    if micros > 0 {
        Some(tokens as f64 / (micros as f64 / 1000.0))
    } else {
        None
    }
}

fn extract_error_code(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["code"]
                .as_str()
                .map(str::to_string)
                .or_else(|| v["error"].as_str().map(str::to_string))
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    #[allow(dead_code)] // shape verification only; ids are llama-internal names
    data: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TokenizeRequest {
    content: String,
}

#[derive(Debug, Deserialize)]
struct TokenizeResponse {
    tokens: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct DetokenizeResponse {
    content: String,
}

#[derive(Debug, Deserialize)]
struct CompletionsResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    text: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    /// Parsed for shape validation and future decode-rate attribution.
    #[serde(default)]
    #[allow(dead_code)]
    completion_tokens: u64,
}

#[async_trait]
impl crate::InferenceRuntime for LlamaCppAdapter {
    fn id(&self) -> RuntimeDescriptor {
        // The pinned build hash is supplied by the node supervisor; the
        // adapter itself only knows its endpoint, so it reports a zero
        // digest-derived placeholder identity that the node re-stamps.
        RuntimeDescriptor::new("llama.cpp", "adapter-http-v1", "0".repeat(64))
            .expect("static descriptor is valid")
    }

    async fn load(&self, profile_id: &str) -> Result<Handle, RuntimeError> {
        if profile_id.is_empty() {
            return Err(RuntimeError::InvalidProfile(profile_id.to_string()));
        }
        // Reachability + auth probe; the llama-internal model id is not the
        // MSP profile id, so we do not match on it here.
        let _models: ModelsResponse = self.get_json("/v1/models").await?;
        let handle_id = self.shared.next_handle.fetch_add(1, Ordering::Relaxed);
        Ok(Handle::new(profile_id, handle_id))
    }

    async fn tokenize(&self, text: &str) -> Result<Vec<u32>, RuntimeError> {
        let response: TokenizeResponse = self
            .post_json(
                "/tokenize",
                &json!(TokenizeRequest {
                    content: text.to_string()
                }),
                self.config.request_timeout,
            )
            .await?;
        Ok(response.tokens)
    }

    async fn detokenize(&self, ids: &[u32]) -> Result<String, RuntimeError> {
        let response: DetokenizeResponse = self
            .post_json(
                "/detokenize",
                &json!({ "tokens": ids }),
                self.config.request_timeout,
            )
            .await?;
        Ok(response.content)
    }

    async fn prefill(&self, _handle: &Handle, ids: &[u32]) -> Result<KvCommitment, RuntimeError> {
        if ids.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "prefill of empty token span".to_string(),
            ));
        }
        let started = Instant::now();
        let response: CompletionsResponse = self
            .post_json(
                "/v1/completions",
                &json!({
                    "prompt": ids,
                    "max_tokens": 1,
                    "stream": false,
                }),
                self.config.request_timeout,
            )
            .await?;
        let prompt_tokens = response
            .usage
            .as_ref()
            .map(|u| u.prompt_tokens)
            .unwrap_or(ids.len() as u64);
        self.record_prefill_rate(prompt_tokens, started.elapsed());
        Ok(KvCommitment::new(0, ids))
    }

    async fn decode_step(
        &self,
        _handle: &Handle,
        prefix: &[u32],
        sampling: &SamplingParams,
    ) -> Result<u32, RuntimeError> {
        if prefix.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "decode_step with empty prefix; prefill first".to_string(),
            ));
        }
        let started = Instant::now();
        let response: CompletionsResponse = self
            .post_json(
                "/v1/completions",
                &json!({
                    "prompt": prefix,
                    "max_tokens": 1,
                    "stream": false,
                    "temperature": sampling.temperature,
                    "top_p": sampling.top_p,
                    "top_k": sampling.top_k,
                }),
                self.config.request_timeout,
            )
            .await?;
        let text = response
            .choices
            .first()
            .ok_or_else(|| {
                RuntimeError::MalformedResponse("completions response had no choices".to_string())
            })?
            .text
            .clone();
        if text.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "completions response produced empty text".to_string(),
            ));
        }
        // Recover the token id deterministically through the tokenizer.
        let tokens: TokenizeResponse = self
            .post_json(
                "/tokenize",
                &json!(TokenizeRequest { content: text }),
                self.config.request_timeout,
            )
            .await?;
        self.record_decode_rate(1, started.elapsed());
        tokens
            .tokens
            .first()
            .copied()
            .ok_or_else(|| RuntimeError::MalformedResponse("tokenize returned no ids".to_string()))
    }

    fn metrics(&self) -> RuntimeMetrics {
        self.shared.metrics.read().map(|m| *m).unwrap_or_default()
    }

    fn cancel(&self, task: TaskId) -> Result<(), RuntimeError> {
        // Best-effort synchronous cancel: the pinned server also cancels
        // work when the requesting connection aborts, so 404/405 from builds
        // without POST /cancel is success for our purposes. The trait method
        // is sync, so the one-shot HTTP call runs on a helper thread with a
        // throwaway current-thread runtime (rare control-path call).
        let url = self.url("/cancel");
        let bearer = self.config.bearer_token.clone();
        let body = json!({ "task_id": task.0 });
        let outcome = std::thread::spawn(move || -> Result<(), RuntimeError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .map_err(|e| RuntimeError::Http(format!("cancel runtime: {e}")))?;
            runtime.block_on(async move {
                let started = Instant::now();
                // A fresh client is required here: the adapter's shared
                // client is bound to whichever async runtime created its
                // connection pool first, and this helper thread has its own.
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .map_err(|e| RuntimeError::Http(format!("cancel client: {e}")))?;
                let request = client
                    .post(url)
                    .bearer_auth(bearer)
                    .json(&body)
                    .build()
                    .map_err(|e| RuntimeError::Http(format!("cancel build failed: {e}")))?;
                match client.execute(request).await {
                    // 404/405 means this build has no POST /cancel: the
                    // server's abort-based cancellation applies, which is
                    // what dropping the request connection already achieved.
                    Ok(_) => Ok(()),
                    Err(e) => Err(reqwest_error(e, started)),
                }
            })
        })
        .join()
        .map_err(|_| RuntimeError::Http("cancel thread panicked".to_string()))?;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InferenceRuntime;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Canned answer for the mini llama.cpp mock server.
    #[derive(Debug, Clone)]
    struct Canned {
        status: u16,
        body: String,
        delay: Option<Duration>,
    }

    impl Canned {
        fn ok(body: impl Into<String>) -> Self {
            Self {
                status: 200,
                body: body.into(),
                delay: None,
            }
        }
        fn err(status: u16, body: impl Into<String>) -> Self {
            Self {
                status,
                body: body.into(),
                delay: None,
            }
        }
    }

    /// Responder closure type for the canned server.
    type CannedResponder = Arc<Mutex<dyn FnMut(&str) -> Canned + Send>>;

    /// A raw-socket HTTP/1.1 server serving canned responses chosen by a
    /// closure that sees the full request text. Lightest possible double:
    /// no axum, no hyper, one accept loop.
    async fn start_canned_server(
        responder: CannedResponder,
    ) -> std::io::Result<std::net::SocketAddr> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let responder = responder.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::with_capacity(8 * 1024);
                    let mut chunk = [0u8; 4 * 1024];
                    loop {
                        // Read one full request (headers + content-length body).
                        let request = loop {
                            let n = match socket.read(&mut chunk).await {
                                Ok(0) => return,
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            buffer.extend_from_slice(&chunk[..n]);
                            if let Some(header_end) = find_header_end(&buffer) {
                                let headers =
                                    String::from_utf8_lossy(&buffer[..header_end]).to_uppercase();
                                let cl = headers
                                    .lines()
                                    .find_map(|l| l.strip_prefix("CONTENT-LENGTH:"))
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                                    .unwrap_or(0);
                                if buffer.len() >= header_end + 4 + cl {
                                    break buffer[..header_end + 4 + cl].to_vec();
                                }
                            }
                        };
                        let canned = (responder.lock().expect("responder"))(
                            &String::from_utf8_lossy(&request),
                        );
                        if let Some(delay) = canned.delay {
                            tokio::time::sleep(delay).await;
                        }
                        let reason = match canned.status {
                            200 => "OK",
                            401 => "Unauthorized",
                            404 => "Not Found",
                            500 => "Internal Server Error",
                            503 => "Service Unavailable",
                            _ => "Status",
                        };
                        let response = format!(
                            "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            canned.status,
                            reason,
                            canned.body.len(),
                            canned.body
                        );
                        if socket.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                        buffer.clear();
                    }
                });
            }
        });
        Ok(addr)
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn adapter(addr: std::net::SocketAddr, timeout: Duration) -> LlamaCppAdapter {
        LlamaCppAdapter::new(LlamaCppConfig {
            base_url: format!("http://{addr}"),
            bearer_token: "internal-secret".to_string(),
            request_timeout: timeout,
        })
        .unwrap()
    }

    const TOK_TOKENS: &str = r#"{"tokens":[72,101,108,108,111]}"#;
    const COMPLETIONS_A: &str =
        r#"{"choices":[{"text":"A"}],"usage":{"prompt_tokens":5,"completion_tokens":1}}"#;
    const MODELS: &str = r#"{"data":[{"id":"tiny.gguf"}]}"#;

    #[tokio::test]
    async fn load_tokenizes_and_steps_through_the_mock_server() {
        let seen_bearer = Arc::new(AtomicUsize::new(0));
        let seen_bearer_clone = seen_bearer.clone();
        let completions = Arc::new(Mutex::new(Vec::<Canned>::new()));
        let completions_for_server = completions.clone();
        // Queue: load(models), tokenize, prefill(completions), step(completions), step tokenize
        completions.lock().unwrap().push(Canned::ok(MODELS));
        completions.lock().unwrap().push(Canned::ok(TOK_TOKENS));
        completions.lock().unwrap().push(Canned::ok(COMPLETIONS_A));
        completions.lock().unwrap().push(Canned::ok(COMPLETIONS_A));
        completions.lock().unwrap().push(Canned::ok(TOK_TOKENS));

        let addr = start_canned_server(Arc::new(Mutex::new(move |request: &str| {
            if request
                .to_ascii_lowercase()
                .contains("authorization: bearer internal-secret")
            {
                seen_bearer_clone.fetch_add(1, Ordering::Relaxed);
            }
            completions_for_server.lock().unwrap().remove(0)
        })))
        .await
        .unwrap();

        let runtime = adapter(addr, Duration::from_secs(5));
        let handle = runtime.load("msp1:aa").await.unwrap();
        assert_eq!(handle.profile_id(), "msp1:aa");

        let ids = runtime.tokenize("Hello").await.unwrap();
        assert_eq!(ids, vec![72, 101, 108, 108, 111]);

        let commitment = runtime.prefill(&handle, &ids).await.unwrap();
        assert_eq!(commitment.token_span, (0, 5));
        assert_eq!(commitment.digest.len(), 64);

        let sampling = SamplingParams::default();
        let token = runtime.decode_step(&handle, &ids, &sampling).await.unwrap();
        assert_eq!(token, 72); // first token id of the canned "A"→tokenize reply

        let metrics = runtime.metrics();
        assert!(metrics.prefill_tokens_per_ms > 0.0);
        assert!(metrics.decode_tokens_per_ms > 0.0);
        assert!(seen_bearer.load(Ordering::Relaxed) >= 5);
    }

    #[tokio::test]
    async fn detokenize_round_trips_through_the_server() {
        let queue = Arc::new(Mutex::new(Vec::<Canned>::new()));
        queue
            .lock()
            .unwrap()
            .push(Canned::ok(r#"{"content":"Hello"}"#));
        let queue_server = queue.clone();
        let addr = start_canned_server(Arc::new(Mutex::new(move |_req: &str| {
            queue_server.lock().unwrap().remove(0)
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_secs(5));
        assert_eq!(runtime.detokenize(&[72, 101]).await.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn error_status_maps_to_api_error() {
        let addr = start_canned_server(Arc::new(Mutex::new(|_req: &str| {
            Canned::err(
                401,
                r#"{"error":{"code":"unauthorized","message":"bad bearer"}}"#,
            )
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_secs(5));
        match runtime.load("msp1:aa").await.unwrap_err() {
            RuntimeError::Api { status, code, .. } => {
                assert_eq!(status, 401);
                assert_eq!(code, "unauthorized");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_body_maps_to_malformed_response() {
        let addr = start_canned_server(Arc::new(Mutex::new(|_req: &str| {
            Canned::ok("this is not json")
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_secs(5));
        assert!(matches!(
            runtime.load("msp1:aa").await.unwrap_err(),
            RuntimeError::MalformedResponse(_)
        ));
    }

    #[tokio::test]
    async fn connection_refused_maps_to_http_error() {
        // Bind then immediately drop the listener to get a free-but-closed port.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let runtime = adapter(addr, Duration::from_secs(2));
        let err = runtime.load("msp1:aa").await.unwrap_err();
        // On this Windows stack a closed loopback port can surface either as
        // a connect error or as a connect timeout; both are transport-class
        // failures and must not be Malformed/Api.
        assert!(
            matches!(err, RuntimeError::Http(_) | RuntimeError::Timeout { .. }),
            "expected transport-class error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn delayed_server_maps_to_timeout() {
        let addr = start_canned_server(Arc::new(Mutex::new(|_req: &str| Canned {
            status: 200,
            body: MODELS.to_string(),
            delay: Some(Duration::from_secs(3)),
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_millis(150));
        let started = Instant::now();
        match runtime.load("msp1:aa").await.unwrap_err() {
            RuntimeError::Timeout { after_ms } => {
                assert!(after_ms >= 100, "timeout after {after_ms} ms");
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn decode_stream_steps_until_max_tokens() {
        // Server answers: models, then for each step one completions + one
        // tokenize. We make the responder stateless by cycling on path.
        let step = Arc::new(AtomicUsize::new(0));
        let step_server = step.clone();
        let addr = start_canned_server(Arc::new(Mutex::new(move |req: &str| {
            let first = req.split_whitespace().nth(1).unwrap_or("");
            if first.starts_with("/v1/completions") {
                step_server.fetch_add(1, Ordering::Relaxed);
                Canned::ok(COMPLETIONS_A)
            } else if first == "/tokenize" {
                Canned::ok(TOK_TOKENS)
            } else {
                Canned::ok(MODELS)
            }
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_secs(5));
        let handle = runtime.load("msp1:aa").await.unwrap();
        let out = runtime
            .decode_stream(
                &handle,
                &[1, 2, 3],
                &SamplingParams::default(),
                4,
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(out, vec![72, 72, 72, 72]);
        assert_eq!(step.load(Ordering::Relaxed), 4);
    }

    // Multi-thread flavor: `cancel` blocks the calling thread on a helper
    // OS thread doing the HTTP round-trip, and the canned server task runs
    // on this test's runtime — it must keep being driven meanwhile.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_tolerates_missing_endpoint() {
        let addr = start_canned_server(Arc::new(Mutex::new(|_req: &str| {
            Canned::err(404, r#"{"error":{"code":"not_found"}}"#)
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_secs(5));
        runtime.cancel(TaskId(7)).unwrap();
    }

    #[tokio::test]
    async fn empty_choices_maps_to_malformed_response() {
        let queue = Arc::new(Mutex::new(Vec::<Canned>::new()));
        queue.lock().unwrap().push(Canned::ok(MODELS));
        queue.lock().unwrap().push(Canned::ok(r#"{"choices":[]}"#));
        let queue_server = queue.clone();
        let addr = start_canned_server(Arc::new(Mutex::new(move |_req: &str| {
            queue_server.lock().unwrap().remove(0)
        })))
        .await
        .unwrap();
        let runtime = adapter(addr, Duration::from_secs(5));
        let handle = runtime.load("msp1:aa").await.unwrap();
        assert!(matches!(
            runtime
                .decode_step(&handle, &[1], &SamplingParams::default())
                .await
                .unwrap_err(),
            RuntimeError::MalformedResponse(_)
        ));
    }
}
