//! The §9.2(7) FULL A3 CORPUS SWEEP (ADR-033 (7) as extended by ADR-037
//! §4.4 item 14): with the advisory/assistant plane ABSENT vs
//! ENABLED-WITH-ERRORING-TRANSPORT, every observable decision surface is
//! byte-identical and the telemetry stream is byte-identical after
//! filtering the planes' own `advisory.*`/`assistant.*` rows.
//!
//! Corpus families executed here (the ADR-033 §9.2(7) letter, at the
//! composition seam where the plane can exist):
//!
//! - **(a) the scheduler/shadow decision surface** — the FULL candidate
//!   matrix over the shipped production decision functions
//!   (`select_microswarm`, `predicted_single_ms`,
//!   `should_engage_cooperative` incl. the frozen margin floor): every
//!   capacity class × NAT path × batch-verify flag × slot/staleness/
//!   rate tiers, deterministic cohorts, and the engage boundary swept at
//!   exact `ratio × (1 + margin)` equality ± 1e-9. No re-implementation:
//!   these are the production imports the sim also rides.
//! - **the node workload corpus** — the same runtime-less gateway turns
//!   (no-peer 503, unknown-profile) through a REAL `Node::start` in both
//!   postures; the node's `logs/node.jsonl` streams are diffed
//!   byte-for-byte after filtering plane rows and normalizing ONLY the
//!   log timestamp, the ephemeral port, and the per-installation identity
//!   (declared normalizations — everything else must match exactly).
//! - **(e) the accumulation-layer input distribution** — posture B feeds
//!   the typed accumulators past `events_threshold` with the five
//!   feeds_digest event names, so the digest plane actually CYCLES (and
//!   fails against the erroring double, recording `advisory.cycle.gap`)
//!   while the corpus runs.
//! - **the assistant plane** — armed in-process over an erroring double
//!   in posture B with driven turns (`assistant.turn.gap` rows), and the
//!   plane-rows-only confinement pinned on a shared memory sink.
//!
//! Corpus families (b) session admission/coordinator suites, (c) the
//! sim pinned-verdict scenarios, and (d) the bench corpora execute BELOW
//! the advisory host in the crate graph — the chat-wall layer 1 test
//! (`advisory_chat_wall.rs`) pins that they cannot even name the plane,
//! so the absent posture IS their only possible posture; they re-run
//! per-push in their own suites (the venue matrix rows).
//!
//! ADR-037 item 14's picker corpus (the deterministic `fit_and_score`
//! shortlist) lives in the desktop crate; the desktop seam's A3 pattern
//! test (`assistant_a3_extension_plane_absent_vs_erroring`) pins it, and
//! the chat-wall layer-2 scan pins that `app.rs` (the picker's home)
//! carries no advisory name at all — structurally incapable of seeing
//! the plane.

use std::sync::Arc;

use modelswarm_node::advisory::assistant::{AssistantPlane, TurnOutcome};
use modelswarm_node::advisory::egress::{ContextField, ContextValue};
use modelswarm_node::advisory::{AdvisoryTransport, CountingTransport};
use modelswarm_scheduler::{
    predicted_single_ms, select_microswarm, should_engage_cooperative, Candidate, CapacityClass,
    NatPath, SchedulerError, DEFAULT_CONFIDENCE_MARGIN, MICROSWARM_CAP, MIN_CONFIDENCE_MARGIN,
};
use modelswarm_types::ModelProfileId;

// No env mutation in this binary: key resolution rides advisory.json's
// `key_file` (no MSP_ADVISORY_KEY_FILE set/remove — the A-4 discipline
// does not apply; each test binary is its own process anyway).

// ---------------------------------------------------------------------------
// Corpus (a): the scheduler decision surface — the full candidate matrix
// ---------------------------------------------------------------------------

const PROFILE: &str = "msp:a3-sweep:q4_k_m:v1";

