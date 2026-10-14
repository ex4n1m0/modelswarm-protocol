//! The pure advisory-cycle state machine (ADR-033 §4, TR-RC-4/TR-AD-2):
//! `step(state, input) -> (state, effects)` over an INJECTED clock (the
//! `now_ms` rides every time-bearing input) and an INJECTED transport
//! double (a CALL is an [`Effect`]; the RUNNER executes it and feeds the
//! outcome back as [`Input::CallOutcome`]). There is NO reqwest, NO
//! wall clock, and NO RNG in here — cycle nonces arrive with the timer
//! input (the runner generates them with `identity::new_nonce()`, the
//! RC-8 generator pin).
//!
//! States (per the ADR's diagram): `DORMANT → COLLECT → REDACT → CALL →
//! PARSE → RECORD → COOLDOWN → DORMANT`. COLLECT/REDACT/PARSE/RECORD
//! are pure computations inside `step`; the only cross-`step` waiting
//! phases are `Dormant` and `AwaitingCall`. COOLDOWN is the
//! `min_interval_ms` gate re-checked on the next timer input.
//!
//! Cap accounting (TR-RC-4): every CALL transition consumes one unit of
//! `max_calls_per_day` regardless of outcome — caps count calls
//! INITIATED, not successes. The daily window is the UTC calendar day
//! (a flood straddling midnight UTC legally earns a fresh allowance —
//! intended, and pinned by test). No in-cycle retry: the next cycle is
//! the retry.

use super::egress::{self, DigestField, HintAdmission};

/// The frozen 12-value gap vocabulary (ADR-033 §6 at this leg;
/// ADR-037 §4.3's `rate_limited`/`request_oversized` land with the
/// assistant leg). A further value needs an amendment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    Disabled,
    NoKey,
    NoConfig,
    CapReached,
    Interval,
    EmptyAfterRedaction,
    Timeout,
    HttpError,
    Quota,
    ResponseOversized,
    Schema,
    /// Diagnostic-only (TR-RC-4/R-6): fires exactly at the discard of a
    /// response arriving after its cycle closed — permitted ONLY
    /// following a `Timeout` gap for the same cycle.
    Late,
}

impl Gap {
    /// The frozen wire label (matches the artifact row byte-for-byte).
    pub fn as_str(self) -> &'static str {
        match self {
            Gap::Disabled => "disabled",
            Gap::NoKey => "no_key",
            Gap::NoConfig => "no_config",
            Gap::CapReached => "cap_reached",
            Gap::Interval => "interval",
            Gap::EmptyAfterRedaction => "empty_after_redaction",
            Gap::Timeout => "timeout",
            Gap::HttpError => "http_error",
            Gap::Quota => "quota",
            Gap::ResponseOversized => "response_oversized",
            Gap::Schema => "schema",
            Gap::Late => "late",
        }
    }

    /// All values, in the artifact's frozen order (the golden enum tie:
    /// this list, the artifact row, and the ADR §6 vocabulary are one).
    pub const ALL: [Gap; 12] = [
        Gap::Disabled,
        Gap::NoKey,
        Gap::NoConfig,
        Gap::CapReached,
        Gap::Interval,
        Gap::EmptyAfterRedaction,
        Gap::Timeout,
        Gap::HttpError,
        Gap::Quota,
        Gap::ResponseOversized,
        Gap::Schema,
        Gap::Late,
    ];
}

/// Frozen suggestion types (ADR-033 §2 — four digest-plane classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionType {
    TriageSummary,
    TuningHint,
    CatalogNote,
    Digest,
}

impl SuggestionType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "triage_summary" => Some(Self::TriageSummary),
            "tuning_hint" => Some(Self::TuningHint),
            "catalog_note" => Some(Self::CatalogNote),
            "digest" => Some(Self::Digest),
            _ => None,
        }
    }
}

/// Frozen confidence levels (ADR-033 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// A suggestion's target: a 16-hex profile prefix or the literal
/// `fleet` (ADR-033 §4 schema).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionTarget {
    Fleet,
    Profile16Hex(String),
}

/// One RECORDED suggestion — the ledger-record type of ADR-033 §5
/// ("No automatic promotion path may exist in code"): its hint values
/// are the ALREADY-CLAMPED, ALREADY-TYPED numbers; the raw provider
/// `hint` map is not exposed. `Display` renders the operator summary
/// under the RC-5 contract (untrusted-provenance label, plain text).
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedSuggestion {
    pub kind: SuggestionType,
    pub target: SuggestionTarget,
    pub confidence: Confidence,
    /// `(parameter, clamped_value, clamped, requires_adr)` — values
    /// inside the frozen clamp table's bounds or `requires_adr`-flagged.
    pub hints: Vec<(String, f64, bool, bool)>,
    /// Display-only free text; NEVER parsed. Rendered plain-text,
    /// capped, with the untrusted-provenance label adjacent (RC-5).
    pub rationale: String,
}

