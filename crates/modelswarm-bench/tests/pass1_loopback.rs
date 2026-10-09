//! 9.6 pass-1 loopback integration tests over REAL serving bridges
//! (environment label: LOOPBACK + INJECTED DELAY — never LAN claims).
//!
//! - [`runner_smoke_over_real_quic`] — the fast CI proof: three real
//!   serving bridges over real QUIC with a 5 ms executor-seam injection,
//!   all arms, frozen-schema records, request-id-level joins, calibration
//!   within a timer-tolerant band, acceptance recorded from real wire
//!   rounds.
//! - [`pass1_loopback_dryrun`] — `#[ignore}]`d owner-invoked dry run of
//!   the full sweep; writes the artifacts committed under
//!   `experiments/raw/PASS-1-LOOPBACK-DRYRUN/` (see the report at
//!   `experiments/reports/PASS-1-LOOPBACK-DRYRUN.md` for the exact
//!   invocation).

use std::time::Duration;

use modelswarm_bench::corpus::PASS1_CORPUS;
use modelswarm_bench::harness::{LoopbackCellSpec, Pass1Harness};
use modelswarm_bench::params::Pass1Params;
use modelswarm_bench::records::{Mode, RunStatus};
use modelswarm_bench::runner::{AcceptanceRegime, BridgeConfig, ENV_LABEL_LOOPBACK_INJECTED};
use modelswarm_identity::InstallationIdentity;
use std::sync::Arc;

/// The experiment client identity (documented seed; the serving-side
/// leases bind to its derived PeerId).
fn harness_identity() -> InstallationIdentity {
    InstallationIdentity::from_bytes(&[0xB0; 32])
}

/// The ADR-011-shaped experiment profile id.
fn profile_id() -> String {
    format!("msp1:{}", "1".repeat(64))
}

/// The asymmetric trio (bench-harness-spec "asymmetric hardware" shape):
/// a free mid host, a free slow host, and a fast-but-busy drafter whose
/// advertised queue ranks it last for single prediction while its decode
/// rate makes it the useful proposer at k=3.
fn asymmetric_bridges() -> Vec<BridgeConfig> {
    // Verifier-class (slow decode, free), a second free host, and a
    // fast-but-busy drafter whose advertised queue ranks it last for
    // single prediction while its decode rate makes it the useful
    // proposer at k=3 (the bench "asymmetric hardware" shape).
    let mut verifier = BridgeConfig::new(0x11, 0.10, 1.5);
    verifier.advertised_queue_ms = 0;
    let mut second = BridgeConfig::new(0x22, 0.08, 1.2);
    second.advertised_queue_ms = 0;
    let mut drafter = BridgeConfig::new(0x33, 2.00, 4.0);
    drafter.advertised_queue_ms = 300;
    drafter.capacity_class = "gpu_high";
    vec![verifier, second, drafter]
}

