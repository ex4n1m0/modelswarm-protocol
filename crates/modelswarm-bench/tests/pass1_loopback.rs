//! 9.6 pass-1 loopback integration tests over REAL serving bridges
//! (environment label: LOOPBACK + INJECTED DELAY — never LAN claims).
//!
//! - [`runner_smoke_over_real_quic`] — the fast CI proof: three real
//!   serving bridges over real QUIC with a 5 ms executor-seam injection,
//!   all arms, frozen-schema records, request-id-level joins, calibration
//!   within a timer-tolerant band, and (since the ADR-032 §4 companion
//!   correction) the never-engage contract: every cooperative row falls
//!   back at the engage gate under the wire-true sequential verification
//!   term, and no speculative round runs.
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
        window_tag: None,
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

    // ADR-032 §4 companion correction (2026-10-11): the acting planner's
    // verification term is wire-true for any cohort whose verifier has
    // not declared batch-verify — every cohort today. The verifier of a
    // cohort here IS the fastest-single peer (ordered[0]), so charging
    // window+1 sequential verifier tokens per round makes an engaged run
    // structurally unable to beat the single: the pass-2 never-engage
    // conclusion, ENFORCED. No speculative round may run on this
    // HTTP-only loopback cell — the acceptance store stays empty and
    // every cooperative row is an engage-gate fallback whose reason
    // names the wire-true term. (The engaged wire-round machinery itself
    // stays pinned by the runner unit tests and the committed pass-2
    // owner-gated runs; it is unreachable in CI by design now.)
    assert!(
        harness.acceptance().snapshot().is_empty(),
        "HTTP-only loopback cohort must never run a speculative round"
    );
    let cooperative: Vec<_> = artifacts
        .join_rows
        .iter()
        .filter(|r| r.arm != "fastest-single")
        .collect();
    assert!(
        !cooperative.is_empty(),
        "cooperative arms recorded (as fallbacks)"
    );
    for row in &cooperative {
        assert_eq!(
            row.status, "FellBackToSingle",
            "row {} must fall back at the engage gate",
            row.request_root
        );
        assert_eq!(row.rounds, 1, "no speculative round ran");
        let reason = row.reason.as_deref().expect("visible fallback reason");
        assert!(
            reason.contains("wire-true sequential verify"),
            "the fallback reason must name the verification term in force: {reason}"
        );
    }
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
                window_tag: None,
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
        AcceptanceRegime::Geometric { p_permille } => 4 + u64::from(p_permille),
    }
}

/// The pass-2 ENGAGEMENT pool (mirror of `pass1_serve --pool pass2`):
/// V = fastest single (fast prefill 3.0, slow decode 0.10, free), D =
/// drafter (decode 4.0 = 40x V, queue 300 so it is drafted from, never
/// the comparator), M/M2 = busy mids. This is the shape that lets the
/// frozen engage rule legitimately pass so engaged speculative win/loss
/// is measurable on the wire.
fn pass2_pool() -> Vec<BridgeConfig> {
    let verifier = BridgeConfig::new(0x51, 0.10, 3.0);
    let mut drafter = BridgeConfig::new(0x66, 4.00, 6.0);
    drafter.advertised_queue_ms = 300;
    drafter.capacity_class = "gpu_high";
    let mut mid = BridgeConfig::new(0x77, 0.25, 1.2);
    mid.advertised_queue_ms = 400;
    let mut mid2 = BridgeConfig::new(0x88, 0.20, 1.0);
    mid2.advertised_queue_ms = 500;
    vec![verifier, drafter, mid, mid2]
}

