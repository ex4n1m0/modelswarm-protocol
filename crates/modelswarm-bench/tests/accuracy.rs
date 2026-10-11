//! Accuracy-benefits study drivers (2026-10-11, the owner directive at
//! ADR-032 acceptance): cross-peer greedy divergence DETECTION — the
//! output-preserving "many is better than one" benefit where the swarm
//! itself is the fault detector. Two same-backend peers hosting the exact
//! same profile decode the same prompt under the same pinned greedy
//! seed; honest peers MUST agree token-for-token (E0), so any divergence
//! flags nondeterminism / a hostile peer / a hardware fault, and the
//! ledger de-ranks the offender from cohort candidacy.
//!
//! - [`divergence_detection_over_real_quic`] — the per-push CI proof:
//!   four REAL serving bridges over real QUIC (loopback), one carrying
//!   the injected divergence fault model; the instrument must detect it
//!   in every check (0 false negatives), flag no honest peer (0 false
//!   positives), suspend exactly the offender, and remove it from the
//!   selected cohort.
//! - [`accuracy_divergence_loopback`] — owner-invoked committed-artifacts
//!   run (clean control cell + injected cell, loopback).
//! - [`accuracy_divergence_lan`] — owner-invoked two-machine LAN run
//!   (`pass1_serve --pool accuracy [--divergent 2]` on machine B);
//!   labels lan-2machine-quic — never WAN claims.
//!
//! Honesty: the synthetic executor is deterministic BY CONSTRUCTION —
//! these runs are STRUCTURAL evidence for the protocol-level detection
//! machinery (inject → detect → report → de-rank), never an
//! engine-divergence-rate claim. Real-engine rates are measured
//! separately (see experiments/raw/ACCURACY-*/2026-10-11 ANALYSIS files)
//! and are never extrapolated from these runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::join_all;
use modelswarm_bench::corpus::{PASS1_CORPUS, PASS1_CORPUS_ID};
use modelswarm_bench::divergence::{
    first_divergence, quorum, stream_digest, AccuracyManifest, AccuracyPoolPeer, AgreementLedger,
    AgreementState, CheckVerdict, CohortDerankRecord, DivergenceCheckRecord,
    ACCURACY_MANIFEST_RECORD_TYPE, COHORT_DERANK_RECORD_TYPE, DIVERGENCE_CHECK_RECORD_TYPE,
};
use modelswarm_bench::runner::{
    accuracy_pool, bridge_peer_id, execute_on, spawn_bridge, BridgeConfig, LiveBridge, WireRequest,
    ACCURACY_POOL_SIZE, ENV_LABEL_ACCURACY_LAN, ENV_LABEL_ACCURACY_LOOPBACK,
    SYNTHETIC_EXECUTOR_NAME,
};
use modelswarm_identity::InstallationIdentity;
use modelswarm_node::measure::PeerMetrics;
use modelswarm_scheduler::{select_microswarm, Candidate, CapacityClass, NatPath};
use modelswarm_telemetry::Telemetry;
use modelswarm_types::ModelProfileId;

/// Harness version recorded in every accuracy manifest (the ANALYSIS
/// files pin the exact commit that produced the committed artifacts).
const HARNESS_VERSION: &str = "accuracy-2026-10-11";
/// Sampling seeds checked per cell (each decoded on the whole pool).
const SEEDS: [u64; 2] = [0x0ACC_0001, 0x0ACC_0002];
/// Output budget per decode (the pass-2 cell shape).
const MAX_TOKENS: u32 = 16;
/// Cohort size the de-rank demonstration selects (want).
const COHORT_WANT: usize = 3;
/// Flat RTT placeholder for the de-rank ranking demo (documented in the
/// record; ranking differences here come from queue + rates, not RTT).
const DEMO_RTT_MS: f64 = 5.0;
/// Prompt token estimate for the ranking demo (the p1-xl shape).
const DEMO_PROMPT_TOKENS: u32 = 153;
/// Scheduler-crate profile for the de-rank candidate demo (the wire-side
/// experiment profile `msp1:<64hex>` is not a `ModelProfileId` shape; the
/// demo needs a parseable one so `select_microswarm` filters by it).
const DEMO_PROFILE: &str = "msp:accuracy:q4_k_m:v1";

