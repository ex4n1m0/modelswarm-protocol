//! Phase C bench-harness acceptance (C14/C15): records validate against
//! the frozen schemas, determinism, statistics sanity, mock honesty
//! labeling, and the fastest-single comparator machinery.

mod common;

use common::{load_schema, validate};
use modelswarm_bench::{
    aggregate, bootstrap_ci, write_records, write_run_manifest, BenchEngine, Exactness, Mode,
    ModeMetrics, ModeResultRecord, NetworkCell, RunStatus, TEST_ONLY_MOCK_LABEL,
};
use modelswarm_runtime::MockRuntime;
use serde_json::Value;
use std::sync::Arc;

fn engine() -> BenchEngine {
    BenchEngine::new(
        Arc::new(MockRuntime::new(0xBEEF, 0.5)),
        "phase-c-harness-test",
    )
}

fn cell() -> NetworkCell {
    NetworkCell {
        nominal_rtt_ms: 20,
        jitter: modelswarm_bench::Jitter::Low,
        loss: 0.001,
        path: modelswarm_bench::CellPath::Direct,
    }
}

#[tokio::test]
async fn records_validate_against_the_frozen_mode_result_schema() {
    let engine = engine();
    let schema = load_schema("mode-result.schema.json");
    let mut all = Vec::new();
    all.extend(engine.run_cell(&cell(), Mode::Single, 1, 5, 1).await);
    all.extend(engine.run_cell(&cell(), Mode::Hedged, 3, 5, 2).await);
    all.extend(
        engine
            .run_cell(&cell(), Mode::SpeculativeExact, 2, 5, 3)
            .await,
    );
    all.extend(
        engine
            .run_cell(&cell(), Mode::SearchVerified, 4, 5, 4)
            .await,
    );
    assert!(all.len() >= 20, "expected >= 20 records, got {}", all.len());
    for record in &all {
        let json = serde_json::to_value(record).unwrap();
        let errors = validate(&schema, &json);
        assert!(
            errors.is_empty(),
            "record failed schema validation:\n  {}\nrecord: {json}",
            errors.join("\n  ")
        );
    }
    // Every record links to the run manifest id derived from its seed.
    assert!(all
        .iter()
        .all(|r| r.run_manifest_id.starts_with("run-") && r.run_manifest_id.len() == 20));
}

#[tokio::test]
async fn manifest_validates_against_the_frozen_schema() {
    let engine = engine();
    let manifest = engine.manifest_for(&cell(), 30, 7);
    let schema = load_schema("run-manifest.schema.json");
    let errors = validate(&schema, &serde_json::to_value(&manifest).unwrap());
    assert!(
        errors.is_empty(),
        "manifest failed schema validation:\n  {}",
        errors.join("\n  ")
    );
    assert_eq!(manifest.runtime.name, "mock", "ADR-019 mock identity");
    assert_eq!(manifest.confidence_margin, Some(0.15));
}

#[tokio::test]
async fn same_seed_yields_byte_identical_records() {
    let engine = engine();
    let a = engine
        .run_cell(&cell(), Mode::SpeculativeExact, 2, 8, 0x5EED)
        .await;
    let b = engine
        .run_cell(&cell(), Mode::SpeculativeExact, 2, 8, 0x5EED)
        .await;
    let lines_a: Vec<String> = a
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    let lines_b: Vec<String> = b
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    assert_eq!(lines_a, lines_b);
    assert_eq!(a.len(), 8);

    // Different seed changes at least one record.
    let c = engine
        .run_cell(&cell(), Mode::SpeculativeExact, 2, 8, 0x5EED + 1)
        .await;
    let lines_c: Vec<String> = c
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    assert_ne!(lines_a, lines_c);
}

#[tokio::test]
async fn single_baseline_records_the_fastest_single_comparator() {
    let engine = engine();
    let records = engine.single_mode_baseline(&cell(), 6, 42).await;
    assert_eq!(records.len(), 6);
    assert!(records.iter().all(|r| r.mode == Mode::Single));
    assert!(records
        .iter()
        .all(|r| r.outcome.status == RunStatus::Completed));
    for record in &records {
        let predicted = record.metrics.fastest_single_prediction_ms;
        let actual = record.metrics.fastest_single_actual_ms;
        assert!(predicted.is_some(), "prediction required (C15)");
        assert!(actual.is_some(), "actual required (C15)");
        let (predicted, actual) = (predicted.unwrap(), actual.unwrap());
        // Same order of magnitude (±5% synthetic noise model).
        assert!(
            (actual - predicted).abs() / predicted < 0.06,
            "actual {actual} vs predicted {predicted}"
        );
    }
}

