//! ADR-032 §1 — the batched speculative-verification family: the
//! `VerifyDrafts` / `VerifyDraftsResult` wire types, their strict
//! two-pass deserialization, and the family's byte-rule helpers.
//!
//! This module is the **type + serialization layer only** (implementation
//! plan §1 / step 1). Verifier-side admission and the state machine
//! (§3), the coordinator side (§4), and the golden-vector fixtures (§2)
//! land in their own sessions; nothing here talks to an engine.
//!
//! Wire form: both messages serialize as the BARE objects of ADR-032 §1 —
//! they are NOT new `SpecMessage` variants (no external tag) — and ride
//! the same `Session::send_json`/`recv_json` path as `SpecMessage`
//! (ADR-018 staging), each as one JSON frame, in the
//! `msp-cooperative-v1` namespace.
//!
//! # Byte rules pinned here (ADR-032 §1, rules 1–5)
//!
//! 1. **Hash wire form:** every hash in this family is BARE lowercase
//!    64-hex ([`is_bare_hash`]). The `"sha256:<…>"` prefixed notation is
//!    hub `bodyDigest` prose and MUST NOT appear on the wire.
//! 2. **Prefix-hash preimage:** `new_prefix_hash` is the shipped
//!    [`crate::compute_prefix_hash`] over `(parent ‖ accepted prefix ‖
//!    [correction])` with [`crate::GENESIS_PREFIX_HASH`] at the root. No
//!    new hash function exists in this module.
//! 3. **`generation_params_hash` preimage:** NEW canonical encoding
//!    ([`batch_generation_params_hash`]): sha256 over canonical JSON of
//!    `{"seed":<null|integer>,"temperature":<integer>,"top_k":<u32>,
//!    "top_p":<integer>}`. Integral `f32` values render as bare
//!    integers; a non-integral value fails closed
//!    ([`ParamsHashError::NonIntegral`]). The shipped
//!    [`crate::spec::default_sampling_params_hash`] (Rust `Debug` string)
//!    is the OLD vocabulary's internal value and is NOT this family's
//!    preimage.
//! 4. **Signature preimage:** canonical JSON of the message with the
//!    `signature` field OMITTED ENTIRELY ([`signing_preimage`]) — not
//!    null, not empty string. Every other declared field, including
//!    null-valued `divergence_point` and an EOG `correction`, is PRESENT.
//!    Floats are forbidden in signed payloads (msp-v1 §2.2;
//!    `modelswarm_types::canonical_json` rejects any float).
//! 5. **Sender binding:** `sender` MUST equal
//!    [`installation_id_for`](modelswarm_identity::installation_id_for)
//!    of the verifying key AND the QUIC-authenticated remote PeerId
//!    (msp-v1 §6.6). [`verify_verify_drafts_signature`] and
//!    [`verify_verify_drafts_result_signature`] enforce the key
//!    derivation FIRST — the `verify_commit_signature` discipline; the
//!    remote-PeerId equality is enforced by the serve loop, the only
//!    place that knows the stream's peer id.
//!
//! Signatures are base64 STANDARD (the alphabet of every existing
//! cooperative signature; base64url is the hub lease-token convention,
//! not this wire).

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use modelswarm_identity::{installation_id_for, InstallationIdentity};
use modelswarm_runtime::SamplingParams;
use modelswarm_transport::ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use modelswarm_types::{canonical_json, CanonicalError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::SpecError;

/// The cooperative namespace version every family message carries
/// (must equal `"msp-cooperative-v1"`).
pub const COOPERATIVE_PROTOCOL_VERSION: &str = "msp-cooperative-v1";

/// Hard cap on `window_tokens` length per `VerifyDrafts` (ADR-032 §1,
/// frozen). A window longer than `min(WINDOW_CAP, the peer's
/// `batch_window_max`)` is `payload_too_large` BEFORE any engine work.
pub const WINDOW_CAP: u32 = 32;

/// Lower bound of the server-side `deadline_ms` clamp (ADR-032 §2, the
/// msp-v1 §6.4 discipline: clamp, not reject).
pub const DEADLINE_CLAMP_MIN_MS: u32 = 1_000;
/// Upper bound of the server-side `deadline_ms` clamp (msp-v1 §6.4).
pub const DEADLINE_CLAMP_MAX_MS: u32 = 120_000;

/// Fixed WAIT_COMMIT grace past the outstanding request's `deadline_ms`
/// before the round is abandoned (ADR-032 §8 RC-5(b)).
pub const WAIT_COMMIT_GRACE_MS: u64 = 250;

/// Allowed clock noise in the `verify_pass_ms` cross-check: a claim above
/// observed round wall time + this is `verify_ms_lie` telemetry
/// (ADR-032 §1).
pub const VERIFY_PASS_CLOCK_NOISE_MS: u64 = 25;

/// A second protocol violation by the same peer within one session tears
/// the session down (ADR-032 §2, RC-3 tail).
pub const TEARDOWN_VIOLATION_THRESHOLD: u32 = 2;

// ---------------------------------------------------------------------------
// Wire messages (ADR-032 §1, field-for-field)
// ---------------------------------------------------------------------------

/// Declared field set of [`VerifyDrafts`] — the two-pass key-set equality
/// check treats a missing OR unknown key as a schema failure (`bad_frame`).
pub const VERIFY_DRAFTS_FIELDS: [&str; 13] = [
    "session_id",
    "protocol_version",
    "seq",
    "round",
    "profile_id",
    "parent_prefix_hash",
    "generation_params_hash",
    "window_tokens",
    "proposer_id",
    "nonce",
    "deadline_ms",
    "sender",
    "signature",
];

/// Declared field set of [`VerifyDraftsResult`] — same two-pass key-set
/// equality rule as [`VERIFY_DRAFTS_FIELDS`].
pub const VERIFY_DRAFTS_RESULT_FIELDS: [&str; 12] = [
    "session_id",
    "protocol_version",
    "seq",
    "round",
    "accepted_prefix_len",
    "divergence_point",
    "correction",
    "new_prefix_hash",
    "verify_pass_ms",
    "nonce_echo",
    "sender",
    "signature",
];

/// Coordinator → verifier: verify ONE draft window in ONE batched engine
/// pass (ADR-032 §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyDrafts {
    /// Echoed from the `SessionOffer`.
    pub session_id: String,
    /// Must equal [`COOPERATIVE_PROTOCOL_VERSION`].
    pub protocol_version: String,
    /// Per `(session, sender)` strictly monotone counter (§2 RC-7).
    pub seq: u64,
    /// Strictly monotone per session; gaps allowed after abandonment
    /// (§8 RC-5(c)).
    pub round: u64,
    /// `"msp1:<64 hex>"` — cross-profile isolation (I7).
    pub profile_id: String,
    /// Bare 64-char lowercase hex (§1 rule 1): the current committed
    /// prefix hash this window chains onto.
    pub parent_prefix_hash: String,
    /// Bare 64-char lowercase hex: this family's canonical
    /// generation-params preimage (§1 rule 3 — NOT the old vocabulary's
    /// `default_sampling_params_hash`).
    pub generation_params_hash: String,
    /// The draft window: `u32` token ids, length
    /// `1..=min(WINDOW_CAP, batch_window_max)`. An EMPTY window is
    /// malformed (there is nothing to verify).
    pub window_tokens: Vec<u32>,
    /// The peer that drafted the window.
    pub proposer_id: String,
    /// Single-use per session (I6).
    pub nonce: String,
    /// Clamped server-side to
    /// `[DEADLINE_CLAMP_MIN_MS, DEADLINE_CLAMP_MAX_MS]`.
    pub deadline_ms: u32,
    /// `installation_id_for` of the signer's verifying key (§1 rule 5).
    pub sender: String,
    /// base64 (STANDARD) Ed25519 over the canonical JSON with
    /// `signature` omitted entirely (§1 rule 4).
    pub signature: String,
}