/// Shared env parsing for the pass-2 drivers.
fn pass2_env() -> (Vec<AcceptanceRegime>, u32, Option<u32>, u32, Option<f64>) {
    let profiles: Vec<AcceptanceRegime> = std::env::var("MSP_BENCH_PROFILES")
        .unwrap_or_else(|_| "high,geo900,geo800,geo700".to_string())
        .split(',')
        .map(|tag| modelswarm_bench::runner::parse_regime(tag).expect("profile parses"))
        .collect();
    let window: u32 = std::env::var("MSP_BENCH_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let window2: Option<u32> = std::env::var("MSP_BENCH_WINDOW2")
        .ok()
        .and_then(|v| v.parse().ok());
    let runs: u32 = std::env::var("MSP_BENCH_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    // Default for this instrument: DISABLED (the engaged-curve
    // measurement); every row records the multiplier AND the
    // default-production counterfactual, so a relaxed run can never
    // masquerade as guard-passing.
    let loss_multiplier = match std::env::var("MSP_BENCH_LOSS_MULTIPLIER") {
        Ok(v) if v.eq_ignore_ascii_case("off") => None,
        Ok(v) => Some(v.parse::<f64>().expect("loss multiplier parses")),
        Err(_) => None,
    };
    (profiles, window, window2, runs, loss_multiplier)
}

/// 9.6 PASS-2 loopback engagement run (owner-invoked). Dials ACCEPTANCE
/// through the geometric prefix-match family (real propose → transport →
/// verify → accept/reject on the wire; never a store-fed regime) over
/// the engagement pool. At window 8 the implied per-round acceptance
/// rates are ~1.0 (high), ~0.64 (geo900), ~0.42 (geo800), ~0.27 (geo700
/// — expected blocked at the engage gate once the EWMA warms; the honest
/// never-worse behavior). Env: `MSP_BENCH_PROFILES` (default
/// `high,geo900,geo800,geo700`), `MSP_BENCH_DELAYS` (default `5,20`),
/// `MSP_BENCH_WINDOW` (+ optional `MSP_BENCH_WINDOW2` second pass),
/// `MSP_BENCH_RUNS` (default 30; loopback runs typically override to
/// 8-12), `MSP_BENCH_LOSS_MULTIPLIER` (default `off`), `MSP_BENCH_OUT`.
/// Labels: loopback+injected-delay — never LAN claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "owner-invoked pass-2 loopback run (writes committed experiment artifacts)"]
async fn pass2_engagement_loopback() {
    let out = std::env::var("MSP_BENCH_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pass2-loopback")
        });
    std::fs::create_dir_all(&out).expect("create out dir");
    let (profiles, window, window2, runs, loss_multiplier) = pass2_env();
    let delays: Vec<u32> = std::env::var("MSP_BENCH_DELAYS")
        .unwrap_or_else(|_| "5,20".to_string())
        .split(',')
        .map(|d| d.parse().expect("delay parses"))
        .collect();
    let env = modelswarm_scheduler::shadow::EnvPolicy {
        policy: modelswarm_scheduler::shadow::QueueDiscountPolicy::DEFAULT,
        env_overrides: 0,
        warnings: Vec::new(),
    };
    let base_params = Pass1Params {
        proposal_window: window,
        output_tokens_target: 16,
        cohort_cap: 4,
        runs_per_arm: runs.max(1),
        warmup_completions: 2,
        loss_multiplier,
        ..Pass1Params::default()
    };
    let mut plan: Vec<(u32, AcceptanceRegime, u32)> = Vec::new();
    for &delay in &delays {
        for &regime in &profiles {
            plan.push((delay, regime, window));
        }
    }
    if let Some(w2) = window2 {
        for &delay in &delays {
            for &regime in &profiles {
                plan.push((delay, regime, w2));
            }
        }
    }
    for (delay, regime, cell_window) in plan {
        let params = Pass1Params {
            proposal_window: cell_window,
            ..base_params.clone()
        };
        let spec = LoopbackCellSpec {
            injected_delay_ms: delay,
            regime,
            bridges: pass2_pool(),
            window_tag: Some(cell_window),
        };
        // FRESH harness + acceptance store per cell: pass 2 measures
        // per-profile acceptance curves, and a cross-profile EWMA
        // carryover would flip gate-blocked profiles into engaged ones
        // (a confound, not a feature). Pass 1 deliberately resumed one
        // store across cells; pass 2 records the isolation instead.
        let acceptance = modelswarm_bench::acceptance::AcceptanceStore::open(
            out.join(format!("acceptance-{}.sqlite", spec.name())),
        )
        .expect("open per-cell acceptance store");
        let mut harness = Pass1Harness::new(
            profile_id(),
            harness_identity(),
            acceptance,
            "9.6-pass2",
            out.clone(),
        );
        let seed = 0xD3A0_0000
            | (u64::from(delay) << 24)
            | (regime_discriminant(regime) << 8)
            | u64::from(cell_window);
        let artifacts = harness
            .run_loopback_cell(&spec, &params, &env, seed)
            .await
            .expect("pass-2 loopback cell");
        eprintln!(
            "[PASS-2-LOOPBACK] cell {} -> {} records (fallbacks {:?})",
            spec.name(),
            artifacts.records.len(),
            artifacts
                .summary
                .arms
                .iter()
                .map(|a| (a.arm.as_str(), a.fallbacks))
                .collect::<Vec<_>>(),
        );
    }
}