fn harness_identity() -> InstallationIdentity {
    InstallationIdentity::from_bytes(&[0xB0; 32])
}

fn profile_id() -> String {
    format!("msp1:{}", "1".repeat(64))
}

/// One cell's measured outcome (returned for assertions; the artifacts
/// are already on disk).
pub struct CellOutcome {
    /// Every comparison row (peer-reference + quorum).
    pub records: Vec<DivergenceCheckRecord>,
    /// The de-rank demonstration.
    pub derank: CohortDerankRecord,
    /// Final per-peer ledger state (peer id → state).
    pub ledger: Vec<(String, AgreementState)>,
    /// Checks executed (prompts × seeds).
    pub checks: usize,
}

impl CellOutcome {
    /// Peer ids the ledger suspended (deterministic de-rank policy).
    pub fn suspended_peers(&self) -> Vec<String> {
        let mut peers: Vec<String> = self
            .ledger
            .iter()
            .filter(|(_, state)| state.suspended())
            .map(|(peer, _)| peer.clone())
            .collect();
        peers.sort();
        peers
    }
}

/// Runs one accuracy-divergence cell over already-connected bridges:
/// every prompt × seed is decoded on the WHOLE pool concurrently with
/// the same request seed; each probe is compared against the
/// verifier-reference (bridge 0) and against the quorum majority; the
/// ledger accumulates per-peer verdicts; the de-rank demonstration
/// selects the cohort before/after suspensions. Artifacts land in
/// `out_dir/<cell>/`.
#[allow(clippy::too_many_arguments)]
async fn run_accuracy_cell(
    cell: &str,
    environment: &str,
    pool: &[BridgeConfig],
    bridges: &[LiveBridge],
    out_root: &Path,
    ops_note: &str,
) -> CellOutcome {
    assert_eq!(
        pool.len(),
        ACCURACY_POOL_SIZE,
        "the pinned accuracy pool has exactly {ACCURACY_POOL_SIZE} members"
    );
    assert_eq!(pool.len(), bridges.len(), "one bridge per pool member");
    for (config, bridge) in pool.iter().zip(bridges) {
        assert_eq!(
            bridge.peer_id,
            bridge_peer_id(config.seed),
            "dialed peer id must equal the pinned pool member's derived identity"
        );
    }
    let out_dir = out_root.join(cell);
    std::fs::create_dir_all(&out_dir).expect("create cell dir");

    let profile = profile_id();
    let mut ledger = AgreementLedger::new();
    let mut records: Vec<DivergenceCheckRecord> = Vec::new();
    let mut checks = 0usize;

    for prompt in PASS1_CORPUS {
        for &seed in &SEEDS {
            let started = Instant::now();
            let request_ids: Vec<String> = (0..bridges.len())
                .map(|index| format!("acc-{cell}-{}-s{seed:08x}-b{index}", prompt.id))
                .collect();
            let requests: Vec<WireRequest> = bridges
                .iter()
                .zip(&request_ids)
                .map(|(bridge, request_id)| WireRequest {
                    executor: &bridge.executor,
                    profile_id: &profile,
                    token: &bridge.token,
                    request_id,
                    prompt: prompt.text,
                    committed: "",
                    max_tokens: MAX_TOKENS,
                    seed,
                })
                .collect();
            let mut futures = Vec::with_capacity(requests.len());
            for request in &requests {
                futures.push(execute_on(request));
            }
            let completions = join_all(futures).await;
            let check_wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut streams = Vec::with_capacity(completions.len());
            for (index, completion) in completions.iter().enumerate() {
                let completion = completion.as_ref().unwrap_or_else(|e| {
                    panic!(
                        "check {cell}/{} s{seed:08x} bridge {index} failed: {e}",
                        prompt.id
                    )
                });
                assert_eq!(
                    completion.deltas.len(),
                    MAX_TOKENS as usize,
                    "full budget decoded"
                );
                streams.push(completion.deltas.clone());
            }
            checks += 1;
            let reference = &completions[0].as_ref().expect("reference completed");
            let reference_digest = stream_digest(&reference.deltas);

            // Verifier-reference comparisons (the ADR-032 §3 audit shape:
            // bridge 0 is the cohort verifier; every other peer's
            // re-execution must match its continuation).
            for index in 1..bridges.len() {
                let probe = completions[index].as_ref().expect("probe completed");
                let position = first_divergence(&reference.deltas, &probe.deltas);
                let verdict = if position.is_none() {
                    CheckVerdict::Agree
                } else {
                    CheckVerdict::Diverge
                };
                ledger.record(&bridges[index].peer_id, position.is_none());
                records.push(DivergenceCheckRecord {
                    record_type: DIVERGENCE_CHECK_RECORD_TYPE.to_string(),
                    check_id: format!("{cell}/{}-s{seed:08x}", prompt.id),
                    cell: cell.to_string(),
                    environment: environment.to_string(),
                    harness_version: HARNESS_VERSION.to_string(),
                    corpus_id: PASS1_CORPUS_ID.to_string(),
                    prompt_id: prompt.id.to_string(),
                    seed,
                    reference_kind: format!("peer:{}", bridges[0].peer_id),
                    probe_peer: bridges[index].peer_id.clone(),
                    verdict,
                    divergence_position: position.map(|p| u32::try_from(p).expect("position")),
                    checked_tokens: MAX_TOKENS,
                    reference_stream_sha256_16: reference_digest.clone(),
                    probe_stream_sha256_16: stream_digest(&probe.deltas),
                    reference_total_ms: reference.total_ms,
                    probe_total_ms: probe.total_ms,
                    raced_min_total_ms: reference.total_ms.min(probe.total_ms),
                    check_wall_ms,
                });
            }

            // Quorum read (reference-free): every pool member vs the
            // per-position strict majority.
            let outcome = quorum(&streams);
            for index in 0..bridges.len() {
                let probe = completions[index].as_ref().expect("probe completed");
                let quorum_divergence = outcome
                    .divergent
                    .iter()
                    .find(|(i, _)| *i == index)
                    .map(|(_, position)| *position);
                records.push(DivergenceCheckRecord {
                    record_type: DIVERGENCE_CHECK_RECORD_TYPE.to_string(),
                    check_id: format!("{cell}/{}-s{seed:08x}", prompt.id),
                    cell: cell.to_string(),
                    environment: environment.to_string(),
                    harness_version: HARNESS_VERSION.to_string(),
                    corpus_id: PASS1_CORPUS_ID.to_string(),
                    prompt_id: prompt.id.to_string(),
                    seed,
                    reference_kind: "quorum-majority".to_string(),
                    probe_peer: bridges[index].peer_id.clone(),
                    verdict: if quorum_divergence.is_none() {
                        CheckVerdict::Agree
                    } else {
                        CheckVerdict::Diverge
                    },
                    divergence_position: quorum_divergence
                        .map(|p| u32::try_from(p).expect("position")),
                    checked_tokens: MAX_TOKENS,
                    reference_stream_sha256_16: stream_digest(&outcome.reference),
                    probe_stream_sha256_16: stream_digest(&probe.deltas),
                    reference_total_ms: reference.total_ms,
                    probe_total_ms: probe.total_ms,
                    raced_min_total_ms: reference.total_ms.min(probe.total_ms),
                    check_wall_ms,
                });
            }
        }
    }

    // De-rank demonstration: the pinned pool as scheduler candidates,
    // selected before and after the ledger's suspensions. Ranking uses
    // the pool's synthetic rates + advertised queues with a FLAT RTT
    // placeholder — this demonstrates roster composition, not latency.
    let parsed_profile = ModelProfileId::new(DEMO_PROFILE).expect("demo profile parses");
    let candidates: Vec<Candidate> = pool
        .iter()
        .zip(bridges)
        .map(|(config, bridge)| Candidate {
            peer_id: bridge.peer_id.clone(),
            profile_id: parsed_profile.clone(),
            measured_rtt_ms: DEMO_RTT_MS,
            advertised_queue_ms: config.advertised_queue_ms as f64,
            prefill_tokens_per_ms: config.prefill_tokens_per_ms,
            decode_tokens_per_ms: config.decode_tokens_per_ms,
            failure_penalty_ms: 0.0,
            stale_advertisement_penalty_ms: 0.0,
            slots: 1,
            capacity_class: match config.capacity_class {
                "gpu_high" => CapacityClass::GpuHigh,
                "gpu_low" => CapacityClass::GpuEntry,
                "cpu" => CapacityClass::Cpu,
                _ => CapacityClass::GpuMid,
            },
            nat_path: NatPath::Direct,
            batch_verify: false,
        })
        .collect();
    let before: Vec<String> = select_microswarm(
        &candidates,
        &parsed_profile,
        DEMO_PROMPT_TOKENS,
        MAX_TOKENS,
        COHORT_WANT,
    )
    .iter()
    .map(|c| c.peer_id.clone())
    .collect();
    let remaining: Vec<Candidate> = candidates
        .iter()
        .filter(|c| !ledger.suspended(&c.peer_id))
        .cloned()
        .collect();
    let after: Vec<String> = select_microswarm(
        &remaining,
        &parsed_profile,
        DEMO_PROMPT_TOKENS,
        MAX_TOKENS,
        COHORT_WANT,
    )
    .iter()
    .map(|c| c.peer_id.clone())
    .collect();
    let suspended: Vec<String> = candidates
        .iter()
        .filter(|c| ledger.suspended(&c.peer_id))
        .map(|c| c.peer_id.clone())
        .collect();
    let derank = CohortDerankRecord {
        record_type: COHORT_DERANK_RECORD_TYPE.to_string(),
        cell: cell.to_string(),
        environment: environment.to_string(),
        suspended_peers: suspended,
        cohort_before: before,
        cohort_after: after,
        selection: format!(
            "select_microswarm(want={COHORT_WANT}) over pool-spec candidates \
             (flat rtt {DEMO_RTT_MS}ms placeholder; ranking demo)"
        ),
    };

    // Artifacts: manifest + rows + derank + ledger snapshot.
    let manifest = AccuracyManifest {
        record_type: ACCURACY_MANIFEST_RECORD_TYPE.to_string(),
        cell: cell.to_string(),
        environment: environment.to_string(),
        harness_version: HARNESS_VERSION.to_string(),
        executor: SYNTHETIC_EXECUTOR_NAME.to_string(),
        pool: pool
            .iter()
            .enumerate()
            .map(|(index, config)| AccuracyPoolPeer {
                index,
                seed: config.seed,
                decode_tokens_per_ms: config.decode_tokens_per_ms,
                prefill_tokens_per_ms: config.prefill_tokens_per_ms,
                advertised_queue_ms: config.advertised_queue_ms,
                capacity_class: config.capacity_class.to_string(),
                divergent: config.divergent,
            })
            .collect(),
        corpus_id: PASS1_CORPUS_ID.to_string(),
        prompt_ids: PASS1_CORPUS.iter().map(|p| p.id.to_string()).collect(),
        seeds: SEEDS.to_vec(),
        max_tokens: MAX_TOKENS,
        reference_policy: "bridge-0 verifier-reference (peer comparisons feed the ledger) + \
                           quorum-majority read; suspend after first divergence"
            .to_string(),
        notes: ops_note.to_string(),
    };
    std::fs::write(
        out_dir.join("accuracy-manifest.json"),
        serde_json::to_string_pretty(&manifest).expect("manifest serializes"),
    )
    .expect("write manifest");
    let mut jsonl = String::new();
    for record in &records {
        jsonl.push_str(&serde_json::to_string(record).expect("row serializes"));
        jsonl.push('\n');
    }
    std::fs::write(out_dir.join("divergence-checks.jsonl"), jsonl).expect("write rows");
    std::fs::write(
        out_dir.join("cohort-derank.json"),
        serde_json::to_string_pretty(&derank).expect("derank serializes"),
    )
    .expect("write derank");
    let ledger_snapshot: Vec<(String, AgreementState)> = ledger.snapshot();
    std::fs::write(
        out_dir.join("agreement-ledger.json"),
        serde_json::to_string_pretty(&ledger_snapshot).expect("ledger serializes"),
    )
    .expect("write ledger");

    CellOutcome {
        records,
        derank,
        ledger: ledger_snapshot,
        checks,
    }
}

