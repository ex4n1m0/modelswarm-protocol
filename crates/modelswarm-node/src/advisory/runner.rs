//! The digest-cycle runner — the ONLY production call site of the
//! advisory plane (ADR-033 invariant A5 + ADR-037 W2: cycles are
//! timer-driven; no call site exists in any request, serving,
//! scheduling, negotiation, gateway, or telemetry-emit path; the
//! chat-wall suite pins this).
//!
//! The runner is the runtime shell around the PURE state machine: it
//! owns the real clock and the real nonce generator
//! (`identity::new_nonce()`, the RC-8 pin), snapshots the typed
//! accumulators, executes `Effect::Call` through the INJECTED
//! transport, and emits the plane's own `advisory.*` telemetry rows
//! (counts and labels only — the frozen event set in the egress
//! artifact). Nothing else in the process drives a cycle.

use std::sync::Arc;

use modelswarm_identity::new_nonce;
use modelswarm_telemetry::Telemetry;

use super::accum::Accumulators;
use super::config::AdvisoryConfig;
use super::state::{step, AdvisoryState, Caps, Effect, Input, TransportFailure};
use super::transport::AdvisoryTransport;

/// Shared feed handle: producing components record into the typed
/// accumulators through this (the aggregation allowlist gates what may
/// feed a digest — see [`Accumulators`]). The handle is inert until a
/// cycle actually runs; recording never performs I/O.
#[derive(Clone)]
pub struct AdvisoryHandle {
    pub accumulators: Arc<std::sync::Mutex<Accumulators>>,
}

impl std::fmt::Debug for AdvisoryHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdvisoryHandle").finish_non_exhaustive()
    }
}

fn system_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn caps_of(config: &AdvisoryConfig) -> Caps {
    Caps {
        enabled: config.enabled,
        key_present: true, // the runner is spawned only with a resolved key
        endpoint_configured: config
            .endpoint_url
            .as_deref()
            .map(super::config::is_https_endpoint)
            .unwrap_or(false)
            && config.model.as_deref().is_some_and(|m| !m.is_empty()),
        max_calls_per_day: config.max_calls_per_day,
        min_interval_ms: config.min_interval_ms,
        events_threshold: config.events_threshold,
        request_timeout_ms: config.request_timeout_ms,
        max_request_bytes: config.max_request_bytes,
        max_response_bytes: config.max_response_bytes,
        max_completion_tokens: config.max_completion_tokens,
    }
}

/// Spawns the timer-driven digest cycle. Call ONLY on an armed plane
/// (enabled + key + https endpoint + model — see [`super::config`]);
/// the caller owns the not-armed gap rows. The tick period is
/// `min_interval_ms` (the natural cadence: no legal call may be closer
/// than that anyway — no extra config key exists for it by design).
pub fn spawn(
    config: &AdvisoryConfig,
    key: String,
    transport: Arc<dyn AdvisoryTransport>,
    telemetry: Arc<Telemetry>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> AdvisoryHandle {
    let accumulators = Arc::new(std::sync::Mutex::new(Accumulators::default()));
    let handle = AdvisoryHandle {
        accumulators: Arc::clone(&accumulators),
    };
    let caps = caps_of(config);
    let tick_ms = config.min_interval_ms.max(1);
    tokio::spawn(async move {
        let mut state = AdvisoryState::new(caps, true, system_now_ms());
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + std::time::Duration::from_millis(tick_ms),
            std::time::Duration::from_millis(tick_ms),
        );
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // How much of the accumulators' event count has been fed into
        // the machine's pending counter already (resynchronized to zero
        // when a call resets the aggregation window).
        let mut fed: u64 = 0;
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow_and_update() {
                        break;
                    }
                }
                _ = ticker.tick() => {
                    let digest = accumulators.lock().unwrap().snapshot();
                    let total = accumulators.lock().unwrap().event_count();
                    let delta = total.saturating_sub(fed);
                    fed = total;
                    let (next, _) = step(state, Input::Events(delta));
                    state = next;
                    let (next, effects) = step(
                        state,
                        Input::Timer {
                            now_ms: system_now_ms(),
                            digest,
                            cycle_nonce: new_nonce(),
                        },
                    );
                    state = next;
                    let (next, call_fired) =
                        run_effects(state, effects, &key, &transport, telemetry.as_ref()).await;
                    state = next;
                    if call_fired {
                        // "Aggregate since last cycle": a call resets the
                        // window on both sides of the seam.
                        accumulators.lock().unwrap().reset();
                        fed = 0;
                    }
                }
            }
        }
    });
    handle
}

/// Executes one tick's effects. Returns the advanced state and whether
/// a provider call fired. The CALL branch is the ONLY place in the
/// process that executes [`AdvisoryTransport::send`].
async fn run_effects(
    mut state: AdvisoryState,
    effects: Vec<Effect>,
    key: &str,
    transport: &Arc<dyn AdvisoryTransport>,
    telemetry: &Telemetry,
) -> (AdvisoryState, bool) {
    let mut call_fired = false;
    for effect in effects {
        match effect {
            Effect::Gap { reason, cycle_id } => {
                telemetry.warn(
                    "advisory.cycle.gap",
                    &[
                        ("reason", reason.as_str()),
                        ("cycle_id", cycle_id.as_deref().unwrap_or("none")),
                    ],
                );
            }
            Effect::GateDropped { count } => {
                // Counted at the gate — a count, never content.
                telemetry.info(
                    "advisory.cycle.gap",
                    &[
                        ("reason", "empty_after_redaction"),
                        ("dropped", &count.to_string()),
                    ],
                );
            }
            Effect::CycleSent { cycle_id } => {
                telemetry.info("advisory.cycle.sent", &[("cycle_id", &cycle_id)]);
            }
            Effect::SuggestionRecorded {
                cycle_id,
                suggestion,
            } => {
                telemetry.info(
                    "advisory.suggestion.recorded",
                    &[
                        ("cycle_id", &cycle_id),
                        ("type", suggestion.kind_str()),
                        ("confidence", suggestion.confidence_str()),
                        ("hints", &suggestion.hints.len().to_string()),
                    ],
                );
            }
            Effect::SuggestionDiscarded { cycle_id } => {
                telemetry.info(
                    "advisory.suggestion.recorded",
                    &[("cycle_id", &cycle_id), ("discarded", "1")],
                );
            }
            Effect::Call(request) => {
                let super::state::Phase::AwaitingCall { cycle_id, .. } = &state.phase else {
                    continue;
                };
                let cycle_id = cycle_id.clone();
                call_fired = true;
                // The key reaches the Authorization header alone; the
                // deadline wraps the injected transport (a late reply is
                // a no-op discard + diagnostic `late`).
                let outcome = tokio::time::timeout(
                    std::time::Duration::from_millis(request.timeout_ms),
                    transport.send(&request, key),
                )
                .await
                .map_err(|_| TransportFailure::Deadline)
                .and_then(|r| r);
                let (next, outcome_effects) = step(state, Input::CallOutcome { cycle_id, outcome });
                state = next;
                for e in outcome_effects {
                    if let Effect::Gap { reason, cycle_id } = e {
                        telemetry.warn(
                            "advisory.cycle.gap",
                            &[
                                ("reason", reason.as_str()),
                                ("cycle_id", cycle_id.as_deref().unwrap_or("none")),
                            ],
                        );
                    }
                }
            }
        }
    }
    (state, call_fired)
}