/// Verifier → coordinator: the one-pass verification outcome (ADR-032
/// §1). `seq` is the VERIFIER's own per-session counter — NOT a
/// continuation of the coordinator's (§2 RC-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyDraftsResult {
    /// Echoed from the request.
    pub session_id: String,
    /// Echoed from the request.
    pub protocol_version: String,
    /// The VERIFIER's own `(session, sender)` counter (§2 RC-7).
    pub seq: u64,
    /// Echoed from the request.
    pub round: u64,
    /// `<= len(window_tokens)` (§2 RC-3 (d)).
    pub accepted_prefix_len: u32,
    /// 0-based index of the first rejected position; `null` on full
    /// acceptance. PRESENT as `null` (never omitted — the two-pass
    /// key-set check enforces presence). On rejection,
    /// `divergence_point == accepted_prefix_len`.
    pub divergence_point: Option<u32>,
    /// Replacement token on rejection, bonus (lookahead) token on full
    /// acceptance; `null` exactly at end-of-generation (plan flag F-2;
    /// PRESENT as `null`, never omitted).
    pub correction: Option<u32>,
    /// Bare 64-char lowercase hex: [`crate::compute_prefix_hash`] over
    /// `(parent ‖ accepted prefix ‖ [correction])` (§1 rule 2).
    pub new_prefix_hash: String,
    /// The verifier's measured engine-pass wall time in WHOLE
    /// milliseconds (u32, rounded to nearest; test pins assert `>= 0`
    /// only, never measured values). MUST be an integer — floats are
    /// forbidden in signed canonical payloads, so a fractional field
    /// would make this message unsignable.
    pub verify_pass_ms: u32,
    /// EQUALS the outstanding request's `nonce` (RC-3 (b)).
    pub nonce_echo: String,
    /// The verifier peer's installation id (§1 rule 5).
    pub sender: String,
    /// base64 (STANDARD) Ed25519, same §1 rule 4 preimage discipline.
    pub signature: String,
}

// ---------------------------------------------------------------------------
// Strict two-pass deserialization (plan §1.2)
// ---------------------------------------------------------------------------

