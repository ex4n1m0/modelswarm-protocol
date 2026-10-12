//! The client half of the auto-update engine (owner request 2026-10-12:
//! "clients update and restart when we ship a new client").
//!
//! Settled design: Tauri 2's updater plugin with OUR minisign keypair. The
//! Windows installers stay UNSIGNED (code-signing remains a separate,
//! unpaid stop — ADR-008/G6); every update PACKAGE carries its own
//! minisign-class signature that each client verifies against the public
//! key embedded in `tauri.conf.json` (`plugins.updater.pubkey`) BEFORE
//! anything is installed — that verification cannot be turned off. The
//! private key lives OUTSIDE this repo; signing happens only at local
//! release staging next to the key.
//!
//! Honesty rules this module enforces (each pinned by a test below):
//! - the status line is the truth — "update available → v…", "downloading
//!   v… N%", "v… ready, restarting…" — rendered VERBATIM by the UI from
//!   [`UpdaterStatus::line`] (same no-mirror rule as the census chips);
//! - a failed check reads "update check failed — <reason>" and NEVER
//!   blocks the app: the supervisor only ever writes status;
//! - a failed download/install leaves the running version intact (the
//!   plugin installs only after signature verification succeeded, and the
//!   NSIS updater runs as a separate process);
//! - `MSP_NO_AUTOUPDATE=1` (dev opt-out) skips every check, stated
//!   honestly as "updates: off";
//! - on Windows the plugin's `install()` EXITS THE PROCESS right after
//!   spawning the passive NSIS updater (`/P /UPDATE /R` restarts the app),
//!   so `Ready` is written BEFORE `install()` and is TERMINAL in the
//!   state machine — a late timer tick must not resurrect the flow — and
//!   the supervised engine children are killed in `on_before_exit` so the
//!   installer never races a live engine binary it is about to replace
//!   (`modelswarm_winjob::close_all_retained_jobs`, pinned by test).
//!
//! The updater's real round-trip (signed manifest + package over the real
//! site) is validated by the release train and a staged `latest.json`,
//! not by unit tests; what IS unit-tested here is the status machine, its
//! failure edges, and the pinned update-channel config.

use std::time::Duration;

use serde::Serialize;
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;

use super::{log_event, DesktopState};

/// One check on start, then every 6h (spec: "on app start + every 6h").
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The update channel's minisign PUBLIC key (public by design; the
/// private half lives outside the repo). Test-only: it exists to pin
/// `tauri.conf.json` against drift — the plugin itself reads the key from
/// the config at runtime.
#[cfg(test)]
pub const UPDATE_PUBKEY_PIN: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDM4RkM5OENCNEIwNjRGMzcKUldRM1R3Wkx5NWo4T0NpWHY2L2U3QzVUMVluTHQyRnErdmVCeFM5ZmF2VUUyT2JudXk0bzdQTmEK";

/// The one update endpoint (pinned by test): the tracker site's STATIC
/// `latest.json`, with the plugin's `{{target}}/{{arch}}/
/// {{current_version}}` templating carried as QUERY parameters — valid
/// today against the static file (query strings are ignored by static
/// serving) and ready for a future dynamic server without a client
/// change.
#[cfg(test)]
const UPDATE_ENDPOINT_PIN: &str = "https://modelswarm.deepflux.space/downloads/latest.json?target={{target}}&arch={{arch}}&current_version={{current_version}}";

/// The honest updater status — one variant per phase of the real flow,
/// serialized to the webview as `{phase, version?, progress?, reason?,
/// line}` (see [`UpdaterStatus::view`]). The UI renders `line` verbatim.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum UpdaterStatus {
    /// `MSP_NO_AUTOUPDATE=1` — dev opt-out; no check will ever run.
    Disabled,
    /// Supervisor not run yet (the first instants after launch).
    Idle,
    Checking,
    UpToDate,
    /// The manifest advertised a newer signed version.
    Available {
        version: String,
    },
    /// Package downloading (verified only after the last byte arrives).
    Downloading {
        version: String,
        done: u64,
        total: Option<u64>,
    },
    /// Downloaded AND signature-verified; install started. TERMINAL: on
    /// Windows the process exits inside `install()` (the NSIS updater
    /// restarts us); on Linux/macOS we relaunch immediately after.
    Ready {
        version: String,
    },
    /// The check itself failed (network/manifest). NEVER blocks the app.
    CheckFailed {
        reason: String,
    },
    /// Download or install failed — the RUNNING version is intact.
    InstallFailed {
        reason: String,
    },
}

