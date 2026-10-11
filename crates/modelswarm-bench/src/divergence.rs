//! Cross-peer greedy divergence detection — the accuracy-benefits study's
//! structural instrument (owner directive 2026-10-11 at ADR-032
//! acceptance: "many is better than one" benefits beyond latency, some or
//! any form of accuracy welcome).
//!
//! # What this instrument is (and is not)
//!
//! Two same-backend peers hosting the exact same profile decode the same
//! prompt under the same pinned greedy sampling parameters; their token
//! streams are compared delta-by-delta. Under E0's findings
//! (`docs/verification/e0-determinism-2026-10-07.md`) greedy decoding is
//! deterministic within a backend family (CPU across thread counts,
//! Vulkan across GPU vendors), so honest same-backend peers MUST agree
//! token-for-token. A divergence therefore flags one of: a nondeterminism
//! regression, a hostile/misbehaving peer, or a hardware fault — a
//! **diagnosable-accuracy benefit the swarm can deliver without changing
//! a single output token** (output-preserving; compatible with the
//! lossless `speculative_exact` contract, which is not weakened).
//!
//! This module is pure comparison + ledger arithmetic (default features,
//! per-push CI). The wire driving lives with the `quic-runner` harness
//! (`tests/accuracy.rs`); the ADR-032 §3 production audit path (the
//! `verifier_divergence` telemetry event on the future `VerifyDrafts`
//! family) is that ADR's surface, not this one — nothing here invents a
//! wire field.
//!
//! # Honesty rules for the artifacts built from this module
//!
//! - Verdicts carry positions, counts, timings, and stream DIGESTS only —
//!   never token text (AGENTS.md rule 5 discipline; token ids/deltas are
//!   completion content in encoded form).
//! - Detection claims are measured against an INJECTED fault model (the
//!   synthetic executor's `divergent` mode) and a clean control cell;
//!   false positives on the clean cell and false negatives on the
//!   injected cell are reported as failures of the instrument.
//! - The synthetic executor is deterministic BY CONSTRUCTION, so a
//!   synthetic run is STRUCTURAL evidence (protocol machinery), never an
//!   engine-divergence-rate claim; real-engine rates need the real models
//!   and are labeled separately in the experiment artifacts.

use std::collections::BTreeMap;

use modelswarm_scheduler::EWMA_ALPHA;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

/// `record_type` of one peer-vs-reference comparison row
/// (`divergence-check/v1`, self-describing JSONL — NOT a frozen-schema
/// mode-result record; the frozen `experiments/schemas/` set is untouched).
pub const DIVERGENCE_CHECK_RECORD_TYPE: &str = "divergence-check/v1";
/// `record_type` of the experiment manifest (`accuracy-manifest/v1`).
pub const ACCURACY_MANIFEST_RECORD_TYPE: &str = "accuracy-manifest/v1";
/// `record_type` of the cohort de-rank demonstration (`cohort-derank/v1`).
pub const COHORT_DERANK_RECORD_TYPE: &str = "cohort-derank/v1";

/// De-rank policy: greedy same-backend peers must agree token-for-token,
/// so the deterministic heuristic suspends a peer from cohort candidacy
/// after its FIRST recorded divergence (no recovery policy yet — the same
/// honest limitation the engaged-loss engage gate records; a reviewed
/// decay/probe policy is future work, and any learned policy waits for
/// production evidence per the standing rule).
pub const SUSPEND_AFTER_DIVERGENCES: u64 = 1;

/// Verdict of comparing one probe stream against a reference stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckVerdict {
    /// Token-equal for every position (lengths included).
    Agree,
    /// At least one position differs (or one stream ended earlier — a
    /// stream that stops is not equal to one that continues).
    Diverge,
}

/// First index at which two delta streams disagree, or `None` when they
/// are token-equal including length. A length difference is a divergence
/// at the shorter stream's end (conservative: greedy continuations of the
/// same conversation with the same budget must have the same shape).
#[must_use]
pub fn first_divergence(reference: &[String], probe: &[String]) -> Option<usize> {
    let common = reference.len().min(probe.len());
    for (position, (a, b)) in reference[..common].iter().zip(&probe[..common]).enumerate() {
        if a != b {
            return Some(position);
        }
    }
    if reference.len() == probe.len() {
        None
    } else {
        Some(common)
    }
}