/// Owner-gated 9.6 PASS-2 LAN run (machines A + B). Serve side:
/// `pass1_serve --pool pass2 --bridges 4` on machine B — a FRESH exe
/// built from this commit (the prefix-match family changes serve-side
/// continuation behavior; a v4-era serve would emit non-matching drafts
/// for the family seeds). Driver env: `MSP_BENCH_PEER_0..9` (B's lines),
/// `MSP_BENCH_PROFILES` (default `high,geo900,geo800,geo700` — implied
/// window-8 acceptance rates ~1.0/~0.64/~0.42/~0.27), `MSP_BENCH_RUNS`
/// (default 30), `MSP_BENCH_WINDOW` (+ optional `MSP_BENCH_WINDOW2`),
/// `MSP_BENCH_COHORT_CAP` (default 4), `MSP_BENCH_LOSS_MULTIPLIER`
/// (default `off`), `MSP_BENCH_OUT` (absolute). Labels:
/// lan-2machine-quic — never WAN claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "owner-invoked LAN pass-2 run (needs pass1_serve --pool pass2 on machine B)"]
async fn pass2_lan_run() {
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
        remotes.len() >= 4,
        "pass-2 LAN needs the engagement pool (>= MSP_BENCH_PEER_0..3 from a --pool pass2 serve)"
    );
    let (profiles, window, window2, runs, loss_multiplier) = pass2_env();
    let cohort_cap: u32 = std::env::var("MSP_BENCH_COHORT_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let out = std::env::var("MSP_BENCH_OUT")
        .map(std::path::PathBuf::from)
        .expect("MSP_BENCH_OUT (absolute path) is required for the LAN pass-2 run");
    std::fs::create_dir_all(&out).expect("create out dir");
    let env = modelswarm_scheduler::shadow::EnvPolicy {
        policy: modelswarm_scheduler::shadow::QueueDiscountPolicy::DEFAULT,
        env_overrides: 0,
        warnings: Vec::new(),
    };
    let base_params = Pass1Params {
        proposal_window: window,
        output_tokens_target: 16,
        cohort_cap,
        runs_per_arm: runs.max(1),
        warmup_completions: 2,
        loss_multiplier,
        ..Pass1Params::default()
    };
    let mut plan: Vec<(AcceptanceRegime, u32)> = profiles
        .iter()
        .map(|&regime| (regime, window))
        .chain(
            window2
                .into_iter()
                .flat_map(|w2| profiles.iter().map(move |&regime| (regime, w2))),
        )
        .collect::<Vec<_>>();
    plan.dedup();
    for (regime, cell_window) in plan {
        // Fresh harness + acceptance store + connections per cell
        // (per-cell pools + warm-up, matching the pass-1 LAN pattern;
        // per-profile acceptance isolation so a cross-profile EWMA
        // carryover cannot flip gate-blocked profiles into engaged ones).
        // Deterministic request ids mean the serve process must be fresh
        // per sweep attempt (replayed_request dedup) — see the pass-1
        // ANALYSIS ops record.
        let acceptance = modelswarm_bench::acceptance::AcceptanceStore::open(out.join(format!(
            "acceptance-inj0ms-{}-w{cell_window}.sqlite",
            regime.cell_tag()
        )))
        .expect("open per-cell acceptance store");
        let mut harness = Pass1Harness::new(
            profile_id(),
            harness_identity(),
            acceptance,
            "9.6-pass2-lan",
            out.clone(),
        );
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
            window_tag: Some(cell_window),
        };
        let params = Pass1Params {
            proposal_window: cell_window,
            ..base_params.clone()
        };
        let seed = 0xD3A1_0000 | (regime_discriminant(regime) << 8) | u64::from(cell_window);
        let artifacts = harness
            .run_loopback_cell(&spec, &params, &env, seed)
            .await
            .expect("pass-2 lan cell");
        assert!(artifacts.dir.join("mode-results.jsonl").exists());
        eprintln!(
            "[PASS-2-LAN] cell {} -> {} records, label {} (fallbacks {:?})",
            spec.name(),
            artifacts.records.len(),
            modelswarm_bench::runner::ENV_LABEL_LAN,
            artifacts
                .summary
                .arms
                .iter()
                .map(|a| (a.arm.as_str(), a.fallbacks))
                .collect::<Vec<_>>(),
        );
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
    // Cohort cap (max k arm): default 3 matches the committed pass-1
    // evidence; raise via env for k-sizing runs against >= 4 serve
    // bridges (k arms are also bounded by the measured candidate count).
    let cohort_cap: u32 = std::env::var("MSP_BENCH_COHORT_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
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
            window_tag: None,
        };
        let params = Pass1Params {
            proposal_window: window,
            output_tokens_target: 16,
            cohort_cap,
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
