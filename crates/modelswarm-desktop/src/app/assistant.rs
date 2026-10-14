//! The assistant PANEL surface (ADR-037 §3.2/§3.6/§4.5 — Windows
//! Product): the typed context-block builders, the §3.6 verbatim UI
//! texts, the label-keyed STATIC HELP CORPUS, the reply renderer
//! (plain text, 4000-char display cap, provenance chip), and the
//! turn/reset drivers behind the `assistant_turn`/`assistant_reset`
//! IPC commands. This file is one of the chat-wall suite's sanctioned
//! advisory-name homes (ADR-037 §4.5 routing); `app.rs` itself stays
//! advisory-name-free — the chat scopes live there (W1/W4).
//!
//! W4 (UI wall): everything here renders into the assistant's OWN
//! section with its own input and its own `assistant-*` webview event
//! namespace; nothing feeds or reads the swarm chat transcript, and no
//! chat surface renders assistant output.

use modelswarm_node::advisory::assistant::{AssistantPlane, TurnOutcome};
use modelswarm_node::advisory::egress::{ContextField, ContextPickerRow, ContextValue};
use modelswarm_telemetry::Telemetry;

use super::{FamilyStatus, HardwareInfo};

/// ADR-037 §3.6 (NORMATIVE UI CONTRACT — verbatim): the first-use
/// disclosure, shown before the FIRST cloud call on an install; the
/// call is BLOCKED until acknowledged.
pub const FIRST_USE_DISCLOSURE: &str = "This assistant sends what you type here, plus facts \
     about your machine and setup (hardware, model list, status labels — never your swarm \
     chats), to a cloud service (DeepSeek) using a key configured on this machine. It is \
     advice only — it cannot change any decision this app makes. You can turn it off any \
     time in Settings.";

/// ADR-037 §3.6 (verbatim): the persistent provenance chip, always
/// adjacent to any displayed cloud reply.
pub const PROVENANCE_CHIP: &str = "Cloud assistant — advice only; this app's rules decide.";

/// ADR-037 §3.6 (verbatim): the degraded-mode line, with the honest gap
/// label from the frozen vocabulary.
pub fn degraded_line(gap_label: &str) -> String {
    format!("Built-in help — the cloud assistant is off/unreachable ({gap_label}).")
}

/// The reply display cap (ADR-037 §3.7 rendering contract).
pub const REPLY_DISPLAY_CAP: usize = 4_000;

/// Renders one assistant reply for display: PLAIN TEXT ONLY (no
/// markdown or HTML interpretation — the webview renders textContent
/// and never parses this), truncated at the display cap.
pub fn render_reply(reply: &str) -> String {
    if reply.chars().count() <= REPLY_DISPLAY_CAP {
        return reply.to_string();
    }
    let head: String = reply.chars().take(REPLY_DISPLAY_CAP).collect();
    format!("{head}…[truncated]")
}

// ---------------------------------------------------------------------------
// The typed context-block builders (ADR-037 §3.5, W3-typed)
// ---------------------------------------------------------------------------

/// The desktop-typed inputs to one turn's context block. Every field is
/// a TYPED fact the app already computed — there is deliberately no
/// member that could carry a conversation, a serving frame, or a
/// carrier payload (the structural W3 guarantee; the chat-wall suite's
/// layer-2 scan pins this module names none of those types).
pub struct ContextInputs<'a> {
    /// The M1-style hardware profile (the picker's `HardwareInfo`).
    pub hardware: &'a HardwareInfo,
    /// The `list_models` cards (signed-catalog rows + the census join).
    pub picker: &'a [serde_json::Value],
    /// The daemon's family status (`FamilyStatus`), if a hosting session
    /// produced one.
    pub family: Option<&'a FamilyStatus>,
    /// Whether hosting is on.
    pub hosting_on: bool,
    /// The advertised QUIC listener address while bound.
    pub listener_addr: Option<&'a str>,
    /// The engine backend in force ("cpu" | "vulkan").
    pub engine_backend: Option<&'a str>,
    /// The engage ladder label of the last swarm turn.
    pub engage_last: Option<&'a str>,
    /// The governor verdict label of the last hosting-start resource
    /// check, when one degraded.
    pub governor_verdict: Option<&'a str>,
    /// The last family-probe pairing verdict.
    pub probe_verdict: Option<&'a str>,
    /// The self-reported capacityClass of this installation's earned
    /// lease, when one was earned.
    pub capacity_class: Option<&'a str>,
    /// The app version (build constant).
    pub app_version: &'a str,
}

