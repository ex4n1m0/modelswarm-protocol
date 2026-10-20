//! ADR-045 execution presets — the five-tier user vocabulary
//! (Fast / Balanced / Deep / Verified / Maximum), accepted by the owner
//! 2026-10-20.
//!
//! P1 scope (this module + the picker): the vocabulary, the persisted
//! selection, and the verbatim disclosure lines. NO behavior is wired to
//! the preset in P1 — family verification and the ADR-042 cross-check keep
//! running exactly as before for every tier; P2 maps tier → engaged
//! behaviors (planner engage-decision for Balanced, cohort widening for
//! Deep, cross-check tiering for Verified/Maximum) with per-turn fallback.
//!
//! Invariants pinned by the ADR: the tiers form a strictly upward
//! gradient (no tier removes the family-verification baseline); presets
//! are CLIENT-side policy (no new msp-v1 wire fields); the disclosure
//! lines below are the owner-approved verbatim texts — changing one word
//! re-runs the ADR's owner review. `tests/preset_disclosure_sync.rs`
//! asserts the UI carries these exact strings.

/// The five ADR-045 presets. `Default` is `Fast` (the ADR's approved
/// default; today's behavior relabeled honestly). Store/wire form is the
/// lowercase name (`rename_all`); an unknown stored value fails closed to
/// `Fast` via `deserialize_fallback_fast` (one bad key must never brick
/// config.json parsing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionPreset {
    #[default]
    Fast,
    Balanced,
    Deep,
    Verified,
    Maximum,
}

impl ExecutionPreset {
    /// All five tiers in owner-display order.
    pub const ALL: [ExecutionPreset; 5] = [
        ExecutionPreset::Fast,
        ExecutionPreset::Balanced,
        ExecutionPreset::Deep,
        ExecutionPreset::Verified,
        ExecutionPreset::Maximum,
    ];

    /// Wire/store form (persisted key value; the receipt `preset` field
    /// reuses the same spelling when labeling becomes possible).
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionPreset::Fast => "fast",
            ExecutionPreset::Balanced => "balanced",
            ExecutionPreset::Deep => "deep",
            ExecutionPreset::Verified => "verified",
            ExecutionPreset::Maximum => "maximum",
        }
    }

    /// The owner-approved verbatim disclosure line (ADR-045 §Decision
    /// table, third column — do not edit without re-running the ADR's
    /// owner review). Served verbatim on `get_status` for the current
    /// tier (the honest status surface), and pinned byte-exact by the
    /// unit tests + `tests/preset_disclosure_sync.rs` against the
    /// webview's copies.
    pub fn disclosure(self) -> &'static str {
        match self {
            ExecutionPreset::Fast => {
                "One host answers; every same-model peer checks it. The everyday speed path."
            }
            ExecutionPreset::Balanced => {
                "Same checks as Fast; your own machine drafts ahead and verifies in one pass when that is measurably faster — otherwise it steps aside."
            }
            ExecutionPreset::Deep => {
                "Every same-model peer verifies every reply; any disagreement forces a clean recompute."
            }
            ExecutionPreset::Verified => {
                "Everything Deep does, plus a second, different model family sanity-checks the answer; the receipt shows it."
            }
            ExecutionPreset::Maximum => {
                "The whole swarm checks everything, and the receipt records it — signed and tamper-evident. Slowest by design."
            }
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "fast" => Some(ExecutionPreset::Fast),
            "balanced" => Some(ExecutionPreset::Balanced),
            "deep" => Some(ExecutionPreset::Deep),
            "verified" => Some(ExecutionPreset::Verified),
            "maximum" => Some(ExecutionPreset::Maximum),
            _ => None,
        }
    }
}

