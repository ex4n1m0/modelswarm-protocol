//! Frozen prompt corpus for the 9.6 pass-1 harness (audit gap §6.6: "No
//! frozen real-prompt corpus — a small fixed corpus must be pinned with a
//! corpus id").
//!
//! Every arm of every cell consumes these prompts identically, in the
//! same order, so arm-vs-arm comparisons never mix prompt distributions.
//! The texts are SYNTHETIC — generated filler in the style of the Phase C
//! `synthetic-mock-v1` set; they contain no real user content and no
//! content that could identify anyone (AGENTS.md rule 5 discipline
//! applied to fixtures too).
//!
//! The committed mirror lives at
//! `experiments/manifests/prompt-corpus-synthetic-pass1-v1.json`; its
//! `digest` field must equal [`pass1_corpus_digest`] (the test at the
//! bottom of this module pins that equality, so the committed file and
//! the compiled-in set cannot drift apart silently).
//!
//! `documented_prompt_tokens` is the chars/4 estimate the shadow planner
//! uses ([`modelswarm_scheduler::shadow::ESTIMATED_CHARS_PER_TOKEN`]);
//! realized records carry the serving side's `Usage.prompt_tokens`
//! instead. Estimates parameterize predictions only.

use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

/// Corpus id recorded in every pass-1 run manifest's `prompt_corpus_id`.
pub const PASS1_CORPUS_ID: &str = "synthetic-pass1-v1";

/// One frozen prompt: stable id, synthetic text, documented estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pass1Prompt {
    /// Stable prompt id (`p1-short`, `p1-medium`, ...).
    pub id: &'static str,
    /// The synthetic prompt text (ASCII filler; no user content).
    pub text: &'static str,
    /// Documented chars/4 token estimate (prediction input, not a
    /// realized count).
    pub documented_prompt_tokens: u32,
}

/// The frozen corpus: four prompts spanning short→long so prefill terms
/// vary across the sweep without changing the prompt distribution between
/// arms. Texts are deterministic concatenations of a fixed filler line.
pub const PASS1_CORPUS: &[Pass1Prompt] = &[
    Pass1Prompt {
        id: "p1-short",
        text: "modelswarm synthetic pass-1 short prompt: summarize the following sequence of labeled measurements in one sentence; ",
        documented_prompt_tokens: 29,
    },
    Pass1Prompt {
        id: "p1-medium",
        text: "modelswarm synthetic pass-1 medium prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; summarize the trade-offs in two sentences and keep every number intact; ",
        documented_prompt_tokens: 65,
    },
    Pass1Prompt {
        id: "p1-long",
        text: "modelswarm synthetic pass-1 long prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; the sluggish amber whale drifts beneath the eager cormorant as seven clerks audit fifteen mismatched ledgers of drained, verified, exactly cancelled speculation; summarize the trade-offs in three sentences, keep every number intact, and end with the word done; ",
        documented_prompt_tokens: 112,
    },
    Pass1Prompt {
        id: "p1-xl",
        text: "modelswarm synthetic pass-1 extra-long prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; the sluggish amber whale drifts beneath the eager cormorant as seven clerks audit fifteen mismatched ledgers of drained, verified, exactly cancelled speculation; the nimble jade heron strides beside the placid ox while nine archivists index nineteen divergent folios of queued, verified, exactly reconciled attribution; summarize the trade-offs in four sentences, keep every number intact, and end with the word done; ",
        documented_prompt_tokens: 153,
    },
];

impl Pass1Prompt {
    /// The chars/4 estimate recomputed from the text — must equal
    /// `documented_prompt_tokens` (asserted by test; the documented value
    /// exists so the committed JSON can carry it without recomputation
    /// ambiguity).
    #[must_use]
    pub fn recomputed_prompt_tokens(&self) -> u32 {
        u32::try_from(self.text.len().div_ceil(4)).unwrap_or(u32::MAX)
    }
}

/// Canonical JSON of the corpus (field order fixed by the struct) — the
/// digest recorded in the committed manifest mirror and in reports.
#[must_use]
pub fn pass1_corpus_canonical_json() -> String {
    let body = serde_json::to_string(&PASS1_CORPUS).expect("static corpus serializes");
    format!("{{\"corpus_id\":\"{PASS1_CORPUS_ID}\",\"prompts\":{body}}}")
}

/// SHA-256 of the canonical JSON (64 hex).
#[must_use]
pub fn pass1_corpus_digest() -> String {
    hex::encode(Sha256::digest(pass1_corpus_canonical_json().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_ids_are_unique_and_texts_ascii() {
        let mut ids: Vec<&str> = PASS1_CORPUS.iter().map(|p| p.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "ids unique");
        assert!(n >= 3, "at least short/medium/long");
        for prompt in PASS1_CORPUS {
            assert!(!prompt.text.is_empty());
            assert!(prompt.text.is_ascii(), "{} must be ascii", prompt.id);
            assert!(prompt.text.starts_with("modelswarm synthetic pass-1"));
        }
    }

    #[test]
    fn documented_tokens_match_chars_div_ceil_4() {
        for prompt in PASS1_CORPUS {
            assert_eq!(
                prompt.recomputed_prompt_tokens(),
                prompt.documented_prompt_tokens,
                "{}: documented estimate drifted from chars/4",
                prompt.id
            );
        }
    }

    #[test]
    fn digest_is_stable_and_hex64() {
        let digest = pass1_corpus_digest();
        assert_eq!(digest.len(), 64);
        assert!(digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(pass1_corpus_digest(), digest, "deterministic");
        // Changing the corpus text must change the digest (guard against
        // accidental edits): a different corpus string digests differently.
        let other = hex::encode(Sha256::digest(b"{\"corpus_id\":\"other\",\"prompts\":[]}"));
        assert_ne!(digest, other);
    }

    #[test]
    fn committed_manifest_mirror_matches_compiled_corpus() {
        // The committed mirror at experiments/manifests/ is part of the
        // reproducibility chain: its digest field must equal the
        // compiled-in digest. Repo-relative from the crate dir.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/manifests/prompt-corpus-synthetic-pass1-v1.json"
        );
        let Ok(raw) = std::fs::read_to_string(path) else {
            panic!("committed corpus mirror missing: {path}");
        };
        let mirror: serde_json::Value = serde_json::from_str(&raw).expect("mirror is valid JSON");
        assert_eq!(mirror["corpus_id"], PASS1_CORPUS_ID);
        assert_eq!(mirror["digest"], pass1_corpus_digest());
        // The mirror may carry annotation fields; the load-bearing triple
        // (id, text, documented tokens) must match the compiled-in set
        // element-for-element.
        let mirror_prompts = mirror["prompts"].as_array().expect("prompts array");
        assert_eq!(mirror_prompts.len(), PASS1_CORPUS.len());
        for (mirror, compiled) in mirror_prompts.iter().zip(PASS1_CORPUS) {
            assert_eq!(mirror["id"], compiled.id);
            assert_eq!(mirror["text"], compiled.text);
            assert_eq!(
                mirror["documented_prompt_tokens"],
                compiled.documented_prompt_tokens
            );
        }
    }
}
