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
                "The whole swarm checks everything, and the receipt proves it. Slowest by design."
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
            "The whole swarm checks everything, and the receipt proves it. Slowest by design."
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