impl RecordedSuggestion {
    /// The frozen wire label of the suggestion type (telemetry label).
    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            SuggestionType::TriageSummary => "triage_summary",
            SuggestionType::TuningHint => "tuning_hint",
            SuggestionType::CatalogNote => "catalog_note",
            SuggestionType::Digest => "digest",
        }
    }

    /// The frozen confidence label (telemetry label).
    pub fn confidence_str(&self) -> &'static str {
        match self.confidence {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

impl std::fmt::Display for RecordedSuggestion {
    /// The RC-5 rendering: the untrusted-provenance label travels WITH
    /// the summary — every consumer of the Display form re-states it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[advisory — untrusted, metadata-derived] {:?} target={} confidence={:?}",
            self.kind,
            match &self.target {
                SuggestionTarget::Fleet => "fleet".to_string(),
                SuggestionTarget::Profile16Hex(h) => h.clone(),
            },
            self.confidence
        )?;
        for (param, value, clamped, requires_adr) in &self.hints {
            write!(
                f,
                " {param}={value}{}{}",
                if *clamped { " (clamped)" } else { "" },
                if *requires_adr { " (requires_adr)" } else { "" }
            )?;
        }
        write!(f, " — {}", truncate_chars(&self.rationale, 2000))
    }
}

fn truncate_chars(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let head: String = text.chars().take(cap).collect();
    format!("{head}…[TRUNCATED]")
}

/// Transport failure classes (the §4 malformed table's injected
/// conditions; produced by the transport double in tests, by the pinned
/// reqwest adapter in production).
#[derive(Debug, Clone, PartialEq)]
pub enum TransportFailure {
    /// Deadline exceeded → gap `timeout` (a late response afterwards is
    /// the no-op discard + diagnostic `late`).
    Deadline,
    /// HTTP status. 3xx arrives here too — redirects are NEVER followed
    /// (ADR-033 §3.3 RC-9); 402 maps to gap `quota`, others to
    /// `http_error`.
    HttpStatus(u16),
    /// The response body exceeded `max_response_bytes` (detected at the
    /// adapter while buffering, or re-checked on returned bytes) → gap
    /// `response_oversized`.
    ResponseOversized,
    /// Network-level failure → gap `http_error`.
    Network(String),
}

/// The provider request — the ONLY egress payload. The endpoint and
/// model are deliberately ABSENT: they live in the config-pinned
/// transport (constructed from `advisory.json` and refused for any
/// non-https endpoint), so no value inside the pure machine — corrupted
/// or otherwise — can redirect a call to another host. The key never
/// appears here either (the transport adds the Authorization header;
/// the allowlist has no key-bearing field type).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRequest {
    /// The serialized, choke-point-verified digest body.
    pub body: String,
    /// Per-call deadline, ms.
    pub timeout_ms: u64,
    /// Provider-side output bound.
    pub max_completion_tokens: u32,
    /// Response body cap (the adapter refuses to buffer beyond it; the
    /// state machine re-checks on the returned bytes).
    pub max_response_bytes: usize,
}

/// Effects the runner must execute. Counts and labels only on the
/// telemetry surface (the plane's own `advisory.*` events are frozen in
/// the egress artifact).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// `advisory.cycle.gap` — one PRIMARY gap per failure edge (`Late`
    /// is the diagnostic-only secondary, fired only after `Timeout` for
    /// the same cycle).
    Gap {
        reason: Gap,
        cycle_id: Option<String>,
    },
    /// `advisory.cycle.sent`.
    CycleSent { cycle_id: String },
    /// `advisory.suggestion.recorded` per parsed suggestion.
    SuggestionRecorded {
        cycle_id: String,
        suggestion: RecordedSuggestion,
    },
    /// Execute one HTTPS call via the injected transport; feed the
    /// outcome back as [`Input::CallOutcome`]. There is no other call
    /// site (invariant A5; pinned by the chat-wall suite).
    Call(ProviderRequest),
    /// Count of fields dropped at the egress gate this cycle (a count,
    /// never content).
    GateDropped { count: u64 },
    /// A single suggestion discarded at parse (counted, never content).
    SuggestionDiscarded { cycle_id: String },
}

/// The inputs. All time is virtual (`now_ms` fields); the cycle nonce
/// arrives here so `step` stays RNG-free.
#[derive(Debug, Clone)]
pub enum Input {
    /// The cycle timer fired at `now_ms` with the accumulated-since-
    /// last-cycle snapshot.
    Timer {
        now_ms: u64,
        digest: Vec<DigestField>,
        /// A fresh 32-hex nonce (runner-generated via
        /// `identity::new_nonce()`); the cycle id is its first 16 hex
        /// (RC-8).
        cycle_nonce: String,
    },
    /// Telemetry events observed (the `events_threshold` feed).
    Events(u64),
    /// The transport outcome for the in-flight call.
    CallOutcome {
        cycle_id: String,
        outcome: Result<Vec<u8>, TransportFailure>,
    },
    /// A response arrived after its cycle closed (late-discard path).
    LateResponse { cycle_id: String },
}

