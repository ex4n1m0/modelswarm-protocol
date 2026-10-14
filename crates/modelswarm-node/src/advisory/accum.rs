//! The typed in-process accumulators (ADR-033 §3.2, RC-4) — the ONLY
//! aggregation source for a digest cycle.
//!
//! Log sinks (JSONL or otherwise) are NEVER parsed as advisory input:
//! the telemetry emitter escapes only `\` and `"` today, so a forged log
//! line could otherwise skew counts, timings, and verdict tallies. The
//! accumulation input set is the enumerated event-type allowlist frozen
//! in [`EGRESS_ARTIFACT`][crate::advisory::egress::EGRESS_ARTIFACT] and
//! it EXCLUDES every `advisory.*` row — the plane's own telemetry never
//! feeds the next cycle's digest.
//!
//! Everything here is counts, integer min/median/max ms, and frozen-
//! vocabulary tallies (counts/labels only). Producing components feed
//! these at emit time; this skeleton owns the typed shape and the
//! digest rendering (the honest sentinels `none`/`unmeasured`/`unknown`
//! map to null on every timing field).

use std::collections::BTreeMap;

use super::egress::{self, DigestField, TypedValue};

/// Cap on retained timing samples per metric (the median needs samples;
/// a bounded ring keeps hostile floods memory-bounded while the
/// min/max/median stay representative).
const MAX_SAMPLES: usize = 1024;

/// One timing aggregate: integer ms only (the float-formatted-timings
/// reduction rule — accumulators emit rounded ms, never `"{:.1}"`
/// strings).
#[derive(Debug, Clone, Default)]
pub struct TimingMs {
    pub samples: Vec<u64>,
}

impl TimingMs {
    fn observe(&mut self, ms: u64) {
        if self.samples.len() == MAX_SAMPLES {
            self.samples.remove(0); // bounded ring
        }
        self.samples.push(ms);
    }
    /// min / median / max as integers; `None` = unmeasured (renders as
    /// null per §3.2's sentinel rule).
    pub fn min_median_max(&self) -> Option<(u64, u64, u64)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let n = sorted.len();
        let median = sorted[n / 2];
        Some((*sorted.first().unwrap(), median, *sorted.last().unwrap()))
    }
}

/// The typed accumulators. Feed via the typed APIs only — there is no
/// stringly entry point by construction.
#[derive(Debug, Clone, Default)]
pub struct Accumulators {
    /// Event-type-keyed counters over the frozen aggregation allowlist.
    counters: BTreeMap<String, u64>,
    /// Integer-ms timing aggregates per metric name.
    timings: BTreeMap<String, TimingMs>,
    /// Frozen-vocabulary tallies: vocabulary name → value → count.
    tallies: BTreeMap<String, BTreeMap<String, u64>>,
    /// Events offered but refused (not on the allowlist) — counted,
    /// never aggregated.
    refused: u64,
}

