# Candidate profiles

This directory holds **reviewed candidate manifests only**. It is intentionally
empty in Phase 0.

Rules (ADR-005):

1. Entries are produced exclusively by the admin resolver script (Phase 2,
   `scripts/`), which resolves the full HF commit hash, exact filename, byte
   size, SHA-256, and license evidence. Hand-written entries are forbidden.
2. No placeholder hashes, revisions, or sizes ever land here — a candidate
   that fails schema validation (`catalog/schema.json`) must not be committed.
3. Promotion `candidate → active` happens only in the hub database after
   test-node validation; this directory records what was reviewed and why
   (`<profileId>.json` + matching review note).
4. First profile target: `msp:qwen3-4b:q4_k_m:v1` — resolved from a real
   `ggml-org` Qwen3-4B-GGUF revision when Phase 2's resolver exists. The 1.7B
   and 8B siblings are added only after the 4B profile passes all acceptance
   tests.