/// The events the supervisor reports into the state machine. Everything
/// the plugin's real flow can produce, and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdaterEvent {
    CheckStarted,
    CheckFailed(String),
    UpToDate,
    UpdateFound(String),
    Progress {
        done: u64,
        total: Option<u64>,
    },
    /// Download finished AND the package signature verified against the
    /// embedded public key; `install()` is about to run.
    InstallBegin,
    InstallFailed(String),
}

/// The pure transition function — the whole status state machine, no I/O.
/// Late/stale events against a state they do not belong to are IGNORED
/// (never fabricate progress), `Ready` and `Disabled` are terminal, and
/// every failure state may be left only by an explicit new cycle
/// (`CheckStarted`) — the 6h timer, not a crash loop.
pub fn step(current: &UpdaterStatus, event: UpdaterEvent) -> UpdaterStatus {
    use UpdaterEvent as E;
    use UpdaterStatus as S;
    match (current, event) {
        // Terminal: the installer is committed (a late timer tick must not
        // resurrect the flow while the process is being replaced), and the
        // dev opt-out never moves.
        (S::Ready { .. }, _) | (S::Disabled, _) => current.clone(),
        (_, E::CheckStarted) => S::Checking,
        (S::Checking, E::CheckFailed(reason)) => S::CheckFailed { reason },
        (S::Checking, E::UpToDate) => S::UpToDate,
        (S::Checking, E::UpdateFound(version)) => S::Available { version },
        // Progress counts only while an update is actually in flight:
        // `Available` is the pre-first-chunk state, `Downloading` refreshes.
        (S::Available { version }, E::Progress { done, total })
        | (S::Downloading { version, .. }, E::Progress { done, total }) => S::Downloading {
            version: version.clone(),
            done,
            total,
        },
        (S::Available { version } | S::Downloading { version, .. }, E::InstallBegin) => S::Ready {
            version: version.clone(),
        },
        (S::Available { .. } | S::Downloading { .. }, E::InstallFailed(reason)) => {
            S::InstallFailed { reason }
        }
        // Stale event against a state it does not belong to — ignored.
        _ => current.clone(),
    }
}

impl UpdaterStatus {
    /// The exact honest status line the UI renders verbatim.
    pub fn line(&self) -> String {
        match self {
            Self::Disabled => "updates: off (MSP_NO_AUTOUPDATE)".into(),
            Self::Idle => "update: not checked yet".into(),
            Self::Checking => "checking for update…".into(),
            Self::UpToDate => "up to date".into(),
            Self::Available { version } => format!("update available → v{version}"),
            Self::Downloading {
                version,
                done,
                total,
            } => match total {
                Some(total) if *total > 0 => {
                    let pct = std::cmp::min(done * 100 / total, 100);
                    format!("downloading v{version}… {pct}%")
                }
                _ => format!("downloading v{version}… {} MB", done / 1_000_000),
            },
            Self::Ready { version } => format!("v{version} ready, restarting…"),
            Self::CheckFailed { reason } => format!("update check failed — {reason}"),
            Self::InstallFailed { reason } => {
                format!("update failed — this version keeps running ({reason})")
            }
        }
    }

    /// The `get_status` payload: `line` always present, the phase's own
    /// fields only. No URLs, no keys — nothing secret exists on this path.
    pub fn view(&self) -> serde_json::Value {
        let (version, progress, reason): (Option<String>, Option<String>, Option<String>) =
            match self {
                Self::Disabled | Self::Idle | Self::Checking | Self::UpToDate => (None, None, None),
                Self::Available { version } => (Some(version.clone()), None, None),
                Self::Downloading {
                    version,
                    done,
                    total,
                } => {
                    let progress = match total {
                        Some(total) if *total > 0 => {
                            format!("{:.1} / {:.1} MB", *done as f64 / 1e6, *total as f64 / 1e6)
                        }
                        _ => format!("{:.1} MB", *done as f64 / 1e6),
                    };
                    (Some(version.clone()), Some(progress), None)
                }
                Self::Ready { version } => (Some(version.clone()), None, None),
                Self::CheckFailed { reason } | Self::InstallFailed { reason } => {
                    (None, None, Some(reason.clone()))
                }
            };
        serde_json::json!({
            "phase": self.phase_name(),
            "version": version,
            "progress": progress,
            "reason": reason,
            "line": self.line(),
        })
    }