/// 16-hex sha256 prefix of a concatenated delta stream (the E0
/// determinism record's comparison discipline). A digest, never content:
/// safe to publish in artifacts while remaining comparable across runs.
#[must_use]
pub fn stream_digest(deltas: &[String]) -> String {
    let mut hasher = Sha256::new();
    for delta in deltas {
        hasher.update(delta.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

/// Per-peer agreement state (deterministic counters + the frozen EWMA
/// family; `EWMA_ALPHA` is the same estimator family the scheduler uses —
/// no new tunable).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AgreementState {
    /// Comparisons recorded against this peer.
    pub checks: u64,
    /// Comparisons that diverged.
    pub divergences: u64,
    /// EWMA of the 1/0 agreement signal (first observation seeds it).
    pub agreement_ewma: f64,
}

impl AgreementState {
    fn new() -> Self {
        Self {
            checks: 0,
            divergences: 0,
            agreement_ewma: 0.0,
        }
    }

    fn record(&mut self, agreed: bool) {
        let signal = f64::from(u8::from(agreed));
        self.agreement_ewma = if self.checks == 0 {
            signal
        } else {
            EWMA_ALPHA * signal + (1.0 - EWMA_ALPHA) * self.agreement_ewma
        };
        self.checks += 1;
        if !agreed {
            self.divergences += 1;
        }
    }

    /// Whether the deterministic de-rank policy suspends this peer from
    /// cohort candidacy ([`SUSPEND_AFTER_DIVERGENCES`]).
    #[must_use]
    pub fn suspended(&self) -> bool {
        self.divergences >= SUSPEND_AFTER_DIVERGENCES
    }
}

/// The agreement ledger: one [`AgreementState`] per peer id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgreementLedger {
    entries: BTreeMap<String, AgreementState>,
}

impl AgreementLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one comparison verdict against a peer.
    pub fn record(&mut self, peer_id: &str, agreed: bool) {
        self.entries
            .entry(peer_id.to_string())
            .or_insert_with(AgreementState::new)
            .record(agreed);
    }

    /// The peer's state, when any check was recorded against it.
    #[must_use]
    pub fn state(&self, peer_id: &str) -> Option<&AgreementState> {
        self.entries.get(peer_id)
    }

    /// Whether the peer is suspended from cohort candidacy by the
    /// deterministic de-rank policy (peers with no checks are not
    /// suspended — suspension is evidence-driven, not prior-driven).
    #[must_use]
    pub fn suspended(&self, peer_id: &str) -> bool {
        self.state(peer_id).is_some_and(AgreementState::suspended)
    }

    /// Serializable snapshot (peer ids + states only, never token text).
    #[must_use]
    pub fn snapshot(&self) -> Vec<(String, AgreementState)> {
        self.entries
            .iter()
            .map(|(peer, state)| (peer.clone(), *state))
            .collect()
    }
}

/// The quorum outcome: a reference-free divergence read over the whole
/// pool (useful when no trusted verifier exists — the majority IS the
/// reference).
#[derive(Debug, Clone, PartialEq)]
pub struct QuorumOutcome {
    /// The per-position strict-majority stream, up to the first position
    /// where no strict majority exists.
    pub reference: Vec<String>,
    /// Pool indexes that deviated from the majority, with the first
    /// deviating position each.
    pub divergent: Vec<(usize, usize)>,
    /// Positions remained majority-decidable for the whole max length.
    pub decisive: bool,
}

/// Reference-free quorum read: at each delta position the strict majority
/// text (> half the streams that reached that position) extends the
/// reference; a stream that lacks a strict majority marks the quorum
/// inconclusive from there. Streams are indexed by their pool position —
/// the caller maps indexes to peer ids.
#[must_use]
pub fn quorum(streams: &[Vec<String>]) -> QuorumOutcome {
    let max_len = streams.iter().map(Vec::len).max().unwrap_or(0);
    let mut reference = Vec::with_capacity(max_len);
    let mut divergent: Vec<(usize, usize)> = Vec::new();
    let mut decisive = true;
    for position in 0..max_len {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut voters = 0usize;
        for stream in streams {
            if let Some(delta) = stream.get(position) {
                *counts.entry(delta.as_str()).or_insert(0) += 1;
                voters += 1;
            }
        }
        let Some((&majority_text, &majority_count)) = counts.iter().max_by_key(|(_, &c)| c) else {
            decisive = false;
            break;
        };
        if majority_count * 2 <= voters {
            // No strict majority (a tie is not evidence).
            decisive = false;
            break;
        }
        for (index, stream) in streams.iter().enumerate() {
            let deviates = match stream.get(position) {
                Some(delta) => delta != majority_text,
                None => true, // ended early: not equal to the majority continuation
            };
            if deviates {
                let entry = (index, position);
                if let Some(existing) = divergent.iter_mut().find(|(i, _)| *i == index) {
                    existing.1 = existing.1.min(position);
                } else {
                    divergent.push(entry);
                }
            }
        }
        reference.push(majority_text.to_string());
    }
    QuorumOutcome {
        reference,
        divergent,
        decisive,
    }
}