/// Schema failure of a family frame — every variant maps onto the
/// admission code `bad_frame` (ADR-032 §2 table row "schema invalid /
/// unknown fields").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameParseError {
    /// The frame is not a JSON object.
    NotAnObject,
    /// A declared field is absent. This is what enforces PRESENT-AS-NULL:
    /// `divergence_point` and an EOG `correction` must appear with `null`
    /// values, not be omitted (serde alone cannot distinguish missing
    /// from null on `Option` fields).
    MissingKey(String),
    /// A key outside the declared field set is present.
    UnknownKey(String),
    /// A value failed its declared type — e.g. a JSON float where an
    /// integer is pinned (`seq`, `round`, `verify_pass_ms`, …); floats
    /// are forbidden in signed payloads (msp-v1 §2.2).
    BadType(String),
    /// `window_tokens` is empty (ADR-032 §1: an empty window is
    /// malformed — there is nothing to verify).
    EmptyWindow,
}

impl std::fmt::Display for FrameParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameParseError::NotAnObject => write!(f, "frame is not a JSON object"),
            FrameParseError::MissingKey(key) => {
                write!(
                    f,
                    "missing declared field {key:?} (present-as-null fields may not be omitted)"
                )
            }
            FrameParseError::UnknownKey(key) => {
                write!(
                    f,
                    "unknown field {key:?} (key set must equal the declared field set)"
                )
            }
            FrameParseError::BadType(why) => write!(f, "field type mismatch: {why}"),
            FrameParseError::EmptyWindow => {
                write!(f, "window_tokens is empty (an empty window is malformed)")
            }
        }
    }
}

impl std::error::Error for FrameParseError {}

/// Two-pass strict parse: (1) assert the object's key set EQUALS the
/// declared field set — missing key or unknown key is a schema failure;
/// (2) deserialize. `#[serde(deny_unknown_fields)]` is deliberately NOT
/// relied on: this check produces exactly the failure the admission table
/// names, and it is what enforces present-as-null.
fn parse_strict<T: DeserializeOwned>(
    value: &Value,
    declared: &[&str],
) -> Result<T, FrameParseError> {
    let Some(map) = value.as_object() else {
        return Err(FrameParseError::NotAnObject);
    };
    for field in declared {
        if !map.contains_key(*field) {
            return Err(FrameParseError::MissingKey((*field).to_string()));
        }
    }
    for key in map.keys() {
        if !declared.contains(&key.as_str()) {
            return Err(FrameParseError::UnknownKey(key.clone()));
        }
    }
    T::deserialize(value).map_err(|e| FrameParseError::BadType(e.to_string()))
}

impl VerifyDrafts {
    /// Strict two-pass parse from a received JSON frame (see
    /// [`FrameParseError`]; every variant is the `bad_frame` path).
    pub fn parse(value: &Value) -> Result<Self, FrameParseError> {
        let msg: Self = parse_strict(value, &VERIFY_DRAFTS_FIELDS)?;
        if msg.window_tokens.is_empty() {
            return Err(FrameParseError::EmptyWindow);
        }
        Ok(msg)
    }
}

impl VerifyDraftsResult {
    /// Strict two-pass parse from a received JSON frame (see
    /// [`FrameParseError`]; every variant is the `bad_frame` path).
    pub fn parse(value: &Value) -> Result<Self, FrameParseError> {
        parse_strict(value, &VERIFY_DRAFTS_RESULT_FIELDS)
    }
}

// ---------------------------------------------------------------------------
// Byte-rule helpers (ADR-032 §1 rules 1–5)
// ---------------------------------------------------------------------------

/// §1 rule 1: a family hash wire form — exactly 64 characters of
/// `[0-9a-f]`. The `"sha256:<…>"` prefixed notation is hub `bodyDigest`
/// prose and is REJECTED here (the P2P commit-hash convention — shipped
/// `compute_prefix_hash` / `GENESIS_PREFIX_HASH` — is bare hex).
pub fn is_bare_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// §1 rule 3 failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamsHashError {
    /// A pinned sampling parameter (`temperature`/`top_p`) is not an
    /// integral `f32` (or is not representable as an integer). This
    /// family renders integral values as bare integers and fails closed —
    /// the canonicalizer rejects floats anyway (msp-v1 §2.2), so a
    /// non-integral value would make the payload unsignable.
    NonIntegral,
    /// Canonical serialization failed. Unreachable in practice: the
    /// rendered object is integer-only by construction.
    Canonical(CanonicalError),
}

impl std::fmt::Display for ParamsHashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamsHashError::NonIntegral => {
                write!(f, "sampling parameter is not integral (fail-closed)")
            }
            ParamsHashError::Canonical(e) => {
                write!(
                    f,
                    "generation-params preimage rejected by canonical JSON: {e}"
                )
            }
        }
    }
}

impl std::error::Error for ParamsHashError {}