    fn phase_name(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::UpToDate => "up_to_date",
            Self::Available { .. } => "available",
            Self::Downloading { .. } => "downloading",
            Self::Ready { .. } => "ready",
            Self::CheckFailed { .. } => "check_failed",
            Self::InstallFailed { .. } => "install_failed",
        }
    }
}

/// Dev opt-out: exactly `MSP_NO_AUTOUPDATE=1` disables. Anything else —
/// including a mistyped value — keeps updates ON (the opt-out must be
/// explicit, and a typo must not silently strand a client on an old
/// version).
fn env_disables_autoupdate(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Spawns the update supervisor: one check immediately on start, then one
/// every 6h. Every transition it makes is a single status write the UI
/// renders; no check outcome can block app usage.
pub fn spawn_supervisor(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if env_disables_autoupdate(std::env::var("MSP_NO_AUTOUPDATE").ok().as_deref()) {
            set_status(&handle, UpdaterStatus::Disabled).await;
            return;
        }
        // interval's first tick completes immediately: check on start.
        let mut ticker = tokio::time::interval(CHECK_INTERVAL);
        loop {
            ticker.tick().await;
            run_cycle(&handle).await;
            // `Ready` is terminal — the process is committed to the
            // restart (on Windows it usually never returns from install()
            // at all). Never start another cycle from Ready: a re-download
            // would double-install.
            let state = handle.state::<DesktopState>();
            if matches!(
                state.inner.lock().await.updater,
                UpdaterStatus::Ready { .. }
            ) {
                break;
            }
        }
    });
}

async fn set_status(handle: &tauri::AppHandle, status: UpdaterStatus) {
    let state = handle.state::<DesktopState>();
    state.inner.lock().await.updater = status;
}

async fn apply_event(handle: &tauri::AppHandle, event: UpdaterEvent) {
    let state = handle.state::<DesktopState>();
    let mut inner = state.inner.lock().await;
    inner.updater = step(&inner.updater, event);
}

/// One check→download→verify→install cycle. Failure modes degrade
/// honestly: a failed check or download writes its status and returns;
/// the app keeps running the current version.
async fn run_cycle(handle: &tauri::AppHandle) {
    let telemetry = handle.state::<DesktopState>().telemetry.clone();
    apply_event(handle, UpdaterEvent::CheckStarted).await;
    let updater = match handle
        .updater_builder()
        // Windows-only hook (the plugin fires it only there): kill every
        // supervised engine BEFORE the NSIS updater runs, so it never
        // races a live engine binary it is about to replace. Plain
        // process death needs no call — the same retained job handles
        // close on exit however the process dies (winjob discipline).
        .on_before_exit(kill_supervised_engines(&telemetry))
        .build()
    {
        Ok(updater) => updater,
        Err(e) => {
            log_event(
                &telemetry,
                "warn",
                "updater.unavailable",
                &[("detail", &short_reason(&e))],
            );
            apply_event(
                handle,
                UpdaterEvent::CheckFailed(format!("updater unavailable: {}", short_reason(&e))),
            )
            .await;
            return;
        }
    };
    match updater.check().await {
        Err(e) => {
            let reason = short_reason(&e);
            log_event(
                &telemetry,
                "warn",
                "updater.check",
                &[("result", "failed"), ("detail", &reason)],
            );
            apply_event(handle, UpdaterEvent::CheckFailed(reason)).await;
        }
        Ok(None) => {
            log_event(
                &telemetry,
                "info",
                "updater.check",
                &[("result", "up_to_date")],
            );
            apply_event(handle, UpdaterEvent::UpToDate).await;
        }
        Ok(Some(update)) => {
            let version = update.version.clone();
            log_event(
                &telemetry,
                "info",
                "updater.check",
                &[("result", "available"), ("version", &version)],
            );
            apply_event(handle, UpdaterEvent::UpdateFound(version.clone())).await;
            // download() verifies the package signature against the
            // embedded public key before returning Ok — a bad signature
            // surfaces here as an error, and install() never runs.
            let bytes = match download_with_progress(handle, &update).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    let reason = short_reason(&e);
                    log_event(
                        &telemetry,
                        "warn",
                        "updater.download",
                        &[("version", &version), ("detail", &reason)],
                    );
                    apply_event(
                        handle,
                        UpdaterEvent::InstallFailed(format!("download: {reason}")),
                    )
                    .await;
                    return;
                }
            };
            // The running install is committed from here: on Windows
            // install() EXITS THE PROCESS after spawning the passive NSIS
            // updater (/P /UPDATE /R restarts the app), so Ready must be
            // durable BEFORE the call — it is the last state the UI ever
            // sees of this process.
            apply_event(handle, UpdaterEvent::InstallBegin).await;
            log_event(
                &telemetry,
                "info",
                "updater.install",
                &[("version", &version)],
            );
            if let Err(e) = update.install(bytes) {
                let reason = short_reason(&e);
                log_event(
                    &telemetry,
                    "warn",
                    "updater.install",
                    &[("version", &version), ("detail", &reason)],
                );
                apply_event(
                    handle,
                    UpdaterEvent::InstallFailed(format!("install: {reason}")),
                )
                .await;
            } else if cfg!(not(windows)) {
                // Linux/macOS: install() returned — relaunch to run the
                // new version. restart() never returns.
                handle.restart();
            }
            // Windows never reaches this comment in practice (the NSIS
            // path exits inside install()); if a future plugin build ever
            // returned Ok, the supervisor loop's Ready-terminal check
            // prevents any second download.
        }
    }
}

