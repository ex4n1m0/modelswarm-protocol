//! Typed client for the tracker (`modelswarm.deepflux.space`), per
//! `protocol/msp-v1.md` §2–§5 and ADR-012.
//!
//! Every mutating call carries the signed envelope
//! (`Authorization: MSP1 <base64url(canonical json)>` + `X-MSP-Session`),
//! built with `modelswarm_identity`. The client is content-blind by
//! construction: no method accepts prompt-shaped payloads, and the
//! request types reject unknown fields at serialization boundaries.
//!
//! Phase B scope: transport + signing + core peer lifecycle. Rendezvous
//! mailbox helpers arrive with the Phase D/F wiring that uses them.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use modelswarm_identity::{InstallationIdentity, SignedEnvelope};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::sync::Arc;
use std::time::Duration;

pub const API_PREFIX: &str = "/api/v1";

#[derive(Debug)]
pub enum TrackerError {
    Network(String),
    Http(u16),
    Api {
        status: u16,
        code: String,
        message: String,
        retryable: bool,
    },
    InvalidResponse(String),
}

impl std::fmt::Display for TrackerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackerError::Network(e) => write!(f, "network: {e}"),
            TrackerError::Http(s) => write!(f, "http {s}"),
            TrackerError::Api {
                status,
                code,
                message,
                retryable,
            } => {
                write!(f, "api {status} {code}: {message} (retryable={retryable})")
            }
            TrackerError::InvalidResponse(e) => write!(f, "invalid response: {e}"),
        }
    }
}
impl std::error::Error for TrackerError {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeerEntry {
    #[serde(rename = "peerId")]
    pub peer_id: String,
    pub addresses: Vec<String>,
    #[serde(rename = "queueMs")]
    pub queue_ms: u64,
    #[serde(rename = "freeSlots")]
    pub free_slots: u8,
    #[serde(rename = "leaseExpiresAt")]
    pub lease_expires_at: String,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: String,
    #[serde(rename = "capacityClass", default)]
    pub capacity_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "peerIds")]
    pub peer_ids: Vec<String>,
    #[serde(default, rename = "tokenNonces")]
    pub token_nonces: Vec<String>,
    #[serde(default, rename = "catalogVersion")]
    pub catalog_version: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    #[serde(rename = "leaseExpiresAt")]
    pub lease_expires_at: String,
    #[serde(default)]
    pub notices: Vec<Notice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseIssued {
    pub lease: String, // base64url(json).base64url(sig)
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
}

pub struct TrackerClient {
    base: String,
    identity: Arc<InstallationIdentity>,
    session_token: tokio::sync::RwLock<Option<String>>,
    http: reqwest::Client,
}

impl TrackerClient {
    pub fn new(base_url: impl Into<String>, identity: Arc<InstallationIdentity>) -> Self {
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            identity,
            session_token: tokio::sync::RwLock::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn set_session(&self, token: String) {
        *self.session_token.try_write().expect("session lock") = Some(token);
    }

    /// GET /health — public.
    pub async fn health(&self) -> Result<serde_json::Value, TrackerError> {
        self.get_json(&format!("{API_PREFIX}/health"), None).await
    }

    /// GET /catalog — public; signature verification is the caller's job
    /// (hub pubkey pinning policy per ADR-011).
    pub async fn fetch_catalog(&self) -> Result<serde_json::Value, TrackerError> {
        self.get_json(&format!("{API_PREFIX}/catalog"), None).await
    }

    /// POST /peers/register (signed).
    pub async fn register(
        &self,
        peer_id: &str,
        addresses: &[String],
        profiles: &[String],
        max_slots: u8,
        runtime: serde_json::Value,
    ) -> Result<serde_json::Value, TrackerError> {
        let body = serde_json::json!({
            "peerId": peer_id,
            "addresses": addresses,
            "profiles": profiles,
            "maxSlots": max_slots,
            "runtime": runtime,
        });
        self.signed_json("POST", &format!("{API_PREFIX}/peers/register"), body)
            .await
    }

    /// POST /peers/heartbeat (signed).
    pub async fn heartbeat(
        &self,
        lease_id: &str,
        active_profiles: &[String],
        free_slots: u8,
        queue_ms: u64,
        draining: bool,
    ) -> Result<HeartbeatResponse, TrackerError> {
        let body = serde_json::json!({
            "leaseId": lease_id,
            "activeProfiles": active_profiles,
            "freeSlots": free_slots,
            "queueMs": queue_ms,
            "draining": draining,
        });
        let v = self
            .signed_json("POST", &format!("{API_PREFIX}/peers/heartbeat"), body)
            .await?;
        serde_decode(v)
    }

    /// POST /peers/drain (signed).
    pub async fn drain(&self, lease_id: &str) -> Result<serde_json::Value, TrackerError> {
        self.signed_json(
            "POST",
            &format!("{API_PREFIX}/peers/drain"),
            serde_json::json!({ "leaseId": lease_id }),
        )
        .await
    }

    /// POST /peers/challenge/start (signed).
    pub async fn challenge_start(
        &self,
        lease_id: &str,
        profile_id: &str,
    ) -> Result<serde_json::Value, TrackerError> {
        self.signed_json(
            "POST",
            &format!("{API_PREFIX}/peers/challenge/start"),
            serde_json::json!({ "leaseId": lease_id, "profileId": profile_id }),
        )
        .await
    }

    /// POST /peers/challenge/complete (signed).
    pub async fn challenge_complete(
        &self,
        lease_id: &str,
        profile_id: &str,
        challenge_id: &str,
        first_token_ms: u64,
        total_ms: u64,
    ) -> Result<serde_json::Value, TrackerError> {
        self.signed_json(
            "POST",
            &format!("{API_PREFIX}/peers/challenge/complete"),
            serde_json::json!({
                "leaseId": lease_id,
                "profileId": profile_id,
                "challengeId": challenge_id,
                "timings": { "firstTokenMs": first_token_ms, "totalMs": total_ms },
            }),
        )
        .await
    }

    /// POST /peers/lease (signed) — ADR-12 issuance; returns the
    /// `base64url(json).base64url(sig)` lease token.
    pub async fn request_lease(
        &self,
        lease_id: &str,
        profile_id: &str,
    ) -> Result<LeaseIssued, TrackerError> {
        let v = self
            .signed_json(
                "POST",
                &format!("{API_PREFIX}/peers/lease"),
                serde_json::json!({ "leaseId": lease_id, "profileId": profile_id }),
            )
            .await?;
        serde_decode(v)
    }

    /// GET /peers?profile_id=… (signed, empty-body digest).
    pub async fn lookup(
        &self,
        profile_id: &str,
        limit: u8,
    ) -> Result<Vec<PeerEntry>, TrackerError> {
        let path = format!("{API_PREFIX}/peers?profile_id={profile_id}&limit={limit}");
        let v = self.get_json(&path, Some(("GET", &path))).await?;
        let peers = v
            .get("peers")
            .cloned()
            .ok_or_else(|| TrackerError::InvalidResponse("missing peers".into()))?;
        serde_json::from_value(peers).map_err(|e| TrackerError::InvalidResponse(e.to_string()))
    }

    /// POST /events/job-result (signed).
    pub async fn job_result(
        &self,
        receipt_digest: &str,
        outcome: &str,
    ) -> Result<serde_json::Value, TrackerError> {
        self.signed_json(
            "POST",
            &format!("{API_PREFIX}/events/job-result"),
            serde_json::json!({ "receiptDigest": receipt_digest, "outcome": outcome }),
        )
        .await
    }

    // ---- internals -----------------------------------------------------

    async fn get_json(
        &self,
        path: &str,
        signed: Option<(&str, &str)>,
    ) -> Result<serde_json::Value, TrackerError> {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some((method, _p)) = signed {
            let header = self.build_envelope(method, path, &serde_json::Value::Null)?;
            req = req
                .header("Authorization", format!("MSP1 {header}"))
                .header(
                    "X-MSP-Session",
                    self.session_token
                        .try_read()
                        .expect("session lock")
                        .clone()
                        .unwrap_or_default(),
                );
        }
        Self::finish(req.send().await, path).await
    }

    async fn signed_json(
        &self,
        method: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, TrackerError> {
        let header = self.build_envelope(method, path, &body)?;
        let req = self
            .http
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).expect("method"),
                format!("{}{path}", self.base),
            )
            .json(&body)
            .header("Authorization", format!("MSP1 {header}"))
            .header(
                "X-MSP-Session",
                self.session_token
                    .try_read()
                    .expect("session lock")
                    .clone()
                    .unwrap_or_default(),
            );
        Self::finish(req.send().await, path).await
    }