/// §1 rule 3: the family's `generation_params_hash` — sha256 over the
/// canonical JSON (msp-v1 §2.2) of the pinned sampling-parameter object
/// `{"seed":<null|integer>,"temperature":<integer>,"top_k":<u32>,
/// "top_p":<integer>}` with `seed` null when unset (rendered as the full
/// integer when set — plan flag F-1).
///
/// Integral `f32` values (`1.0`) render as bare integers (`1`); any
/// non-integral value returns [`ParamsHashError::NonIntegral`] and fails
/// closed. This is NOT the old vocabulary's
/// [`crate::spec::default_sampling_params_hash`] (a Rust `Debug` string)
/// — that value remains internal to the old vocabulary.
pub fn batch_generation_params_hash(params: &SamplingParams) -> Result<String, ParamsHashError> {
    let temperature = integral_i64(params.temperature)?;
    let top_p = integral_i64(params.top_p)?;
    let preimage = serde_json::json!({
        "seed": params.seed,
        "temperature": temperature,
        "top_k": params.top_k,
        "top_p": top_p,
    });
    let canonical = canonical_json(&preimage).map_err(ParamsHashError::Canonical)?;
    Ok(hex::encode(Sha256::digest(canonical.as_bytes())))
}

/// Renders an integral `f32` as `i64`, failing closed on non-integral,
/// non-finite, or out-of-integer-range values.
fn integral_i64(value: f32) -> Result<i64, ParamsHashError> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(ParamsHashError::NonIntegral);
    }
    let rendered = value as i64;
    // Saturation guard: an integral f32 beyond i64 range must not render
    // silently as i64::MAX/MIN (fail closed).
    if rendered as f32 != value {
        return Err(ParamsHashError::NonIntegral);
    }
    Ok(rendered)
}

/// §1 rule 4: the signature preimage — canonical JSON of the message with
/// the `signature` field OMITTED ENTIRELY (not null, not empty string).
/// Every other declared field — including a null-valued `divergence_point`
/// and an EOG `correction` — is PRESENT in the canonical form.
///
/// Uses `modelswarm_types::canonical_json` (the `Result`-returning
/// implementation with `FloatRejected`); the old vocabulary's
/// `modelswarm_identity::canonical_json` adapter is untouched (no
/// cross-cutting edits).
pub fn signing_preimage<T: Serialize + ?Sized>(msg: &T) -> Result<String, CanonicalError> {
    let mut value = serde_json::to_value(msg).map_err(|e| CanonicalError::Serde(e.to_string()))?;
    let Some(map) = value.as_object_mut() else {
        return Err(CanonicalError::Serde(
            "signed family messages must be JSON objects".to_string(),
        ));
    };
    map.remove("signature");
    canonical_json(&value)
}

/// Signs a [`VerifyDrafts`] with `signer`'s Ed25519 key (base64 STANDARD,
/// detached, over the [`signing_preimage`] canonical bytes). The caller
/// sets `sender` to the signer's installation id and assigns the returned
/// signature; verification enforces the sender binding.
pub fn sign_verify_drafts(
    msg: &VerifyDrafts,
    signer: &InstallationIdentity,
) -> Result<String, SpecError> {
    let preimage = signing_preimage(msg).map_err(|e| {
        SpecError::Protocol(format!(
            "verify_drafts payload rejected by canonical JSON: {e}"
        ))
    })?;
    let key = SigningKey::from_bytes(&signer.to_bytes());
    Ok(BASE64.encode(key.sign(preimage.as_bytes()).to_bytes()))
}

/// Signs a [`VerifyDraftsResult`] (same discipline as
/// [`sign_verify_drafts`]; the verifier signs with its own key).
pub fn sign_verify_drafts_result(
    msg: &VerifyDraftsResult,
    signer: &InstallationIdentity,
) -> Result<String, SpecError> {
    let preimage = signing_preimage(msg).map_err(|e| {
        SpecError::Protocol(format!(
            "verify_drafts_result payload rejected by canonical JSON: {e}"
        ))
    })?;
    let key = SigningKey::from_bytes(&signer.to_bytes());
    Ok(BASE64.encode(key.sign(preimage.as_bytes()).to_bytes()))
}

/// §1 rule 5 + the `verify_commit_signature` discipline (base64 STANDARD,
/// 64-byte `Signature::from_slice`, sender binding FIRST):
/// `installation_id_for(expected_key) == sender` must hold BEFORE the
/// Ed25519 check over the signature-omitted canonical form. The
/// QUIC-authenticated remote-PeerId equality is enforced by the serve
/// loop (msp-v1 §6.6), which is the only place that knows the stream's
/// peer id.
fn verify_family_signature(
    msg: &impl Serialize,
    sender: &str,
    signature: &str,
    expected_key: &VerifyingKey,
    what: &str,
) -> Result<(), SpecError> {
    if installation_id_for(&expected_key.to_bytes()) != sender {
        return Err(SpecError::Protocol(format!(
            "{what} sender is not the authenticated session key"
        )));
    }
    let preimage = signing_preimage(msg).map_err(|e| {
        SpecError::Protocol(format!("{what} payload rejected by canonical JSON: {e}"))
    })?;
    let bytes = BASE64
        .decode(signature)
        .map_err(|_| SpecError::Protocol(format!("{what} signature is not valid base64")))?;
    let sig = Signature::from_slice(&bytes)
        .map_err(|_| SpecError::Protocol(format!("{what} signature is not 64 bytes")))?;
    expected_key
        .verify(preimage.as_bytes(), &sig)
        .map_err(|_| SpecError::Protocol("bad_signature".to_string()))
}

