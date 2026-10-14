//! The egress content gate (ADR-033 §3.2 revision 2): ONE value-typed
//! allowlist, mechanically driven by the frozen in-repo artifact
//! [`protocol/advisory-egress-v1.json`] (RC-2/RC-11 — the vocabulary of
//! record), with the shipped [`Redactor`] as a SECONDARY scrub net.
//!
//! The artifact is embedded with [`include_str!`] and parsed once per
//! process: the compiled gate reads EXACTLY the frozen file — drift
//! between source and artifact is impossible by construction, and the
//! guards completeness tie (R-1) diffs emit-site vocabularies against
//! the same file. The clamp table
//! [`protocol/advisory-clamp-table-v1.json`] (RC-7/R-2) rides the same
//! mechanism.
//!
//! No chat surface is reachable from this module: it knows nothing of
//! chat logs, chat turns, serving frames, or carrier payloads (the
//! ADR-037 §3.1 W3 data wall, pinned by the chat-wall suite).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use modelswarm_telemetry::redact::Redactor;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The frozen egress vocabulary artifact (ADR-033 §9.4: created by this
/// first implementation session, then frozen; changes only by
/// ADR-033 amendment).
pub const EGRESS_ARTIFACT: &str = include_str!("../../../../protocol/advisory-egress-v1.json");
/// The frozen per-target clamp table (RC-7; R-2).
pub const CLAMP_ARTIFACT: &str = include_str!("../../../../protocol/advisory-clamp-table-v1.json");

// ---------------------------------------------------------------------------
// Artifact schemas (mechanical mirrors of the JSON files)
// ---------------------------------------------------------------------------

/// Parsed [`EGRESS_ARTIFACT`].
#[derive(Debug, Deserialize)]
pub struct EgressVocabulary {
    #[serde(rename = "vocabularies")]
    pub vocabularies: BTreeMap<String, VocabularySet>,
    #[serde(rename = "aggregation_event_allowlist")]
    pub aggregation: AggregationAllowlist,
    #[serde(rename = "plane_telemetry_events")]
    pub plane_telemetry_events: Vec<String>,
    #[serde(default, rename = "reduction_rules")]
    pub reduction_rules: Vec<ReductionRule>,
    /// ADR-033 R-1 (protocol review 2026-10-14): the EXHAUSTIVE emit-site
    /// event vocabulary — event name -> feeds_digest | excluded | plane_own.
    /// The guards completeness tie diffs the source emit-site vocabulary
    /// against this map (and the fixture manifest) in both directions.
    #[serde(default, rename = "emit_site_events")]
    pub emit_site_events: EmitSiteEventsSection,
    /// ADR-037 §4.2: the assistant-plane typed context block (the frozen
    /// field set + typed shapes + bounds). `admit_context` and
    /// `choke_point_rescan_assistant` below are driven by this section.
    #[serde(default, rename = "assistant_context_block")]
    pub assistant_context_block: AssistantContextSection,
    /// ADR-037 §4.2: the plane enum (`digest | assistant`) for the plane's
    /// own telemetry rows.
    #[serde(default, rename = "plane_enum")]
    pub plane_enum: PlaneEnumSection,
}

/// The ADR-037 §3.5 assistant context-block contract: the frozen field
/// set, the typed shapes, and the picker-row sub-schema.
#[derive(Debug, Default, Deserialize)]
pub struct AssistantContextSection {
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default, rename = "value_types")]
    pub value_types: BTreeMap<String, String>,
    #[serde(default, rename = "picker_row_fields")]
    pub picker_row_fields: BTreeMap<String, String>,
    #[serde(default, rename = "max_picker_rows")]
    pub max_picker_rows: usize,
}

/// The `plane_enum` section.
#[derive(Debug, Default, Deserialize)]
pub struct PlaneEnumSection {
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub values: Vec<String>,
}

/// The R-1 exhaustive emit-site vocabulary section: the tie note plus the
/// event -> classification map.
#[derive(Debug, Default, Deserialize)]
pub struct EmitSiteEventsSection {
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub events: std::collections::BTreeMap<String, String>,
}

/// One row of the §3.2 stringly-field reduction table, carried
/// mechanically (the `integer_ms_from_typed_accumulators` row is the
/// frozen TIMING-METRIC allowlist the accumulators enforce).
#[derive(Debug, Deserialize)]
pub struct ReductionRule {
    #[serde(default)]
    pub field: StringOrVec,
    #[serde(default)]
    pub rule: String,
}

/// The artifact writes `field` as either one string or an array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

impl Default for StringOrVec {
    fn default() -> Self {
        StringOrVec::One(String::new())
    }
}