// ---------------------------------------------------------------------------
// Self-describing artifact records (NOT frozen-schema mode-results)
// ---------------------------------------------------------------------------

/// One comparison row: probe stream vs reference stream for one check
/// (one check = one prompt x one seed, decoded on the whole pool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DivergenceCheckRecord {
    /// Always [`DIVERGENCE_CHECK_RECORD_TYPE`].
    pub record_type: String,
    /// Unique check id (`<cell>/<prompt>/<seed>`).
    pub check_id: String,
    /// Cell tag (`clean` / `injected` / …).
    pub cell: String,
    /// Environment label (loopback / LAN — never a WAN claim).
    pub environment: String,
    /// Harness version string (commit-anchored).
    pub harness_version: String,
    /// Prompt corpus id.
    pub corpus_id: String,
    /// Stable prompt id inside the corpus.
    pub prompt_id: String,
    /// The pinned sampling seed all peers decoded under.
    pub seed: u64,
    /// `"peer:<peer-id>"` or `"quorum-majority"`.
    pub reference_kind: String,
    /// The peer whose stream was compared.
    pub probe_peer: String,
    /// The comparison verdict.
    pub verdict: CheckVerdict,
    /// First differing delta position (None on agree).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_position: Option<u32>,
    /// Tokens compared (the common length; the reference length on agree).
    pub checked_tokens: u32,
    /// sha256-16 digest of the reference stream (never the text).
    pub reference_stream_sha256_16: String,
    /// sha256-16 digest of the probe stream.
    pub probe_stream_sha256_16: String,
    /// Reference completion wall time (ms).
    pub reference_total_ms: f64,
    /// Probe completion wall time (ms).
    pub probe_total_ms: f64,
    /// min(reference, probe) — the redundant-racing availability signal
    /// from the SAME completions (no extra requests).
    pub raced_min_total_ms: f64,
    /// Whole-check wall time (ms): one decode on every pool peer,
    /// concurrently.
    pub check_wall_ms: f64,
}

/// One pool peer as pinned in the experiment manifest (roster facts +
/// synthetic speeds; the driver and the serve side share the definition).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AccuracyPoolPeer {
    /// Pool index (0 = the verifier-class reference peer).
    pub index: usize,
    /// Bridge config seed (deterministic identity derivation).
    pub seed: u64,
    /// Synthetic decode rate (tokens/ms).
    pub decode_tokens_per_ms: f64,
    /// Synthetic prefill rate (tokens/ms).
    pub prefill_tokens_per_ms: f64,
    /// Advertised queue backlog (ms; roster input).
    pub advertised_queue_ms: u64,
    /// Roster capacity class.
    pub capacity_class: String,
    /// Whether the serve side injected the divergence fault model.
    pub divergent: bool,
}

/// The experiment manifest for one accuracy-divergence cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AccuracyManifest {
    /// Always [`ACCURACY_MANIFEST_RECORD_TYPE`].
    pub record_type: String,
    /// Cell tag.
    pub cell: String,
    /// Environment label.
    pub environment: String,
    /// Harness version string (commit-anchored).
    pub harness_version: String,
    /// Executor label (`synthetic-token-executor` for structural runs).
    pub executor: String,
    /// The pinned pool (both sides derive identity from these seeds).
    pub pool: Vec<AccuracyPoolPeer>,
    /// Prompt corpus id.
    pub corpus_id: String,
    /// Prompt ids checked.
    pub prompt_ids: Vec<String>,
    /// Sampling seeds checked (each decoded on the whole pool).
    pub seeds: Vec<u64>,
    /// Output budget per decode.
    pub max_tokens: u32,
    /// Reference policy description.
    pub reference_policy: String,
    /// Free-text ops note (machines, commands; no prompts, no content).
    pub notes: String,
}

