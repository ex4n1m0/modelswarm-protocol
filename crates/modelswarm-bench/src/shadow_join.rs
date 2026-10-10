//! The 9.6 shadow divergence join: `sched.shadow.decision` (the plan the
//! shadow scheduler would execute) LEFT-JOINed with `sched.shadow.realized`
//! (what production actually did) out of node/desktop telemetry JSONL.
//!
//! # Why a LEFT join on the realized side (the load-bearing rule)
//!
//! `f4472f3` made every total-failure path in `try_swarm_chat` emit a
//! `sched.shadow.realized` row with `served_by=failed`. Pre-plan failures
//! (tracker/lookup/identity) carry a FRESH trace and therefore have NO
//! `sched.shadow.decision` row — they are realized-only samples. An inner
//! join drops exactly those rows, hiding the divergence data the join
//! exists to surface (a swarm attempt that produced no reply is still a
//! realizable sample: "no plan existed AND production failed" is a
//! distinct, countable outcome). This join therefore keeps EVERY realized
//! row; plan fields are `None` when no decision row shares the trace, and
//! decision-only rows are counted and visibly reported as dropped (they
//! have no realized outcome to join — an incomplete turn, not a sample).
//!
//! Input shape: lines as written by `modelswarm-telemetry` (flat JSON
//! objects: `ts`, `level`, `event`, then string-valued fields; the
//! redactor has already scrubbed them). Parsing is tolerant: unrelated
//! events are skipped, unparsable lines are counted (never fatal — a
//! telemetry file interleaves every event the node emits), and numeric
//! fields that carry the sentinels `"unmeasured"`/`"unknown"` parse to
//! `None` rather than failing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Telemetry event name of the shadow plan row (the decision side).
pub const DECISION_EVENT: &str = "sched.shadow.decision";
/// Telemetry event name of the realized row (the LEFT side, always kept).
pub const REALIZED_EVENT: &str = "sched.shadow.realized";

/// One joined divergence row: the realized outcome with the plan fields
/// attached when a decision row shared its trace (`has_plan == false` for
/// realized-only rows — the served_by=failed pre-plan set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowJoinRow {
    // ---- realized side (always present: the LEFT side of the join) ----
    pub trace: String,
    pub label: String,
    pub profile: String,
    /// `remote` | `local_fallback` | `failed`.
    pub served_by: String,
    pub production_peer: String,
    pub shadow_pick: Option<String>,
    /// Wall ms of the attempt (`None` for `unknown`).
    pub wall_ms: Option<f64>,
    /// Failure-path phase code (realized rows only; optional vocabulary).
    pub reason: Option<String>,
    // ---- plan side (all `None` when no decision row joined) ----
    /// Whether a `sched.shadow.decision` row shared this trace.
    pub has_plan: bool,
    pub plan_pick: Option<String>,
    pub plan_pick_predicted_ms: Option<f64>,
    pub plan_production_pick: Option<String>,
    /// `true`/`false`/`unknown` (the decision row's own vocabulary).
    pub plan_agree: Option<String>,
    pub plan_candidate_count: Option<u32>,
    pub plan_measured_count: Option<u32>,
}

/// Join statistics (visible counts — dropped decision rows are reported,
/// never silently discarded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowJoinStats {
    /// Realized rows read (== rows emitted: the LEFT side is complete).
    pub realized_rows: usize,
    /// Rows that joined against a decision row.
    pub joined_with_plan: usize,
    /// Realized-only rows (no decision row; `has_plan == false`).
    pub realized_only: usize,
    /// Decision rows with no realized row (incomplete turns) — counted
    /// and dropped from the output, visible here.
    pub decision_only_dropped: usize,
    /// Lines that were neither event (skipped, expected: telemetry
    /// interleaves every node event).
    pub unrelated_lines: usize,
    /// Lines that failed JSON parsing (counted, never fatal).
    pub unparsable_lines: usize,
}

