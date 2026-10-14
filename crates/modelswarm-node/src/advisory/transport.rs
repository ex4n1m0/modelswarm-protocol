//! The advisory transport seam: an injectable trait (the state machine's
//! doubles implement it; the pinned reqwest adapter is the production
//! implementation) and the ADR-033 §3.3 transport pinning (RC-9):
//! **https-only, redirects NEVER followed**.
//!
//! The state logic ([`crate::advisory::state`]) has NO reqwest
//! dependency and never performs I/O — a CALL is an `Effect`; the runner
//! executes it through this seam and feeds the outcome back. A 3xx
//! response is `HttpStatus(3xx)` (gap `http_error`): the request —
//! including the Authorization header — is never re-sent to any other
//! host.
//!
//! Key handling: the key reaches ONLY the Authorization header, added
//! here from the runner-supplied value. It never appears in a request
//! payload (the egress allowlist has no key-bearing field type), in
//! state, or in telemetry (the Redactor `sk-` scrub). No rotation
//! anywhere (ADR-037 §3.6 standing owner directive).

use async_trait::async_trait;

use super::state::{ProviderRequest, TransportFailure};

/// One advisory HTTPS call. Implementations return the model's reply
/// CONTENT bytes on HTTP 2xx (the provider envelope is stripped by the
/// implementation; the pure state machine parses only the strict
/// suggestion schema).
#[async_trait]
pub trait AdvisoryTransport: Send + Sync + std::fmt::Debug {
    async fn send(&self, request: &ProviderRequest, key: &str)
        -> Result<Vec<u8>, TransportFailure>;
}

/// The pinned reqwest client: **no redirects** (a 3xx comes back as the
/// response; the caller maps it to gap `http_error`), rustls TLS. The
/// SAME builder serves the adapter and the redirect tests, so the pin
/// cannot drift between them.
pub fn pinned_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client")
}

/// The production transport: https-only (the constructor refuses any
/// non-`https://` endpoint), no redirects, bearer key in the
/// Authorization header only. The endpoint AND the model id are pinned
/// HERE at construction (from `advisory.json`) — a request cannot name
/// another host or model.
#[derive(Debug)]
pub struct ReqwestTransport {
    http: reqwest::Client,
    endpoint_url: String,
    model: String,
}

impl ReqwestTransport {
    /// Fails for a non-https endpoint (ADR-033 §3.3 — the caller records
    /// gap `no_config`; the client never speaks plain http).
    pub fn new(endpoint_url: &str, model: &str) -> Result<Self, TransportFailure> {
        if !super::config::is_https_endpoint(endpoint_url) {
            return Err(TransportFailure::Network(format!(
                "endpoint must be https://, refusing: {endpoint_url:?}"
            )));
        }
        Ok(Self {
            http: pinned_client(),
            endpoint_url: endpoint_url.to_string(),
            model: model.to_string(),
        })
    }
}

/// The fixed, repo-versioned digest system prompt (ADR-033 §3.2): no
/// secrets, no user content, and it states the metadata-only contract.
/// The allowlist runs regardless of the prompt — the structural gate,
/// not prompt compliance, is the defense.
pub const DIGEST_SYSTEM_PROMPT: &str = "\
You are the management brain of a decentralized inference swarm. You \
receive ONLY operational metadata: counts, integer timings, frozen \
reason codes, and 16-hex peer/profile prefixes. You never receive or \
request prompts, completions, chat content, token ids, keys, or file \
paths. Reply with ONE JSON object exactly of the shape \
{\"schema\":\"advisory-suggestion/v1\",\"cycle_id\":\"<echo the given \
cycle_id>\",\"suggestions\":[{\"type\":\"triage_summary|tuning_hint|\
catalog_note|digest\",\"target\":\"fleet or a 16-hex profile \
prefix\",\"hint\":{parameter:number},\"confidence\":\"low|medium|high\",\
\"rationale\":\"short display text\"}]}. Numeric hint parameters may \
only be MSP_SHADOW_QUEUE_ADVERTISED_WEIGHT, \
MSP_SHADOW_QUEUE_MEASURED_WEIGHT, or MSP_SHADOW_QUEUE_CAP_MS; anything \
else is recorded as requiring an architecture decision. Your output is \
advice only: it never changes any protocol decision.";

#[async_trait]
impl AdvisoryTransport for ReqwestTransport {
    async fn send(
        &self,
        request: &ProviderRequest,
        key: &str,
    ) -> Result<Vec<u8>, TransportFailure> {
        // The system prompt is a repo-versioned constant pinned to the
        // plane (`digest` default, the assistant plane's override) —
        // never caller text.
        let system = request.system_prompt.unwrap_or(DIGEST_SYSTEM_PROMPT);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": request.body},
            ],
            "max_tokens": request.max_completion_tokens,
            "stream": false,
        });
        let result = self
            .http
            .post(&self.endpoint_url)
            .timeout(std::time::Duration::from_millis(request.timeout_ms))
            .bearer_auth(key)
            .json(&body)
            .send()
            .await;
        let response = match result {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(TransportFailure::Deadline),
            Err(e) => return Err(TransportFailure::Network(e.to_string())),
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // 3xx included: no redirect is ever followed (RC-9) — the
            // request, Authorization included, dies here.
            return Err(TransportFailure::HttpStatus(status));
        }
        // Response body cap: stream and refuse to buffer beyond the
        // request's cap (the state machine re-checks the final bytes).
        let mut bytes = Vec::new();
        let mut stream = response;
        while let Some(chunk) = stream
            .chunk()
            .await
            .map_err(|e| TransportFailure::Network(e.to_string()))?
        {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > request.max_response_bytes {
                return Err(TransportFailure::ResponseOversized);
            }
        }
        // The provider envelope is stripped here: choices[0].message.content.
        let parsed: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| TransportFailure::Network(format!("provider envelope: {e}")))?;
        let content = parsed
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| TransportFailure::Network("provider envelope: no content".into()))?;
        Ok(content.as_bytes().to_vec())
    }
}