/// Verifies a [`VerifyDrafts`]: sender binding FIRST, then the Ed25519
/// signature over the [`signing_preimage`] canonical form. An invalid
/// signature is the `bad_frame` fail-closed path at admission
/// (unauthenticated = malformed).
pub fn verify_verify_drafts_signature(
    msg: &VerifyDrafts,
    expected_key: &VerifyingKey,
) -> Result<(), SpecError> {
    verify_family_signature(
        msg,
        &msg.sender,
        &msg.signature,
        expected_key,
        "verify_drafts",
    )
}

/// Verifies a [`VerifyDraftsResult`] under the session's pinned verifier
/// key (same discipline as [`verify_verify_drafts_signature`]; RC-3 (a)).
pub fn verify_verify_drafts_result_signature(
    msg: &VerifyDraftsResult,
    expected_key: &VerifyingKey,
) -> Result<(), SpecError> {
    verify_family_signature(
        msg,
        &msg.sender,
        &msg.signature,
        expected_key,
        "verify_drafts_result",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::default_sampling_params_hash;
    use crate::{compute_prefix_hash, GENESIS_PREFIX_HASH};
    use modelswarm_identity::canonical_json as identity_canonical_json;
    use serde_json::json;

    /// Fixture-only identity in the `[seed; 32]` style (never production).
    fn fixture_identity(seed: u8) -> InstallationIdentity {
        InstallationIdentity::from_bytes(&[seed; 32])
    }

    fn sample_request(sender: &str, signature: &str) -> VerifyDrafts {
        VerifyDrafts {
            session_id: "sess-1".to_string(),
            protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
            seq: 12,
            round: 3,
            profile_id: format!("msp1:{}", "ab".repeat(32)),
            parent_prefix_hash: GENESIS_PREFIX_HASH.to_string(),
            generation_params_hash: batch_generation_params_hash(&SamplingParams::default())
                .expect("default sampling params are integral"),
            window_tokens: vec![101, 102, 103, 104, 105, 106, 107, 108],
            proposer_id: "12D3KooProposer".to_string(),
            nonce: "a1b2c3d4e5f60718".to_string(),
            deadline_ms: 2_500,
            sender: sender.to_string(),
            signature: signature.to_string(),
        }
    }

    fn sample_result(sender: &str, signature: &str) -> VerifyDraftsResult {
        VerifyDraftsResult {
            session_id: "sess-1".to_string(),
            protocol_version: COOPERATIVE_PROTOCOL_VERSION.to_string(),
            seq: 4,
            round: 3,
            accepted_prefix_len: 8,
            divergence_point: None,
            correction: Some(999),
            new_prefix_hash: compute_prefix_hash(GENESIS_PREFIX_HASH, &[999]).unwrap(),
            verify_pass_ms: 35,
            nonce_echo: "a1b2c3d4e5f60718".to_string(),
            sender: sender.to_string(),
            signature: signature.to_string(),
        }
    }

    // -- §1.5: round-trip + exact serde_json::to_value shape ----------------

    #[test]
    fn request_wire_shape_is_snake_case_and_round_trips() {
        let msg = sample_request("coordinator-id", "sig");
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            value,
            json!({
                "session_id": "sess-1",
                "protocol_version": "msp-cooperative-v1",
                "seq": 12,
                "round": 3,
                "profile_id": format!("msp1:{}", "ab".repeat(32)),
                "parent_prefix_hash": GENESIS_PREFIX_HASH,
                "generation_params_hash": msg.generation_params_hash,
                "window_tokens": [101, 102, 103, 104, 105, 106, 107, 108],
                "proposer_id": "12D3KooProposer",
                "nonce": "a1b2c3d4e5f60718",
                "deadline_ms": 2_500,
                "sender": "coordinator-id",
                "signature": "sig"
            })
        );
        // Plain serde round-trip (permissive) AND the strict two-pass parse.
        assert_eq!(
            serde_json::from_value::<VerifyDrafts>(value.clone()).unwrap(),
            msg
        );
        assert_eq!(VerifyDrafts::parse(&value).unwrap(), msg);
    }

    #[test]
    fn result_wire_shape_is_snake_case_and_round_trips() {
        let msg = sample_result("verifier-id", "sig");
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            value,
            json!({
                "session_id": "sess-1",
                "protocol_version": "msp-cooperative-v1",
                "seq": 4,
                "round": 3,
                "accepted_prefix_len": 8,
                "divergence_point": null,
                "correction": 999,
                "new_prefix_hash": msg.new_prefix_hash,
                "verify_pass_ms": 35,
                "nonce_echo": "a1b2c3d4e5f60718",
                "sender": "verifier-id",
                "signature": "sig"
            })
        );
        assert_eq!(
            serde_json::from_value::<VerifyDraftsResult>(value.clone()).unwrap(),
            msg
        );
        assert_eq!(VerifyDraftsResult::parse(&value).unwrap(), msg);
        // The seq pair of the §1 example is per-sender: request 12, result 4.
        assert_eq!(sample_request("c", "s").seq, 12);
        assert_eq!(msg.seq, 4);
    }

    // -- §1.5: present-as-null ---------------------------------------------

    #[test]
    fn divergence_point_is_present_as_null() {
        let value = serde_json::to_value(sample_result("v", "sig")).unwrap();
        let object = value.as_object().unwrap();
        assert!(object.contains_key("divergence_point"));
        assert_eq!(object.get("divergence_point"), Some(&Value::Null));
        // Omitting the key is a schema failure (bad_frame path).
        let mut missing = object.clone();
        missing.remove("divergence_point");
        assert_eq!(
            VerifyDraftsResult::parse(&Value::Object(missing)).unwrap_err(),
            FrameParseError::MissingKey("divergence_point".to_string())
        );
    }

    #[test]
    fn eog_correction_is_present_as_null() {
        // F-2: the FIELD is always present; null exactly at end-of-generation.
        let mut msg = sample_result("v", "sig");
        msg.correction = None;
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value.get("correction"), Some(&Value::Null));
        assert_eq!(VerifyDraftsResult::parse(&value).unwrap(), msg);
        let mut missing = value.as_object().unwrap().clone();
        missing.remove("correction");
        assert_eq!(
            VerifyDraftsResult::parse(&Value::Object(missing)).unwrap_err(),
            FrameParseError::MissingKey("correction".to_string())
        );
    }

    #[test]
    fn unknown_or_missing_keys_and_non_objects_are_schema_failures() {
        let value = serde_json::to_value(sample_request("c", "sig")).unwrap();
        let mut with_unknown = value.as_object().unwrap().clone();
        with_unknown.insert("extra".to_string(), json!(1));
        assert_eq!(
            VerifyDrafts::parse(&Value::Object(with_unknown)).unwrap_err(),
            FrameParseError::UnknownKey("extra".to_string())
        );
        let mut missing_nonce = value.as_object().unwrap().clone();
        missing_nonce.remove("nonce");
        assert_eq!(
            VerifyDrafts::parse(&Value::Object(missing_nonce)).unwrap_err(),
            FrameParseError::MissingKey("nonce".to_string())
        );
        assert_eq!(
            VerifyDrafts::parse(&json!([1, 2])).unwrap_err(),
            FrameParseError::NotAnObject
        );
        assert_eq!(
            VerifyDraftsResult::parse(&json!("nope")).unwrap_err(),
            FrameParseError::NotAnObject
        );
    }

    #[test]
    fn float_fields_fail_deserialization() {
        // verify_pass_ms MUST be an integer: a float is unsignable and
        // fails the strict parse (fail-closed).
        let value = serde_json::to_value(sample_result("v", "sig")).unwrap();
        let mut float_ms = value.clone();
        *float_ms.get_mut("verify_pass_ms").unwrap() = json!(35.5);
        assert!(matches!(
            VerifyDraftsResult::parse(&float_ms),
            Err(FrameParseError::BadType(_))
        ));
        // Even an integral-looking JSON float (35.0) is a float on the wire.
        *float_ms.get_mut("verify_pass_ms").unwrap() = json!(35.0);
        assert!(matches!(
            VerifyDraftsResult::parse(&float_ms),
            Err(FrameParseError::BadType(_))
        ));
        let mut float_seq = serde_json::to_value(sample_request("c", "sig")).unwrap();
        *float_seq.get_mut("seq").unwrap() = json!(12.0);
        assert!(matches!(
            VerifyDrafts::parse(&float_seq),
            Err(FrameParseError::BadType(_))
        ));
    }

    #[test]
    fn empty_window_is_malformed() {
        let mut msg = sample_request("c", "sig");
        msg.window_tokens.clear();
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            VerifyDrafts::parse(&value).unwrap_err(),
            FrameParseError::EmptyWindow
        );
    }

    // -- §1.5: bare-hash rule -----------------------------------------------

    #[test]
    fn is_bare_hash_rejects_prefixed_and_malformed_forms() {
        assert!(is_bare_hash(GENESIS_PREFIX_HASH));
        assert!(is_bare_hash(&"0f".repeat(32)));
        // §1 rule 1: the "sha256:" prefixed hub bodyDigest form is rejected.
        assert!(!is_bare_hash(&format!("sha256:{GENESIS_PREFIX_HASH}")));
        assert!(!is_bare_hash(&GENESIS_PREFIX_HASH.to_uppercase()));
        assert!(!is_bare_hash(&"0".repeat(63)));
        assert!(!is_bare_hash(&"0".repeat(65)));
        assert!(!is_bare_hash(&"g".repeat(64)));
        assert!(!is_bare_hash(""));
    }

    #[test]
    fn prefix_chain_reuses_the_shipped_hash_function() {
        // §1 rule 2: no new hash function — genesis is hex(sha256("")),
        // and new_prefix_hash chains via the shipped compute_prefix_hash
        // over (parent ‖ accepted prefix ‖ [correction]).
        assert_eq!(GENESIS_PREFIX_HASH, hex::encode(Sha256::digest(b"")));
        let window = [101, 102, 103, 104, 105, 106, 107, 108];
        let accepted = &window[..3];
        let correction = 999u32;
        let mut committed = accepted.to_vec();
        committed.push(correction);
        let new_hash = compute_prefix_hash(GENESIS_PREFIX_HASH, &committed).unwrap();
        assert!(is_bare_hash(&new_hash));
        assert_ne!(new_hash, GENESIS_PREFIX_HASH);
        // The result's new_prefix_hash pins exactly this construction.
        let msg = sample_result("v", "sig");
        assert_eq!(
            msg.new_prefix_hash,
            compute_prefix_hash(GENESIS_PREFIX_HASH, &[999]).unwrap()
        );
    }

    // -- §1.5: signature preimage / sign / verify ---------------------------

    fn signed_request() -> (VerifyDrafts, InstallationIdentity) {
        let identity = fixture_identity(0x11);
        let mut msg = sample_request(&identity.installation_id(), "placeholder");
        msg.signature = sign_verify_drafts(&msg, &identity).expect("default params are signable");
        (msg, identity)
    }

    fn signed_result() -> (VerifyDraftsResult, InstallationIdentity) {
        let identity = fixture_identity(0x22);
        let mut msg = sample_result(&identity.installation_id(), "placeholder");
        msg.signature =
            sign_verify_drafts_result(&msg, &identity).expect("default params are signable");
        (msg, identity)
    }

    #[test]
    fn signature_verifies_and_tampering_breaks_it() {
        let (msg, identity) = signed_request();
        verify_verify_drafts_signature(&msg, &identity.verifying_key()).unwrap();

        // Flipping ANY signed field breaks the signature.
        let mut tampered = msg.clone();
        tampered.deadline_ms = 2_501;
        assert!(verify_verify_drafts_signature(&tampered, &identity.verifying_key()).is_err());
        let mut tampered_round = msg.clone();
        tampered_round.round = 4;
        assert!(
            verify_verify_drafts_signature(&tampered_round, &identity.verifying_key()).is_err()
        );
        let mut tampered_window = msg.clone();
        tampered_window.window_tokens[0] = 1;
        assert!(
            verify_verify_drafts_signature(&tampered_window, &identity.verifying_key()).is_err()
        );

        // Tampering the signature itself breaks it.
        let mut bad_sig = msg.clone();
        bad_sig.signature = BASE64.encode([0u8; 64]);
        let err = verify_verify_drafts_signature(&bad_sig, &identity.verifying_key()).unwrap_err();
        assert_eq!(err.to_string(), "spec protocol violation: bad_signature");

        // A signature that is not 64 bytes fails the length check.
        let mut short_sig = msg.clone();
        short_sig.signature = BASE64.encode([0u8; 10]);
        let err =
            verify_verify_drafts_signature(&short_sig, &identity.verifying_key()).unwrap_err();
        assert!(err.to_string().contains("not 64 bytes"));

        // Not base64 at all.
        let mut not_b64 = msg.clone();
        not_b64.signature = "!!!not-base64!!!".to_string();
        let err = verify_verify_drafts_signature(&not_b64, &identity.verifying_key()).unwrap_err();
        assert!(err.to_string().contains("not valid base64"));

        // Wrong expected key fails.
        let other = fixture_identity(0x33);
        assert!(verify_verify_drafts_signature(&msg, &other.verifying_key()).is_err());

        // Same discipline for the result message.
        let (result, verifier) = signed_result();
        verify_verify_drafts_result_signature(&result, &verifier.verifying_key()).unwrap();
        let mut tampered_result = result.clone();
        tampered_result.accepted_prefix_len = 7;
        assert!(
            verify_verify_drafts_result_signature(&tampered_result, &verifier.verifying_key())
                .is_err()
        );
    }

    #[test]
    fn sender_binding_is_checked_first() {
        // Signed by 0x22 with sender = 0x22's installation id, but verified
        // against 0x11's key: the sender check must fire BEFORE the Ed25519
        // check (the verify_commit_signature discipline, §1 rule 5).
        let (result, _verifier) = signed_result();
        let other = fixture_identity(0x11);
        let err =
            verify_verify_drafts_result_signature(&result, &other.verifying_key()).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("sender is not the authenticated session key"),
            "{text}"
        );
        assert!(!text.contains("bad_signature"), "{text}");
        // A signature valid under the expected key but carrying a foreign
        // sender label is likewise rejected on the sender check.
        let (mut msg, identity) = signed_request();
        msg.sender = "someone-else".to_string();
        let err = verify_verify_drafts_signature(&msg, &identity.verifying_key()).unwrap_err();
        assert!(err
            .to_string()
            .contains("sender is not the authenticated session key"));
    }

    #[test]
    fn signing_preimage_omits_signature_entirely() {
        let (msg, _identity) = signed_request();
        let preimage = signing_preimage(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&preimage).unwrap();
        let object = parsed.as_object().unwrap();
        // Omitted entirely — not null, not empty string.
        assert!(!object.contains_key("signature"));
        // Every other declared field is present.
        for field in VERIFY_DRAFTS_FIELDS {
            if field == "signature" {
                continue;
            }
            assert!(object.contains_key(field), "preimage missing {field}");
        }
        // The full message's canonical form differs (it HAS the signature).
        let full = canonical_json(&serde_json::to_value(&msg).unwrap()).unwrap();
        assert_ne!(preimage, full);

        // Null-valued fields survive into the preimage (§1 rule 4): a
        // full-acceptance result keeps divergence_point present-as-null.
        let (result, _verifier) = signed_result();
        let result_preimage = signing_preimage(&result).unwrap();
        let parsed: Value = serde_json::from_str(&result_preimage).unwrap();
        assert_eq!(parsed.get("divergence_point"), Some(&Value::Null));
        for field in VERIFY_DRAFTS_RESULT_FIELDS {
            if field == "signature" {
                continue;
            }
            assert!(parsed.as_object().unwrap().contains_key(field));
        }
    }

    // -- §1.5: batch_generation_params_hash ---------------------------------

    #[test]
    fn batch_generation_params_hash_is_stable_64_hex() {
        let params = SamplingParams::default();
        let once = batch_generation_params_hash(&params).unwrap();
        let twice = batch_generation_params_hash(&params).unwrap();
        assert_eq!(once, twice);
        assert!(is_bare_hash(&once));
    }

    #[test]
    fn batch_generation_params_hash_renders_integrals_as_integers() {
        // The default params (temperature 1.0 / top_p 1.0 / top_k 40 /
        // seed None) render as the bare-integer canonical object
        // {"seed":null,"temperature":1,"top_k":40,"top_p":1}.
        let params = SamplingParams::default();
        let expected_preimage = json!({
            "seed": null,
            "temperature": 1,
            "top_k": 40,
            "top_p": 1
        });
        let expected = hex::encode(Sha256::digest(
            canonical_json(&expected_preimage).unwrap().as_bytes(),
        ));
        assert_eq!(batch_generation_params_hash(&params).unwrap(), expected);
        // Canonical bytes themselves: keys sorted, no floats, no ".0".
        assert_eq!(
            canonical_json(&expected_preimage).unwrap(),
            r#"{"seed":null,"temperature":1,"top_k":40,"top_p":1}"#
        );

        // A set seed renders the full integer (F-1) and changes the hash.
        let seeded = SamplingParams {
            seed: Some(7),
            ..SamplingParams::default()
        };
        assert_ne!(
            batch_generation_params_hash(&seeded).unwrap(),
            batch_generation_params_hash(&params).unwrap()
        );
        let seeded_preimage = json!({
            "seed": 7,
            "temperature": 1,
            "top_k": 40,
            "top_p": 1
        });
        assert_eq!(
            batch_generation_params_hash(&seeded).unwrap(),
            hex::encode(Sha256::digest(
                canonical_json(&seeded_preimage).unwrap().as_bytes()
            ))
        );
    }

    #[test]
    fn batch_generation_params_hash_fails_closed_on_non_integral() {
        let hot = SamplingParams {
            temperature: 0.7,
            ..SamplingParams::default()
        };
        assert_eq!(
            batch_generation_params_hash(&hot).unwrap_err(),
            ParamsHashError::NonIntegral
        );
        let narrow_p = SamplingParams {
            top_p: 0.95,
            ..SamplingParams::default()
        };
        assert_eq!(
            batch_generation_params_hash(&narrow_p).unwrap_err(),
            ParamsHashError::NonIntegral
        );
        let nan = SamplingParams {
            temperature: f32::NAN,
            ..SamplingParams::default()
        };
        assert_eq!(
            batch_generation_params_hash(&nan).unwrap_err(),
            ParamsHashError::NonIntegral
        );
    }

    #[test]
    fn family_params_hash_is_not_the_old_vocabulary_hash() {
        // §1 rule 3: the Debug-string preimage of the OLD vocabulary is
        // NOT this family's wire preimage.
        assert_ne!(
            batch_generation_params_hash(&SamplingParams::default()).unwrap(),
            default_sampling_params_hash()
        );
    }

    // -- §1.5: canonicalizer parity pin --------------------------------------

    #[test]
    fn canonicalizers_agree_byte_for_byte_on_family_payloads() {
        let (request, _c) = signed_request();
        let (result, _v) = signed_result();
        let payloads = [
            serde_json::to_value(&request).unwrap(),
            serde_json::to_value(&result).unwrap(),
            json!({"seed": null, "temperature": 1, "top_k": 40, "top_p": 1}),
            json!({"window_tokens": [1, 2, 3], "nested": {"b": [true, null], "a": "x"}}),
        ];
        for payload in &payloads {
            assert_eq!(
                identity_canonical_json(payload).expect("identity canonicalizer accepts"),
                canonical_json(payload).expect("types canonicalizer accepts"),
                "canonicalizer divergence on {payload}"
            );
        }
        // Parity on rejection too: both refuse floats.
        let floaty = json!({"verify_pass_ms": 35.5});
        assert!(identity_canonical_json(&floaty).is_none());
        assert_eq!(
            canonical_json(&floaty).unwrap_err(),
            CanonicalError::FloatRejected
        );
    }
}