/// Config deserializer: unknown stored value → `Fast` (fail-closed, never
/// a config parse error).
pub fn deserialize_fallback_fast<'de, D>(deserializer: D) -> Result<ExecutionPreset, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
    Ok(ExecutionPreset::from_str(&raw).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_fast() {
        assert_eq!(ExecutionPreset::default(), ExecutionPreset::Fast);
    }

    #[test]
    fn round_trips_through_store_form() {
        for preset in [
            ExecutionPreset::Fast,
            ExecutionPreset::Balanced,
            ExecutionPreset::Deep,
            ExecutionPreset::Verified,
            ExecutionPreset::Maximum,
        ] {
            assert_eq!(ExecutionPreset::from_str(preset.as_str()), Some(preset));
        }
        assert_eq!(ExecutionPreset::from_str("turbo"), None);
    }

    #[test]
    fn disclosure_lines_are_the_adr_verbatim_texts() {
        // Pinned to ADR-045 §Decision (owner-approved 2026-10-20). A change
        // here means the ADR table changed — re-run the owner review.
        assert_eq!(
            ExecutionPreset::Fast.disclosure(),
            "One host answers; every same-model peer checks it. The everyday speed path."
        );
        assert_eq!(
            ExecutionPreset::Balanced.disclosure(),
            "Same checks as Fast; your own machine drafts ahead and verifies in one pass when that is measurably faster — otherwise it steps aside."
        );
        assert_eq!(
            ExecutionPreset::Deep.disclosure(),
            "Every same-model peer verifies every reply; any disagreement forces a clean recompute."
        );
        assert_eq!(
            ExecutionPreset::Verified.disclosure(),
            "Everything Deep does, plus a second, different model family sanity-checks the answer; the receipt shows it."
        );
        assert_eq!(
            ExecutionPreset::Maximum.disclosure(),
            "The whole swarm checks everything, and the receipt records it — signed and tamper-evident. Slowest by design."
        );
    }

    /// Mirrors the PersistedConfig field wiring exactly: unknown stored
    /// value fails CLOSED to Fast (never a config parse error — one bad
    /// key must not brick config.json), missing key uses the default.
    #[derive(serde::Deserialize)]
    struct CfgMirror {
        #[serde(
            default = "default_fast",
            deserialize_with = "deserialize_fallback_fast"
        )]
        preset: ExecutionPreset,
    }

    fn default_fast() -> ExecutionPreset {
        ExecutionPreset::Fast
    }

    #[test]
    fn persisted_field_fails_closed_to_fast() {
        assert_eq!(
            serde_json::from_str::<CfgMirror>(r#"{"preset":"bogus"}"#)
                .expect("unknown preset must not fail the parse")
                .preset,
            ExecutionPreset::Fast
        );
        assert_eq!(
            serde_json::from_str::<CfgMirror>("{}")
                .expect("missing key uses the default")
                .preset,
            ExecutionPreset::Fast
        );
        assert_eq!(
            serde_json::from_str::<CfgMirror>(r#"{"preset":"verified"}"#)
                .expect("valid value parses")
                .preset,
            ExecutionPreset::Verified
        );
    }
}

/// ADR-045 P2 (the pre-registered protocol,
/// `experiments/reports/P2-BALANCED-FALLBACK-PROTOCOL.md`, gates 1–5):
/// the honest engagement label for a turn that will execute LOCALLY.
/// `None` = the turn is not engage-eligible — the Fast baseline never
/// claims engagement, so its replies carry no `spec_engage` at all.
///
/// P2 SEAM SCOPE: the composed engine is not wired to turns yet, so the
/// cascade can only label `engaged: false` with the FIRST failing gate's
/// honest reason — gate 5 refuses conservatively
/// (`prediction_insufficient_evidence`) until in-situ measurements
/// validate the cost model. The composed execution (the next chunk)
/// replaces that refusal with a real prediction + execution, and gate
/// 6's per-turn failure label `composed_failed_before_first_token`
/// lands with it.
pub fn spec_engage_label(
    preset: ExecutionPreset,
    temperature: f32,
    capi_seam_live: bool,
) -> Option<serde_json::Value> {
    if preset == ExecutionPreset::Fast {
        return None;
    }
    let reason = if temperature != 0.0 {
        // Gate 2: speculative_exact is greedy-only — a sampled turn
        // cannot be bit-verified, so it must never engage.
        "temperature_not_greedy"
    } else if !capi_seam_live {
        // Gate 4: no hash-verified in-process engine seam this turn.
        "capi_seam_unavailable"
    } else {
        // Gate 5: conservative refusal until measured acceptance exists.
        "prediction_insufficient_evidence"
    };
    Some(serde_json::json!({ "engaged": false, "reason": reason }))
}

/// The same label for a turn that was served by a REMOTE peer: the
/// composed path is in-process by construction, so a remote-served turn
/// can never engage (gate 3, `turn_not_local`) — the label still rides
/// the reply so a Balanced turn is honest on every path.
pub fn spec_engage_label_remote(preset: ExecutionPreset) -> Option<serde_json::Value> {
    if preset == ExecutionPreset::Fast {
        return None;
    }
    Some(serde_json::json!({ "engaged": false, "reason": "turn_not_local" }))
}

#[cfg(test)]
mod spec_engage_tests {
    use super::*;

    fn reason(label: &serde_json::Value) -> &str {
        label["reason"].as_str().unwrap()
    }

    #[test]
    fn fast_never_carries_a_label() {
        // The baseline tier claims nothing — no spec_engage at all.
        assert!(spec_engage_label(ExecutionPreset::Fast, 0.0, true).is_none());
        assert!(spec_engage_label_remote(ExecutionPreset::Fast).is_none());
    }

    #[test]
    fn gate_cascade_labels_the_first_failing_gate() {
        // Gate 2: a sampled turn can never be bit-verified.
        let sampled = spec_engage_label(ExecutionPreset::Balanced, 0.7, true).unwrap();
        assert_eq!(sampled["engaged"], false);
        assert_eq!(reason(&sampled), "temperature_not_greedy");
        // Gate 4: no in-process engine seam this turn.
        let no_seam = spec_engage_label(ExecutionPreset::Balanced, 0.0, false).unwrap();
        assert_eq!(reason(&no_seam), "capi_seam_unavailable");
        // Gate 5: the conservative pre-measurement refusal — the seam
        // exists but no in-situ measurement has validated the cost model
        // yet, so engagement refuses rather than guess.
        let refusing = spec_engage_label(ExecutionPreset::Balanced, 0.0, true).unwrap();
        assert_eq!(reason(&refusing), "prediction_insufficient_evidence");
        // The cascade covers every engage-eligible tier (Deep+ inherit
        // Balanced's composed machinery per the ADR-045 table).
        for preset in [
            ExecutionPreset::Deep,
            ExecutionPreset::Verified,
            ExecutionPreset::Maximum,
        ] {
            assert_eq!(
                reason(&spec_engage_label(preset, 0.0, true).unwrap()),
                "prediction_insufficient_evidence"
            );
        }
    }

    #[test]
    fn the_seam_never_claims_engagement_yet() {
        // P2 seam scope: until the composed execution lands, NO input can
        // produce engaged: true — this is the fixture pin for that honest
        // boundary (the R1 engaged-set discipline).
        for preset in ExecutionPreset::ALL {
            for temp in [0.0f32, 0.5, 2.0] {
                for seam in [true, false] {
                    if let Some(label) = spec_engage_label(preset, temp, seam) {
                        assert_eq!(label["engaged"], false);
                    }
                }
            }
            if let Some(label) = spec_engage_label_remote(preset) {
                assert_eq!(label["engaged"], false);
                assert_eq!(reason(&label), "turn_not_local");
            }
        }
    }
}