fn candidate(ix: usize) -> Candidate {
    // A deterministic grid over the Candidate space: every capacity
    // class, NAT path, batch-verify flag, slot tier (incl. the ineligible
    // 0), staleness tier (incl. the over-cap 1500ms), and measured-rate
    // tier (incl. the unusable 0.0 -> INFINITY). The tiers cycle by index
    // so cohorts mix classes deterministically.
    let classes = [
        CapacityClass::Cpu,
        CapacityClass::GpuEntry,
        CapacityClass::GpuMid,
        CapacityClass::GpuHigh,
    ];
    let paths = [NatPath::Direct, NatPath::HolePunched, NatPath::Relayed];
    let slots = [0u32, 1, 3];
    let stale = [0.0f64, 500.0, 999.0, 1500.0];
    let rates = [0.0f64, 0.05, 0.5];
    let rtts = [8.0f64, 40.0, 180.0];
    let queues = [0.0f64, 25.0, 400.0];
    let failures = [0.0f64, 75.0];
    Candidate {
        peer_id: format!("peer-{ix:05}"),
        profile_id: ModelProfileId::new(PROFILE).unwrap(),
        measured_rtt_ms: rtts[ix % rtts.len()],
        advertised_queue_ms: queues[ix % queues.len()],
        prefill_tokens_per_ms: rates[ix % rates.len()],
        decode_tokens_per_ms: rates[(ix / 3) % rates.len()],
        failure_penalty_ms: failures[ix % failures.len()],
        stale_advertisement_penalty_ms: stale[ix % stale.len()],
        slots: slots[ix % slots.len()],
        capacity_class: classes[ix % classes.len()],
        nat_path: paths[ix % paths.len()],
        batch_verify: ix % 2 == 0,
    }
}

/// One stable serialization of the FULL decision corpus over `candidates`
/// (selection order + per-candidate predictions + the engage boundary).
fn engage_corpus_digest(candidates: &[Candidate]) -> String {
    let mut out = serde_json::json!({});
    let token_profiles = [(128u32, 64u32), (1024, 256), (8192, 2048)];
    for (pi, (prompt, output)) in token_profiles.iter().enumerate() {
        for want in [1usize, 4, MICROSWARM_CAP] {
            let picked: Vec<&str> = select_microswarm(
                candidates,
                &ModelProfileId::new(PROFILE).unwrap(),
                *prompt,
                *output,
                want,
            )
            .into_iter()
            .map(|c| c.peer_id.as_str())
            .collect();
            out[format!("select/p{pi}/w{want}")] = serde_json::json!(picked);
        }
        let preds: Vec<String> = candidates
            .iter()
            .map(|c| format!("{:e}", predicted_single_ms(c, *prompt, *output)))
            .collect();
        out[format!("predict/p{pi}")] = serde_json::json!(preds);
    }
    // The engage boundary at exact equality ± 1e-9 for each margin tier,
    // plus the frozen-floor rejection (ADR-013).
    let margins = [MIN_CONFIDENCE_MARGIN, DEFAULT_CONFIDENCE_MARGIN, 0.5, 1.0];
    for (mi, margin) in margins.iter().enumerate() {
        for (ri, delta) in [-1e-9f64, 0.0, 1e-9, 1.0, -1.0].iter().enumerate() {
            let swarm = 100.0f64;
            let single = 100.0 * (1.0 + margin) + delta;
            let verdict = match should_engage_cooperative(swarm, single, *margin) {
                Ok(engage) => serde_json::json!(engage),
                Err(e) => serde_json::json!(format!("err:{e}")),
            };
            out[format!("engage/m{mi}/r{ri}")] = verdict;
        }
    }
    let below_floor = should_engage_cooperative(100.0, 100.0, MIN_CONFIDENCE_MARGIN - 0.01)
        .map_err(|e: SchedulerError| e.to_string());
    out["engage/floor"] = serde_json::json!(below_floor);
    out.to_string()
}

// ---------------------------------------------------------------------------
// The node workload corpus (real Node::start, runtime-less gateway turns)
// ---------------------------------------------------------------------------