/// `update.download` with honest, throttled progress: chunks arrive far
/// faster than the UI polls, so a status write goes out only on integer
/// percent boundaries (every 8 MiB when the length is unknown) — never
/// per chunk.
async fn download_with_progress(
    handle: &tauri::AppHandle,
    update: &tauri_plugin_updater::Update,
) -> Result<Vec<u8>, tauri_plugin_updater::Error> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, Option<u64>)>();
    let drain_handle = handle.clone();
    let drain = tauri::async_runtime::spawn(async move {
        while let Some((done, total)) = rx.recv().await {
            apply_event(&drain_handle, UpdaterEvent::Progress { done, total }).await;
        }
    });
    let mut done = 0u64;
    let mut last_boundary: Option<u64> = None;
    let result = update
        .download(
            move |chunk, total| {
                done += chunk as u64;
                let boundary = match total {
                    Some(total) if total > 0 => Some(done * 100 / total),
                    _ => Some(done / (8 * 1024 * 1024)),
                };
                if boundary != last_boundary {
                    last_boundary = boundary;
                    let _ = tx.send((done, total));
                }
            },
            || {},
        )
        .await;
    // The closure owning `tx` is dropped when download() returns, closing
    // the channel; the drain task then ends on its own.
    let _ = drain.await;
    result
}

/// The `on_before_exit` body (Windows): close every retained
/// kill-on-close job handle so all supervised engine children die BEFORE
/// the NSIS updater starts replacing files.
fn kill_supervised_engines(
    telemetry: &std::sync::Arc<modelswarm_telemetry::Telemetry>,
) -> impl Fn() + Send + Sync + 'static {
    let telemetry = telemetry.clone();
    move || {
        #[cfg(windows)]
        {
            let closed = modelswarm_winjob::close_all_retained_jobs();
            log_event(
                &telemetry,
                "info",
                "updater.before_exit",
                &[("jobs_closed", &closed.to_string())],
            );
        }
        #[cfg(not(windows))]
        {
            // The plugin only fires this hook on Windows; the binding is
            // kept so the closure compiles everywhere the shell does.
            let _ = telemetry;
        }
    }
}