/// Builds the typed context block (ADR-037 §3.5): M1 hardware fields,
/// picker rows (`display_name`/`quant_method` from the signed catalog
/// rows only), and status labels/counts from the frozen vocabularies.
/// Unknown-to-the-vocabulary values are still passed through — the
/// value-typed gate in the node drops and counts them (this builder
/// never sanitizes, it TYPES).
pub fn build_context(inputs: &ContextInputs) -> Vec<ContextField> {
    let mut fields = vec![
        ContextField {
            name: "hw.logical_cpus".into(),
            value: ContextValue::Int(i64::from(inputs.hardware.cpu_cores)),
        },
        ContextField {
            name: "hw.total_ram_mb".into(),
            value: ContextValue::Int(inputs.hardware.ram_mb as i64),
        },
    ];
    if let Some(gpu) = &inputs.hardware.gpu {
        fields.push(ContextField {
            name: "hw.gpu.name".into(),
            value: ContextValue::GpuName(gpu.name.clone()),
        });
        fields.push(ContextField {
            name: "hw.gpu.vram_mb".into(),
            value: ContextValue::Int(gpu.vram_mb as i64),
        });
    }
    // The picker rows: bounded to the frozen max (8) — the shortlist
    // plus the census join the picker already computed. The census
    // peers field is present only when the census knew (null never
    // sent — unknown stays absent, never a guessed zero).
    let rows: Vec<ContextPickerRow> = inputs
        .picker
        .iter()
        .take(8)
        .map(|card| ContextPickerRow {
            display_name: card["display_name"].as_str().unwrap_or("").to_string(),
            quant_method: card["quant_method"].as_str().unwrap_or("").to_string(),
            quant_bits: card["quant_bits"].as_i64().unwrap_or(0),
            download_mb: card["hw"]["download_mb"].as_i64().unwrap_or(0),
            ram_mb_est: card["hw"]["ram_mb_est"].as_i64().unwrap_or(0),
            fit: card["fit"].as_str().map(str::to_string),
            recommended: card["recommended"].as_bool().unwrap_or(false),
            census_peers: card["peers"].as_i64(),
        })
        .collect();
    fields.push(ContextField {
        name: "picker.rows".into(),
        value: ContextValue::PickerRows(rows),
    });
    if let Some(family) = inputs.family {
        fields.push(ContextField {
            name: "family.state".into(),
            value: ContextValue::Enum {
                vocabulary: "family_state",
                value: family.state.clone(),
            },
        });
        if let Some(reason) = &family.reason {
            fields.push(ContextField {
                name: "family.off_reason".into(),
                value: ContextValue::Enum {
                    vocabulary: "family_off_reason",
                    value: reason.clone(),
                },
            });
        }
    }
    let enum_field =
        |fields: &mut Vec<ContextField>, name: &str, vocabulary: &'static str, value: &str| {
            fields.push(ContextField {
                name: name.into(),
                value: ContextValue::Enum {
                    vocabulary,
                    value: value.to_string(),
                },
            });
        };
    if let Some(label) = inputs.engage_last {
        enum_field(
            &mut fields,
            "family.engage_last",
            "family_engage_label",
            label,
        );
    }
    if let Some(label) = inputs.governor_verdict {
        enum_field(
            &mut fields,
            "family.governor_verdict",
            "governor_verdict",
            label,
        );
    }
    if let Some(label) = inputs.probe_verdict {
        enum_field(
            &mut fields,
            "family.probe_verdict",
            "family_probe_verdict",
            label,
        );
    }
    fields.push(ContextField {
        name: "hosting.on".into(),
        value: ContextValue::Bool(inputs.hosting_on),
    });
    enum_field(
        &mut fields,
        "hosting.listener",
        "hosting_listener_state",
        if inputs.listener_addr.is_some() {
            "bound"
        } else {
            "none"
        },
    );
    if let Some(backend) = inputs.engine_backend {
        enum_field(&mut fields, "serving.backend", "engine_backend", backend);
    }
    if let Some(class) = inputs.capacity_class {
        enum_field(&mut fields, "capacity.class", "capacity_class", class);
    }
    fields.push(ContextField {
        name: "app.version".into(),
        value: ContextValue::Version(inputs.app_version.to_string()),
    });
    enum_field(&mut fields, "app.os", "os_platform", std::env::consts::OS);
    fields
}

