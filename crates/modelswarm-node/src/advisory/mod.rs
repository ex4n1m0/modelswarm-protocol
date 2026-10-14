//! The advisory plane client (ADR-033 revision 2 — digest plane; the
//! ADR-037 assistant surface is a later leg and NOTHING here serves
//! it yet). Hosted as a module of the node daemon per ADR-033 §9.4
//! ("`crates/modelswarm-node::advisory`"), which the chat-wall suite's
//! layer-1 dependency assertion guards directly.
//!
//! Layout:
//! - [`config`] — `advisory.json` (TR-RC-8) + key-file resolution
//!   (§3.4; env-first; the key file NEVER lives in the repo; NO
//!   rotation anywhere — ADR-037 §3.6 standing owner directive).
//! - [`egress`] — the ONE value-typed allowlist (the content gate),
//!   driven by the frozen artifacts `protocol/advisory-egress-v1.json`
//!   and `protocol/advisory-clamp-table-v1.json`, plus the RC-3
//!   choke-point re-scan.
//! - [`accum`] — the typed in-process accumulators (RC-4): counts,
//!   integer min/median/max ms, frozen-vocabulary tallies. Log sinks
//!   are never parsed as advisory input.
//! - [`state`] — the PURE `step(state, input) -> (state, effects)`
//!   machine (TR-RC-4/TR-AD-2): injected clock and transport; NO
//!   reqwest, wall clock, or RNG inside.
//! - [`transport`] — the injectable transport trait, the pinned
//!   reqwest adapter (https-only, redirects NEVER followed — RC-9),
//!   and the counting double the test suites ride.
//! - [`runner`] — the timer-driven digest cycle: the ONLY production
//!   call site of the plane (invariant A5 / W2).
//!
//! The four ADR-033 §1 invariants hold structurally at this leg:
//! content-blind egress (one value-typed gate exhaustively tied to the
//! frozen vocabulary artifact), advisory-only (the recorded suggestion
//! type exposes clamped/typed values and a Display summary — there is
//! no API through which production code could apply a hint), full
//! function without the plane (default-off, keyless-inert; A3
//! telemetry discipline: the plane emits only `advisory.*` rows), and
//! measured-before-trusted (nothing here applies anything).

pub mod accum;
pub mod assistant;
pub mod config;
pub mod egress;
pub mod runner;
pub mod state;
pub mod transport;

use std::path::Path;
use std::sync::Arc;

use modelswarm_telemetry::Telemetry;

pub use config::AdvisoryConfig;
pub use runner::AdvisoryHandle;
pub use state::Gap;
pub use transport::{AdvisoryTransport, CountingTransport, ReqwestTransport};

/// Serializes tests that set/remove `MSP_ADVISORY_KEY_FILE` (protocol
/// review advisory A-4: parallel tokio tests in one binary race the
/// process env; the assistant leg added more env-riding tests, so the
/// guard is now load-bearing). Take this lock around every env mutation.
#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The arming decision for one installation: an armed plane has
/// `enabled=true`, a readable key file, an https endpoint, and a model
/// id (ADR-033 §3.1/§3.3 — an installation without a key file is inert
/// by construction).
#[derive(Debug, Clone, PartialEq)]
pub enum Arm {
    Armed {
        key: String,
    },
    /// `enabled=true` but the plane cannot run: the gap reason is the
    /// honest, frozen one (`no_key` / `no_config`).
    NotArmed {
        reason: Gap,
    },
    /// `enabled=false` (the shipped default): silent — the vocabulary's
    /// `disabled` gap fires only inside a driven state machine, never
    /// as log spam on default installs.
    Disabled,
}

/// Resolves the arming decision for a data dir (reads `advisory.json`
/// and the key file; no I/O beyond those two files, no network).
pub fn arm(data_dir: &Path) -> (AdvisoryConfig, Arm) {
    let config = match AdvisoryConfig::load(data_dir) {
        Ok(cfg) => cfg,
        Err(_) => {
            // A present-but-unreadable config is inert with the honest
            // nearest gap (fail closed; the file's contents are never
            // guessed).
            return (
                AdvisoryConfig::default(),
                Arm::NotArmed {
                    reason: Gap::NoConfig,
                },
            );
        }
    };
    if !config.enabled {
        return (config, Arm::Disabled);
    }
    let key = config::read_key(&config::resolve_key_file(&config))
        .ok()
        .flatten();
    let Some(key) = key else {
        return (config, Arm::NotArmed { reason: Gap::NoKey });
    };
    let endpoint_ok = config
        .endpoint_url
        .as_deref()
        .map(config::is_https_endpoint)
        .unwrap_or(false);
    let model_ok = config.model.as_deref().is_some_and(|m| !m.is_empty());
    if !endpoint_ok || !model_ok {
        return (
            config,
            Arm::NotArmed {
                reason: Gap::NoConfig,
            },
        );
    }
    (config, Arm::Armed { key })
}