/// The join result.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowJoinReport {
    pub rows: Vec<ShadowJoinRow>,
    pub stats: ShadowJoinStats,
}

/// Runs the LEFT join over telemetry lines (any order; correlation key is
/// `trace`).
pub fn join_shadow_telemetry<I, L>(lines: I) -> ShadowJoinReport
where
    I: IntoIterator<Item = L>,
    L: AsRef<str>,
{
    let mut decisions: BTreeMap<String, ParsedDecision> = BTreeMap::new();
    let mut realized: Vec<ParsedRealized> = Vec::new();
    let mut stats = ShadowJoinStats {
        realized_rows: 0,
        joined_with_plan: 0,
        realized_only: 0,
        decision_only_dropped: 0,
        unrelated_lines: 0,
        unparsable_lines: 0,
    };
    for line in lines {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.as_ref()) else {
            stats.unparsable_lines += 1;
            continue;
        };
        match value.get("event").and_then(|e| e.as_str()) {
            Some(DECISION_EVENT) => {
                if let Some(parsed) = ParsedDecision::parse(&value) {
                    decisions.insert(parsed.trace.clone(), parsed);
                } else {
                    stats.unparsable_lines += 1;
                }
            }
            Some(REALIZED_EVENT) => match ParsedRealized::parse(&value) {
                Some(parsed) => realized.push(parsed),
                None => stats.unparsable_lines += 1,
            },
            _ => stats.unrelated_lines += 1,
        }
    }
    stats.realized_rows = realized.len();
    let mut realized_traces: BTreeMap<String, ()> = BTreeMap::new();
    for event in &realized {
        realized_traces.insert(event.trace.clone(), ());
    }
    // Decision traces that never realized (incomplete turns — counted and
    // dropped visibly). Duplicated realized traces all attach to the one
    // decision row that shares their trace.
    stats.decision_only_dropped = decisions
        .keys()
        .filter(|trace| !realized_traces.contains_key(*trace))
        .count();
    let mut rows = Vec::with_capacity(realized.len());
    for event in realized {
        let plan = decisions.get(&event.trace);
        if plan.is_some() {
            stats.joined_with_plan += 1;
        } else {
            stats.realized_only += 1;
        }
        rows.push(ShadowJoinRow {
            trace: event.trace,
            label: event.label,
            profile: event.profile,
            served_by: event.served_by,
            production_peer: event.production_peer,
            shadow_pick: event.shadow_pick,
            wall_ms: event.wall_ms,
            reason: event.reason,
            has_plan: plan.is_some(),
            plan_pick: plan.and_then(|p| p.pick.clone()),
            plan_pick_predicted_ms: plan.and_then(|p| p.pick_predicted_ms),
            plan_production_pick: plan.and_then(|p| p.production_pick.clone()),
            plan_agree: plan.and_then(|p| p.agree.clone()),
            plan_candidate_count: plan.and_then(|p| p.candidate_count),
            plan_measured_count: plan.and_then(|p| p.measured_count),
        });
    }
    ShadowJoinReport { rows, stats }
}

// -- parsing helpers (all telemetry values are JSON strings) -------------

struct ParsedRealized {
    trace: String,
    label: String,
    profile: String,
    served_by: String,
    production_peer: String,
    shadow_pick: Option<String>,
    wall_ms: Option<f64>,
    reason: Option<String>,
}

impl ParsedRealized {
    fn parse(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            trace: string_field(value, "trace")?,
            label: string_field(value, "label").unwrap_or_default(),
            profile: string_field(value, "profile").unwrap_or_default(),
            served_by: string_field(value, "served_by").unwrap_or_default(),
            production_peer: string_field(value, "production_peer").unwrap_or_default(),
            shadow_pick: optional_field(value, "shadow_pick"),
            wall_ms: optional_field(value, "wall_ms").and_then(|w| w.parse().ok()),
            reason: optional_field(value, "reason"),
        })
    }
}