impl Accumulators {
    /// Counts one telemetry event. Returns `false` (and counts a refuse)
    /// when the event type is not on the frozen aggregation allowlist —
    /// the plane's own `advisory.*` rows are excluded by that same list
    /// (RC-4).
    pub fn record_event(&mut self, event: &str) -> bool {
        if egress::vocabulary()
            .aggregation
            .events
            .iter()
            .any(|e| e == event)
        {
            *self.counters.entry(event.to_string()).or_insert(0) += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }

    /// Observes one integer-ms timing sample. The metric NAME is
    /// allowlisted against the frozen §3.2 reduction-table timing row —
    /// a hostile string offered as a metric cannot become a digest
    /// field name (free-form text never reaches the digest, under any
    /// name, in any position).
    pub fn observe_ms(&mut self, metric: &str, ms: u64) -> bool {
        if !egress::vocabulary().timing_metrics().contains(&metric) {
            self.refused += 1;
            return false;
        }
        self.timings
            .entry(metric.to_string())
            .or_default()
            .observe(ms);
        true
    }

    /// Tallies one frozen-vocabulary value (verdict tallies, gap
    /// distributions, reject codes). An unknown vocabulary OR a value
    /// outside the frozen set is refused (counted) — values are frozen,
    /// never free strings.
    pub fn tally(&mut self, vocabulary: &str, value: &str) -> bool {
        let known = egress::vocabulary()
            .vocabularies
            .get(vocabulary)
            .is_some_and(|set| set.values.iter().any(|v| v == value));
        if !known {
            self.refused += 1;
            return false;
        }
        *self
            .tallies
            .entry(vocabulary.to_string())
            .or_default()
            .entry(value.to_string())
            .or_insert(0) += 1;
        true
    }

    /// Total allowlisted events observed (the `events_threshold` input).
    pub fn event_count(&self) -> u64 {
        self.counters.values().sum()
    }

    /// Renders the digest payload: everything reduces to aggregates only
    /// — counts, integer min/median/max ms, verdict tallies, all
    /// value-typed for the egress gate. Names are `event.count` /
    /// `metric.{min,median,max}_ms` / `vocabulary.value` (dot-joined);
    /// unmeasured timings are omitted (the null sentinel rendering lands
    /// with full emit-site wiring, where the declared field set exists).
    pub fn snapshot(&self) -> Vec<DigestField> {
        let mut fields = Vec::new();
        for (event, count) in &self.counters {
            fields.push(DigestField {
                name: format!("{event}.count"),
                value: TypedValue::Int(i64::try_from(*count).unwrap_or(i64::MAX)),
            });
        }
        for (metric, timing) in &self.timings {
            if let Some((min, median, max)) = timing.min_median_max() {
                for (suffix, v) in [("min_ms", min), ("median_ms", median), ("max_ms", max)] {
                    fields.push(DigestField {
                        name: format!("{metric}.{suffix}"),
                        value: TypedValue::Int(v as i64),
                    });
                }
            }
        }
        for (vocabulary, values) in &self.tallies {
            for (value, count) in values {
                fields.push(DigestField {
                    name: format!("{vocabulary}.{value}"),
                    value: TypedValue::Int(i64::try_from(*count).unwrap_or(i64::MAX)),
                });
            }
        }
        fields
    }

    /// The count of refused (non-allowlisted) offers — a diagnostic
    /// count, never aggregated content.
    pub fn refused_count(&self) -> u64 {
        self.refused
    }

    /// Resets the aggregates after a cycle (called by the runner when a
    /// call is initiated — "aggregate since last cycle").
    pub fn reset(&mut self) {
        self.counters.clear();
        self.timings.clear();
        self.tallies.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregation_allowlist_excludes_advisory_rows() {
        let mut acc = Accumulators::default();
        assert!(acc.record_event("serving.session_ended"));
        assert!(
            !acc.record_event("advisory.cycle.gap"),
            "RC-4: the plane's own telemetry never feeds a digest"
        );
        assert!(!acc.record_event("totally.unknown.event"));
        assert_eq!(acc.event_count(), 1);
        assert_eq!(acc.refused_count(), 2);
    }

    #[test]
    fn timings_are_integer_ms_aggregates() {
        let mut acc = Accumulators::default();
        assert!(acc.observe_ms("probe_p50_ms", 120));
        assert!(acc.observe_ms("probe_p50_ms", 80));
        assert!(acc.observe_ms("probe_p50_ms", 200));
        // The frozen timing-metric set is EXACTLY the §3.2 reduction
        // row — and a hostile metric NAME is refused (it must never
        // become a digest field name).
        assert_eq!(
            egress::vocabulary().timing_metrics(),
            vec![
                "probe_p50_ms",
                "ttft_ewma_ms",
                "total_ewma_ms",
                "pick_predicted_ms"
            ]
        );
        assert!(!acc.observe_ms("{\"prompt\":\"inject\"}", 1));
        assert!(!acc.observe_ms("not json at all", 0));
        let fields = acc.snapshot();
        let get = |n: &str| {
            fields
                .iter()
                .find(|f| f.name == n)
                .map(|f| f.value.clone())
                .unwrap()
        };
        assert_eq!(get("probe_p50_ms.min_ms"), TypedValue::Int(80));
        assert_eq!(get("probe_p50_ms.median_ms"), TypedValue::Int(120));
        assert_eq!(get("probe_p50_ms.max_ms"), TypedValue::Int(200));
        // Unmeasured metric: omitted entirely (null sentinel discipline
        // rides the declared field set at full wiring).
        assert!(!fields.iter().any(|f| f.name.starts_with("ttft_ewma_ms")));
    }

    #[test]
    fn tallies_accept_frozen_vocabularies_only() {
        let mut acc = Accumulators::default();
        assert!(acc.tally("advisory_gap", "timeout"));
        assert!(acc.tally("advisory_gap", "timeout"));
        assert!(acc.tally("advisory_gap", "http_error"));
        assert!(!acc.tally("not_a_vocabulary", "x"));
        assert!(!acc.tally("advisory_gap", "not_a_gap_value"));
        let snapshot = acc.snapshot();
        // Tally VALUES are frozen-vocabulary members used as name
        // fragments; the VALUES emitted are counts (integers) —
        // labels-only, counts-only.
        assert!(snapshot
            .iter()
            .any(|f| f.name == "advisory_gap.timeout" && f.value == TypedValue::Int(2)));
        assert!(snapshot
            .iter()
            .any(|f| f.name == "advisory_gap.http_error" && f.value == TypedValue::Int(1)));
        assert_eq!(acc.refused_count(), 2);
    }

    /// RC-4 from the attack side: an arbitrary forged JSONL line has no
    /// parse path into the accumulators — the only entry points are the
    /// typed APIs above, and the digest is built from their state alone.
    #[test]
    fn forged_jsonl_never_moves_accumulator_state() {
        let mut acc = Accumulators::default();
        acc.record_event("serving.clamped");
        let before = acc.snapshot();
        // A hostile "log line" offered through the only string-bearing
        // API (event names) is refused unless allowlisted:
        acc.record_event("serving.clamped\", \"prompt\": \"inject me");
        let after = acc.snapshot();
        assert_eq!(before.len(), after.len());
        assert_eq!(acc.refused_count(), 1);
        let body = serde_json::to_string(&egress::admit(after).body).unwrap();
        assert!(!body.contains("inject"));
    }
}