/// One honest, bounded reason string for status lines and telemetry.
fn short_reason(e: &impl std::fmt::Display) -> String {
    let full = e.to_string();
    full.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(
        states: impl IntoIterator<Item = UpdaterStatus>,
        events: &[UpdaterEvent],
    ) -> Vec<UpdaterStatus> {
        let mut current = states.into_iter().next().unwrap();
        let mut seen = Vec::new();
        for event in events.iter().cloned() {
            current = step(&current, event);
            seen.push(current.clone());
        }
        seen
    }

    #[test]
    fn happy_path_reaches_ready_with_exact_honest_lines() {
        let seen = walk(
            [UpdaterStatus::Idle],
            &[
                UpdaterEvent::CheckStarted,
                UpdaterEvent::UpdateFound("0.2.22".into()),
                UpdaterEvent::Progress {
                    done: 10_000_000,
                    total: Some(100_000_000),
                },
                UpdaterEvent::Progress {
                    done: 100_000_000,
                    total: Some(100_000_000),
                },
                UpdaterEvent::InstallBegin,
            ],
        );
        assert_eq!(seen[0], UpdaterStatus::Checking);
        assert_eq!(
            seen[1],
            UpdaterStatus::Available {
                version: "0.2.22".into()
            }
        );
        assert_eq!(
            seen[2],
            UpdaterStatus::Downloading {
                version: "0.2.22".into(),
                done: 10_000_000,
                total: Some(100_000_000)
            }
        );
        assert_eq!(seen[3].line(), "downloading v0.2.22… 100%");
        assert_eq!(
            seen[4],
            UpdaterStatus::Ready {
                version: "0.2.22".into()
            }
        );
        // The spec's exact wording at each phase:
        assert_eq!(seen[1].line(), "update available → v0.2.22");
        assert_eq!(seen[2].line(), "downloading v0.2.22… 10%");
        assert_eq!(seen[4].line(), "v0.2.22 ready, restarting…");
    }

    #[test]
    fn ready_is_terminal_even_against_a_late_timer_tick() {
        // The Windows exit window: install() kills the process; whatever
        // fires meanwhile must not resurrect or corrupt the flow.
        for event in [
            UpdaterEvent::CheckStarted,
            UpdaterEvent::UpdateFound("9.9.9".into()),
            UpdaterEvent::Progress {
                done: 1,
                total: Some(2),
            },
            UpdaterEvent::InstallBegin,
            UpdaterEvent::InstallFailed("late".into()),
        ] {
            assert_eq!(
                step(
                    &UpdaterStatus::Ready {
                        version: "0.2.22".into()
                    },
                    event.clone()
                ),
                UpdaterStatus::Ready {
                    version: "0.2.22".into()
                },
                "Ready must ignore {event:?}"
            );
        }
    }

    #[test]
    fn check_failure_is_honest_and_never_blocks_the_next_cycle() {
        let seen = walk(
            [UpdaterStatus::Checking],
            &[
                UpdaterEvent::CheckFailed("dns lookup failed".into()),
                UpdaterEvent::CheckStarted,
                UpdaterEvent::UpToDate,
            ],
        );
        assert_eq!(
            seen[0],
            UpdaterStatus::CheckFailed {
                reason: "dns lookup failed".into()
            }
        );
        assert_eq!(seen[0].line(), "update check failed — dns lookup failed");
        // The 6h timer moves on from failure — no crash loop, no dead end.
        assert_eq!(seen[1], UpdaterStatus::Checking);
        assert_eq!(seen[2], UpdaterStatus::UpToDate);
    }

    #[test]
    fn install_failure_keeps_the_running_version_and_shrugs_off_late_progress() {
        let seen = walk(
            [UpdaterStatus::Available {
                version: "0.2.22".into(),
            }],
            &[
                UpdaterEvent::Progress {
                    done: 5,
                    total: Some(9),
                },
                UpdaterEvent::InstallFailed("signature verification failed".into()),
                // Late chunk from the abandoned download: ignored.
                UpdaterEvent::Progress {
                    done: 9,
                    total: Some(9),
                },
                UpdaterEvent::InstallBegin,
            ],
        );
        assert_eq!(
            seen[1],
            UpdaterStatus::InstallFailed {
                reason: "signature verification failed".into()
            }
        );
        assert_eq!(
            seen[1].line(),
            "update failed — this version keeps running (signature verification failed)"
        );
        assert_eq!(
            seen[2], seen[1],
            "late progress must not resurrect the download"
        );
        assert_eq!(
            seen[3], seen[1],
            "late install-begin must not fake readiness"
        );
    }

    #[test]
    fn progress_is_ignored_outside_an_in_flight_update() {
        for state in [
            UpdaterStatus::Idle,
            UpdaterStatus::Checking,
            UpdaterStatus::UpToDate,
            UpdaterStatus::CheckFailed {
                reason: "offline".into(),
            },
            UpdaterStatus::InstallFailed {
                reason: "download: 404".into(),
            },
        ] {
            assert_eq!(
                step(
                    &state,
                    UpdaterEvent::Progress {
                        done: 42,
                        total: Some(100)
                    }
                ),
                state,
                "stale progress must not move {state:?}"
            );
        }
    }

    #[test]
    fn disabled_is_terminal_and_only_the_exact_env_value_opts_out() {
        for event in [
            UpdaterEvent::CheckStarted,
            UpdaterEvent::UpdateFound("1.0.0".into()),
        ] {
            assert_eq!(
                step(&UpdaterStatus::Disabled, event),
                UpdaterStatus::Disabled
            );
        }
        assert_eq!(
            UpdaterStatus::Disabled.line(),
            "updates: off (MSP_NO_AUTOUPDATE)"
        );
        // Exactly "1" — a typo stays ON (never silently strand a client).
        assert!(!env_disables_autoupdate(None));
        assert!(!env_disables_autoupdate(Some("")));
        assert!(!env_disables_autoupdate(Some("0")));
        assert!(!env_disables_autoupdate(Some("true")));
        assert!(env_disables_autoupdate(Some("1")));
    }

    #[test]
    fn every_transition_stays_a_valid_honest_state() {
        // Cross-product: no event/state pair panics, drifts off the enum,
        // or produces an empty/silent line. This is the no-crash-loop and
        // never-silent invariant in one sweep.
        let states = vec![
            UpdaterStatus::Disabled,
            UpdaterStatus::Idle,
            UpdaterStatus::Checking,
            UpdaterStatus::UpToDate,
            UpdaterStatus::Available {
                version: "1.2.3".into(),
            },
            UpdaterStatus::Downloading {
                version: "1.2.3".into(),
                done: 1,
                total: Some(2),
            },
            UpdaterStatus::Ready {
                version: "1.2.3".into(),
            },
            UpdaterStatus::CheckFailed { reason: "r".into() },
            UpdaterStatus::InstallFailed { reason: "r".into() },
        ];
        let events = vec![
            UpdaterEvent::CheckStarted,
            UpdaterEvent::CheckFailed("x".into()),
            UpdaterEvent::UpToDate,
            UpdaterEvent::UpdateFound("2.0.0".into()),
            UpdaterEvent::Progress {
                done: 1,
                total: None,
            },
            UpdaterEvent::InstallBegin,
            UpdaterEvent::InstallFailed("x".into()),
        ];
        for state in &states {
            for event in &events {
                let next = step(state, event.clone());
                let line = next.line();
                assert!(
                    !line.trim().is_empty(),
                    "{state:?} + {event:?} -> {next:?} went silent"
                );
                // Only an in-flight update may claim downloading/ready.
                if matches!(
                    next,
                    UpdaterStatus::Downloading { .. } | UpdaterStatus::Ready { .. }
                ) {
                    assert!(
                        matches!(
                            state,
                            UpdaterStatus::Available { .. }
                                | UpdaterStatus::Downloading { .. }
                                | UpdaterStatus::Ready { .. }
                        ),
                        "{state:?} + {event:?} fabricated {next:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn status_view_shape_is_pinned() {
        let view = UpdaterStatus::Downloading {
            version: "0.2.22".into(),
            done: 10_000_000,
            total: Some(100_000_000),
        }
        .view();
        let obj = view.as_object().unwrap();
        assert_eq!(obj.len(), 5, "exactly the pinned keys: {view}");
        assert_eq!(obj["phase"], "downloading");
        assert_eq!(obj["version"], "0.2.22");
        assert_eq!(obj["progress"], "10.0 / 100.0 MB");
        assert!(obj["reason"].is_null());
        assert_eq!(obj["line"], "downloading v0.2.22… 10%");

        let view = UpdaterStatus::CheckFailed {
            reason: "offline".into(),
        }
        .view();
        assert_eq!(view["phase"], "check_failed");
        assert_eq!(view["reason"], "offline");
        assert!(view["version"].is_null());

        for state in [
            UpdaterStatus::Disabled,
            UpdaterStatus::Idle,
            UpdaterStatus::UpToDate,
        ] {
            let view = state.view();
            assert_eq!(view["phase"], state.phase_name());
            assert!(
                view["version"].is_null() && view["progress"].is_null() && view["reason"].is_null()
            );
        }
    }

    #[test]
    fn check_interval_is_six_hours_and_reasons_are_bounded() {
        assert_eq!(CHECK_INTERVAL, Duration::from_secs(6 * 60 * 60));
        let long = "x".repeat(500);
        assert!(short_reason(&long).chars().count() == 160);
    }

    /// The update channel is config, and config drifts — pin the exact
    /// `tauri.conf.json` surface the release train signs against: the
    /// owner-provided public key (decoding to a minisign key comment), the
    /// one https endpoint with target/arch/current_version templating,
    /// signed-version pinning (anti-downgrade), passive Windows installs,
    /// and (mirroring the CI guard) NO top-level version key.
    #[test]
    fn tauri_conf_pins_the_update_channel() {
        let conf: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"),
            )
            .expect("tauri.conf.json readable"),
        )
        .expect("tauri.conf.json parses");
        assert!(
            !conf.as_object().unwrap().contains_key("version"),
            "version single-sources from workspace Cargo.toml (CI guard mirror)"
        );
        let updater = &conf["plugins"]["updater"];
        let updater = updater.as_object().expect("plugins.updater object");
        assert_eq!(updater["pubkey"].as_str().unwrap(), UPDATE_PUBKEY_PIN);
        // The embedded key is a minisign PUBLIC key by construction: the
        // base64 payload decodes to the canonical comment header.
        assert!(
            UPDATE_PUBKEY_PIN.starts_with("dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6"),
            "pubkey must decode to 'untrusted comment: minisign public key:'"
        );
        let endpoints = updater["endpoints"].as_array().unwrap();
        assert_eq!(endpoints.len(), 1, "exactly one update endpoint");
        assert_eq!(endpoints[0].as_str().unwrap(), UPDATE_ENDPOINT_PIN);
        assert!(UPDATE_ENDPOINT_PIN
            .starts_with("https://modelswarm.deepflux.space/downloads/latest.json"));
        for template in ["{{target}}", "{{arch}}", "{{current_version}}"] {
            assert!(UPDATE_ENDPOINT_PIN.contains(template), "missing {template}");
        }
        // Anti-downgrade: the signature's trusted comment must name the
        // version it was signed for, so a compromised manifest cannot pair
        // an inflated version with an older genuine artifact.
        assert!(updater["requireSignedVersion"].as_bool().unwrap());
        // Our own status line is the UI: passive NSIS (progress only, no
        // wizard), and the plugin restarts the app (/R).
        assert_eq!(
            updater["windows"]["installMode"].as_str().unwrap(),
            "passive"
        );
        // Security review 2026-10-13: NOTHING in the config may weaken the
        // chain. A config regression that re-enables downgrade installs,
        // relaxes TLS, or allows non-https endpoints must fail HERE, in
        // this suite, not in the field. `allowDowngrades` must be absent
        // or false (absent -> the plugin's strict "strictly newer"
        // comparison applies), and the dangerous* flags must not exist at
        // all (they all default to false only when unset).
        assert!(
            !updater
                .get("allowDowngrades")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "allowDowngrades must never be enabled (anti-downgrade)"
        );
        for flag in [
            "dangerousAcceptInvalidCerts",
            "dangerousAcceptInvalidHostnames",
            "dangerousInsecureTransportProtocol",
        ] {
            assert!(
                updater.get(flag).is_none(),
                "{flag} must not appear in the updater config"
            );
        }
        assert!(
            endpoints
                .iter()
                .all(|e| e.as_str().unwrap().starts_with("https://")),
            "every update endpoint must be https"
        );
        // No webview capability may grant the updater's invoke commands
        // (check/download/install/download_and_install): the supervisor
        // drives updates from Rust and only status LINES cross into the
        // UI. Pins the ACL against a future capability grant — in EVERY
        // capability file, not just the default one.
        let caps_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let files = std::fs::read_dir(&caps_dir)
            .expect("capabilities/ directory readable")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"));
        for file in files {
            let text = std::fs::read_to_string(file.path()).expect("capability file readable");
            assert!(
                !text.contains("updater:"),
                "the webview must hold no updater permission (found one in {})",
                file.path().display()
            );
        }
    }
}