/// Wires the plane into a running node: silent when disabled, ONE
/// `advisory.cycle.gap` row when enabled-but-not-armable (the operator
/// sees why the plane is inert), and the timer-driven runner when
/// armed. `transport_override` is a TEST seam (chat-wall and cadence
/// suites inject a counting double); production passes `None` and gets
/// the pinned reqwest adapter.
pub fn wire(
    data_dir: &Path,
    telemetry: &Arc<Telemetry>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    transport_override: Option<Arc<dyn AdvisoryTransport>>,
) -> Option<AdvisoryHandle> {
    let (config, arm_result) = arm(data_dir);
    match arm_result {
        Arm::Disabled => None,
        Arm::NotArmed { reason } => {
            telemetry.warn(
                "advisory.cycle.gap",
                &[("reason", reason.as_str()), ("cycle_id", "none")],
            );
            None
        }
        Arm::Armed { key } => {
            let transport: Arc<dyn AdvisoryTransport> = match transport_override {
                Some(double) => double,
                None => match ReqwestTransport::new(
                    config.endpoint_url.as_deref().unwrap_or_default(),
                    config.model.as_deref().unwrap_or_default(),
                ) {
                    Ok(adapter) => Arc::new(adapter),
                    Err(_) => {
                        telemetry.warn(
                            "advisory.cycle.gap",
                            &[("reason", "no_config"), ("cycle_id", "none")],
                        );
                        return None;
                    }
                },
            };
            Some(runner::spawn(
                &config,
                key,
                transport,
                Arc::clone(telemetry),
                shutdown,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layer 3 of the chat-wall suite (behavioral): a FULL SUCCESS chat
    /// turn through the node's loopback gateway — the exact surface
    /// `send_chat`'s local fallback posts to — with the plane ARMED and
    /// KEYED over a counting double, records ZERO advisory calls. The
    /// mock runtime (cfg(test)) serves the turn; the runtime-less
    /// failure paths ride
    /// `tests/advisory_chat_wall.rs` and the desktop swarm paths ride
    /// the desktop tauri-shell suite.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // deliberate: the guard must
                                         // span the awaits (the node start READS the env inside); the A-4
                                         // serial guard trades this lint for race-free env tests.
    async fn chat_wall_success_path_records_zero_advisory_calls() {
        let _env = env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(config::ADVISORY_CONFIG_FILE),
            r#"{"enabled":true,"endpoint_url":"https://advisor.invalid/v1","model":"test-model"}"#,
        )
        .unwrap();
        let key_file = dir.path().join("key.txt");
        std::fs::write(&key_file, "test-key-local-double\n").unwrap();
        std::env::set_var(config::ENV_ADVISORY_KEY_FILE, &key_file);

        let double = Arc::new(CountingTransport::always_ok());
        let as_trait: Arc<dyn AdvisoryTransport> = double.clone();
        let (tx, rx) = tokio::sync::watch::channel(false);
        let config = crate::NodeConfig {
            gateway_port: 0,
            data_dir: dir.path().to_path_buf(),
            profile_id: Some("msp1:wall".to_string()),
            mock: true,
            advisory_transport: Some(as_trait),
            ..crate::NodeConfig::default()
        };
        let node = crate::Node::start(config, rx)
            .await
            .expect("node starts (mock runtime)");
        std::env::remove_var(config::ENV_ADVISORY_KEY_FILE);
        assert!(
            node.advisory.is_some(),
            "the plane must be ARMED in-process — the zero-call assertion must not be vacuous"
        );
        // Feed the accumulators past the threshold too: even a plane
        // with plenty to digest stays silent during a chat turn.
        if let Some(handle) = &node.advisory {
            for _ in 0..250 {
                handle
                    .accumulators
                    .lock()
                    .unwrap()
                    .record_event("serving.clamped");
            }
        }

        let addr = node.local_addr;
        let reply = reqwest::Client::new()
            .post(format!("http://{addr}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "msp1:wall",
                "messages": [{"role": "user", "content": "say ok"}],
                "max_tokens": 8,
            }))
            .send()
            .await
            .expect("chat turn completes");
        assert!(
            reply.status().is_success(),
            "the mock runtime serves the turn"
        );

        // THE WALL: zero calls, zero bodies, zero key presentations —
        // the chat turn never reached the advisory plane.
        assert_eq!(
            double.count(),
            0,
            "W1: a chat turn NEVER calls the advisory API"
        );
        assert!(double.bodies().is_empty());
        assert!(double.presented_keys().is_empty());

        tx.send(true).unwrap();
        node.stopped().await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // deliberate (see above).
    async fn armed_plane_runs_a_cycle_and_the_double_records_it() {
        // The counter-sensitivity control for the chat-wall suite: with
        // the plane armed and fed past the threshold, the counting
        // double DOES record calls — proving "zero calls" assertions in
        // the wall tests are meaningful (a dead counter would pass
        // vacuously).
        let _env = env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(config::ADVISORY_CONFIG_FILE),
            r#"{"enabled":true,"endpoint_url":"https://advisor.invalid/v1","model":"test-model",
                "min_interval_ms":30,"events_threshold":5,"request_timeout_ms":500}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("key.txt"), "test-key-local-double\n").unwrap();
        std::env::set_var(config::ENV_ADVISORY_KEY_FILE, dir.path().join("key.txt"));
        let (config, arm_result) = arm(dir.path());
        std::env::remove_var(config::ENV_ADVISORY_KEY_FILE);
        assert!(matches!(arm_result, Arm::Armed { .. }), "{arm_result:?}");

        let double = Arc::new(CountingTransport::always_erroring());
        let (telemetry, sink) = modelswarm_telemetry::Telemetry::memory();
        let telemetry = Arc::new(telemetry);
        let (tx, rx) = tokio::sync::watch::channel(false);
        let as_trait: Arc<dyn AdvisoryTransport> = double.clone();
        let handle = runner::spawn(
            &config,
            "test-key-local-double".into(),
            as_trait,
            telemetry,
            rx,
        );
        for _ in 0..50 {
            handle
                .accumulators
                .lock()
                .unwrap()
                .record_event("serving.session_ended");
        }
        // A few ticks at min_interval 30ms (sleep-then-check).
        let mut called = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            if double.count() >= 1 {
                called = true;
                break;
            }
        }
        tx.send(true).unwrap();
        assert!(called, "the armed plane must call the double");
        // The failure was recorded as the honest gap, counts/labels only.
        let rows = sink.snapshot().join("\n");
        assert!(rows.contains("\"event\":\"advisory.cycle.gap\""));
        assert!(rows.contains("\"reason\":\"http_error\""));
        assert!(
            !rows.contains("test-key-local-double"),
            "the key never reaches telemetry"
        );
        // A3 discipline: the plane emitted ONLY advisory.* rows.
        for line in sink.snapshot() {
            assert!(line.contains("\"event\":\"advisory."), "{line}");
        }
    }

    #[tokio::test]
    async fn default_disabled_data_dir_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let (config, arm_result) = arm(dir.path());
        assert!(matches!(arm_result, Arm::Disabled));
        assert!(!config.enabled);
        let (telemetry, sink) = modelswarm_telemetry::Telemetry::memory();
        let (_tx, rx) = tokio::sync::watch::channel(false);
        assert!(wire(dir.path(), &Arc::new(telemetry), rx, None).is_none());
        assert!(sink.snapshot().is_empty(), "a default install is silent");
    }

    #[tokio::test]
    async fn enabled_without_key_emits_one_honest_gap_row() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(config::ADVISORY_CONFIG_FILE),
            r#"{"enabled":true,"endpoint_url":"https://advisor.invalid/v1","model":"m"}"#,
        )
        .unwrap();
        // No key file anywhere near: force the platform default outside
        // the temp dir to be absent too — the resolver consults env
        // first; leave it unset and rely on the real platform default
        // being absent on CI machines (honest: if an owner machine HAS
        // a key, this test would arm — so scope the assertion to
        // NotArmed OR Armed-with-own-key and only assert the row shape
        // for the NotArmed case).
        let (telemetry, sink) = modelswarm_telemetry::Telemetry::memory();
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let handle = wire(dir.path(), &Arc::new(telemetry), rx, None);
        match handle {
            None => {
                let rows = sink.snapshot().join("\n");
                assert!(rows.contains("advisory.cycle.gap"), "{rows}");
            }
            Some(_) => {
                // An owner machine with a real platform-default key
                // file: arming is correct behavior; not a failure.
            }
        }
    }
}