/// A counting transport double (TEST-ONLY; the chat-wall and cadence
/// suites ride it — production code never constructs it). Records every
/// attempted call and replays a programmed outcome per call.
#[derive(Debug)]
pub struct CountingTransport {
    calls: std::sync::Mutex<Vec<ProviderRequest>>,
    keys: std::sync::Mutex<Vec<String>>,
    /// Replayed per call in order; the last entry repeats.
    script: std::sync::Mutex<Vec<Result<Vec<u8>, TransportFailure>>>,
    /// When set, every call is answered with a valid assistant reply
    /// echoing the request's turn (overrides the script).
    assistant_echo: Option<String>,
}

impl CountingTransport {
    pub fn new(script: Vec<Result<Vec<u8>, TransportFailure>>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            keys: std::sync::Mutex::new(Vec::new()),
            script: std::sync::Mutex::new(script),
            assistant_echo: None,
        }
    }

    /// A double that always succeeds with a minimal valid digest reply
    /// echoing whatever cycle it is asked about (the parse happens in
    /// the state machine; content here is just bytes).
    pub fn always_ok() -> Self {
        Self::new(vec![Ok(br#"{"schema":"advisory-suggestion/v1"}"#.to_vec())])
    }

    /// A double that always fails at the network layer.
    pub fn always_erroring() -> Self {
        Self::new(vec![Err(TransportFailure::Network("double".into()))])
    }

    /// A double that answers every call with a VALID assistant reply
    /// (schema `advisory-assistant/v1`) echoing the request's
    /// conversation_id/turn — the assistant panel's success-path tests
    /// ride it (the plane's conversation id is internally generated, so
    /// a canned body could never echo it).
    pub fn assistant_echo(reply_text: String) -> Self {
        Self::new(vec![Err(TransportFailure::Network(
            "placeholder, replaced in send".into(),
        ))])
        .with_assistant_echo(reply_text)
    }

    fn with_assistant_echo(mut self, reply_text: String) -> Self {
        self.assistant_echo = Some(reply_text);
        self
    }

    /// Number of calls INITIATED through this double.
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Every request body sent (chat-wall canary assertions walk these).
    pub fn bodies(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.body.clone())
            .collect()
    }

    /// The keys presented per call (the key must appear ONLY in the
    /// header; the double captures it for the wall assertions).
    pub fn presented_keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }
}

#[async_trait]
impl AdvisoryTransport for CountingTransport {
    async fn send(
        &self,
        request: &ProviderRequest,
        key: &str,
    ) -> Result<Vec<u8>, TransportFailure> {
        self.calls.lock().unwrap().push(request.clone());
        self.keys.lock().unwrap().push(key.to_string());
        if let Some(reply) = &self.assistant_echo {
            // Echo the request's own turn (a canned body could not know
            // the plane's internally-generated conversation id).
            let body: serde_json::Value = serde_json::from_str(&request.body)
                .map_err(|e| TransportFailure::Network(format!("echo double: {e}")))?;
            let turn = body
                .get("turn")
                .cloned()
                .ok_or_else(|| TransportFailure::Network("echo double: no turn".into()))?;
            return Ok(serde_json::json!({
                "schema": "advisory-assistant/v1",
                "conversation_id": turn["conversation_id"],
                "turn": turn["turn"],
                "reply": reply,
                "hints": [],
            })
            .to_string()
            .into_bytes());
        }
        let mut script = self.script.lock().unwrap();
        if script.is_empty() {
            return Err(TransportFailure::Network("script exhausted".into()));
        }
        if script.len() > 1 {
            script.remove(0)
        } else {
            script[0].clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_refuses_non_https_endpoints() {
        assert!(ReqwestTransport::new("https://api.example.test/v1", "model-x").is_ok());
        for refused in [
            "http://api.example.test/v1",
            "ftp://x",
            "https://",
            "",
            "https://evil.example/@other",
        ] {
            assert!(
                ReqwestTransport::new(refused, "model-x").is_err(),
                "{refused}"
            );
        }
    }

    /// RC-9 (mock venue; live-endpoint confirmation is owner-local
    /// only): the pinned client does NOT follow redirects — a 302 comes
    /// back as the 302. Served over plain http against a loopback
    /// listener purely to exercise the client's redirect policy; the
    /// ADAPTER itself still refuses non-https endpoints (test above).
    #[tokio::test]
    async fn pinned_client_never_follows_redirects() {
        use axum::routing::get;
        let app =
            axum::Router::new().route(
                "/redirect",
                get(|| async move {
                    axum::response::Redirect::temporary("http://127.0.0.1:1/elsewhere")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let client = pinned_client();
        let response = client
            .get(format!("http://{addr}/redirect"))
            .send()
            .await
            .expect("redirect response arrives");
        assert_eq!(
            response.status().as_u16(),
            307,
            "the 3xx is returned, not followed"
        );
        assert!(response.headers().get("location").is_some());
    }
}