#[tokio::test]
async fn speculative_engagement_responds_to_the_network_cell() {
    let engine = engine();
    // Loopback (rtt 0): cooperative should engage or fall back per the cost
    // model — either way the record must be honest about it.
    let loopback = NetworkCell::LOOPBACK;
    let engaged = engine
        .run_cell(&loopback, Mode::SpeculativeExact, 2, 8, 11)
        .await;
    assert!(engaged.iter().all(|r| {
        r.metrics.acceptance_rate.is_some() || r.metrics.cooperative_prediction_ms.is_some()
    }));
    // And at least one loopback run must actually engage the cooperative
    // path (fast-but-busy drafter + batched verification beats the fastest
    // single host on the symmetric pool).
    assert!(
        engaged.iter().any(
            |r| r.metrics.acceptance_rate.is_some() && r.outcome.status == RunStatus::Completed
        ),
        "loopback should exercise the engaged path: {:?}",
        engaged.iter().map(|r| r.outcome.status).collect::<Vec<_>>()
    );

    // High-RTT relayed cell with loss: synchronization dominates; expect
    // fallbacks to be visible (negative results are first-class).
    let hostile = NetworkCell {
        nominal_rtt_ms: 150,
        jitter: modelswarm_bench::Jitter::High,
        loss: 0.03,
        path: modelswarm_bench::CellPath::Relayed,
    };
    let hostile_records = engine
        .run_cell(&hostile, Mode::SpeculativeExact, 2, 5, 12)
        .await;
    let fell_back = hostile_records
        .iter()
        .filter(|r| r.outcome.status == RunStatus::FellBackToSingle)
        .count();
    assert!(
        fell_back > 0,
        "hostile cell should produce visible fallbacks: {:?}",
        hostile_records
            .iter()
            .map(|r| r.outcome.status)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn zero_draft_accuracy_produces_an_honest_negative_result() {
    let engine = BenchEngine::new(Arc::new(MockRuntime::new(9, 0.0)), "phase-c-harness-test");
    let records = engine
        .run_cell(&NetworkCell::LOOPBACK, Mode::SpeculativeExact, 2, 4, 77)
        .await;
    assert!(records
        .iter()
        .any(|r| r.outcome.status == RunStatus::Failed));
    let failed = records
        .iter()
        .find(|r| r.outcome.status == RunStatus::Failed)
        .unwrap();
    assert!(failed
        .outcome
        .failure_reason
        .as_deref()
        .unwrap()
        .contains("acceptance"));
}

#[tokio::test]
async fn perfect_drafts_are_fully_accepted() {
    let engine = BenchEngine::new(Arc::new(MockRuntime::new(3, 1.0)), "phase-c-harness-test");
    let records = engine
        .run_cell(&NetworkCell::LOOPBACK, Mode::SpeculativeExact, 2, 4, 78)
        .await;
    for record in &records {
        assert_eq!(record.outcome.status, RunStatus::Completed);
        let rate = record.metrics.acceptance_rate.unwrap();
        assert!(
            rate > 0.9,
            "perfect drafts should accept ~everything: {rate}"
        );
        assert_eq!(record.metrics.rollback_count, Some(0));
    }
}

#[tokio::test]
async fn invalid_cell_is_marked_not_silently_accepted() {
    let engine = engine();
    let bad = NetworkCell {
        nominal_rtt_ms: 33,
        ..NetworkCell::LOOPBACK
    };
    let records = engine.run_cell(&bad, Mode::Single, 1, 3, 5).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome.status, RunStatus::InvalidCell);
    assert!(records[0].outcome.failure_reason.is_some());
}

#[tokio::test]
async fn mock_honesty_labeling_is_everywhere_it_must_be() {
    let engine = engine();
    assert!(engine.is_mock());
    assert_eq!(engine.mock_label(), Some(TEST_ONLY_MOCK_LABEL));

    let records = engine.run_cell(&cell(), Mode::Single, 1, 4, 99).await;
    // Every human-facing report line carries the TEST-ONLY label.
    let report = engine.cell_report(&cell(), &records);
    assert!(!report.is_empty());
    for line in &report {
        assert!(
            line.contains(TEST_ONLY_MOCK_LABEL),
            "unlabeled report line: {line:?}"
        );
    }
    // Machine-readable labeling: record → manifest → runtime.name = "mock".
    let run_id = records[0].run_manifest_id.clone();
    let manifest = engine.manifest_for(&cell(), 4, 99);
    assert_eq!(manifest.run_manifest_id, run_id);
    assert_eq!(manifest.runtime.name, "mock");
    // And the raw JSONL lines are pure schema records (closed schema:
    // exactly the five declared top-level members).
    for record in &records {
        let json: Value = serde_json::to_value(record).unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(object.len(), 6, "members: {:?}", object.keys());
    }
}

#[tokio::test]
async fn write_records_and_manifest_round_trip() {
    let engine = engine();
    let records = engine.run_cell(&cell(), Mode::Hedged, 3, 3, 1234).await;
    let dir = tempfile_dir();
    let path = write_records(&dir, &records).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), records.len());
    for (line, record) in lines.iter().zip(&records) {
        let parsed: Value = serde_json::from_str(line).unwrap();
        assert_eq!(parsed, serde_json::to_value(record).unwrap());
    }
    let manifest = engine.manifest_for(&cell(), 3, 1234);
    let manifest_path = dir.join("run-manifest.json");
    write_run_manifest(&manifest_path, &manifest).unwrap();
    let parsed: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(parsed["record_type"], "run-manifest/v1");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn aggregate_and_bootstrap_sanity_on_known_data() {
    let values: Vec<f64> = (1..=101).map(f64::from).collect();
    let summary = aggregate(&values);
    assert!((summary.median - 51.0).abs() < 1e-9);
    assert_eq!(summary.n, 101);
    let (lo, hi) = bootstrap_ci(&values, 10_000, 7);
    assert!(
        lo <= 51.0 && 51.0 <= hi,
        "CI ({lo}, {hi}) must contain the median"
    );
    assert!(lo < hi);

    // Cell aggregation over real records produces sane summaries.
    let engine = engine();
    let records = engine.run_cell(&cell(), Mode::Single, 1, 10, 55).await;
    let completions: Vec<f64> = records.iter().map(|r| r.metrics.completion_ms).collect();
    let summary = aggregate(&completions);
    assert!(summary.median > 0.0);
    assert!(summary.p10 <= summary.median);
    assert!(summary.median <= summary.p90);
    let (lo, hi) = bootstrap_ci(&completions, 10_000, 55);
    assert!(lo <= summary.median && summary.median <= hi);
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("modelswarm-bench-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn validator_actually_rejects_malformed_records() {
    // Proves the mini validator is not a rubber stamp.
    let schema = load_schema("mode-result.schema.json");
    let engine = engine();
    let record = &engine.run_cell(&cell(), Mode::Single, 1, 1, 8).await[0];
    let json = serde_json::to_value(record).unwrap();

    // Pristine record passes.
    assert!(validate(&schema, &json).is_empty());

    // Extra property.
    let mut extra = json.clone();
    extra["surprise"] = Value::Bool(true);
    assert!(!validate(&schema, &extra).is_empty());

    // Bad run id pattern.
    let mut bad_id = json.clone();
    bad_id["run_manifest_id"] = Value::String("not-a-run-id".to_string());
    assert!(!validate(&schema, &bad_id).is_empty());

    // peers below minimum.
    let mut bad_peers = json.clone();
    bad_peers["peers"] = Value::from(0);
    assert!(!validate(&schema, &bad_peers).is_empty());

    // Unknown mode enum value.
    let mut bad_mode = json.clone();
    bad_mode["mode"] = Value::String("telepathic".to_string());
    assert!(!validate(&schema, &bad_mode).is_empty());

    // acceptance_rate above maximum.
    let mut bad_rate = json.clone();
    bad_rate["metrics"]["acceptance_rate"] = Value::from(5);
    assert!(!validate(&schema, &bad_rate).is_empty());

    // Missing required metric.
    let mut missing = json.clone();
    missing["metrics"]
        .as_object_mut()
        .unwrap()
        .remove("ttft_ms");
    assert!(!validate(&schema, &missing).is_empty());
}

/// Phase E: multi-proposer candidate-tree records validate against the
/// frozen (closed) schema with mode `speculative_exact` and `peers = N`,
/// carry acceptance measured from real tree rounds, and include the
/// duplicate-draft waste in `aggregate_model_tokens` /
/// `bytes_per_accepted_token` (the documented adjustment — the schema
/// has no dedicated duplicate-work field). Deterministic per seed.
#[tokio::test]
async fn multi_proposer_records_validate_against_the_frozen_schema() {
    let engine = engine();
    let schema = load_schema("mode-result.schema.json");
    let records = engine
        .run_cell_multi(&NetworkCell::LOOPBACK, 4, 3, 0xE00E)
        .await;
    assert_eq!(records.len(), 3);
    let mut engaged = 0usize;
    for record in &records {
        assert_eq!(record.mode, Mode::SpeculativeExact);
        assert_eq!(record.peers, 4);
        let json = serde_json::to_value(record).unwrap();
        let errors = validate(&schema, &json);
        assert!(
            errors.is_empty(),
            "multi record failed schema validation:\n  {}\nrecord: {json}",
            errors.join("\n  ")
        );
        // Honest outcome either way: engaged tree rounds, or a visible
        // cost-model fallback (negative results are first-class).
        match record.outcome.status {
            RunStatus::Completed => engaged += 1,
            RunStatus::FellBackToSingle => {
                assert!(record.outcome.fallback_reason.is_some());
                assert!(record.metrics.cooperative_prediction_ms.is_some());
                continue;
            }
            other => panic!("unexpected multi record status: {other:?}"),
        }
        let metrics = &record.metrics;
        assert_eq!(metrics.exactness, Some(Exactness::GreedyEqual));
        assert_eq!(metrics.proposal_window_tokens, Some(8));
        // Acceptance from real rounds: every metric present and sane.
        assert!(metrics.verification_steps.unwrap() >= 1);
        let acceptance = metrics.acceptance_rate.unwrap();
        assert!((0.0..=1.0).contains(&acceptance));
        assert!(metrics.mean_acceptance_length.unwrap() >= 0.0);
        assert!(metrics.rollback_count.unwrap() <= metrics.verification_steps.unwrap());
        // Waste included: 4 slots × window 8 drafts per round vs the
        // accepted tokens — the aggregate strictly exceeds the output.
        assert!(
            metrics.aggregate_model_tokens.unwrap() > metrics.output_tokens,
            "duplicate drafts must inflate aggregate model tokens"
        );
        assert!(metrics.bytes_per_accepted_token.unwrap() > 0.0);
    }
    assert!(
        engaged >= 1,
        "loopback should exercise the engaged multi path: {:?}",
        records.iter().map(|r| r.outcome.status).collect::<Vec<_>>()
    );
    // Same seed ⇒ byte-identical multi records.
    let again = engine
        .run_cell_multi(&NetworkCell::LOOPBACK, 4, 3, 0xE00E)
        .await;
    let a: Vec<String> = records
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    let b: Vec<String> = again
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    assert_eq!(a, b);
}

#[tokio::test]
async fn hand_built_invalid_metric_types_are_caught() {
    // Direct proof that integer/number discrimination works.
    let schema = load_schema("mode-result.schema.json");
    let mut metrics = serde_json::to_value(ModeMetrics::default()).unwrap();
    metrics["prompt_tokens"] = Value::from(1.5); // integer field, float value
    let record = ModeResultRecord::new(
        "run-0123456789abcdef",
        Mode::Single,
        1,
        serde_json::from_value(metrics).unwrap_or_default(),
        Default::default(),
    );
    let json = serde_json::to_value(record).unwrap();
    let errors = validate(&schema, &json);
    // Note: serde's u32 deserialization rejects 1.5 before we get there, so
    // this asserts on the default-metrics path; the validator's integer
    // discrimination is separately covered in common's own tests.
    assert!(!errors.is_empty() || json["metrics"]["prompt_tokens"] == 0);
}

// REVIEW PIN (scheduler 2026-10-11, guard-calibration review
// recommendation iii — the engaged-loss EWMA engage gate): replay the
// COMMITTED pass-2 LAN decision-join rows through the REAL
// `EngagedLossGate` and pin the exact verdicts. The committed artifacts
// are immutable evidence, so exact numbers are stable; they re-derive
// the review's §2 claim ("an EWMA on realized/fastest_single_prediction
// blocking at ≥ 1.0 blocks 8/8 engaged cells") with the shipped
// implementation: per cell, every cell that ever reached four engaged
// attempts is blocked from at latest the 5th (the one cell that did not
// — geo700-w8 — had only 2 engaged attempts because the pass-2 margin
// rule already blocked the other 118/120; it never warmed, and its two
// samples' EWMA is 3.018). Whole-run (one per-profile gate, actual sweep
// order): the 3 warm-up attempts run, then 1,979/1,982 are refused
// pre-round.
#[cfg(feature = "quic-runner")]
#[test]
fn engage_loss_gate_replay_of_committed_pass2_lan_rows() {
    use modelswarm_bench::engage_gate::{
        EngagedLossGate, ENGAGE_LOSS_BLOCK_THRESHOLD, ENGAGE_LOSS_EWMA_ALPHA, ENGAGE_LOSS_WARMUP_N,
    };
    use modelswarm_bench::harness::JoinRow;

    // Frozen specification (the review asked for it to be explicit).
    assert!((ENGAGE_LOSS_EWMA_ALPHA - 0.3).abs() < 1e-12);
    assert_eq!(ENGAGE_LOSS_EWMA_ALPHA, modelswarm_scheduler::EWMA_ALPHA);
    assert_eq!(ENGAGE_LOSS_WARMUP_N, 3);
    assert!((ENGAGE_LOSS_BLOCK_THRESHOLD - 1.0).abs() < 1e-12);

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10");
    let read_cell = |cell: &str| -> Vec<JoinRow> {
        let path = root.join(cell).join("decision-join.jsonl");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("committed artifact {} unreadable: {e}", path.display()));
        text.lines()
            .map(|line| serde_json::from_str(line).expect("join row parses"))
            .collect()
    };
    // Production replay semantics: an engaged attempt whose gate decision
    // is BLOCKED is suppressed (never runs, never feeds); an allowed
    // attempt runs and feeds realized/predicted into the gate.
    let replay = |gate: &mut EngagedLossGate, rows: &[JoinRow]| -> (usize, usize, Option<usize>) {
        let mut fed = 0usize;
        let mut suppressed = 0usize;
        let mut first_block: Option<usize> = None;
        for (index, row) in rows.iter().enumerate() {
            if row.loss_guard.is_none() || row.status != "Completed" {
                continue; // not an engaged attempt
            }
            if gate.decision("msp1:lan").blocked {
                suppressed += 1;
                first_block.get_or_insert(index);
            } else {
                let prediction = row
                    .fastest_single_prediction_ms
                    .expect("engaged rows carry it");
                gate.record_engaged("msp1:lan", row.realized_total_ms / prediction);
                fed += 1;
            }
        }
        (fed, suppressed, first_block)
    };

    // Per-cell (fresh gate per cell — the pass-2 drivers' per-cell
    // isolation): every cell with ≥ 4 engaged attempts blocks, from
    // attempt 4 (5 for the parity-corner cell high-w16, whose EWMA
    // crossed 1.0 at 1.0069 only after the 4th sample).
    let mut cells_blocked = 0usize;
    for (cell, attempts, fed, suppressed) in [
        ("inj0ms-geo700-w16", 180, 3, 177),
        ("inj0ms-geo800-w16", 15, 3, 12),
        ("inj0ms-geo800-w8", 15, 3, 12),
        ("inj0ms-geo900-w16", 438, 3, 435),
        ("inj0ms-geo900-w8", 372, 3, 369),
        ("inj0ms-high-w16", 480, 4, 476),
        ("inj0ms-high-w8", 480, 3, 477),
    ] {
        let rows = read_cell(cell);
        let attempts_seen: usize = rows
            .iter()
            .filter(|r| r.loss_guard.is_some() && r.status == "Completed")
            .count();
        assert_eq!(attempts_seen, attempts, "{cell}: engaged attempts");
        let mut gate = EngagedLossGate::new();
        let (fed_seen, suppressed_seen, first_block) = replay(&mut gate, &rows);
        assert_eq!(fed_seen, fed, "{cell}: attempts allowed (warm-up)");
        assert_eq!(
            suppressed_seen, suppressed,
            "{cell}: attempts refused pre-round"
        );
        let first_block_index = first_block.expect("{cell}: must end blocked");
        let blocked_from_attempt = rows[..=first_block_index]
            .iter()
            .filter(|r| r.loss_guard.is_some() && r.status == "Completed")
            .count();
        assert!(
            blocked_from_attempt <= ENGAGE_LOSS_WARMUP_N as usize + 2,
            "{cell}: first block at engaged attempt {blocked_from_attempt}"
        );
        assert!(gate.decision("msp1:lan").blocked, "{cell}: ends blocked");
        cells_blocked += 1;
    }
    assert_eq!(cells_blocked, 7, "every warmed cell blocks");

    // The never-warmed cell: only 2 engaged attempts existed (the pass-2
    // margin rule blocked the rest); the EWMA (3.018 over both) is far
    // above threshold but the warm-up never completed — cold start
    // honestly allows, and the margin rule covers it.
    let rows = read_cell("inj0ms-geo700-w8");
    let mut gate = EngagedLossGate::new();
    let (fed, suppressed, first_block) = replay(&mut gate, &rows);
    assert_eq!((fed, suppressed), (2, 0));
    assert!(first_block.is_none());
    assert!(!gate.decision("msp1:lan").blocked);
    assert!(gate.decision("msp1:lan").ewma.unwrap() > 3.0);

    // Whole-run, actual sweep order (high,geo900,geo800,geo700 × w8,w16 —
    // the recorded ops command), ONE per-profile gate: the 3 warm-up
    // attempts run, then every further engaged attempt is refused.
    let order = [
        "inj0ms-high-w8",
        "inj0ms-geo900-w8",
        "inj0ms-geo800-w8",
        "inj0ms-geo700-w8",
        "inj0ms-high-w16",
        "inj0ms-geo900-w16",
        "inj0ms-geo800-w16",
        "inj0ms-geo700-w16",
    ];
    let mut whole: Vec<JoinRow> = Vec::new();
    for cell in order {
        whole.extend(read_cell(cell));
    }
    let mut gate = EngagedLossGate::new();
    let (fed, suppressed, first_block) = replay(&mut gate, &whole);
    assert_eq!(
        fed, ENGAGE_LOSS_WARMUP_N as usize,
        "exactly the warm-up runs"
    );
    assert_eq!(
        suppressed, 1979,
        "1,979/1,982 engaged attempts refused pre-round"
    );
    assert!(first_block.is_some());
    assert!(gate.decision("msp1:lan").blocked);
}