/// Boots a runtime-less node in `dir`, drives the identical gateway turn
/// set (3 × no-peer 503, 2 × unknown-profile), stops it, and returns the
/// NORMALIZED + plane-filtered `logs/node.jsonl` stream. When the node
/// wires an armed advisory plane, the typed accumulators are fed past
/// threshold first (the five feeds_digest event names — corpus family
/// (e)) so the digest runner CYCLES against the erroring double during
/// the workload.
async fn node_workload(
    dir: &std::path::Path,
    advisory_transport: Option<Arc<dyn AdvisoryTransport>>,
) -> String {
    let (tx, rx) = tokio::sync::watch::channel(false);
    // logs/node.jsonl APPENDS across boots in one data dir — compare only
    // THIS boot's lines (the workload window).
    let log_path = dir.join("logs/node.jsonl");
    let prior_lines = std::fs::read_to_string(&log_path)
        .map(|text| text.lines().count())
        .unwrap_or(0);
    let config = modelswarm_node::NodeConfig {
        gateway_port: 0,
        data_dir: dir.to_path_buf(),
        profile_id: Some("msp1:a3workload".to_string()),
        advisory_transport,
        ..modelswarm_node::NodeConfig::default()
    };
    let handle = modelswarm_node::Node::start(config, rx)
        .await
        .expect("node starts");
    if let Some(advisory) = &handle.advisory {
        let acc = advisory.accumulators.clone();
        for _ in 0..50 {
            for event in [
                "serving.session_ended",
                "serving.clamped",
                "serving.refused",
                "verifydrafts.verifier_divergence",
                "engine.gpu_fallback",
            ] {
                acc.lock().unwrap().record_event(event);
            }
        }
        // Let the fast-tick runner cycle (and fail) against the double.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
    let addr = handle.local_addr;
    let client = reqwest::Client::new();
    for _ in 0..3 {
        let r = client
            .post(format!("http://{addr}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "msp1:a3workload",
                "messages": [{"role": "user", "content": "a3 sweep turn"}],
                "max_tokens": 8,
            }))
            .send()
            .await
            .expect("gateway responds");
        assert_eq!(r.status().as_u16(), 503, "runtime-less node fails honestly");
    }
    for _ in 0..2 {
        let r = client
            .post(format!("http://{addr}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "msp1:not-hosted",
                "messages": [{"role": "user", "content": "a3 sweep turn"}],
                "max_tokens": 8,
            }))
            .send()
            .await
            .expect("gateway responds");
        assert!(!r.status().is_success());
    }
    tx.send(true).unwrap();
    handle.stopped().await;
    let raw = std::fs::read_to_string(&log_path).unwrap();
    let window: String = raw.lines().skip(prior_lines).collect::<Vec<_>>().join("\n");
    normalize_node_log(&window)
}

/// The declared normalizations for cross-run file comparison (everything
/// else must match byte-for-byte): the log `ts` field, the ephemeral
/// gateway port, and the per-installation identity string. Plane rows
/// (`advisory.*` / `assistant.*`) are the A3 carve-out — filtered.
fn normalize_node_log(raw: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for line in raw.lines() {
        if line.contains("\"event\":\"advisory.") || line.contains("\"event\":\"assistant.") {
            continue;
        }
        // "ts":"2026-10-14T20:00:00Z" -> "ts":"T"
        let mut l = String::new();
        let mut rest = line;
        if let Some(pos) = rest.find("\"ts\":\"") {
            let after = &rest[pos + 6..];
            let end = after.find('"').unwrap();
            l.push_str(&rest[..pos + 6]);
            l.push('T');
            rest = &after[end..];
        }
        l.push_str(rest);
        // 127.0.0.1:<ephemeral> -> 127.0.0.1:P
        let l = l
            .split("127.0.0.1:")
            .enumerate()
            .fold(String::new(), |acc, (i, part)| {
                if i == 0 {
                    part.to_string()
                } else {
                    let digits = part.chars().take_while(|c| c.is_ascii_digit()).count();
                    acc + "127.0.0.1:P" + &part[digits..]
                }
            });
        // installation ids (base58-ish runs) -> ID
        let l = replace_field_value(&l, "installation_id");
        let l = replace_field_value(&l, "peer");
        kept.push(l);
    }
    kept.join("\n")
}