/// The machine's waiting phase.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Dormant,
    /// A call is in flight for `cycle_id` (started at `now_ms`).
    AwaitingCall {
        cycle_id: String,
        now_ms: u64,
    },
}

/// The advisory cycle state. Pure data — no I/O, no clock, no RNG.
#[derive(Debug, Clone)]
pub struct AdvisoryState {
    /// The resolved digest-plane caps (ADR-033 §4 config table).
    pub caps: Caps,
    /// `enabled && key_present && endpoint+model configured (https)`.
    pub armed: bool,
    pub phase: Phase,
    /// UTC day number the cap window counts (`now_ms / 86_400_000`).
    pub day: u64,
    /// CALLS INITIATED today (failures included).
    pub calls_today: u32,
    /// When the last call was initiated (the `min_interval_ms` gate).
    pub last_call_ms: Option<u64>,
    /// New allowlisted telemetry events since the last initiated call.
    pub events_pending: u64,
    /// Recently closed-with-timeout cycle ids (bounded) — the only
    /// cycles a `LateResponse` may diagnose.
    timed_out: Vec<String>,
}

/// The digest-plane caps carried by the state (from
/// [`AdvisoryConfig`][crate::advisory::config::AdvisoryConfig]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caps {
    pub enabled: bool,
    pub key_present: bool,
    pub endpoint_configured: bool,
    pub max_calls_per_day: u32,
    pub min_interval_ms: u64,
    pub events_threshold: u64,
    pub request_timeout_ms: u64,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_completion_tokens: u32,
}

impl Caps {
    /// The armed default used by tests: everything configured, caps at
    /// the frozen defaults.
    pub fn armed_defaults() -> Self {
        Self {
            enabled: true,
            key_present: true,
            endpoint_configured: true,
            max_calls_per_day: 24,
            min_interval_ms: 900_000,
            events_threshold: 200,
            request_timeout_ms: 30_000,
            max_request_bytes: 16_384,
            max_response_bytes: 65_536,
            max_completion_tokens: 2_048,
        }
    }
}

impl AdvisoryState {
    /// A fresh, dormant machine.
    pub fn new(caps: Caps, armed: bool, now_ms: u64) -> Self {
        Self {
            caps,
            armed,
            phase: Phase::Dormant,
            day: utc_day(now_ms),
            calls_today: 0,
            last_call_ms: None,
            events_pending: 0,
            timed_out: Vec::new(),
        }
    }

    fn close(&mut self) {
        self.phase = Phase::Dormant;
    }

    fn record_gap(&mut self, effects: &mut Vec<Effect>, reason: Gap, cycle_id: Option<String>) {
        if reason == Gap::Timeout {
            if let Some(id) = cycle_id.clone() {
                self.timed_out.retain(|t| t != &id);
                self.timed_out.push(id);
                // Bounded: the recent window only.
                let len = self.timed_out.len();
                if len > 16 {
                    self.timed_out.drain(0..len - 16);
                }
            }
        }
        effects.push(Effect::Gap { reason, cycle_id });
        self.close();
    }
}

/// UTC calendar-day number of a virtual timestamp (the frozen daily
/// window — TR-RC-4).
pub fn utc_day(now_ms: u64) -> u64 {
    now_ms / 86_400_000
}

/// The pure transition. Deterministic in `(state, input)` alone.
pub fn step(mut state: AdvisoryState, input: Input) -> (AdvisoryState, Vec<Effect>) {
    let mut effects = Vec::new();
    match input {
        Input::Events(count) => {
            state.events_pending += count;
        }
        Input::Timer {
            now_ms,
            digest,
            cycle_nonce,
        } => {
            step_timer(&mut state, now_ms, digest, cycle_nonce, &mut effects);
        }
        Input::CallOutcome { cycle_id, outcome } => {
            step_call_outcome(&mut state, &cycle_id, outcome, &mut effects);
        }
        Input::LateResponse { cycle_id } => {
            // Diagnostic-only (R-6): permitted ONLY after a timeout gap
            // for the same cycle; every other late arrival is a no-op
            // discard (the gap was already recorded).
            if state.timed_out.contains(&cycle_id) {
                state.timed_out.retain(|t| *t != cycle_id);
                effects.push(Effect::Gap {
                    reason: Gap::Late,
                    cycle_id: Some(cycle_id),
                });
            }
        }
    }
    (state, effects)
}