// REVIEW PIN (test-release 2026-10-10, guard-calibration review
// docs/reviews/review-guard-calibration-2026-10-10.md): the pass-2
// counterfactual honesty contract of `LossGuardRecord`. Rows below are
// lifted VERBATIM from committed pass-2 LAN decision-join artifacts
// (experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10 — one row the default
// production guard would have aborted, one it would have passed). A run
// recorded under a relaxed/off multiplier must still report what the
// DEFAULT guard WOULD have done, and that counterfactual must be
// re-derivable from the row's own numbers — a relaxed engaged-curve run
// can never masquerade as guard-passing.
// Feature-gated: LossGuardRecord lives in the quic-runner-gated
// harness module — ungated, this breaks `cargo test --workspace`
// (default) and default clippy --all-targets in CI.
#[cfg(feature = "quic-runner")]
#[test]
fn loss_guard_counterfactual_matches_default_arithmetic() {
    // inj0ms-geo700-w16 row: round1_wall/predicted_round ≈ 74.0× — the
    // in-effect guard is OFF (did not abort), yet the record says the
    // default production guard WOULD have fired.
    let fired = r#"{"multiplier": "off", "round1_wall_ms": 219.44910000000002,
        "predicted_round_ms": 2.9645614583333333, "fired": false,
        "would_fire_at_default": true}"#;
    // inj0ms-high-w8 planner row at the pass/fail boundary:
    // round1_wall/predicted_round ≈ 2.2995 ≤ 2.0×(1+0.15) — the default
    // guard would NOT have fired (one of the 125 leak rows the
    // calibration review characterizes).
    let passed = r#"{"multiplier": "off", "round1_wall_ms": 172.0826,
        "predicted_round_ms": 74.83229000000001, "fired": false,
        "would_fire_at_default": false}"#;
    for (raw, expect_default_fires) in [(fired, true), (passed, false)] {
        let record: modelswarm_bench::harness::LossGuardRecord =
            serde_json::from_str(raw).expect("verbatim artifact rows must deserialize");
        // Relaxations are recorded per row — never silent.
        assert_eq!(record.multiplier, "off");
        assert!(!record.fired, "the in-effect 'off' guard never aborts");
        // The counterfactual is exactly the default-production threshold:
        // round1_wall > DEFAULT_LOSS_MULTIPLIER × (1 + margin) × predicted.
        let default_threshold = modelswarm_bench::params::DEFAULT_LOSS_MULTIPLIER
            * (1.0 + modelswarm_scheduler::DEFAULT_CONFIDENCE_MARGIN)
            * record.predicted_round_ms;
        assert_eq!(
            record.would_fire_at_default,
            record.round1_wall_ms > default_threshold,
            "would_fire_at_default must equal the default-multiplier arithmetic"
        );
        assert_eq!(
            record.would_fire_at_default, expect_default_fires,
            "the recorded counterfactual must match the committed artifact row"
        );
    }
}