fn smoke_params() -> Pass1Params {
    Pass1Params {
        proposal_window: 8,
        output_tokens_target: 16,
        cohort_cap: 3,
        runs_per_arm: 1,
        warmup_completions: 1,
        ..Pass1Params::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runner_smoke_over_real_quic() {
    let out = tempfile::tempdir().unwrap();
    let mut harness = Pass1Harness::new(
        profile_id(),
        harness_identity(),
        modelswarm_bench::acceptance::AcceptanceStore::memory(),
        "9.6-pass1-smoke",
        out.path().to_path_buf(),
    );
    let params = smoke_params();
    let env = modelswarm_scheduler::shadow::EnvPolicy {
        policy: modelswarm_scheduler::shadow::QueueDiscountPolicy::DEFAULT,
        env_overrides: 0,
        warnings: Vec::new(),
    };
    let spec = LoopbackCellSpec {
        injected_delay_ms: 5,
        regime: AcceptanceRegime::High,
        bridges: asymmetric_bridges(),
    };
    let artifacts = harness
        .run_loopback_cell(&spec, &params, &env, 0x5A0C_3D01)
        .await
        .expect("cell runs");

    // Every arm produced join rows, labeled loopback+injected-delay.
    let mut arms: Vec<String> = artifacts.join_rows.iter().map(|r| r.arm.clone()).collect();
    arms.sort();
    arms.dedup();
    assert_eq!(
        arms,
        vec!["fastest-single", "fixed-k2", "fixed-k3", "planner-k"],
        "all four arms recorded"
    );
    assert!(artifacts
        .join_rows
        .iter()
        .all(|r| r.label == ENV_LABEL_LOOPBACK_INJECTED));
    assert!(artifacts
        .join_rows
        .iter()
        .all(|r| r.realized_ttft_ms > 0.0 && r.realized_total_ms > 0.0));

    // Records conform to the frozen mode-result shape and every record
    // joins to a manifest + a join row at request-id level.
    assert_eq!(artifacts.records.len(), artifacts.join_rows.len());
    let manifest_ids: Vec<&String> = artifacts
        .manifests
        .iter()
        .map(|(_, m)| &m.run_manifest_id)
        .collect();
    for record in &artifacts.records {
        assert!(manifest_ids.contains(&&record.run_manifest_id));
        assert!(record.metrics.ttft_ms > 0.0);
        assert!(record.metrics.completion_ms > 0.0);
    }
    for (record, join) in artifacts.records.iter().zip(&artifacts.join_rows) {
        assert_eq!(record.run_manifest_id, join.run_manifest_id);
    }

    // The single comparator ran independently and completed.
    let singles: Vec<_> = artifacts
        .records
        .iter()
        .filter(|r| r.mode == Mode::Single)
        .collect();
    assert_eq!(
        singles.len(),
        PASS1_CORPUS.len(),
        "one single run per corpus prompt"
    );
    assert!(singles
        .iter()
        .all(|r| r.outcome.status == RunStatus::Completed));

    // Cooperative rows are honest: either completed with measured
    // acceptance, or fell back with a recorded reason — never silent.
    for record in &artifacts.records {
        if record.mode != Mode::SpeculativeExact {
            continue;
        }
        match record.outcome.status {
            RunStatus::Completed => {
                assert!(record.metrics.acceptance_rate.is_some());
                assert!(record.metrics.cooperative_prediction_ms.is_some());
                assert!(
                    record.metrics.fastest_single_actual_ms.is_some(),
                    "S4: independent single citation"
                );
            }
            RunStatus::FellBackToSingle => {
                assert!(record.outcome.fallback_reason.is_some(), "visible fallback");
            }
            other => panic!("unexpected cooperative status {other:?}"),
        }
    }

    // Calibration measured the injected delay within a timer-tolerant
    // band (CI timer overshoot makes the strict 10% manifest tolerance
    // flaky here; the artifact records the strict flag separately).
    let calibration = &artifacts.calibration;
    assert_eq!(calibration.label, ENV_LABEL_LOOPBACK_INJECTED);
    assert!(
        calibration.measured_injected_ms_p50 >= 2.5 && calibration.measured_injected_ms_p50 <= 7.5,
        "5ms injection measured at {:.2}ms",
        calibration.measured_injected_ms_p50
    );

    // The round machinery executed on the real wire: the k=3 cohort with
    // the fast drafter passes the engage gate (model prediction beats the
    // slow verifier-class single), runs wire rounds (acceptance recorded
    // per proposer), and the honest outcome here is the observed-loss
    // rule-6 abort or a completed run — either way, visible.
    assert!(
        !harness.acceptance().snapshot().is_empty(),
        "acceptance store filled from real wire rounds"
    );
    let round_evidence: Vec<_> = artifacts
        .join_rows
        .iter()
        .filter(|r| {
            r.rounds > 1
                || r.reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("observed loss"))
        })
        .collect();
    assert!(
        !round_evidence.is_empty(),
        "expected wire-round evidence (completed rounds or an observed-loss abort)"
    );
    let planner_rows: Vec<_> = artifacts
        .join_rows
        .iter()
        .filter(|r| r.planner_sweep.is_some())
        .collect();
    assert!(!planner_rows.is_empty(), "planner sweep telemetry recorded");

    // Summary ratios computed (script, not hand) with the single median.
    let summary = &artifacts.summary;
    assert!(summary.fastest_single_median_ms.is_some());
    assert!(summary
        .arms
        .iter()
        .all(|a| a.median_vs_fastest_single.is_some()));

    // Artifacts landed on disk in experiments/ shape.
    assert!(artifacts.dir.join("mode-results.jsonl").exists());
    assert!(artifacts.dir.join("decision-join.jsonl").exists());
    assert!(artifacts.dir.join("calibration.json").exists());
    assert!(artifacts.dir.join("acceptance-snapshot.json").exists());
    assert!(artifacts.dir.join("summary.json").exists());
    for (tag, _) in &artifacts.manifests {
        assert!(artifacts.dir.join(format!("manifest-{tag}.json")).exists());
    }
}

/// The full-harness dry run (owner-invoked; `#[ignore]` keeps it out of
/// CI — wall-clock). Sweep: injected {5, 20} ms × acceptance regimes
/// {high, zero, mixed} over the asymmetric trio, window 4, target 16,
/// 2 reps/arm. Emits the artifacts committed under
/// experiments/raw/PASS-1-LOOPBACK-DRYRUN/.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "owner-invoked dry run (writes the committed experiment artifacts; ~2-4 min)"]
async fn pass1_loopback_dryrun() {
    let out = std::env::var("MSP_BENCH_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pass1-dryrun")
        });
    std::fs::create_dir_all(&out).expect("create out dir");
    // Persistent acceptance store: cross-cell resume is part of the
    // machinery under test.
    let acceptance_path = out.join("acceptance.sqlite");
    let acceptance = modelswarm_bench::acceptance::AcceptanceStore::open(&acceptance_path)
        .expect("open acceptance store");
    let mut harness = Pass1Harness::new(
        profile_id(),
        harness_identity(),
        acceptance,
        "9.6-pass1",
        out.clone(),
    );
    let params = Pass1Params {
        proposal_window: 4,
        output_tokens_target: 16,
        cohort_cap: 3,
        runs_per_arm: 2,
        warmup_completions: 2,
        ..Pass1Params::default()
    };
    let env = modelswarm_scheduler::shadow::EnvPolicy {
        policy: modelswarm_scheduler::shadow::QueueDiscountPolicy::DEFAULT,
        env_overrides: 0,
        warnings: Vec::new(),
    };
    for delay in [5u32, 20] {
        for regime in [
            AcceptanceRegime::High,
            AcceptanceRegime::Zero,
            AcceptanceRegime::Mixed,
        ] {
            let spec = LoopbackCellSpec {
                injected_delay_ms: delay,
                regime,
                bridges: asymmetric_bridges(),
            };
            let seed = 0xD2A0_0000 | (u64::from(delay) << 8) | regime_discriminant(regime);
            let artifacts = harness
                .run_loopback_cell(&spec, &params, &env, seed)
                .await
                .expect("dry-run cell");
            // Every cell must have produced its core artifacts before
            // the next one starts.
            assert!(artifacts.dir.join("mode-results.jsonl").exists());
            assert!(artifacts.records.len() == artifacts.join_rows.len());
            eprintln!(
                "[PASS-1-LOOPBACK-DRYRUN] cell {} -> {} records, calibration {:.2}ms (valid={:?})",
                spec.name(),
                artifacts.records.len(),
                artifacts.calibration.measured_injected_ms_p50,
                artifacts.calibration.calibration_valid,
            );
        }
    }
    let _ = Duration::ZERO;
}