impl StringOrVec {
    fn list(&self) -> Vec<&str> {
        match self {
            StringOrVec::One(s) => vec![s.as_str()],
            StringOrVec::Many(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

impl EgressVocabulary {
    /// The frozen timing-metric names (the §3.2 reduction row for
    /// float-formatted timings): the ONLY metric names the typed
    /// accumulators accept.
    pub fn timing_metrics(&self) -> Vec<&str> {
        self.reduction_rules
            .iter()
            .filter(|r| r.rule == "integer_ms_from_typed_accumulators")
            .flat_map(|r| r.field.list())
            .collect()
    }
}

/// One frozen vocabulary: either a bare value set or a rule-bearing set.
#[derive(Debug, Deserialize)]
pub struct VocabularySet {
    pub values: Vec<String>,
    #[serde(default)]
    pub rule: Option<String>,
}

/// The enumerated event-type allowlist whose typed accumulators may feed
/// a digest (EXCLUDES every `advisory.*` row — RC-4).
#[derive(Debug, Deserialize)]
pub struct AggregationAllowlist {
    pub events: Vec<String>,
}

/// One clamp-table row: `target → [min, max] → source of truth →
/// {promise_metric, direction}`.
#[derive(Debug, Deserialize, Clone)]
pub struct ClampRow {
    pub target: String,
    pub min: f64,
    pub max: f64,
    #[serde(rename = "source_of_truth")]
    pub source_of_truth: String,
    #[serde(rename = "promise_metric")]
    pub promise_metric: String,
    pub direction: String,
}

/// Parsed [`CLAMP_ARTIFACT`].
#[derive(Debug, Deserialize)]
pub struct ClampTable {
    pub targets: Vec<ClampRow>,
}

/// The parsed egress vocabulary (once per process).
pub fn vocabulary() -> &'static EgressVocabulary {
    static VOCAB: OnceLock<EgressVocabulary> = OnceLock::new();
    VOCAB.get_or_init(|| {
        serde_json::from_str(EGRESS_ARTIFACT).expect("the frozen egress artifact must parse")
    })
}

/// The parsed clamp table (once per process).
pub fn clamp_table() -> &'static ClampTable {
    static TABLE: OnceLock<ClampTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(CLAMP_ARTIFACT).expect("the frozen clamp table must parse")
    })
}

// ---------------------------------------------------------------------------
// The value-typed allowlist (the ONE content gate)
// ---------------------------------------------------------------------------

/// A digest field value. Every field leaving the process for the
/// advisory endpoint must be one of these typed shapes (ADR-033 §3.2);
/// there is deliberately NO free-text variant — the assistant-plane
/// user-text exception is ADR-037 §3.5's separate, later surface.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedValue {
    /// Integer (counts, ms timings, tallies).
    Int(i64),
    /// Boolean.
    Bool(bool),
    /// ISO-8601 UTC timestamp (RFC 3339, Z).
    TimestampUtc(String),
    /// String VALUE-matched against a frozen vocabulary named in the
    /// artifact.
    Enum {
        vocabulary: &'static str,
        value: String,
    },
    /// Peer reference: exactly 16 lowercase hex (first 16 of SHA-256
    /// over the full base58btc PeerId string — RC-1).
    PeerRef16Hex(String),
    /// Profile reference: the 16 hex following `msp1:`.
    ProfileRef16Hex(String),
}

impl TypedValue {
    /// Validates this value against the frozen artifact and renders the
    /// JSON value it may be sent as. `Err(())` = drop and count.
    fn admit(self, vocab: &EgressVocabulary) -> Result<serde_json::Value, ()> {
        match self {
            TypedValue::Int(i) => Ok(serde_json::Value::from(i)),
            TypedValue::Bool(b) => Ok(serde_json::Value::from(b)),
            TypedValue::TimestampUtc(ts) => {
                // ISO-8601 UTC, RFC 3339 SHAPE (YYYY-MM-DDTHH:MM:SSZ,
                // optional fractional seconds): strict positional
                // validation — a float-formatted timing ("823.7") or any
                // digit-bearing adversarial text fails, not just free
                // prose (the §3.2 reduction table's own rule).
                if is_utc_timestamp(&ts) {
                    Ok(serde_json::Value::String(ts))
                } else {
                    Err(())
                }
            }
            TypedValue::Enum { vocabulary, value } => {
                let set = vocab.vocabularies.get(vocabulary).ok_or(())?;
                if set.values.contains(&value) {
                    Ok(serde_json::Value::String(value))
                } else {
                    Err(())
                }
            }
            TypedValue::PeerRef16Hex(h) | TypedValue::ProfileRef16Hex(h) => {
                if is_16_lower_hex(&h) {
                    Ok(serde_json::Value::String(h))
                } else {
                    Err(())
                }
            }
        }
    }
}