// ---------------------------------------------------------------------------
// The static help corpus (ADR-037 §3.6) — every frozen label has an entry
// ---------------------------------------------------------------------------

/// One static help entry: the frozen label and its honest explanation.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpEntry {
    pub label: &'static str,
    pub text: &'static str,
}

/// The repo-versioned, label-keyed static corpus. Coverage against the
/// frozen vocabularies is pinned by test below (every FamilyStatus
/// off-reason, governor label, ladder label, probe verdict, and fit
/// tier — plus every gap label the degraded line can name).
pub static HELP_CORPUS: &[HelpEntry] = &[
    // ---- FamilyStatus off-reasons -----------------------------------
    HelpEntry { label: "family_prompt_unpinned", text: "The family verify engine needs the pinned session prompt token ids (family-prompt-tokens.txt in the data folder). Until the file exists and parses, family verification stays off; normal single-model hosting and chat are unaffected." },
    HelpEntry { label: "engine_bridge_failed", text: "The bridge to the batch-verify engine failed to initialize (the engine crashed or its library did not load). Hosting continues without family verification; retry hosting or check the engine install." },
    HelpEntry { label: "engine_bridge_not_compiled", text: "This build was compiled without the C-API batch-verify engine. Family verification is unavailable in this build; everything else works." },
    HelpEntry { label: "verifier_identity_unavailable", text: "The installation identity could not be loaded or created, so nothing can sign verification results. Hosting continues without family verification." },
    HelpEntry { label: "pairing_disabled", text: "The 'allow swarm pairing' settings toggle is off, so the serving listener and the family tier do not start. Turn the toggle on if you want swarm pairing." },
    HelpEntry { label: "listener_bind_failed", text: "The QUIC serving listener could not bind (usually a firewall or a busy port). Hosting serves locally; remote peers cannot reach you until the bind succeeds." },
    HelpEntry { label: "resource_ceiling", text: "The resource governor refused to start the family verify engine because its projected memory would break the enforced 70% system-RAM ceiling. This machine keeps serving the model it already hosts; the family tier stays off." },
    // ---- Governor verdict labels ------------------------------------
    HelpEntry { label: "ram_ceiling", text: "The projected memory (weights + runtime) exceeds 70% of total system RAM — the enforced coexistence ceiling. The governor refuses the admission; a smaller model or quantization fits." },
    HelpEntry { label: "vram_ceiling", text: "The projected GPU memory exceeds what the adapter can hold. Partial offload or a smaller quantization fits; the governor admits only what fits." },
    HelpEntry { label: "ram_unobservable", text: "Total system RAM could not be measured, so the governor degrades honestly instead of guessing. A reboot or updated drivers usually restore the measurement." },
    HelpEntry { label: "vram_unobservable", text: "GPU memory usage could not be measured for the projection, so the governor degrades honestly instead of guessing." },
    HelpEntry { label: "vram_total_unknown", text: "The adapter's total VRAM is unknown (zero reported), so no VRAM projection is possible. CPU execution is used; the number comes from the engine's device listing." },
    HelpEntry { label: "weights_not_positive", text: "The model artifact reported a non-positive size, which means the file is broken or truncated. Re-download the model." },
    HelpEntry { label: "kv_unmodeled", text: "KV-cache overhead is not yet modelable before the engine is constructed, so the projection carries this gap explicitly: the 70% ceiling is enforced on what IS known, with headroom for the unknown part." },
    HelpEntry { label: "not_enforced_arm_failed", text: "The OS refused the memory commit cap the governor tried to arm, so admission degrades honestly rather than run unbounded." },
    HelpEntry { label: "not_enforced_unsupported_platform", text: "This platform does not support the memory commit cap mechanism, so the governor refuses the admission instead of running unbounded." },
    // ---- The family.engage ladder -----------------------------------
    HelpEntry { label: "capability_unknown", text: "No family session has run with this peer yet, so its batch-verify capability is unknown. The probe fills this in." },
    HelpEntry { label: "capability_false", text: "This peer's engine does not support batch verification, so cooperative turns never route through it." },
    HelpEntry { label: "backend_mismatch", text: "This peer runs a different engine backend (cpu vs vulkan) — a cross-backend family session would reject at open, so the ladder stops here." },
    HelpEntry { label: "gate_unmeasured", text: "No measured family-round timings exist yet for this peer, so no prediction of a win is possible. Never-worse: no predicted win means no engagement." },
    HelpEntry { label: "gate_no_win", text: "The measured prediction says the cooperative path would NOT beat the fastest eligible single host this turn, so the plain path serves. Cooperative modes must beat the fastest single host or fall back — by design." },
    HelpEntry { label: "blocked_content_contract", text: "The gate would engage, but the automatic path never carries user content: only YOUR explicit family-verified-chat election opens a carrier session. The plain swarm path served this turn." },
    HelpEntry { label: "gate_misconfigured", text: "The engage gate's parameters are inconsistent (margin/safety values), so no engagement decision is made this turn. The plain path serves." },
    HelpEntry { label: "elected_fell_back", text: "You elected family-verified chat, but the carrier session could not open this turn (peer refused, form not available, or the engine is down); the plain swarm path served instead, honestly labeled." },
    // ---- Probe/pairing verdicts -------------------------------------
    HelpEntry { label: "pairing_confirmed", text: "The probe's one pairing round fully accepted: this machine and the peer chain identical tokenization on the pinned prompt — family verification is meaningful between you." },
    HelpEntry { label: "pin_mismatch", text: "The pairing round did not fully accept: the peer's tokenization of the pinned prompt diverges from yours (or the round failed), so family verification would compare different text. The round detail says which." },
    HelpEntry { label: "open_refused", text: "The family session was refused at open (usually the lease gate failing closed). The peer closed without an accept." },
    HelpEntry { label: "peer_rejected", text: "The peer rejected the session request outright." },
    HelpEntry { label: "verifier_ineligible", text: "This installation is not eligible to open the session (no lease for the profile or it is not hosting it)." },
    HelpEntry { label: "invalid_accept", text: "The peer's session accept was malformed, so the session was torn down honestly rather than trusted." },
    HelpEntry { label: "protocol", text: "A protocol-level error occurred during the family negotiation." },
    HelpEntry { label: "misconfigured", text: "The family client is misconfigured (identity, prompt pin, or engine source). Fix the configuration and retry the probe." },
    // ---- Picker fit tiers -------------------------------------------
    HelpEntry { label: "gpu_full", text: "The weights fit free VRAM with headroom, so the whole model offloads to the GPU — the best experience this machine can offer for that model." },
    HelpEntry { label: "gpu_partial", text: "Some layers fit in VRAM and the rest stay in system RAM (partial offload). Faster than CPU-only, slower than a full fit; a smaller quantization may fully fit." },
    HelpEntry { label: "ram_only", text: "No usable GPU (or none detected), so the model runs on the CPU. The responsive zone for CPU decode is around 8 GB of weights; giant dense models run but are not recommendation material." },
    // ---- The gap vocabulary (the degraded line's labels) ------------
    HelpEntry { label: "disabled", text: "The cloud assistant is switched off (its master switch in advisory.json or the Settings toggle). Built-in help still works and the deterministic recommendation is always live." },
    HelpEntry { label: "no_key", text: "No provider key file is configured on this machine, so the cloud assistant cannot run. Built-in help still works." },
    HelpEntry { label: "no_config", text: "The advisory configuration is missing or incomplete (endpoint or model), so the cloud assistant cannot run." },
    HelpEntry { label: "cap_reached", text: "The daily call cap (300 by default) or the conversation turn cap (40) is spent. The caps are the contract; the counters reset with the UTC day, a 15-minute idle, or 'reset'." },
    HelpEntry { label: "rate_limited", text: "The hourly burst bound (60 calls per UTC hour) is spent. Wait for the hour to roll over; the next user turn retries." },
    HelpEntry { label: "empty_after_redaction", text: "Nothing survived the egress gate for this turn (the text was empty or entirely redactable material). Nothing was sent; rephrase and retry." },
    HelpEntry { label: "timeout", text: "The provider did not answer within the 45-second deadline. Nothing was sent beyond the request; the next user turn retries." },
    HelpEntry { label: "http_error", text: "The provider answered with a transport or HTTP error. The turn failed honestly; the next user turn retries." },
    HelpEntry { label: "quota", text: "The provider account's quota is exhausted (HTTP 402). The cloud assistant pauses; built-in help still works." },
    HelpEntry { label: "response_oversized", text: "The provider's reply exceeded the 64 KiB response cap and was discarded whole." },
    HelpEntry { label: "schema", text: "The provider's reply did not match the assistant response schema (or did not echo this conversation), so it was discarded whole — never partially trusted." },
    HelpEntry { label: "request_oversized", text: "The request body exceeded the 48 KiB cap even with no history. Nothing was sent; shorten the question." },
    HelpEntry { label: "late", text: "A late provider response arrived after its turn had already timed out; it was discarded." },
    HelpEntry { label: "interval", text: "The digest-plane minimum interval (15 minutes) has not elapsed. This label never applies to assistant turns." },
];