/// Spawns the pinned accuracy pool on loopback and returns the bridges.
async fn spawn_loopback_pool(pool: &[BridgeConfig]) -> Vec<LiveBridge> {
    let telemetry = Arc::new(Telemetry::memory().0);
    let metrics = Arc::new(PeerMetrics::memory(Arc::clone(&telemetry)));
    let identity = harness_identity();
    let mut bridges = Vec::with_capacity(pool.len());
    for config in pool {
        bridges.push(
            spawn_bridge(
                &profile_id(),
                &identity,
                config,
                std::time::Duration::ZERO,
                Arc::clone(&telemetry),
                Arc::clone(&metrics),
            )
            .await
            .expect("loopback bridge spawns"),
        );
    }
    bridges
}

/// The per-push CI proof: structural divergence detection over real QUIC
/// with the fault model injected at pool index 2 — detect every time,
/// flag no honest peer, suspend + de-rank exactly the offender.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn divergence_detection_over_real_quic() {
    let out = tempfile::tempdir().unwrap();
    let pool = accuracy_pool(&[2]);
    let bridges = spawn_loopback_pool(&pool).await;
    let outcome = run_accuracy_cell(
        "injected",
        ENV_LABEL_ACCURACY_LOOPBACK,
        &pool,
        &bridges,
        out.path(),
        "CI loopback smoke",
    )
    .await;

    assert_eq!(outcome.checks, 8, "4 prompts x 2 seeds");
    // 8 checks x (3 peer-reference + 4 quorum) rows.
    assert_eq!(outcome.records.len(), 56);

    // Zero false negatives: the injected peer diverges at position 0 in
    // EVERY peer-reference check (the synthetic fault model disagrees
    // from the first token).
    let faulty = &bridges[2].peer_id;
    let peer_rows: Vec<_> = outcome
        .records
        .iter()
        .filter(|r| r.reference_kind.starts_with("peer:"))
        .collect();
    let faulty_rows: Vec<_> = peer_rows
        .iter()
        .filter(|r| r.probe_peer == *faulty)
        .collect();
    assert_eq!(faulty_rows.len(), 8);
    for row in &faulty_rows {
        assert_eq!(row.verdict, CheckVerdict::Diverge);
        assert_eq!(row.divergence_position, Some(0), "diverges at token 0");
    }
    // Zero false positives: the honest probes agree everywhere.
    for row in &peer_rows {
        if row.probe_peer != *faulty {
            assert_eq!(
                row.verdict,
                CheckVerdict::Agree,
                "honest peer flagged: {row:?}"
            );
            assert_eq!(row.divergence_position, None);
        }
    }
    // Quorum agrees with the reference read, reference-free.
    let quorum_rows: Vec<_> = outcome
        .records
        .iter()
        .filter(|r| r.reference_kind == "quorum-majority")
        .collect();
    assert_eq!(quorum_rows.len(), 32);
    for row in &quorum_rows {
        if row.probe_peer == *faulty {
            assert_eq!(
                row.verdict,
                CheckVerdict::Diverge,
                "quorum flags the faulty peer"
            );
            assert_eq!(row.divergence_position, Some(0));
        } else {
            assert_eq!(
                row.verdict,
                CheckVerdict::Agree,
                "quorum flags an honest peer"
            );
        }
    }
    // Ledger: exactly the faulty peer suspended, agreement EWMA alive.
    assert_eq!(outcome.suspended_peers(), vec![faulty.clone()]);
    let faulty_state = outcome
        .ledger
        .iter()
        .find(|(peer, _)| peer == faulty)
        .expect("faulty peer recorded");
    assert_eq!(faulty_state.1.checks, 8);
    assert_eq!(faulty_state.1.divergences, 8);
    assert!(faulty_state.1.agreement_ewma < 0.01, "EWMA collapsed");
    for (peer, state) in &outcome.ledger {
        if peer != faulty {
            assert_eq!(state.divergences, 0);
            assert!((state.agreement_ewma - 1.0).abs() < 1e-9);
        }
    }
    // De-rank: the faulty peer sat at rank 3 (V, D, M, M2 by prediction);
    // detection removes it and promotes M2.
    assert_eq!(
        outcome.derank.cohort_before,
        vec![
            bridges[0].peer_id.clone(),
            bridges[1].peer_id.clone(),
            bridges[2].peer_id.clone(),
        ],
        "without detection the faulty peer is IN the cohort"
    );
    assert_eq!(
        outcome.derank.cohort_after,
        vec![
            bridges[0].peer_id.clone(),
            bridges[1].peer_id.clone(),
            bridges[3].peer_id.clone(),
        ],
        "with detection the faulty peer is OUT (M2 promoted)"
    );
    assert_eq!(outcome.derank.suspended_peers, vec![faulty.clone()]);

    // Artifacts on disk; rows never carry token text (digests are hex).
    let cell_dir = out.path().join("injected");
    for name in [
        "accuracy-manifest.json",
        "divergence-checks.jsonl",
        "cohort-derank.json",
        "agreement-ledger.json",
    ] {
        assert!(cell_dir.join(name).exists(), "missing artifact {name}");
    }
    let rows = std::fs::read_to_string(cell_dir.join("divergence-checks.jsonl")).unwrap();
    for line in rows.lines() {
        let json: serde_json::Value = serde_json::from_str(line).expect("row parses");
        for field in ["reference_stream_sha256_16", "probe_stream_sha256_16"] {
            let digest = json[field].as_str().expect("digest field");
            assert!(
                digest.len() == 16 && digest.chars().all(|c| c.is_ascii_hexdigit()),
                "digest must be 16-hex, got {digest}"
            );
        }
    }
}

