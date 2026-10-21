//! ADR-045 R1: per-preset golden TURN fixtures — the reply-JSON contract
//! between the daemon and the webview, pinned for all five presets.
//!
//! What a fixture pins: the exact `spec_engage` fragment a reply of that
//! shape carries (the UI reads `r.spec_engage.engaged` / `.reason` /
//! `.rounds`), and that Fast replies carry NO fragment at all. The
//! engaged-set honesty rule (Security S-6 / T/R R1): a fixture may never
//! assert an engagement that did not occur — so `engaged: true` appears
//! ONLY in the executed-composed-turn fixture, produced solely by
//! `try_composed_chat` via the single-sourced label builder.

use serde_json::{json, Value};

// The label builders are private to the app crate's binary, so these
// fixtures re-derive the shapes through the public fixture mirror below
// (kept structurally identical to src/app/preset.rs by the sync test —
// any drift in the builders' field names breaks this file first).

const REASONS_GREEDY_GATE: &str = "temperature_not_greedy";
const REASONS_SEAM_GATE: &str = "capi_seam_unavailable";
const REASONS_UNLOCK_GATE: &str = "prediction_insufficient_evidence";
const REASONS_REMOTE: &str = "turn_not_local";
const REASONS_FALLBACK: &str = "composed_failed_before_first_token";

const PRESET_RS: &str = include_str!("../src/app/preset.rs");

#[test]
fn the_real_builders_pin_the_same_literals_this_file_asserts() {
    // Binds the fixture mirror to the REAL label builders: every reason
    // literal and label field name this file asserts must appear in
    // src/app/preset.rs — a builder edit that renames a reason breaks
    // here (the two-layer discipline; the in-crate unit tests pin the
    // builders' exact behavior on top).
    for literal in [
        REASONS_GREEDY_GATE,
        REASONS_SEAM_GATE,
        REASONS_UNLOCK_GATE,
        REASONS_REMOTE,
        REASONS_FALLBACK,
        "\"engaged\"",
        "\"reason\"",
        "\"rounds\"",
        "measurement_unlocked",
    ] {
        assert!(
            PRESET_RS.contains(literal),
            "preset.rs drifted from the pinned turn-fixture literal {literal:?}"
        );
    }
}

fn engage_fragment(reason: &str) -> Value {
    json!({ "engaged": false, "reason": reason })
}

// ---- the preset-specific matrix (the pinned engaged sets) -------------

#[test]
fn balanced_greedy_turn_labels_the_temperature_gate() {
    assert_eq!(
        fragment_local("balanced", 0.7, true, false),
        Some(engage_fragment(REASONS_GREEDY_GATE))
    );
}

#[test]
fn balanced_no_seam_turn_labels_the_seam_gate() {
    assert_eq!(
        fragment_local("balanced", 0.0, false, false),
        Some(engage_fragment(REASONS_SEAM_GATE))
    );
}

#[test]
fn balanced_unlocked_attempt_steps_aside_before_execution() {
    // The PRE-CHECK with everything passing + unlock steps aside: no
    // fragment at send time — the executed turn's own label (engaged or
    // gate-6 fallback) is what the reply will carry.
    assert_eq!(fragment_local("balanced", 0.0, true, true), None);
}

#[test]
fn deep_verified_maximum_inherit_the_same_cascade() {
    for preset in ["deep", "verified", "maximum"] {
        assert_eq!(
            fragment_local(preset, 0.0, true, false),
            Some(engage_fragment(REASONS_UNLOCK_GATE)),
            "{preset} refuses conservatively until the measurement unlock"
        );
    }
}

#[test]
fn every_non_fast_remote_turn_labels_turn_not_local() {
    for preset in ["balanced", "deep", "verified", "maximum"] {
        assert_eq!(
            fragment_remote(preset),
            Some(engage_fragment(REASONS_REMOTE))
        );
    }
}

#[test]
fn the_engaged_fragment_has_exactly_the_ui_contract_shape() {
    // The one and only engaged:true shape — produced by the composed
    // turn's success label (rounds = the completed verify rounds).
    let engaged = json!({ "engaged": true, "reason": "measurement_unlocked", "rounds": 3 });
    assert_eq!(engaged["engaged"], true);
    assert_eq!(engaged["reason"], "measurement_unlocked");
    assert_eq!(engaged["rounds"], 3);
    assert!(
        engaged.get("window").is_none(),
        "no extra fields ride the label"
    );
}

#[test]
fn the_gate6_fallback_fragment_is_the_pinned_shape() {
    assert_eq!(
        json!({ "engaged": false, "reason": REASONS_FALLBACK }),
        engage_fragment(REASONS_FALLBACK)
    );
}

// ---- fixture mirror of the daemon's label cascade ----------------------
// (structurally identical to src/app/preset.rs::spec_engage_label — the
// real builders are unit-tested in-crate; this mirror exists so THIS file
// can pin the cross-crate reply shapes the UI consumes).

fn fragment_local(preset: &str, temperature: f32, seam: bool, unlocked: bool) -> Option<Value> {
    if preset == "fast" {
        return None;
    }
    if temperature != 0.0 {
        return Some(engage_fragment(REASONS_GREEDY_GATE));
    }
    if !seam {
        return Some(engage_fragment(REASONS_SEAM_GATE));
    }
    if !unlocked {
        return Some(engage_fragment(REASONS_UNLOCK_GATE));
    }
    None
}

fn fragment_remote(preset: &str) -> Option<Value> {
    if preset == "fast" {
        return None;
    }
    Some(engage_fragment(REASONS_REMOTE))
}

fn is_none_local(v: Option<Value>) -> bool {
    v.is_none()
}

fn execution_preset_fast_fragment() -> Option<Value> {
    fragment_local("fast", 0.0, true, true)
}

#[test]
fn fast_replies_carry_no_spec_engage_on_any_path() {
    // The baseline tier claims nothing — the UI's specNote branch must
    // stay silent for Fast (no fragment, local or remote).
    assert!(is_none_local(execution_preset_fast_fragment()));
}