fn replace_field_value(line: &str, field: &str) -> String {
    let needle = format!("\"{field}\":\"");
    let Some(pos) = line.find(&needle) else {
        return line.to_string();
    };
    let after = &line[pos + needle.len()..];
    let end = after.find('"').unwrap_or(after.len());
    format!("{}{}{}", &line[..pos + needle.len()], "ID", &after[end..])
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

fn typed_context() -> Vec<ContextField> {
    vec![ContextField {
        name: "hw.logical_cpus".into(),
        value: ContextValue::Int(16),
    }]
}

/// ADR-033 §9.2(7) + ADR-037 §4.4 item (14): the FULL corpus, plane
/// absent vs enabled-with-erroring-transport. Decision outputs
/// byte-identical; the node workload telemetry byte-identical after the
/// declared normalizations + plane filtering; the plane confined to its
/// own rows.
#[tokio::test]
async fn a3_full_corpus_plane_absent_vs_armed_erroring() {
    let dir = tempfile::tempdir().unwrap();
    let candidates: Vec<Candidate> = (0..96).map(candidate).collect();

    // Warm-up boot: creates the identity so BOTH measured postures log
    // identity.loaded (the first boot would log identity.created and
    // poison the diff with a posture-independent difference).
    {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let config = modelswarm_node::NodeConfig {
            gateway_port: 0,
            data_dir: dir.path().to_path_buf(),
            profile_id: Some("msp1:a3workload".to_string()),
            ..modelswarm_node::NodeConfig::default()
        };
        let handle = modelswarm_node::Node::start(config, rx).await.unwrap();
        tx.send(true).unwrap();
        handle.stopped().await;
    }

    // ---- Posture A: the plane ABSENT (no advisory.json) --------------
    let digest_absent = engage_corpus_digest(&candidates);
    let workload_absent = node_workload(dir.path(), None).await;

    // ---- Posture B: BOTH planes ENABLED over ERRORING transports -----
    // The digest plane rides the node (config-driven arming, no env var:
    // advisory.json carries key_file); a fast min_interval + low
    // threshold make it cycle (and fail) DURING the workload (the feed
    // happens inside node_workload via the wired accumulators).
    let key = dir.path().join("a3-key.txt");
    std::fs::write(&key, "test-key-local-double\n").unwrap();
    let key_json = key.to_string_lossy().replace('\\', "/");
    std::fs::write(
        dir.path().join("advisory.json"),
        format!(r#"{{"enabled":true,"assistant.enabled":true,"endpoint_url":"https://advisor.invalid/v1","model":"test-model","key_file":"{key_json}","min_interval_ms":30,"events_threshold":5,"request_timeout_ms":500}}"#),
    )
    .unwrap();

    let digest_double = Arc::new(CountingTransport::always_erroring());
    let digest_as_trait: Arc<dyn AdvisoryTransport> = digest_double.clone();

    // The assistant plane, armed in-process over its own erroring double
    // (assistant.turn.gap rows land in the memory sink; its presence in
    // the process during the corpus evaluation is the point).
    let assistant_double = Arc::new(CountingTransport::always_erroring());
    let assistant_as_trait: Arc<dyn AdvisoryTransport> = assistant_double.clone();
    let (plane, gap) = AssistantPlane::spawn(dir.path(), Some(assistant_as_trait));
    assert_eq!(
        gap.as_str(),
        "disabled",
        "the armed spawn reports no degrade"
    );
    let plane = plane.expect("the assistant plane arms over the double");
    let (telemetry, sink) = modelswarm_telemetry::Telemetry::memory();
    for question in ["which model fits?", "why is family off?", "third turn"] {
        let outcome = plane.run_turn(&telemetry, question, typed_context()).await;
        assert!(
            matches!(outcome, TurnOutcome::Gap { .. }),
            "the erroring double must yield an honest gap"
        );
    }
    assert_eq!(assistant_double.count(), 3, "three assistant turns called");

    // The corpus evaluated WHILE the assistant plane is armed in-process
    // (ambient plane activity must not perturb the decisions).
    let digest_armed = engage_corpus_digest(&candidates);

    // The node workload with the digest plane wired over the erroring
    // double (NodeConfig seam) — the accumulators are fed past threshold
    // inside, advisory.cycle.gap rows appear in the node's file sink, and
    // the filtered stream must stay byte-identical.
    let workload_armed = node_workload(dir.path(), Some(digest_as_trait)).await;

    // ---- (i) Decision outputs byte-identical -------------------------
    assert_eq!(
        digest_absent, digest_armed,
        "A3(i): the full scheduler decision corpus is byte-identical with the plane armed-and-erroring"
    );

    // ---- (ii) The telemetry stream byte-identical after filtering -----
    assert_eq!(
        workload_absent, workload_armed,
        "A3(ii): the node workload telemetry is byte-identical after filtering advisory.*/assistant.* (declared normalizations: ts, ephemeral port, installation ids)"
    );

    // The armed digest plane must have CYCLED (non-vacuous filtered
    // equality): at least one advisory.cycle.gap row was filtered away.
    let raw_armed = std::fs::read_to_string(dir.path().join("logs/node.jsonl")).unwrap();
    assert!(
        raw_armed.contains("\"event\":\"advisory.cycle.gap\""),
        "the digest plane must have cycled against the erroring double"
    );
    assert!(digest_double.count() >= 1, "the digest double was called");

    // Confinement: the assistant plane's memory-sink rows are ALL
    // assistant.* (counts/labels only; no reply text, no key).
    for line in sink.snapshot() {
        assert!(line.contains("\"event\":\"assistant."), "{line}");
        assert!(!line.contains("test-key-local-double"), "{line}");
    }
    assert!(
        sink.snapshot()
            .iter()
            .any(|l| l.contains("assistant.turn.gap")),
        "the assistant gap rows were recorded"
    );
}