/// Owner-invoked committed-artifacts loopback run (clean control cell +
/// injected cell, real QUIC, synthetic executors — STRUCTURAL evidence
/// only). Env: `MSP_BENCH_OUT` (default target/accuracy-loopback).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "owner-invoked accuracy run (writes committed experiment artifacts)"]
async fn accuracy_divergence_loopback() {
    let out = std::env::var("MSP_BENCH_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/accuracy-loopback")
        });
    std::fs::create_dir_all(&out).expect("create out dir");

    let clean_pool = accuracy_pool(&[]);
    let bridges = spawn_loopback_pool(&clean_pool).await;
    let clean = run_accuracy_cell(
        "clean",
        ENV_LABEL_ACCURACY_LOOPBACK,
        &clean_pool,
        &bridges,
        &out,
        "owner loopback run: clean control cell",
    )
    .await;
    drop(bridges); // loopback bridges stop with their handles

    // Control: zero divergence anywhere, nobody suspended, cohorts equal.
    assert_eq!(clean.suspended_peers(), Vec::<String>::new());
    for row in &clean.records {
        assert_eq!(
            row.verdict,
            CheckVerdict::Agree,
            "control cell flagged: {row:?}"
        );
    }
    assert_eq!(clean.derank.cohort_before, clean.derank.cohort_after);

    let injected_pool = accuracy_pool(&[2]);
    let bridges = spawn_loopback_pool(&injected_pool).await;
    let injected = run_accuracy_cell(
        "injected",
        ENV_LABEL_ACCURACY_LOOPBACK,
        &injected_pool,
        &bridges,
        &out,
        "owner loopback run: divergence injected at pool index 2",
    )
    .await;
    assert_eq!(
        injected.suspended_peers(),
        vec![bridges[2].peer_id.clone()],
        "the injected peer is detected and suspended"
    );
    eprintln!(
        "[ACCURACY-LOOPBACK] clean: {} checks, {} rows; injected: suspended {:?}",
        clean.checks,
        clean.records.len(),
        injected.suspended_peers()
    );
}

