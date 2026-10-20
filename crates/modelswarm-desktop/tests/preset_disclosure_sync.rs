//! ADR-045 disclosure-sync gate (T/R R2, two-layer): the five
//! owner-approved disclosure lines are pinned byte-exact in BOTH sources —
//! the Rust enum (`src/preset.rs::ExecutionPreset::disclosure`) and the
//! webview (`ui/index.html`). This test carries the canonical strings and
//! asserts each source contains them verbatim, plus the picker's element
//! IDs and the write-through IPC name. A failure here means either source
//! drifted from the owner-approved ADR-045 table — fix by restoring the
//! approved text, never by editing this test (that re-runs the ADR's
//! owner review per its own rule).

/// (store name, owner-approved disclosure line) — ADR-045 §Decision.
const CANONICAL: &[(&str, &str)] = &[
    (
        "fast",
        "One host answers; every same-model peer checks it. The everyday speed path.",
    ),
    (
        "balanced",
        "Same checks as Fast; your own machine drafts ahead and verifies in one pass when that is measurably faster — otherwise it steps aside.",
    ),
    (
        "deep",
        "Every same-model peer verifies every reply; any disagreement forces a clean recompute.",
    ),
    (
        "verified",
        "Everything Deep does, plus a second, different model family sanity-checks the answer; the receipt shows it.",
    ),
    (
        "maximum",
        "The whole swarm checks everything, and the receipt proves it. Slowest by design.",
    ),
];

const PRESET_RS: &str = include_str!("../src/app/preset.rs");
const UI_HTML: &str = include_str!("../ui/index.html");

#[test]
fn rust_enum_carries_the_verbatim_disclosures() {
    for (name, line) in CANONICAL {
        assert!(
            PRESET_RS.contains(line),
            "preset.rs drifted from the ADR-045 approved text for {name:?}"
        );
    }
}

#[test]
fn webview_carries_the_verbatim_disclosures() {
    for (name, line) in CANONICAL {
        assert!(
            UI_HTML.contains(line),
            "ui/index.html drifted from the ADR-045 approved text for {name:?}"
        );
    }
}

#[test]
fn picker_surface_exists_in_the_webview() {
    // The picker panel + its rows container + the write-through IPC name
    // (the guards job greps for the same markers — this test is the
    // stronger, compiled layer).
    for marker in [
        r#"id="panel-preset""#,
        r#"id="preset-list""#,
        "set_execution_preset",
        "execution_preset",
    ] {
        assert!(
            UI_HTML.contains(marker),
            "ui/index.html is missing the ADR-045 picker marker {marker:?}"
        );
    }
}