fn step_timer(
    state: &mut AdvisoryState,
    now_ms: u64,
    digest: Vec<DigestField>,
    cycle_nonce: String,
    effects: &mut Vec<Effect>,
) {
    // UTC-day rollover resets the cap window (intended; pinned by test).
    let day = utc_day(now_ms);
    if day != state.day {
        state.day = day;
        state.calls_today = 0;
    }

    // DORMANT gate conditions (ADR-033 §4) — first failure wins.
    if !state.caps.enabled || !state.armed {
        state.record_gap(effects, Gap::Disabled, None);
        return;
    }
    if !state.caps.key_present {
        state.record_gap(effects, Gap::NoKey, None);
        return;
    }
    if !state.caps.endpoint_configured {
        state.record_gap(effects, Gap::NoConfig, None);
        return;
    }
    if state.caps.max_calls_per_day == 0 || state.calls_today >= state.caps.max_calls_per_day {
        state.record_gap(effects, Gap::CapReached, None);
        return;
    }
    if let Some(last) = state.last_call_ms {
        if now_ms.saturating_sub(last) < state.caps.min_interval_ms {
            state.record_gap(effects, Gap::Interval, None);
            return;
        }
    }
    // Quiet operation, not a failure: below the events threshold the
    // cycle simply does not start (no gap in the frozen vocabulary
    // covers it — by design).
    if state.events_pending < state.caps.events_threshold {
        return;
    }

    // COLLECT + REDACT: the typed snapshot through the ONE value-typed
    // gate, Redactor net included.
    let admitted = egress::admit(digest);
    if admitted.dropped > 0 {
        effects.push(Effect::GateDropped {
            count: admitted.dropped,
        });
    }
    let body = serde_json::to_string(&admitted.body).unwrap_or_default();
    if admitted.body.is_empty() {
        // Zero fields survived: nothing is sent.
        state.record_gap(effects, Gap::EmptyAfterRedaction, None);
        return;
    }
    // The choke-point re-scan on the FINAL serialized bytes (RC-3). A
    // digest body over the byte cap fails closed the same way — the
    // 12-value digest vocabulary has no request_oversized (that value
    // arrives with the assistant leg, ADR-037 §4.3); nothing is sent.
    if egress::choke_point_rescan(&body, state.caps.max_request_bytes).is_err() {
        state.record_gap(effects, Gap::EmptyAfterRedaction, None);
        return;
    }

    // cycle_id: the first 16 hex of the runner-injected fresh nonce
    // (RC-8's generator pin lives in the runner).
    let cycle_id: String = cycle_nonce.chars().take(16).collect();
    if cycle_id.len() != 16 || !cycle_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        // A malformed nonce is a config-side defect: fail closed, no call.
        state.record_gap(effects, Gap::EmptyAfterRedaction, None);
        return;
    }

    // CALL: consumes cap budget (calls INITIATED, not successes) and
    // starts the cooldown window.
    state.calls_today += 1;
    state.last_call_ms = Some(now_ms);
    state.events_pending = 0;
    state.phase = Phase::AwaitingCall {
        cycle_id: cycle_id.clone(),
        now_ms,
    };
    effects.push(Effect::Call(ProviderRequest {
        body,
        timeout_ms: state.caps.request_timeout_ms,
        max_completion_tokens: state.caps.max_completion_tokens,
        max_response_bytes: state.caps.max_response_bytes,
    }));
}

fn step_call_outcome(
    state: &mut AdvisoryState,
    cycle_id: &str,
    outcome: Result<Vec<u8>, TransportFailure>,
    effects: &mut Vec<Effect>,
) {
    // Response after cycle closed / duplicate cycle_id: no-op discard
    // (the gap was already recorded).
    let Phase::AwaitingCall {
        cycle_id: in_flight,
        ..
    } = &state.phase
    else {
        return;
    };
    if in_flight.as_str() != cycle_id {
        return;
    }

    match outcome {
        Err(failure) => {
            let gap = match failure {
                TransportFailure::Deadline => Gap::Timeout,
                TransportFailure::HttpStatus(402) => Gap::Quota,
                TransportFailure::HttpStatus(_) => Gap::HttpError,
                TransportFailure::ResponseOversized => Gap::ResponseOversized,
                TransportFailure::Network(_) => Gap::HttpError,
            };
            state.record_gap(effects, gap, Some(cycle_id.to_string()));
        }
        Ok(bytes) => {
            if bytes.len() > state.caps.max_response_bytes {
                state.record_gap(effects, Gap::ResponseOversized, Some(cycle_id.to_string()));
                return;
            }
            match parse_response(&bytes, cycle_id) {
                Err(()) => {
                    // Bad JSON / unknown schema / cycle_id echo mismatch:
                    // whole body discarded.
                    state.record_gap(effects, Gap::Schema, Some(cycle_id.to_string()));
                }
                Ok(suggestions) => {
                    for s in suggestions {
                        match s {
                            Ok(s) => {
                                effects.push(Effect::SuggestionRecorded {
                                    cycle_id: cycle_id.to_string(),
                                    suggestion: s,
                                });
                            }
                            Err(()) => effects.push(Effect::SuggestionDiscarded {
                                cycle_id: cycle_id.to_string(),
                            }),
                        }
                    }
                    effects.push(Effect::CycleSent {
                        cycle_id: cycle_id.to_string(),
                    });
                    state.close();
                }
            }
        }
    }
}