struct ParsedDecision {
    trace: String,
    pick: Option<String>,
    pick_predicted_ms: Option<f64>,
    production_pick: Option<String>,
    agree: Option<String>,
    candidate_count: Option<u32>,
    measured_count: Option<u32>,
}

impl ParsedDecision {
    fn parse(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            trace: string_field(value, "trace")?,
            pick: optional_field(value, "pick"),
            pick_predicted_ms: optional_field(value, "pick_predicted_ms")
                .and_then(|v| v.parse().ok()),
            production_pick: optional_field(value, "production_pick"),
            agree: optional_field(value, "agree"),
            candidate_count: optional_field(value, "candidates").and_then(|v| v.parse().ok()),
            measured_count: optional_field(value, "measured").and_then(|v| v.parse().ok()),
        })
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// A field's value, with the telemetry sentinels mapped to `None`
/// (`shadow_pick=none`, `pick_predicted_ms=unmeasured`, `agree=unknown`,
/// `wall_ms=unknown` — the emitters' honest "no value" spellings).
fn optional_field(value: &serde_json::Value, key: &str) -> Option<String> {
    let raw = string_field(value, key)?;
    match raw.as_str() {
        "none" | "unmeasured" | "unknown" => None,
        _ => Some(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One decision + one realized row sharing a trace, one realized-only
    /// row (the served_by=failed pre-plan set), one decision-only row
    /// (incomplete turn), and noise — the full join semantics in one input.
    #[test]
    fn realized_rows_survive_without_a_plan_row() {
        let lines = [
            // Joined pair: decision first, realized second (order-free).
            r#"{"ts":"2026-10-10T00:00:00Z","level":"info","event":"sched.shadow.decision","label":"shadow-no-action","trace":"t-join","profile":"msp1:aa","pick":"12D3KooWfast","pick_predicted_ms":"142.1","production_pick":"12D3KooWslow","agree":"false","candidates":"3","measured":"2","decision":"{}"}"#,
            r#"{"ts":"2026-10-10T00:00:01Z","level":"info","event":"sched.shadow.realized","label":"shadow-no-action","trace":"t-join","profile":"msp1:aa","production_peer":"12D3KooWslow","shadow_pick":"12D3KooWfast","served_by":"remote","wall_ms":"150","ttft_ewma_ms":"60.0","total_ewma_ms":"150.0"}"#,
            // Realized-only: pre-plan failure with a fresh trace (f4472f3
            // record_shadow_failure with plan=None). NO decision row exists.
            r#"{"ts":"2026-10-10T00:00:02Z","level":"info","event":"sched.shadow.realized","label":"shadow-no-action","trace":"shadow-abc123","profile":"msp1:aa","production_peer":"none","shadow_pick":"none","served_by":"failed","reason":"tracker","wall_ms":"unknown"}"#,
            // Decision-only: an incomplete turn (no realized outcome).
            r#"{"ts":"2026-10-10T00:00:03Z","level":"info","event":"sched.shadow.decision","label":"shadow-no-action","trace":"t-dangling","profile":"msp1:aa","pick":"none","pick_predicted_ms":"unmeasured","production_pick":"none","agree":"unknown","candidates":"1","measured":"0","decision":"{}"}"#,
            // Unrelated telemetry + garbage.
            r#"{"ts":"2026-10-10T00:00:04Z","level":"warn","event":"chat.swarm.error","phase":"lookup"}"#,
            "not json at all",
        ];
        let report = join_shadow_telemetry(lines);
        // The LEFT-side invariant: every realized row produced a row.
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.stats.realized_rows, 2);
        assert_eq!(report.stats.joined_with_plan, 1);
        assert_eq!(report.stats.realized_only, 1);
        assert_eq!(report.stats.decision_only_dropped, 1);
        assert_eq!(report.stats.unrelated_lines, 1);
        assert_eq!(report.stats.unparsable_lines, 1);

        // The joined pair carries its plan fields.
        let joined = report.rows.iter().find(|r| r.trace == "t-join").unwrap();
        assert!(joined.has_plan);
        assert_eq!(joined.plan_pick.as_deref(), Some("12D3KooWfast"));
        assert!((joined.plan_pick_predicted_ms.unwrap() - 142.1).abs() < 1e-9);
        assert_eq!(joined.plan_agree.as_deref(), Some("false"));
        assert_eq!(joined.plan_candidate_count, Some(3));
        assert_eq!(joined.plan_measured_count, Some(2));
        assert_eq!(joined.served_by, "remote");
        assert!((joined.wall_ms.unwrap() - 150.0).abs() < 1e-9);

        // THE PIN: the realized-only failure row survives the join with
        // every plan field null — an inner join would have dropped it.
        let only = report
            .rows
            .iter()
            .find(|r| r.trace == "shadow-abc123")
            .unwrap();
        assert!(!only.has_plan);
        assert_eq!(only.served_by, "failed");
        assert_eq!(only.reason.as_deref(), Some("tracker"));
        assert_eq!(only.wall_ms, None, "wall_ms=unknown parses to None");
        assert_eq!(only.plan_pick, None);
        assert_eq!(only.plan_pick_predicted_ms, None);
        assert_eq!(only.plan_production_pick, None);
        assert_eq!(only.plan_agree, None);
        assert_eq!(only.plan_candidate_count, None);
        assert_eq!(only.plan_measured_count, None);
        assert_eq!(only.shadow_pick, None, "shadow_pick=none parses to None");
    }

    /// Realized-before-decision ordering joins identically (correlation is
    /// by trace, not file order).
    #[test]
    fn join_is_order_independent() {
        let realized = r#"{"event":"sched.shadow.realized","trace":"t1","label":"l","profile":"p","production_peer":"peerA","shadow_pick":"peerB","served_by":"local_fallback","wall_ms":"12"}"#;
        let decision = r#"{"event":"sched.shadow.decision","trace":"t1","label":"l","pick":"peerB","pick_predicted_ms":"10.5","production_pick":"peerA","agree":"true","candidates":"2","measured":"2"}"#;
        let a = join_shadow_telemetry([realized, decision]);
        let b = join_shadow_telemetry([decision, realized]);
        assert_eq!(a.rows, b.rows);
        assert_eq!(a.stats, b.stats);
        assert_eq!(a.stats.joined_with_plan, 1);
    }

    /// A realized row missing its correlation key cannot be joined
    /// honestly — it is counted as unparsable rather than silently
    /// dropped or joined against the wrong plan.
    #[test]
    fn traceless_rows_are_counted_not_dropped_silently() {
        let lines = [
            r#"{"event":"sched.shadow.realized","served_by":"failed"}"#,
            r#"{"event":"sched.shadow.decision","pick":"x"}"#,
        ];
        let report = join_shadow_telemetry(lines);
        assert!(report.rows.is_empty());
        assert_eq!(report.stats.unparsable_lines, 2);
        assert_eq!(report.stats.realized_rows, 0);
    }

    /// Duplicate realized rows for one trace (a retried turn reusing a
    /// trace id) each survive; the shared plan attaches to both.
    #[test]
    fn duplicate_traces_keep_every_realized_row() {
        let lines = [
            r#"{"event":"sched.shadow.decision","trace":"dup","pick":"p1","pick_predicted_ms":"5","candidates":"1","measured":"1"}"#,
            r#"{"event":"sched.shadow.realized","trace":"dup","served_by":"failed","reason":"remote_dead"}"#,
            r#"{"event":"sched.shadow.realized","trace":"dup","served_by":"local_fallback"}"#,
        ];
        let report = join_shadow_telemetry(lines);
        assert_eq!(report.rows.len(), 2);
        assert!(report.rows.iter().all(|r| r.has_plan));
        assert_eq!(report.stats.realized_only, 0);
    }
}