fn is_16_lower_hex(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Strict `YYYY-MM-DDTHH:MM:SS[.f…]Z` (UTC only — offsets are not part
/// of the frozen shape; the digest's timestamps are UTC by construction).
fn is_utc_timestamp(ts: &str) -> bool {
    let b = ts.as_bytes();
    if b.len() < 20 || b.len() > 30 {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| {
        b.get(r)
            .is_some_and(|w| w.iter().all(|c| c.is_ascii_digit()))
    };
    digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && (b[10] == b'T' || b[10] == b't')
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && match b[19] {
            b'Z' | b'z' => b.len() == 20,
            b'.' => {
                let rest = &b[20..];
                rest.len() >= 2
                    && rest[..rest.len() - 1].iter().all(|c| c.is_ascii_digit())
                    && (rest[rest.len() - 1] == b'Z' || rest[rest.len() - 1] == b'z')
            }
            _ => false,
        }
}

/// Renders a peer reference per RC-1: the first 16 hex characters of
/// SHA-256 over the FULL base58btc PeerId string. The raw base58 string
/// is never emitted.
pub fn peer_ref16(peer_id: &str) -> TypedValue {
    let digest = Sha256::digest(peer_id.as_bytes());
    TypedValue::PeerRef16Hex(hex::encode(digest)[..16].to_string())
}

/// Renders a profile reference: the 16 hex following `msp1:`. Returns
/// `None` for anything that is not the pinned shape (drop and count).
pub fn profile_ref16(profile_id: &str) -> Option<TypedValue> {
    let rest = profile_id.strip_prefix("msp1:")?;
    if is_16_lower_hex(rest) {
        Some(TypedValue::ProfileRef16Hex(rest.to_string()))
    } else {
        None
    }
}

/// Whether `s` is the bare 16-hex profile-reference form (a suggestion
/// `target` is either this or the literal `fleet`).
pub fn is_profile_ref16(s: &str) -> bool {
    is_16_lower_hex(s)
}

/// A named digest field entering the gate.
#[derive(Debug, Clone)]
pub struct DigestField {
    pub name: String,
    pub value: TypedValue,
}

/// The result of the allowlist pass: the admitted body object plus the
/// count of dropped fields (counted, never sent).
#[derive(Debug, Clone)]
pub struct Admitted {
    /// The admitted fields as a JSON object (insertion order kept).
    pub body: serde_json::Map<String, serde_json::Value>,
    /// Fields dropped by the gate (typed-shape failure, forbidden name,
    /// or unknown vocabulary value).
    pub dropped: u64,
}

/// The gate: every `(name, value)` must match a typed shape; forbidden
/// names (the Redactor's set) drop; everything admitted serializes into
/// the digest body object. Free text under an allowed name is dropped
/// here by TYPE (there is no allowed string type outside the frozen
/// vocabularies) — the Redactor remains the secondary net below.
pub fn admit(fields: Vec<DigestField>) -> Admitted {
    let vocab = vocabulary();
    let redactor = Redactor::default();
    let mut body = serde_json::Map::new();
    let mut dropped = 0u64;
    for field in fields {
        if redactor
            .redact_field(&field.name, "x")
            .starts_with("[REDACTED:")
        {
            dropped += 1; // forbidden name — the Redactor net drops it
            continue;
        }
        match field.value.admit(vocab) {
            Ok(v) => {
                body.insert(field.name.clone(), v);
            }
            Err(()) => dropped += 1,
        }
    }
    Admitted { body, dropped }
}

/// Why the choke-point re-scan rejected a final body (ADR-033 RC-3).
#[derive(Debug, PartialEq)]
pub enum RescanRejection {
    /// Re-parse failed.
    Unparseable,
    /// A value did not re-match the allowlist.
    ValueFailed,
    /// `scrub_content` would change the bytes (secret-shaped content).
    ScrubWouldAlter,
    /// Body exceeds `max_request_bytes`.
    TooLarge,
}

/// The choke-point re-scan (RC-3): BEFORE any CALL, the FINAL serialized
/// body is re-parsed, every value must re-match the §3.2 allowlist, and
/// the bytes must pass `scrub_content` UNCHANGED. Any failure ⇒ nothing
/// is sent (the caller records gap `empty_after_redaction`).
pub fn choke_point_rescan(body: &str, max_request_bytes: usize) -> Result<(), RescanRejection> {
    if body.len() > max_request_bytes {
        return Err(RescanRejection::TooLarge);
    }
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RescanRejection::Unparseable)?;
    let obj = parsed.as_object().ok_or(RescanRejection::Unparseable)?;
    let redactor = Redactor::default();
    for (name, value) in obj {
        // Forbidden names are structurally absent (the gate dropped
        // them), but the net re-checks by name here anyway.
        if redactor.redact_field(name, "x").starts_with("[REDACTED:") {
            return Err(RescanRejection::ValueFailed);
        }
        let ok = match value {
            // Integers only — a float fraction means a non-integer
            // escaped the typed accumulators.
            serde_json::Value::Number(n) => n.as_i64().is_some(),
            serde_json::Value::Bool(_) => true,
            // Honest timing sentinels (§3.2: unmeasured maps to null).
            serde_json::Value::Null => true,
            serde_json::Value::String(s) => {
                is_16_lower_hex(s)
                    || TypedValue::TimestampUtc(s.clone())
                        .admit(vocabulary())
                        .is_ok()
                    || vocabulary()
                        .vocabularies
                        .values()
                        .any(|set| set.values.contains(s))
            }
            // Arrays/objects never pass the gate.
            _ => false,
        };
        if !ok {
            return Err(RescanRejection::ValueFailed);
        }
    }
    if redactor.scrub_content(body) != body {
        return Err(RescanRejection::ScrubWouldAlter);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The assistant-plane context-block gate (ADR-037 §3.5, one edit with the
// artifact's assistant_context_block section)
// ---------------------------------------------------------------------------

/// One picker row of the context block (§3.2a): `display_name` and
/// `quant_method` are the RC-11 owner-controlled string class (signed
/// catalog/candidate rows only); everything else is numeric or a frozen
/// enum; `census_peers` is `None` when the census is unreachable (the
/// field is omitted — never a guessed zero).
#[derive(Debug, Clone, PartialEq)]
pub struct ContextPickerRow {
    pub display_name: String,
    pub quant_method: String,
    pub quant_bits: i64,
    pub download_mb: i64,
    pub ram_mb_est: i64,
    /// `gpu_full | gpu_partial | ram_only`; `None` = fit unknown (the
    /// field is omitted from the row).
    pub fit: Option<String>,
    pub recommended: bool,
    pub census_peers: Option<i64>,
}

/// A context-block field value. The typed shapes of the artifact's
/// `assistant_context_block.value_types` — there is deliberately NO
/// free-text variant: the user's assistant-channel text is the one
/// disclosed exception and rides the request's turn/history members,
/// never the context block (ADR-037 §3.5).
#[derive(Debug, Clone, PartialEq)]
pub enum ContextValue {
    Int(i64),
    Bool(bool),
    Enum {
        vocabulary: &'static str,
        value: String,
    },
    /// RC-11 class: bounded 128 chars, no control chars.
    DisplayName(String),
    /// Bounded 128 chars, no control chars (engine device-listing text).
    GpuName(String),
    /// Bounded 32 chars, charset `[A-Za-z0-9.+-]`.
    Version(String),
    /// Keys VALUE-matched against a frozen vocabulary, integer values.
    VocabCountMap {
        vocabulary: &'static str,
        counts: BTreeMap<String, i64>,
    },
    PickerRows(Vec<ContextPickerRow>),
}

/// A named context-block field entering the gate (the assistant-plane
/// analogue of [`DigestField`]).
#[derive(Debug, Clone)]
pub struct ContextField {
    pub name: String,
    pub value: ContextValue,
}

fn is_bound_display_string(s: &str, max_chars: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max_chars && !s.chars().any(|c| c.is_control())
}

fn is_version_string(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

fn vocab_contains(vocab_name: &str, value: &str) -> bool {
    vocabulary()
        .vocabularies
        .get(vocab_name)
        .is_some_and(|set| set.values.iter().any(|v| v == value))
}

fn context_type_of(field: &str) -> Option<&str> {
    vocabulary()
        .assistant_context_block
        .fields
        .get(field)
        .map(String::as_str)
}

impl ContextValue {
    /// Validates this value against the field's declared artifact type
    /// and renders the JSON value it may be sent as. `Err(())` = drop
    /// and count.
    fn admit(&self, declared: &str) -> Result<serde_json::Value, ()> {
        match declared {
            "integer" => match self {
                ContextValue::Int(i) => Ok(serde_json::Value::from(*i)),
                _ => Err(()),
            },
            "boolean" => match self {
                ContextValue::Bool(b) => Ok(serde_json::Value::from(*b)),
                _ => Err(()),
            },
            _ if declared.starts_with("enum:") => match self {
                ContextValue::Enum { vocabulary, value } => {
                    // The field's declared vocabulary IS the authority
                    // (a builder cannot borrow another vocabulary's
                    // shape to smuggle a value).
                    if declared.strip_prefix("enum:") == Some(vocabulary)
                        && vocab_contains(vocabulary, value)
                    {
                        Ok(serde_json::Value::String(value.clone()))
                    } else {
                        Err(())
                    }
                }
                _ => Err(()),
            },
            "display_name_string" => match self {
                ContextValue::DisplayName(s) if is_bound_display_string(s, 128) => {
                    Ok(serde_json::Value::String(s.clone()))
                }
                _ => Err(()),
            },
            "gpu_name_string" => match self {
                ContextValue::GpuName(s) if is_bound_display_string(s, 128) => {
                    Ok(serde_json::Value::String(s.clone()))
                }
                _ => Err(()),
            },
            "version_string" => match self {
                ContextValue::Version(s) if is_version_string(s) => {
                    Ok(serde_json::Value::String(s.clone()))
                }
                _ => Err(()),
            },
            _ if declared.starts_with("vocab_count_map:") => match self {
                ContextValue::VocabCountMap { vocabulary, counts } => {
                    if declared.strip_prefix("vocab_count_map:") != Some(vocabulary) {
                        return Err(());
                    }
                    let mut map = serde_json::Map::new();
                    for (value, count) in counts {
                        if !vocab_contains(vocabulary, value) {
                            return Err(());
                        }
                        map.insert(value.clone(), serde_json::Value::from(*count));
                    }
                    Ok(serde_json::Value::Object(map))
                }
                _ => Err(()),
            },
            "picker_row[]" => match self {
                ContextValue::PickerRows(rows) => {
                    let block = &vocabulary().assistant_context_block;
                    if rows.len() > block.max_picker_rows.max(1) {
                        return Err(());
                    }
                    let mut out = Vec::with_capacity(rows.len());
                    for row in rows {
                        out.push(serde_json::json!({
                            "display_name": admit_bound(&row.display_name)?,
                            "quant_method": admit_bound(&row.quant_method)?,
                            "quant_bits": row.quant_bits,
                            "download_mb": row.download_mb,
                            "ram_mb_est": row.ram_mb_est,
                            "fit": match &row.fit {
                                // The row's fit is the frozen picker_fit
                                // vocabulary, or omitted when unknown.
                                Some(fit) if vocab_contains("picker_fit", fit) => {
                                    serde_json::Value::String(fit.clone())
                                }
                                Some(_) => return Err(()),
                                None => serde_json::Value::Null,
                            },
                            "recommended": row.recommended,
                            "census_peers": row
                                .census_peers
                                .map(serde_json::Value::from)
                                .unwrap_or(serde_json::Value::Null),
                        }));
                    }
                    Ok(serde_json::Value::Array(out))
                }
                _ => Err(()),
            },
            _ => Err(()),
        }
    }
}

fn admit_bound(s: &str) -> Result<serde_json::Value, ()> {
    if is_bound_display_string(s, 128) {
        Ok(serde_json::Value::String(s.to_string()))
    } else {
        Err(())
    }
}

/// Re-validates one SERIALIZED context value against the field's
/// declared type (the choke-point re-scan's context pass — RC-3/A-1
/// defense-in-depth: name→type binding re-checked on the final bytes).
fn context_value_rematches(declared: &str, value: &serde_json::Value) -> bool {
    match declared {
        "integer" => value.as_i64().is_some(),
        "boolean" => value.as_bool().is_some(),
        _ if declared.starts_with("enum:") => value
            .as_str()
            .is_some_and(|v| vocab_contains(declared.strip_prefix("enum:").unwrap_or(""), v)),
        "display_name_string" | "gpu_name_string" => value
            .as_str()
            .is_some_and(|s| is_bound_display_string(s, 128)),
        "version_string" => value.as_str().is_some_and(is_version_string),
        _ if declared.starts_with("vocab_count_map:") => {
            let vocab = declared.strip_prefix("vocab_count_map:").unwrap_or("");
            value.as_object().is_some_and(|map| {
                map.iter()
                    .all(|(k, v)| vocab_contains(vocab, k) && v.as_i64().is_some())
            })
        }
        "picker_row[]" => value.as_array().is_some_and(|rows| {
            rows.len() <= vocabulary().assistant_context_block.max_picker_rows.max(1)
                && rows.iter().all(|row| {
                    row.as_object().is_some_and(|obj| {
                        let fields = &vocabulary().assistant_context_block.picker_row_fields;
                        obj.len() == fields.len()
                            && obj.iter().all(|(k, v)| {
                                fields
                                    .get(k)
                                    .is_some_and(|declared| context_value_rematches(declared, v))
                            })
                    })
                })
        }),
        _ => false,
    }
}

/// The assistant-plane context gate: every field name must be on the
/// frozen artifact set and every value must match its declared type;
/// forbidden names (the Redactor set) drop. Returns the admitted context
/// object and the drop count (counted, never sent).
pub fn admit_context(fields: Vec<ContextField>) -> Admitted {
    let redactor = Redactor::default();
    let mut body = serde_json::Map::new();
    let mut dropped = 0u64;
    for field in fields {
        if redactor
            .redact_field(&field.name, "x")
            .starts_with("[REDACTED:")
        {
            dropped += 1;
            continue;
        }
        let Some(declared) = context_type_of(&field.name) else {
            dropped += 1; // not on the frozen field set
            continue;
        };
        match field.value.admit(declared) {
            Ok(v) => {
                body.insert(field.name.clone(), v);
            }
            Err(()) => dropped += 1,
        }
    }
    Admitted { body, dropped }
}

/// The assistant-plane choke-point re-scan (ADR-037 §3.5): on the FINAL
/// serialized body, the `context` object must re-match the frozen field
/// set (name→type binding included — advisory A-1), the history/turn
/// strings must pass `scrub_content` unchanged, and the body must be
/// within `max_request_bytes`. Any failure ⇒ nothing is sent.
pub fn choke_point_rescan_assistant(
    body: &str,
    max_request_bytes: usize,
) -> Result<(), RescanRejection> {
    if body.len() > max_request_bytes {
        return Err(RescanRejection::TooLarge);
    }
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RescanRejection::Unparseable)?;
    let obj = parsed.as_object().ok_or(RescanRejection::Unparseable)?;
    for (key, value) in obj {
        match key.as_str() {
            "context" => {
                let ctx = value.as_object().ok_or(RescanRejection::ValueFailed)?;
                for (name, v) in ctx {
                    let declared = context_type_of(name).ok_or(RescanRejection::ValueFailed)?;
                    if !context_value_rematches(declared, v) {
                        return Err(RescanRejection::ValueFailed);
                    }
                }
            }
            "history" => {
                let entries = value.as_array().ok_or(RescanRejection::ValueFailed)?;
                if entries.len() > 64 {
                    return Err(RescanRejection::ValueFailed);
                }
                for entry in entries {
                    let entry = entry.as_object().ok_or(RescanRejection::ValueFailed)?;
                    let role = entry.get("role").and_then(|r| r.as_str());
                    let content = entry.get("content").and_then(|c| c.as_str());
                    match (role, content) {
                        (Some("user" | "assistant"), Some(_)) => {}
                        _ => return Err(RescanRejection::ValueFailed),
                    }
                }
            }
            "turn" => {
                let turn = value.as_object().ok_or(RescanRejection::ValueFailed)?;
                let id = turn.get("conversation_id").and_then(|t| t.as_str());
                let number = turn.get("turn").and_then(|t| t.as_u64());
                if !id.is_some_and(is_16_lower_hex) || number.is_none() {
                    return Err(RescanRejection::ValueFailed);
                }
                if turn.get("text").and_then(|t| t.as_str()).is_none() {
                    return Err(RescanRejection::ValueFailed);
                }
            }
            _ => return Err(RescanRejection::Unparseable),
        }
    }
    let redactor = Redactor::default();
    if redactor.scrub_content(body) != body {
        return Err(RescanRejection::ScrubWouldAlter);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The clamp table application (§4 malformed table, RC-7)
// ---------------------------------------------------------------------------

/// Outcome of clamping one numeric hint value against the frozen table.
#[derive(Debug, Clone, PartialEq)]
pub enum HintAdmission {
    /// In range: recorded as-is, not clamped.
    InRange(f64),
    /// Out of range: clamped to the boundary, `clamped=true` recorded.
    Clamped(f64),
    /// The target is absent from the table: `requires_adr=true`, never
    /// an improvised bound.
    RequiresAdr,
}

/// Clamps `v` for `target` per [`clamp_table`]. Non-finite values are
/// NOT handled here — the §4 malformed table discards the whole
/// suggestion before hints are read (the parser is strict; TR-AD-1).
pub fn clamp_hint(target: &str, v: f64) -> HintAdmission {
    match clamp_table()
        .targets
        .iter()
        .find(|row| row.target == target)
    {
        Some(row) => {
            if v < row.min {
                HintAdmission::Clamped(row.min)
            } else if v > row.max {
                HintAdmission::Clamped(row.max)
            } else {
                HintAdmission::InRange(v)
            }
        }
        None => HintAdmission::RequiresAdr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_parse_and_carry_the_frozen_digest_set() {
        let vocab = vocabulary();
        // §6.5 effective registry (16 values, ADR-028 §5 amended).
        let registry = &vocab.vocabularies["msp_v1_section_6_5_error_registry"].values;
        assert_eq!(registry.len(), 16);
        assert!(registry.contains(&"invalid_lease".to_string()));
        assert!(registry.contains(&"replayed_request".to_string()));
        // Gap vocabulary: the 12 digest-plane values PLUS the two
        // ADR-037 §4.3 assistant-leg values (rate_limited,
        // request_oversized) — 14 total, tied to the Gap enum's golden
        // test in state.rs.
        let gaps = &vocab.vocabularies["advisory_gap"].values;
        assert_eq!(
            gaps,
            &[
                "disabled",
                "no_key",
                "no_config",
                "cap_reached",
                "interval",
                "empty_after_redaction",
                "timeout",
                "http_error",
                "quota",
                "response_oversized",
                "schema",
                "late",
                "rate_limited",
                "request_oversized"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
        );
        // Suggestion types: the four digest-plane classes only.
        assert_eq!(
            vocab.vocabularies["suggestion_type"].values,
            vec![
                "triage_summary".to_string(),
                "tuning_hint".to_string(),
                "catalog_note".to_string(),
                "digest".to_string()
            ]
        );
        // The plane's own events are excluded from aggregation (RC-4).
        for event in &vocab.aggregation.events {
            assert!(
                !event.starts_with("advisory."),
                "aggregation allowlist must exclude advisory.* rows: {event}"
            );
        }
        assert_eq!(
            vocab.plane_telemetry_events,
            vec![
                "advisory.cycle.sent".to_string(),
                "advisory.cycle.gap".to_string(),
                "advisory.suggestion.recorded".to_string()
            ]
        );
        // Clamp table: exactly the three ADR-033 §4 rows.
        let table = clamp_table();
        assert_eq!(table.targets.len(), 3);
        assert!(table
            .targets
            .iter()
            .all(|row| row.promise_metric == "ttft_p50_ms" && row.direction == "down"));
    }

    #[test]
    fn clamp_table_golden_rows() {
        let t = |name: &str| {
            clamp_table()
                .targets
                .iter()
                .find(|r| r.target == name)
                .unwrap()
        };
        assert_eq!(t("MSP_SHADOW_QUEUE_ADVERTISED_WEIGHT").min, 0.0);
        assert_eq!(t("MSP_SHADOW_QUEUE_ADVERTISED_WEIGHT").max, 1.0);
        assert_eq!(t("MSP_SHADOW_QUEUE_MEASURED_WEIGHT").min, 0.0);
        assert_eq!(t("MSP_SHADOW_QUEUE_MEASURED_WEIGHT").max, 1.0);
        assert_eq!(t("MSP_SHADOW_QUEUE_CAP_MS").min, 1000.0);
        assert_eq!(t("MSP_SHADOW_QUEUE_CAP_MS").max, 120_000.0);
    }

    #[test]
    fn peer_refs_are_sha256_prefixes_never_base58() {
        let v = peer_ref16("12D3KooWExamplePeerIdStringBase58");
        match &v {
            TypedValue::PeerRef16Hex(h) => {
                assert_eq!(h.len(), 16);
                assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(!h.contains(['K', 'W', 'o']), "no base58 survives");
            }
            _ => panic!("wrong type"),
        }
        assert!(profile_ref16("msp1:0123456789abcdef").is_some());
        assert!(profile_ref16("msp1:XYZ").is_none());
        assert!(profile_ref16("not-a-profile").is_none());
    }

    #[test]
    fn admit_drops_untyped_and_unknown_enum_values() {
        let f = |name: &str, value: TypedValue| DigestField {
            name: name.to_string(),
            value,
        };
        let admitted = admit(vec![
            f("cycles", TypedValue::Int(3)),
            f("ok", TypedValue::Bool(true)),
            f(
                "gap",
                TypedValue::Enum {
                    vocabulary: "advisory_gap",
                    value: "timeout".into(),
                },
            ),
            f(
                "bogus_gap",
                TypedValue::Enum {
                    vocabulary: "advisory_gap",
                    value: "not_a_gap".into(),
                },
            ),
            f("prompt", TypedValue::Int(1)), // forbidden name
            f(
                "ts",
                TypedValue::TimestampUtc("2026-10-14T12:00:00Z".into()),
            ),
            f("bad_ts", TypedValue::TimestampUtc("hello world".into())),
        ]);
        assert_eq!(admitted.dropped, 3);
        assert_eq!(admitted.body["cycles"], 3);
        assert_eq!(admitted.body["gap"], "timeout");
        assert!(admitted.body.get("bogus_gap").is_none());
        assert!(admitted.body.get("prompt").is_none());
        assert_eq!(admitted.body["ts"], "2026-10-14T12:00:00Z");
    }

    #[test]
    fn choke_point_stops_corrupted_bodies() {
        // Clean body passes.
        let clean = r#"{"cycles":3,"gap":"timeout"}"#;
        assert_eq!(choke_point_rescan(clean, 1024), Ok(()));
        // A float fraction (non-integer timing) fails.
        assert_eq!(
            choke_point_rescan(r#"{"ttft_ewma_ms":12.5}"#, 1024),
            Err(RescanRejection::ValueFailed)
        );
        // Free text that is no frozen vocabulary value fails.
        assert_eq!(
            choke_point_rescan(r#"{"note":"you have been a good model"}"#, 1024),
            Err(RescanRejection::ValueFailed)
        );
        // A ≥32-hex run smuggled into a post-gate corruption (here: the
        // field NAME — the exact RC-3 seam) fails the scrub-would-alter
        // check even though every value re-matched.
        let hex_name = format!("{{\"{}\":3}}", "ab".repeat(20));
        assert_eq!(
            choke_point_rescan(&hex_name, 1024),
            Err(RescanRejection::ScrubWouldAlter)
        );
        // Oversize fails.
        assert_eq!(choke_point_rescan(clean, 8), Err(RescanRejection::TooLarge));
    }

    #[test]
    fn clamp_property_over_the_frozen_table() {
        // §9.2(4): for every target, for adversarial numeric v (negative,
        // zero, huge): recorded value within [min,max], clamped=true when
        // out of range; absent target => requires_adr, never improvised.
        for row in &clamp_table().targets {
            for v in [-1.0, 0.0, -999_999.0, row.min - 0.5, row.max + 0.5, 1e12] {
                match clamp_hint(&row.target, v) {
                    HintAdmission::InRange(x) => {
                        assert!((row.min..=row.max).contains(&x), "{v}");
                        assert!((row.min..=row.max).contains(&v));
                    }
                    HintAdmission::Clamped(x) => {
                        assert!(v < row.min || v > row.max);
                        assert!((row.min..=row.max).contains(&x));
                        assert!(x == row.min || x == row.max);
                    }
                    HintAdmission::RequiresAdr => panic!("table target must resolve"),
                }
            }
            // In-range values stay untouched.
            let mid = (row.min + row.max) / 2.0;
            assert_eq!(clamp_hint(&row.target, mid), HintAdmission::InRange(mid));
            assert_eq!(
                clamp_hint(&row.target, row.min),
                HintAdmission::InRange(row.min)
            );
            assert_eq!(
                clamp_hint(&row.target, row.max),
                HintAdmission::InRange(row.max)
            );
        }
        assert_eq!(
            clamp_hint("MSP_SOMETHING_NEW", 5.0),
            HintAdmission::RequiresAdr
        );
    }
}

#[cfg(test)]
mod emit_site_events_tie_tests {
    use super::*;

    /// ADR-033 R-1 (protocol review 2026-10-14): the artifact's
    /// `emit_site_events` is the EXHAUSTIVE emit-site vocabulary — every
    /// `feeds_digest` row is an aggregation allowlist event, every
    /// `plane_own` row is a plane telemetry event, classifications are
    /// from the frozen trio, and the verified source set is pinned so a
    /// source event added without a row (or a row without a source) is a
    /// red build once T/R's two-sided mirror lands.
    #[test]
    fn emit_site_events_tie_the_allowlists_and_the_source_set() {
        let vocab = vocabulary();
        let emit = &vocab.emit_site_events.events;
        assert!(
            emit.len() >= 35,
            "the verified source set is ~34 events + any additions: {emit:?}"
        );
        for (event, class) in emit {
            assert!(
                matches!(class.as_str(), "feeds_digest" | "excluded" | "plane_own"),
                "unknown classification {class} on {event}"
            );
        }
        for event in &vocab.aggregation.events {
            assert_eq!(
                emit.get(event).map(String::as_str),
                Some("feeds_digest"),
                "aggregation event {event} must classify feeds_digest"
            );
        }
        for event in &vocab.plane_telemetry_events {
            assert_eq!(
                emit.get(event).map(String::as_str),
                Some("plane_own"),
                "plane event {event} must classify plane_own"
            );
        }
        // The verified source set (protocol review §1; the mechanical
        // source scan T/R's tie performs extends this in the same
        // confirmation edit — after freeze, by amendment).
        for event in [
            "store.opened",
            "tracker.configured",
            "gateway.failed",
            "engine.gpu_fallback",
            "identity.loaded",
            "identity.created",
            "serving.session_ended",
            "serving.clamped",
            "serving.refused",
            "serving.stopped",
            "verifydrafts.verify_ms_lie",
            "verifydrafts.verifier_divergence",
            "verifydrafts.budget_refused",
            "net.metrics.store_unavailable",
            "net.metrics.invalid_key",
            "net.metrics.persist_failed",
            "listener.error",
            "listener.bound",
            "family.pin",
            "family.fallback",
            "family.engine_ready",
            "family.enabled",
            "family.probe",
            "family.engage",
            "chat.cleared",
            "chat.swarm.error",
            "chat.swarm.fallback",
            "chat.swarm.empty",
            "lease.earn",
            "sched.shadow.policy",
            "sched.shadow.decision",
            "sched.shadow.realized",
            "advisory.cycle.sent",
            "advisory.cycle.gap",
            "advisory.suggestion.recorded",
            "assistant.turn.sent",
            "assistant.turn.gap",
        ] {
            assert!(
                emit.contains_key(event),
                "source event {event} missing a row"
            );
        }
        // ADR-037 §4.2: the assistant turn events are plane_own rows and
        // the plane enum is exactly digest | assistant.
        assert_eq!(
            emit.get("assistant.turn.sent").map(String::as_str),
            Some("plane_own")
        );
        assert_eq!(
            emit.get("assistant.turn.gap").map(String::as_str),
            Some("plane_own")
        );
        assert_eq!(
            vocab.plane_enum.values,
            vec!["digest".to_string(), "assistant".to_string()]
        );
    }

    /// ADR-037 §3.5: the context-block gate drops unknown names, wrong
    /// types, out-of-vocabulary enums, over-bound strings, foreign
    /// vocabulary borrows, and oversized picker arrays — and admits the
    /// typed shapes. The post-serialization re-scan re-matches the SAME
    /// bindings (advisory A-1's defense-in-depth).
    #[test]
    fn assistant_context_gate_admits_typed_and_drops_the_rest() {
        use super::{admit_context, choke_point_rescan_assistant, ContextField, ContextValue};
        let f = |name: &str, value: ContextValue| ContextField {
            name: name.to_string(),
            value,
        };
        let mut counts = std::collections::BTreeMap::new();
        counts.insert("timeout".to_string(), 3);
        counts.insert("schema".to_string(), 1);
        let admitted = admit_context(vec![
            f("hw.logical_cpus", ContextValue::Int(16)),
            f("hosting.on", ContextValue::Bool(true)),
            f("app.version", ContextValue::Version("0.2.24+meta".into())),
            f(
                "hw.gpu.name",
                ContextValue::GpuName("NVIDIA GeForce RTX 5080 Laptop GPU".into()),
            ),
            f(
                "family.off_reason",
                ContextValue::Enum {
                    vocabulary: "family_off_reason",
                    value: "resource_ceiling".into(),
                },
            ),
            // A value from ANOTHER vocabulary under this field's name:
            // dropped (the declared vocabulary is the authority).
            f(
                "family.state",
                ContextValue::Enum {
                    vocabulary: "family_state",
                    value: "resource_ceiling".into(),
                },
            ),
            f(
                "status.advisory_gap_counts",
                ContextValue::VocabCountMap {
                    vocabulary: "advisory_gap",
                    counts: counts.clone(),
                },
            ),
            f(
                "picker.rows",
                ContextValue::PickerRows(vec![super::ContextPickerRow {
                    display_name: "Qwen2.5 1.5B Instruct Q4_K_M".into(),
                    quant_method: "q4_k_m".into(),
                    quant_bits: 4,
                    download_mb: 105,
                    ram_mb_est: 512,
                    fit: Some("gpu_full".into()),
                    recommended: true,
                    census_peers: Some(2),
                }]),
            ),
            f("not.a.field", ContextValue::Int(1)),
            f("prompt", ContextValue::Int(1)), // forbidden name
            f(
                "hw.gpu.name",
                ContextValue::GpuName(format!("X{}", "x".repeat(200))),
            ),
            f("app.version", ContextValue::Version("bad version!".into())),
        ]);
        assert_eq!(
            admitted.dropped, 5,
            "unknown field, forbidden name, foreign enum, over-bound gpu name, bad version"
        );
        assert_eq!(admitted.body["hw.logical_cpus"], 16);
        assert_eq!(admitted.body["hosting.on"], true);
        assert_eq!(admitted.body["app.version"], "0.2.24+meta");
        assert_eq!(admitted.body["family.off_reason"], "resource_ceiling");
        assert!(admitted.body.get("family.state").is_none());
        let gaps = admitted.body["status.advisory_gap_counts"]
            .as_object()
            .unwrap();
        assert_eq!(gaps["timeout"], 3);
        assert_eq!(gaps["schema"], 1);
        assert_eq!(
            admitted.body["picker.rows"][0]["display_name"],
            "Qwen2.5 1.5B Instruct Q4_K_M"
        );
        // A poisoned count map (one key outside the frozen vocabulary)
        // drops WHOLE — fail closed, never a partial count set.
        let mut poisoned = std::collections::BTreeMap::new();
        poisoned.insert("timeout".to_string(), 3);
        poisoned.insert("not_a_gap".to_string(), 1);
        let strict = admit_context(vec![f(
            "status.advisory_gap_counts",
            ContextValue::VocabCountMap {
                vocabulary: "advisory_gap",
                counts: poisoned,
            },
        )]);
        assert_eq!(strict.dropped, 1);
        assert!(strict.body.is_empty());
        // The serialized context re-matches through the assistant
        // choke-point re-scan (role: context only here).
        let body = serde_json::json!({
            "context": admitted.body,
            "history": [{"role": "user", "content": "hello"}],
            "turn": {"conversation_id": "0123456789abcdef", "turn": 1, "text": "hi"}
        })
        .to_string();
        assert!(choke_point_rescan_assistant(&body, 65_536).is_ok());
        // Post-gate corruption of one context field stops the request.
        let mut parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        parsed["context"]["hw.logical_cpus"] = serde_json::json!("sixteen");
        assert!(choke_point_rescan_assistant(&parsed.to_string(), 65_536).is_err());
        // A smuggled field name in the context fails the re-scan.
        let mut parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        parsed["context"]["smuggled"] = serde_json::json!(1);
        assert!(choke_point_rescan_assistant(&parsed.to_string(), 65_536).is_err());
        // Oversize fails.
        assert!(choke_point_rescan_assistant(&body, 8).is_err());
    }
}