/// Deterministic static lookup: every frozen label whose text appears
/// in the question (word-boundary) gets its entry; no entry is
/// invented, and no-match is stated honestly with the vocabularies
/// that exist.
pub fn static_help_lookup(question: &str) -> Vec<&'static HelpEntry> {
    let lower = question.to_lowercase();
    HELP_CORPUS
        .iter()
        .filter(|entry| {
            lower
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|word| word == entry.label)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The plane wrapper + the turn driver (the `assistant_turn` IPC core;
// also the test surface)
// ---------------------------------------------------------------------------

/// The app's handle on the armed assistant plane (a thin wrapper so
/// `app.rs` never names an advisory type — the chat-wall layer-2 scan
/// keeps the chat scopes clean while THIS file is the §4.5 sanctioned
/// home).
pub struct Plane {
    inner: AssistantPlane,
}

impl Plane {
    /// Arms the plane for a data dir (production: the pinned reqwest
    /// transport). `None` = not armed; the caller keeps the honest
    /// degrade gap for the built-in-help mode line.
    pub fn spawn(data_dir: &std::path::Path) -> (Option<Self>, String) {
        let (plane, gap) = AssistantPlane::spawn(data_dir, None);
        (plane.map(|inner| Self { inner }), gap.as_str().to_string())
    }

    /// The test seam: arms over an injected transport double.
    #[cfg(test)]
    pub fn spawn_with_double(
        data_dir: &std::path::Path,
        double: std::sync::Arc<dyn modelswarm_node::advisory::AdvisoryTransport>,
    ) -> (Option<Self>, String) {
        let (plane, gap) = AssistantPlane::spawn(data_dir, Some(double));
        (plane.map(|inner| Self { inner }), gap.as_str().to_string())
    }

    pub fn reset(&self) {
        self.inner.reset();
    }

    async fn run(
        &self,
        telemetry: &Telemetry,
        question: &str,
        inputs: &ContextInputs<'_>,
    ) -> PanelOutcome {
        let context = build_context(inputs);
        match self.inner.run_turn(telemetry, question, context).await {
            TurnOutcome::Reply {
                conversation_id,
                turn,
                reply,
                hints,
                hints_discarded,
            } => PanelOutcome::CloudReply {
                reply: render_reply(&reply),
                conversation_id,
                turn,
                hints: hints.iter().map(|h| h.to_string()).collect(),
                hints_discarded,
            },
            TurnOutcome::Gap { reason, .. } => PanelOutcome::CloudGap {
                gap: reason.as_str().to_string(),
            },
        }
    }
}

/// What the panel renders for one `assistant_turn`.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelOutcome {
    /// Cloud mode is armed but the §3.6 disclosure is not yet
    /// acknowledged — the call is BLOCKED (the UI shows the disclosure
    /// verbatim with its acknowledge button).
    NeedsAcknowledgement,
    /// A cloud reply: display-only plain text (already capped), the
    /// provenance chip is always adjacent, and any hints are
    /// display-only labels.
    CloudReply {
        reply: String,
        conversation_id: String,
        turn: u32,
        hints: Vec<String>,
        hints_discarded: usize,
    },
    /// A cloud turn failed honestly: the gap label renders in the
    /// panel; history is retained; the next user turn retries.
    CloudGap { gap: String },
    /// Built-in help (the plane is off/opted out/unconfigured, or the
    /// question is being answered statically): the corpus entries plus
    /// the honest mode line.
    StaticHelp {
        entries: Vec<&'static HelpEntry>,
        line: String,
    },
}

/// The dependencies of one assistant turn (all injected — the tests
/// construct these directly; the IPC command builds them from
/// `DesktopState`).
pub struct TurnDeps<'a> {
    /// The armed plane, when the master switch + key + config armed it.
    pub plane: Option<&'a Plane>,
    /// The honest degrade gap label when `plane` is None.
    pub degrade_gap: &'a str,
    /// The §3.6 first-use disclosure acknowledgement (persisted).
    pub acknowledged: bool,
    /// The Settings opt-out toggle (persisted; `false` = opted out).
    pub opt_out: bool,
    pub telemetry: &'a Telemetry,
    pub inputs: &'a ContextInputs<'a>,
}