/// The cohort de-rank demonstration record: what selection chose before
/// and after the ledger's suspensions (deterministic evidence that
/// detection changes roster composition, not just a log line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CohortDerankRecord {
    /// Always [`COHORT_DERANK_RECORD_TYPE`].
    pub record_type: String,
    /// Cell tag.
    pub cell: String,
    /// Environment label.
    pub environment: String,
    /// Peers the ledger suspended (divergence-detected).
    pub suspended_peers: Vec<String>,
    /// Cohort selected WITHOUT detection (pool order, fastest first).
    pub cohort_before: Vec<String>,
    /// Cohort selected WITH detection (suspended peers removed first).
    pub cohort_after: Vec<String>,
    /// Selection rule description.
    pub selection: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn identical_streams_agree_everywhere() {
        let a = stream(&["w1", "w2", "w3"]);
        assert_eq!(first_divergence(&a, &a), None);
    }

    #[test]
    fn divergence_position_is_the_first_differing_index() {
        let a = stream(&["w1", "w2", "w3"]);
        let head = stream(&["w1", "w2", "wX"]);
        assert_eq!(first_divergence(&a, &head), Some(2));
        let immediate = stream(&["wX", "w2", "w3"]);
        assert_eq!(first_divergence(&a, &immediate), Some(0));
        // A divergent synthetic executor disagrees from the first token;
        // the instrument must not miss token-0 divergence.
        let divergent_model = stream(&["w0", "w2", "w3"]);
        assert_eq!(first_divergence(&a, &divergent_model), Some(0));
    }

    #[test]
    fn length_difference_is_a_divergence_at_the_shorter_end() {
        let full = stream(&["w1", "w2", "w3"]);
        let truncated = stream(&["w1", "w2"]);
        assert_eq!(first_divergence(&full, &truncated), Some(2));
        assert_eq!(first_divergence(&truncated, &full), Some(2));
        assert_eq!(first_divergence(&full, &Vec::new()), Some(0));
        assert_eq!(first_divergence(&Vec::new(), &Vec::new()), None);
    }

    #[test]
    fn stream_digest_is_stable_and_content_sensitive() {
        let a = stream(&["w1", "w2"]);
        let b = stream(&["w1", "w2"]);
        let c = stream(&["w1", "w3"]);
        assert_eq!(stream_digest(&a), stream_digest(&b));
        assert_ne!(stream_digest(&a), stream_digest(&c));
        assert_eq!(stream_digest(&a).len(), 16, "16-hex prefix");
        // Delimiter sensitivity: [ab] is not [a, b].
        assert_ne!(
            stream_digest(&stream(&["ab"])),
            stream_digest(&stream(&["a", "b"]))
        );
    }

    #[test]
    fn ledger_suspends_on_first_divergence_and_tracks_the_ewma() {
        let mut ledger = AgreementLedger::new();
        for _ in 0..5 {
            ledger.record("honest", true);
        }
        ledger.record("faulty", true);
        ledger.record("faulty", false);
        let honest = ledger.state("honest").expect("recorded");
        assert_eq!(honest.checks, 5);
        assert_eq!(honest.divergences, 0);
        assert!((honest.agreement_ewma - 1.0).abs() < 1e-12);
        assert!(!ledger.suspended("honest"));
        let faulty = ledger.state("faulty").expect("recorded");
        // EWMA after [1.0, 0.0]: 0.3*0 + 0.7*1 = 0.7.
        assert!((faulty.agreement_ewma - 0.7).abs() < 1e-12);
        assert_eq!(faulty.divergences, 1);
        assert!(ledger.suspended("faulty"), "suspended after 1 divergence");
        // Unknown peers are not suspended (evidence-driven, not prior).
        assert!(!ledger.suspended("never-checked"));
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.iter().all(|(peer, _)| peer != "token text"));
    }

    #[test]
    fn quorum_detects_a_single_divergent_minority() {
        let honest = stream(&["a", "b", "c"]);
        let streams = vec![
            honest.clone(),
            honest.clone(),
            stream(&["X", "b", "c"]),
            honest.clone(),
        ];
        let outcome = quorum(&streams);
        assert!(outcome.decisive, "3-of-4 majority at every position");
        assert_eq!(outcome.reference, honest);
        assert_eq!(outcome.divergent, vec![(2, 0)], "peer 2 diverged at 0");
    }

    #[test]
    fn quorum_flags_early_enders_and_stays_decisive() {
        let honest = stream(&["a", "b", "c"]);
        let streams = vec![
            honest.clone(),
            honest.clone(),
            stream(&["a", "b"]), // ended early: diverges at 2
            honest,
        ];
        let outcome = quorum(&streams);
        assert!(outcome.decisive);
        assert_eq!(outcome.divergent, vec![(2, 2)]);
    }

    #[test]
    fn quorum_ties_are_inconclusive_not_evidence() {
        let block_a = stream(&["a", "a"]);
        let block_b = stream(&["b", "b"]);
        // 2 vs 2: no strict majority at position 0.
        let split = quorum(&[block_a.clone(), block_b.clone(), block_a, block_b]);
        assert!(!split.decisive);
        assert!(split.reference.is_empty());
        assert!(split.divergent.is_empty(), "a tie suspends nobody");
    }

    #[test]
    fn quorum_on_all_agreeing_streams_has_no_divergent_peers() {
        let honest = stream(&["a", "b", "c", "d"]);
        let outcome = quorum(&[honest.clone(), honest.clone(), honest.clone()]);
        assert!(outcome.decisive);
        assert_eq!(outcome.reference, honest);
        assert!(outcome.divergent.is_empty());
        // Empty pool: decisive with nothing to say.
        let empty = quorum(&[]);
        assert!(empty.decisive);
        assert!(empty.reference.is_empty());
    }

    #[test]
    fn check_records_serialize_without_token_text() {
        let record = DivergenceCheckRecord {
            record_type: DIVERGENCE_CHECK_RECORD_TYPE.to_string(),
            check_id: "clean/p1-short/7".to_string(),
            cell: "clean".to_string(),
            environment: "loopback-quic".to_string(),
            harness_version: "test".to_string(),
            corpus_id: "synthetic-pass1-v1".to_string(),
            prompt_id: "p1-short".to_string(),
            seed: 7,
            reference_kind: "peer:12D3KooEXAMPLE".to_string(),
            probe_peer: "12D3KooPROBE".to_string(),
            verdict: CheckVerdict::Diverge,
            divergence_position: Some(0),
            checked_tokens: 16,
            reference_stream_sha256_16: stream_digest(&stream(&["a"])),
            probe_stream_sha256_16: stream_digest(&stream(&["b"])),
            reference_total_ms: 100.0,
            probe_total_ms: 110.0,
            raced_min_total_ms: 100.0,
            check_wall_ms: 115.0,
        };
        let json = serde_json::to_value(&record).expect("serializes");
        assert_eq!(json["record_type"], DIVERGENCE_CHECK_RECORD_TYPE);
        assert_eq!(json["verdict"], "diverge");
        assert_eq!(json["divergence_position"], 0);
        // Privacy rule 5: no token text in any serialized form — the
        // struct has no text field, and digests are 16-hex.
        let text = json.to_string();
        assert!(!text.contains("\"w"), "no synthetic token text leaks");
        let round: DivergenceCheckRecord = serde_json::from_value(json).expect("round-trips");
        assert_eq!(round, record);
        // Agree rows omit the position (None, not 0).
        let agree = DivergenceCheckRecord {
            verdict: CheckVerdict::Agree,
            divergence_position: None,
            ..record
        };
        let agree_json = serde_json::to_value(&agree).expect("serializes");
        assert!(agree_json.get("divergence_position").is_none());
    }

    #[test]
    fn manifest_and_derank_records_carry_their_record_types() {
        let manifest = AccuracyManifest {
            record_type: ACCURACY_MANIFEST_RECORD_TYPE.to_string(),
            cell: "clean".to_string(),
            environment: "loopback-quic".to_string(),
            harness_version: "test".to_string(),
            executor: "synthetic-token-executor".to_string(),
            pool: vec![AccuracyPoolPeer {
                index: 0,
                seed: 0x51,
                decode_tokens_per_ms: 0.10,
                prefill_tokens_per_ms: 3.0,
                advertised_queue_ms: 0,
                capacity_class: "gpu_mid".to_string(),
                divergent: false,
            }],
            corpus_id: crate::corpus::PASS1_CORPUS_ID.to_string(),
            prompt_ids: vec!["p1-short".to_string()],
            seeds: vec![7],
            max_tokens: 16,
            reference_policy: "bridge-0 verifier-reference + quorum".to_string(),
            notes: "unit".to_string(),
        };
        let json = serde_json::to_value(&manifest).expect("serializes");
        assert_eq!(json["record_type"], ACCURACY_MANIFEST_RECORD_TYPE);
        let derank = CohortDerankRecord {
            record_type: COHORT_DERANK_RECORD_TYPE.to_string(),
            cell: "injected".to_string(),
            environment: "loopback-quic".to_string(),
            suspended_peers: vec!["12D3KooFAULTY".to_string()],
            cohort_before: vec!["v".to_string(), "faulty".to_string(), "d".to_string()],
            cohort_after: vec!["v".to_string(), "d".to_string()],
            selection: "select_microswarm(want=3)".to_string(),
        };
        let json = serde_json::to_value(&derank).expect("serializes");
        assert_eq!(json["record_type"], COHORT_DERANK_RECORD_TYPE);
    }
}