    fn build_envelope(
        &self,
        method: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<String, TrackerError> {
        let body_digest = format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(
                serde_json::to_vec(body).unwrap_or_default()
            ))
        );
        let envelope = SignedEnvelope::sign(
            &self.identity,
            method,
            path,
            modelswarm_identity::rfc3339_now(),
            modelswarm_identity::new_nonce(),
            body_digest,
        );
        // Header carries the FULL canonical envelope (signature included);
        // the signature itself covers the payload fields without it.
        let value = serde_json::to_value(&envelope)
            .map_err(|e| TrackerError::InvalidResponse(e.to_string()))?;
        let canonical = modelswarm_identity::canonical_json(&value)
            .ok_or_else(|| TrackerError::InvalidResponse("canonical json".into()))?;
        Ok(B64URL.encode(canonical.as_bytes()))
    }

    async fn finish(
        resp: Result<reqwest::Response, reqwest::Error>,
        path: &str,
    ) -> Result<serde_json::Value, TrackerError> {
        let resp = resp.map_err(|e| TrackerError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| TrackerError::Network(e.to_string()))?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| TrackerError::InvalidResponse(format!("{path}: {e}")))?;
        if (200..300).contains(&status) {
            Ok(v)
        } else if let Some(err) = v.get("error").cloned() {
            Err(TrackerError::Api {
                status,
                code: err
                    .get("code")
                    .and_then(|c| c.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                message: err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string(),
                retryable: err
                    .get("retryable")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false),
            })
        } else {
            Err(TrackerError::Http(status))
        }
    }
}

fn serde_decode<T: DeserializeOwned>(v: serde_json::Value) -> Result<T, TrackerError> {
    serde_json::from_value(v).map_err(|e| TrackerError::InvalidResponse(e.to_string()))
}

/// Parses a `base64url(json).base64url(sig)` lease token into its JSON
/// payload (signature verification happens in modelswarm-eligibility).
pub fn split_lease_token(token: &str) -> Result<serde_json::Value, TrackerError> {
    let (payload, _sig) = token
        .split_once('.')
        .ok_or_else(|| TrackerError::InvalidResponse("lease token missing '.'".into()))?;
    let bytes = B64URL
        .decode(payload)
        .map_err(|e| TrackerError::InvalidResponse(e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|e| TrackerError::InvalidResponse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_token_roundtrip() {
        let json = serde_json::json!({"peerId": "p1", "expiresAt": "soon"});
        let token = format!(
            "{}.{}",
            B64URL.encode(json.to_string().as_bytes()),
            B64URL.encode([1u8; 64])
        );
        assert_eq!(split_lease_token(&token).unwrap(), json);
        assert!(split_lease_token("no-dot").is_err());
    }
}