/// Owner-invoked two-machine LAN run. Serve side (machine B):
/// `pass1_serve --pool accuracy --bridges 4 [--divergent 2] --bind-ip
/// 0.0.0.0 --profile <msp1:64hex> --client-peer <driver peer>`. Driver
/// env: `MSP_BENCH_PEER_0..3` (B's lines), `MSP_BENCH_CELL`
/// (`clean` | `injected`), `MSP_BENCH_EXPECT_DIVERGENT` (comma-separated
/// pool indexes the serve side injected — the ground truth the driver
/// self-checks against; empty for clean), `MSP_BENCH_OUT` (absolute,
/// the experiment root). Labels: lan-2machine-quic — never WAN claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "owner-invoked LAN accuracy run (needs pass1_serve --pool accuracy on machine B)"]
async fn accuracy_divergence_lan() {
    let mut remotes = Vec::new();
    for i in 0..ACCURACY_POOL_SIZE {
        let line = std::env::var(format!("MSP_BENCH_PEER_{i}"))
            .unwrap_or_else(|_| panic!("MSP_BENCH_PEER_{i} is required"));
        remotes.push(
            modelswarm_bench::runner::parse_peer_line(&line).expect("MSP_BENCH_PEER line parses"),
        );
    }
    let cell = std::env::var("MSP_BENCH_CELL").unwrap_or_else(|_| "clean".to_string());
    let expected: Vec<usize> = std::env::var("MSP_BENCH_EXPECT_DIVERGENT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| {
            v.split(',')
                .map(|i| i.trim().parse().expect("MSP_BENCH_EXPECT_DIVERGENT parses"))
                .collect()
        })
        .unwrap_or_default();
    let out = std::env::var("MSP_BENCH_OUT")
        .map(PathBuf::from)
        .expect("MSP_BENCH_OUT (absolute path) is required for the LAN accuracy run");

    let pool = accuracy_pool(&expected);
    let telemetry = Arc::new(Telemetry::memory().0);
    let metrics = Arc::new(PeerMetrics::memory(Arc::clone(&telemetry)));
    let identity = harness_identity();
    let mut bridges = Vec::with_capacity(remotes.len());
    for remote in &remotes {
        bridges.push(
            modelswarm_bench::runner::connect_remote(remote, &identity, Arc::clone(&metrics))
                .await
                .expect("connect remote bridge"),
        );
    }
    let ops_note = format!(
        "LAN two-machine run, cell {cell}, injected indexes {expected:?}; \
         deterministic request ids mean the serve task must be fresh per cell"
    );
    let outcome = run_accuracy_cell(
        &cell,
        ENV_LABEL_ACCURACY_LAN,
        &pool,
        &bridges,
        &out,
        &ops_note,
    )
    .await;

    // Self-check against the serve-side ground truth: the detected set
    // must equal the injected set (peer ids are deterministic).
    let mut detected_expected: Vec<String> = expected
        .iter()
        .map(|&i| bridges[i].peer_id.clone())
        .collect();
    detected_expected.sort();
    assert_eq!(
        outcome.suspended_peers(),
        detected_expected,
        "detection must equal the injected ground truth"
    );
    eprintln!(
        "[ACCURACY-LAN] cell {cell}: {} checks, {} rows, suspended {:?}",
        outcome.checks,
        outcome.records.len(),
        outcome.suspended_peers()
    );
}