/// Runs one assistant turn for the panel. The W6 rule: assistant
/// absence, failure, or refusal changes nothing about any other
/// surface — this function touches only its own plane and telemetry.
pub async fn turn(deps: TurnDeps<'_>, question: &str) -> PanelOutcome {
    let Some(plane) = deps.plane else {
        // The plane is not armed: static help with the honest gap.
        return PanelOutcome::StaticHelp {
            entries: static_help_lookup(question),
            line: degraded_line(deps.degrade_gap),
        };
    };
    if deps.opt_out {
        return PanelOutcome::StaticHelp {
            entries: static_help_lookup(question),
            line: degraded_line("disabled"),
        };
    }
    // §3.6: the call is BLOCKED until the disclosure is acknowledged.
    if !deps.acknowledged {
        return PanelOutcome::NeedsAcknowledgement;
    }
    plane.run(deps.telemetry, question, deps.inputs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelswarm_node::advisory::egress::vocabulary;

    fn hardware() -> HardwareInfo {
        HardwareInfo {
            ram_mb: 32_000,
            cpu_cores: 12,
            gpu: Some(super::super::GpuInfo {
                name: "NVIDIA GeForce RTX 5080 Laptop GPU".into(),
                vram_mb: 16_003,
            }),
        }
    }

    fn picker_cards() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({
                "profile_id": "msp1:aa", "display_name": "Alpha 9B",
                "quant_method": "q4_k_m", "quant_bits": 4,
                "hw": {"download_mb": 5_680, "ram_mb_est": 6_144},
                "fit": "gpu_full", "recommended": true, "peers": 2,
            }),
            serde_json::json!({
                "profile_id": "msp1:dd", "display_name": "Delta 27B",
                "quant_method": "q8_0", "quant_bits": 8,
                "hw": {"download_mb": 18_973, "ram_mb_est": 19_968},
                "fit": "gpu_partial", "recommended": false, "peers": null,
            }),
        ]
    }

    fn inputs<'a>(
        hardware: &'a HardwareInfo,
        picker: &'a [serde_json::Value],
    ) -> ContextInputs<'a> {
        ContextInputs {
            hardware,
            picker,
            family: None,
            hosting_on: true,
            listener_addr: Some("/ip4/192.168.1.9/udp/42420/quic-v1"),
            engine_backend: Some("vulkan"),
            engage_last: Some("gate_no_win"),
            governor_verdict: None,
            probe_verdict: Some("pairing_confirmed"),
            capacity_class: Some("gpu_mid"),
            app_version: "0.2.24",
        }
    }

    /// §3.5/§4.2: the builder's output ADMITS through the frozen
    /// context gate — every field type-checks against the artifact.
    #[test]
    fn builder_output_passes_the_frozen_context_gate() {
        let hw = hardware();
        let cards = picker_cards();
        let fields = build_context(&inputs(&hw, &cards));
        let admitted = modelswarm_node::advisory::egress::admit_context(fields);
        assert_eq!(
            admitted.dropped, 0,
            "every well-typed desktop fact admits: {admitted:?}"
        );
        assert_eq!(admitted.body["hw.logical_cpus"], 12);
        assert_eq!(admitted.body["hw.gpu.vram_mb"], 16_003);
        let rows = admitted.body["picker.rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["display_name"], "Alpha 9B");
        assert_eq!(rows[0]["census_peers"], 2);
        assert_eq!(rows[1]["census_peers"], serde_json::Value::Null);
        assert_eq!(admitted.body["family.engage_last"], "gate_no_win");
        assert_eq!(admitted.body["hosting.listener"], "bound");
        assert_eq!(admitted.body["serving.backend"], "vulkan");
        assert_eq!(admitted.body["capacity.class"], "gpu_mid");
        assert_eq!(admitted.body["app.version"], "0.2.24");
        // The OS label admits on every shipped platform.
        assert!(matches!(
            admitted.body["app.os"].as_str(),
            Some("windows" | "macos" | "linux")
        ));
    }

    /// ADR-037 §3.6: static-corpus coverage against the FROZEN
    /// vocabularies — every FamilyStatus off-reason, governor label,
    /// ladder label, probe verdict, fit tier, and gap value has an
    /// entry. A new vocabulary value without a corpus entry is a red
    /// build.
    #[test]
    fn static_help_covers_every_frozen_vocabulary_label() {
        let vocab = vocabulary();
        for vocabulary_name in [
            "family_off_reason",
            "governor_verdict",
            "governor_gap",
            "family_engage_label",
            "family_probe_verdict",
            "picker_fit",
            "advisory_gap",
        ] {
            let values = &vocab.vocabularies[vocabulary_name].values;
            assert!(
                !values.is_empty(),
                "{vocabulary_name} missing from artifact"
            );
            for label in values {
                assert!(
                    HELP_CORPUS.iter().any(|e| e.label == label),
                    "frozen label {label} (vocabulary {vocabulary_name}) has no static help entry — ADR-037 §3.6 coverage"
                );
            }
        }
        // The corpus carries only labels that exist (no dead entries).
        let all: std::collections::HashSet<&str> = [
            "family_off_reason",
            "governor_verdict",
            "governor_gap",
            "family_engage_label",
            "family_probe_verdict",
            "picker_fit",
            "advisory_gap",
        ]
        .iter()
        .flat_map(|name| vocab.vocabularies[*name].values.iter().map(String::as_str))
        .collect();
        let extras: std::collections::HashSet<&str> = [
            "not_enforced_arm_failed",
            "not_enforced_unsupported_platform",
            "open_refused",
            "peer_rejected",
            "verifier_ineligible",
            "invalid_accept",
            "protocol",
            "misconfigured",
        ]
        .into_iter()
        .collect();
        for entry in HELP_CORPUS {
            assert!(
                all.contains(entry.label) || extras.contains(entry.label),
                "corpus label {} is not a frozen vocabulary value",
                entry.label
            );
        }
    }

    #[test]
    fn static_lookup_is_deterministic_and_honest_on_no_match() {
        let hits = static_help_lookup("why does it say resource_ceiling and kv_unmodeled?");
        let labels: Vec<&str> = hits.iter().map(|e| e.label).collect();
        assert_eq!(labels, vec!["resource_ceiling", "kv_unmodeled"]);
        // Substrings do not match (word boundaries).
        assert!(static_help_lookup("ram_ceilingXXX").is_empty());
        assert!(static_help_lookup("what model should I pick?").is_empty());
    }

    /// §3.7 rendering contract: plain text preserved, cap enforced with
    /// the honest truncation marker.
    #[test]
    fn reply_renderer_caps_at_4000_chars() {
        assert_eq!(render_reply("short"), "short");
        let long: String = "x".repeat(4_001);
        let rendered = render_reply(&long);
        assert_eq!(
            rendered.chars().count(),
            REPLY_DISPLAY_CAP + "…[truncated]".chars().count()
        );
        assert!(rendered.ends_with("…[truncated]"));
        // Exactly at the cap: untouched.
        let exact: String = "x".repeat(REPLY_DISPLAY_CAP);
        assert_eq!(render_reply(&exact).chars().count(), REPLY_DISPLAY_CAP);
        assert!(!render_reply(&exact).contains("truncated"));
    }

    /// §3.6 texts are VERBATIM (the ADR is the contract; drift is a UI
    /// defect).
    #[test]
    fn disclosure_and_chip_are_verbatim() {
        assert!(FIRST_USE_DISCLOSURE.starts_with("This assistant sends what you type here"));
        assert!(FIRST_USE_DISCLOSURE.contains("never your swarm chats"));
        assert!(FIRST_USE_DISCLOSURE.ends_with("You can turn it off any time in Settings."));
        assert_eq!(
            PROVENANCE_CHIP,
            "Cloud assistant — advice only; this app's rules decide."
        );
        assert_eq!(
            degraded_line("no_key"),
            "Built-in help — the cloud assistant is off/unreachable (no_key)."
        );
    }
}