/// PARSE (strict suggestion schema, first-failure-wins per the §4
/// malformed table). Envelope failures ⇒ `Err(())` (whole-body
/// discard, gap `schema`); per-suggestion failures ⇒ `Ok(vec![Err(()),
/// …])` siblings still parse.
fn parse_response(bytes: &[u8], cycle_id: &str) -> Result<Vec<Result<RecordedSuggestion, ()>>, ()> {
    let parsed: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    let obj = parsed.as_object().ok_or(())?;
    if obj.get("schema").and_then(|s| s.as_str()) != Some("advisory-suggestion/v1") {
        return Err(());
    }
    // RC-8: a response whose cycle_id does not echo the in-flight
    // cycle's is discarded whole.
    if obj.get("cycle_id").and_then(|s| s.as_str()) != Some(cycle_id) {
        return Err(());
    }
    let Some(raw_suggestions) = obj.get("suggestions").and_then(|s| s.as_array()) else {
        return Err(());
    };
    let mut out = Vec::new();
    for raw in raw_suggestions {
        out.push(parse_one_suggestion(raw));
    }
    Ok(out)
}

fn parse_one_suggestion(raw: &serde_json::Value) -> Result<RecordedSuggestion, ()> {
    let obj = raw.as_object().ok_or(())?;
    let kind =
        SuggestionType::parse(obj.get("type").and_then(|t| t.as_str()).ok_or(())?).ok_or(())?;
    let confidence =
        Confidence::parse(obj.get("confidence").and_then(|c| c.as_str()).ok_or(())?).ok_or(())?;
    let target = match obj.get("target").and_then(|t| t.as_str()) {
        Some("fleet") => SuggestionTarget::Fleet,
        Some(hex16) => {
            if !egress::is_profile_ref16(hex16) {
                return Err(());
            }
            SuggestionTarget::Profile16Hex(hex16.to_string())
        }
        None => return Err(()),
    };
    let rationale = obj
        .get("rationale")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();

    // Hints: numeric values clamped per the frozen table; non-finite
    // (NaN/Inf, however encoded) discards the SUGGESTION, siblings
    // parse (TR-AD-1); an absent target is requires_adr=true — never an
    // improvised bound.
    let mut hints = Vec::new();
    if let Some(map) = obj.get("hint").and_then(|h| h.as_object()) {
        for (param, value) in map {
            let num = value.as_f64().ok_or(())?;
            if !num.is_finite() {
                return Err(());
            }
            match egress::clamp_hint(param, num) {
                HintAdmission::InRange(v) => hints.push((param.clone(), v, false, false)),
                HintAdmission::Clamped(v) => hints.push((param.clone(), v, true, false)),
                HintAdmission::RequiresAdr => hints.push((param.clone(), num, false, true)),
            }
        }
    }
    Ok(RecordedSuggestion {
        kind,
        target,
        confidence,
        hints,
        rationale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn armed(now_ms: u64) -> AdvisoryState {
        AdvisoryState::new(Caps::armed_defaults(), true, now_ms)
    }

    fn digest_of(count: u64) -> Vec<DigestField> {
        vec![DigestField {
            name: "serving.session_ended.count".to_string(),
            value: super::super::egress::TypedValue::Int(count as i64),
        }]
    }

    fn timer(now_ms: u64, nonce: &str) -> Input {
        Input::Timer {
            now_ms,
            digest: digest_of(42),
            cycle_nonce: nonce.to_string(),
        }
    }

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn golden_gap_enum_is_the_twelve_frozen_values() {
        // TR-AD-3: a new gap value without an amendment note is a red
        // build — here, in the artifact row, and in the ADR §6 text.
        let labels: Vec<&str> = Gap::ALL.iter().map(|g| g.as_str()).collect();
        assert_eq!(
            labels,
            vec![
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
                "late"
            ]
        );
        // And the artifact row matches this enum exactly.
        let artifact = &super::super::egress::vocabulary().vocabularies["advisory_gap"].values;
        assert_eq!(&labels, artifact);
    }

    #[test]
    fn disabled_keyless_and_unconfigured_are_primary_gaps_and_dormant() {
        let mut s = armed(0);
        s.caps.enabled = false;
        let (s2, e) = step(s, timer(1_000, NONCE));
        assert_eq!(
            e,
            vec![Effect::Gap {
                reason: Gap::Disabled,
                cycle_id: None
            }]
        );
        assert_eq!(s2.phase, Phase::Dormant);

        let mut s = armed(0);
        s.caps.key_present = false;
        let (_, e) = step(s, timer(1_000, NONCE));
        assert_eq!(
            e,
            vec![Effect::Gap {
                reason: Gap::NoKey,
                cycle_id: None
            }]
        );

        let mut s = armed(0);
        s.caps.endpoint_configured = false;
        let (_, e) = step(s, timer(1_000, NONCE));
        assert_eq!(
            e,
            vec![Effect::Gap {
                reason: Gap::NoConfig,
                cycle_id: None
            }]
        );
    }

    #[test]
    fn below_threshold_is_quiet_not_a_gap() {
        let s = armed(0);
        let (s2, e) = step(s, timer(1_000, NONCE));
        assert!(e.is_empty(), "no gate has failed and 0 < 200 events");
        assert_eq!(s2.phase, Phase::Dormant);
    }

    #[test]
    fn armed_cycle_emits_one_call_and_consumes_budget() {
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let (s2, e) = step(s, timer(1_000, NONCE));
        assert_eq!(s2.calls_today, 1);
        assert_eq!(s2.last_call_ms, Some(1_000));
        assert_eq!(s2.events_pending, 0);
        assert_eq!(
            s2.phase,
            Phase::AwaitingCall {
                cycle_id: "0123456789abcdef".into(),
                now_ms: 1_000
            }
        );
        assert_eq!(e.len(), 1);
        match &e[0] {
            Effect::Call(r) => {
                assert_eq!(r.timeout_ms, 30_000);
                assert_eq!(r.max_completion_tokens, 2_048);
                assert!(r.body.contains("serving.session_ended.count"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_after_redaction_when_nothing_survives_the_gate() {
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let input = Input::Timer {
            now_ms: 1_000,
            // Every field fails the gate (forbidden name / free text).
            digest: vec![DigestField {
                name: "prompt".into(),
                value: crate::advisory::egress::TypedValue::Int(1),
            }],
            cycle_nonce: NONCE.into(),
        };
        let (s2, e) = step(s, input);
        assert_eq!(s2.phase, Phase::Dormant);
        assert_eq!(s2.calls_today, 0, "no call initiated, no budget spent");
        assert!(matches!(
            e.as_slice(),
            [
                Effect::GateDropped { count: 1 },
                Effect::Gap {
                    reason: Gap::EmptyAfterRedaction,
                    cycle_id: None
                }
            ]
        ));
    }

    #[test]
    fn transport_failures_map_to_primary_gaps_and_stay_dormant() {
        for (failure, gap) in [
            (TransportFailure::Deadline, Gap::Timeout),
            (TransportFailure::HttpStatus(500), Gap::HttpError),
            (TransportFailure::HttpStatus(302), Gap::HttpError),
            (TransportFailure::HttpStatus(402), Gap::Quota),
            (TransportFailure::Network("dns".into()), Gap::HttpError),
        ] {
            let s = armed(0);
            let s = step(s, Input::Events(200)).0;
            let s = step(s, timer(1_000, NONCE)).0;
            assert_eq!(
                s.calls_today, 1,
                "the failed call consumed budget ({gap:?})"
            );
            let (s2, e) = step(
                s,
                Input::CallOutcome {
                    cycle_id: "0123456789abcdef".into(),
                    outcome: Err(failure),
                },
            );
            assert_eq!(s2.phase, Phase::Dormant, "{gap:?}");
            assert_eq!(
                e,
                vec![Effect::Gap {
                    reason: gap,
                    cycle_id: Some("0123456789abcdef".into())
                }],
                "failure class {gap:?}"
            );
        }
    }

    #[test]
    fn late_fires_only_after_timeout_for_the_same_cycle() {
        let cycle = "0123456789abcdef".to_string();
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let (s, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: cycle.clone(),
                outcome: Err(TransportFailure::Deadline),
            },
        );
        assert_eq!(e.len(), 1, "exactly one PRIMARY gap (timeout)");
        // The late response for the SAME cycle: diagnostic late fires.
        let (s, e) = step(
            s,
            Input::LateResponse {
                cycle_id: cycle.clone(),
            },
        );
        assert_eq!(
            e,
            vec![Effect::Gap {
                reason: Gap::Late,
                cycle_id: Some(cycle.clone())
            }]
        );
        // Twice-late: no-op (already diagnosed).
        let (_, e) = step(
            s,
            Input::LateResponse {
                cycle_id: cycle.clone(),
            },
        );
        assert!(e.is_empty());
        // A late response for a cycle that did NOT time out: no-op.
        let other = "ffffffffffffffff".to_string();
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let (_, e) = step(s, Input::LateResponse { cycle_id: other });
        assert!(e.is_empty());
    }

    fn ok_body(cycle_id: &str) -> Vec<u8> {
        format!(
            r#"{{"schema":"advisory-suggestion/v1","cycle_id":"{cycle_id}","suggestions":[{{"type":"digest","target":"fleet","confidence":"low","rationale":"fleet healthy"}}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn success_records_suggestions_and_returns_dormant() {
        let cycle = "0123456789abcdef".to_string();
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let (s2, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: cycle.clone(),
                outcome: Ok(ok_body(&cycle)),
            },
        );
        assert_eq!(s2.phase, Phase::Dormant);
        assert_eq!(e.len(), 2, "one suggestion.recorded + cycle.sent: {e:?}");
        assert!(matches!(&e[0], Effect::SuggestionRecorded { cycle_id: c, .. } if c == &cycle));
        assert!(matches!(&e[1], Effect::CycleSent { cycle_id: c } if c == &cycle));
    }

    #[test]
    fn foreign_cycle_id_discards_whole_body_with_schema_gap() {
        let cycle = "0123456789abcdef".to_string();
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let (s2, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: cycle.clone(),
                outcome: Ok(ok_body("aaaabbbbccccdddd")),
            },
        );
        assert_eq!(s2.phase, Phase::Dormant);
        assert_eq!(
            e,
            vec![Effect::Gap {
                reason: Gap::Schema,
                cycle_id: Some(cycle)
            }]
        );
        // Invalid JSON and unknown schema too.
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let (_, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: "0123456789abcdef".into(),
                outcome: Ok(b"{nope".to_vec()),
            },
        );
        assert!(matches!(
            e.as_slice(),
            [Effect::Gap {
                reason: Gap::Schema,
                ..
            }]
        ));
    }

    #[test]
    fn response_over_cap_is_a_primary_gap() {
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let huge = vec![b'x'; 65_537];
        let (s2, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: "0123456789abcdef".into(),
                outcome: Ok(huge),
            },
        );
        assert_eq!(s2.phase, Phase::Dormant);
        assert!(matches!(
            e.as_slice(),
            [Effect::Gap {
                reason: Gap::ResponseOversized,
                ..
            }]
        ));
    }

    #[test]
    fn malformed_suggestions_discard_and_count_siblings_parse() {
        let cycle = "0123456789abcdef".to_string();
        let body = format!(
            r#"{{"schema":"advisory-suggestion/v1","cycle_id":"{cycle}","suggestions":[
                {{"type":"digest","target":"fleet","confidence":"low","rationale":"ok"}},
                {{"type":"not_a_type","target":"fleet","confidence":"low","rationale":"bad type"}},
                {{"type":"digest","target":"fleet","confidence":"ultra","rationale":"bad confidence"}},
                {{"type":"tuning_hint","target":"fleet","confidence":"high","hint":{{"MSP_SHADOW_QUEUE_CAP_MS":9999999.0}},"rationale":"clamps"}},
                {{"type":"tuning_hint","target":"fleet","confidence":"high","hint":{{"MSP_UNKNOWN_KNOB":5.0}},"rationale":"adr"}},
                {{"type":"tuning_hint","target":"fleet","confidence":"high","hint":{{"MSP_SHADOW_QUEUE_CAP_MS":"NaN"}},"rationale":"nonfinite"}},
                {{"type":"tuning_hint","target":"not-hex","confidence":"low","rationale":"bad target"}}
            ]}}"#
        );
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        let (s2, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: cycle.clone(),
                outcome: Ok(body.into_bytes()),
            },
        );
        assert_eq!(s2.phase, Phase::Dormant);
        let recorded: Vec<_> = e
            .iter()
            .filter_map(|x| match x {
                Effect::SuggestionRecorded { suggestion, .. } => Some(suggestion.clone()),
                _ => None,
            })
            .collect();
        let discarded = e
            .iter()
            .filter(|x| matches!(x, Effect::SuggestionDiscarded { .. }))
            .count();
        assert_eq!(recorded.len(), 3, "{recorded:?}");
        assert_eq!(discarded, 4);
        // The clamp fired: 9_999_999 → 120_000 with clamped=true.
        let clamped = recorded.iter().find(|s| s.rationale == "clamps").unwrap();
        assert_eq!(
            clamped.hints,
            vec![(
                "MSP_SHADOW_QUEUE_CAP_MS".to_string(),
                120_000.0,
                true,
                false
            )]
        );
        // The absent target: requires_adr=true, value kept, never bounded.
        let adr = recorded.iter().find(|s| s.rationale == "adr").unwrap();
        assert_eq!(
            adr.hints,
            vec![("MSP_UNKNOWN_KNOB".to_string(), 5.0, false, true)]
        );
        // Body still counts as sent.
        assert!(e.iter().any(|x| matches!(x, Effect::CycleSent { .. })));
    }

    #[test]
    fn duplicate_or_stale_outcomes_are_noops() {
        let s = armed(0);
        let s = step(s, Input::Events(200)).0;
        let s = step(s, timer(1_000, NONCE)).0;
        // Outcome for a foreign cycle while one is in flight: ignored.
        let (s, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: "ffffffffffffffff".into(),
                outcome: Ok(ok_body("ffffffffffffffff")),
            },
        );
        assert!(e.is_empty());
        assert_eq!(
            s.phase,
            Phase::AwaitingCall {
                cycle_id: "0123456789abcdef".into(),
                now_ms: 1_000
            }
        );
        // After the cycle closed, a duplicate outcome: no-op.
        let s = step(
            s,
            Input::CallOutcome {
                cycle_id: "0123456789abcdef".into(),
                outcome: Ok(ok_body("0123456789abcdef")),
            },
        )
        .0;
        let (_, e) = step(
            s,
            Input::CallOutcome {
                cycle_id: "0123456789abcdef".into(),
                outcome: Ok(ok_body("0123456789abcdef")),
            },
        );
        assert!(e.is_empty(), "gap already recorded — duplicate is a no-op");
    }

    #[test]
    fn utc_midnight_rollover_resets_the_cap_window() {
        // Day 10 starts at 10*86_400_000 ms; a flood straddling midnight
        // legally earns a fresh allowance (intended, pinned).
        let midnight = 10 * 86_400_000u64;
        let mut s = armed(midnight - 1_000);
        s.caps.max_calls_per_day = 1;
        // Spend the single call of day 9.
        let s = step(s, Input::Events(200)).0;
        let s = step(
            s,
            Input::Timer {
                now_ms: midnight - 1_000,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            },
        )
        .0;
        assert_eq!(s.calls_today, 1);
        assert_eq!(s.day, 9);
        // Still day 9: cap_reached (checked before the interval gate).
        let (s, e) = step(
            s,
            Input::Timer {
                now_ms: midnight - 100,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            },
        );
        assert!(matches!(
            e.as_slice(),
            [Effect::Gap {
                reason: Gap::CapReached,
                ..
            }]
        ));
        assert_eq!(s.calls_today, 1);
        // After midnight UTC (and min_interval elapsed): fresh allowance.
        let s = step(s, Input::Events(200)).0;
        let (s2, e) = step(
            s,
            Input::Timer {
                now_ms: midnight + 900_000,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            },
        );
        assert_eq!(s2.day, 10);
        assert_eq!(s2.calls_today, 1, "the new day's first call");
        assert!(matches!(e.as_slice(), [Effect::Call(_)]));
    }

    #[test]
    fn min_interval_enforced_and_counts_failures_as_spending() {
        let s = armed(1_000);
        let s = step(s, Input::Events(300)).0;
        let s = step(
            s,
            Input::Timer {
                now_ms: 1_000,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            },
        )
        .0;
        // An early second timer: interval gap, even though events re-fed.
        let s = step(s, Input::Events(300)).0;
        let (s, e) = step(
            s,
            Input::Timer {
                now_ms: 1_000 + 899_999,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            },
        );
        assert!(matches!(
            e.as_slice(),
            [Effect::Gap {
                reason: Gap::Interval,
                ..
            }]
        ));
        // Exactly min_interval later: allowed.
        let (s2, e) = step(
            s,
            Input::Timer {
                now_ms: 1_000 + 900_000,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            },
        );
        assert!(matches!(e.as_slice(), [Effect::Call(_)]));
        assert_eq!(s2.calls_today, 2);
    }

    /// Adversarial arrival sweep (§9.2(3)): for any schedule the caps
    /// hold — calls per UTC day ≤ cap, spacing ≥ min_interval.
    #[test]
    fn adversarial_flood_never_beats_caps_on_the_virtual_clock() {
        // A splitmix-style schedule that always claims "enough events"
        // and fires the timer as fast as possible.
        let mut s = armed(0);
        s.caps.max_calls_per_day = 24;
        s = step(s, Input::Events(500)).0;
        let mut now = 0u64;
        let mut calls = 0u64;
        let mut last_call: Option<u64> = None;
        let mut per_day: BTreeMap<u64, u64> = Default::default();
        let mut seed = 0x9E3779B97F4A7C15u64;
        for _ in 0..5_000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let jump = seed % 200_000; // sometimes sub-interval, sometimes not
            now += jump;
            let input = Input::Timer {
                now_ms: now,
                digest: digest_of(1),
                cycle_nonce: NONCE.into(),
            };
            let s2 = step(s, input);
            s = s2.0;
            for effect in s2.1 {
                if let Effect::Call(_) = effect {
                    calls += 1;
                    *per_day.entry(utc_day(now)).or_insert(0) += 1;
                    if let Some(last) = last_call {
                        assert!(
                            now - last >= 900_000,
                            "spacing violated at {now} after {last}"
                        );
                    }
                    last_call = Some(now);
                    // Feed the outcome (always failing): failures consume
                    // budget, no retry this cycle.
                    let cycle = match &s.phase {
                        Phase::AwaitingCall { cycle_id, .. } => cycle_id.clone(),
                        _ => unreachable!(),
                    };
                    let s3 = step(
                        s,
                        Input::CallOutcome {
                            cycle_id: cycle,
                            outcome: Err(TransportFailure::HttpStatus(500)),
                        },
                    );
                    s = s3.0;
                    assert_eq!(s.phase, Phase::Dormant);
                    s = step(s, Input::Events(500)).0; // adversarial re-feed
                }
            }
        }
        assert!(
            calls > 24,
            "the sweep must actually exercise calls ({calls})"
        );
        for (day, count) in per_day {
            assert!(count <= 24, "day {day} saw {count} calls");
        }
    }
}