fn regime_discriminant(regime: AcceptanceRegime) -> u64 {
    match regime {
        AcceptanceRegime::High => 1,
        AcceptanceRegime::Zero => 2,
        AcceptanceRegime::Mixed => 3,
    }
}

/// Owner-gated LAN pass-1 (machines A + B). The serve side is the
/// `pass1_serve` example on machine B (real QUIC listener, real RTT,
/// TEST-ONLY synthetic executor); the driver dials the addresses B
/// printed. Env: `MSP_BENCH_PEER_0..9` (B's lines), `MSP_BENCH_RUNS`
/// (default 30), `MSP_BENCH_WINDOW` (default 8), `MSP_BENCH_OUT` (default
/// `experiments/raw/PASS-1-LAN-<unix-secs>`). Labels: lan-2machine-quic —
/// never WAN claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "owner-invoked LAN run (needs pass1_serve on machine B)"]
async fn pass1_lan_run() {
    let mut remotes = Vec::new();
    for i in 0..10 {
        if let Ok(line) = std::env::var(format!("MSP_BENCH_PEER_{i}")) {
            remotes.push(
                modelswarm_bench::runner::parse_peer_line(&line)
                    .expect("MSP_BENCH_PEER line parses"),
            );
        }
    }
    assert!(
        remotes.len() >= 2,
        "need at least MSP_BENCH_PEER_0 and _1 from the serve side"
    );
    let runs: u32 = std::env::var("MSP_BENCH_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let window: u32 = std::env::var("MSP_BENCH_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out = std::env::var("MSP_BENCH_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../experiments/raw/PASS-1-LAN-{stamp}"))
        });
    std::fs::create_dir_all(&out).expect("create out dir");
    let acceptance =
        modelswarm_bench::acceptance::AcceptanceStore::open(out.join("acceptance.sqlite"))
            .expect("open acceptance store");
    let mut harness = Pass1Harness::new(
        profile_id(),
        harness_identity(),
        acceptance,
        "9.6-pass1-lan",
        out.clone(),
    );
    let env = modelswarm_scheduler::shadow::EnvPolicy {
        policy: modelswarm_scheduler::shadow::QueueDiscountPolicy::DEFAULT,
        env_overrides: 0,
        warnings: Vec::new(),
    };
    for regime in [
        AcceptanceRegime::High,
        AcceptanceRegime::Zero,
        AcceptanceRegime::Mixed,
    ] {
        // Fresh connections per cell (per-cell pools + warm-up, matching
        // the dry run's bridge lifetimes).
        let metrics = Arc::clone(harness.metrics());
        let mut bridges = Vec::with_capacity(remotes.len());
        for remote in &remotes {
            bridges.push(
                modelswarm_bench::runner::connect_remote(
                    remote,
                    &harness_identity(),
                    Arc::clone(&metrics),
                )
                .await
                .expect("connect remote bridge"),
            );
        }
        harness.set_lan_bridges(bridges);
        let spec = LoopbackCellSpec {
            injected_delay_ms: 0,
            regime,
            bridges: Vec::new(),
        };
        let params = Pass1Params {
            proposal_window: window,
            output_tokens_target: 16,
            cohort_cap: 3,
            runs_per_arm: runs,
            warmup_completions: 2,
            ..Pass1Params::default()
        };
        let seed = 0xD2A1_0000 | regime_discriminant(regime);
        let artifacts = harness
            .run_loopback_cell(&spec, &params, &env, seed)
            .await
            .expect("lan cell");
        assert!(artifacts.dir.join("mode-results.jsonl").exists());
        eprintln!(
            "[PASS-1-LAN] cell {} -> {} records, label {}",
            spec.name(),
            artifacts.records.len(),
            modelswarm_bench::runner::ENV_LABEL_LAN,
        );
    }
}
