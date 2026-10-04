# Fuzz & Adversarial Test Targets (design in Phase A; grow per phase)

Owner: Security Engineer · Threat model: `docs/threat-model.md` (extended
below) · Rule: fuzzers run in CI from Phase B onward for decoders.

## Decoder fuzz targets (Phase B)

- Canonical-JSON parser: arbitrary bytes → must reject or produce canonical
  round-trip-identical output; no panics, no unbounded allocation.
- Manifest deserializer: mutated `protocol/vectors` corpora (bit flips,
  truncation, key reordering, duplicate keys, deep nesting) → schema
  violations, never type confusion.
- Envelope/lease signature decoders: wrong lengths, non-canonical encodings,
  high-bit garbage.

## Property targets (Phase B gate, ADR-015)

Session-state machine: stale / duplicated / reordered / cross-profile /
cross-epoch messages can never advance state. Commit idempotence: replaying a
signed commit N times yields exactly one state advance.

## Adversarial scenarios (Phases D–F; from the revision's threat list)

1. Fabricated token proposals with plausible logprobs (verifier must catch;
   acceptance contract holds).
2. Lying about proposal probabilities → verification prevents output
   corruption; liar's receipts accumulate evidence → suspension policy.
3. Prefix equivocation: two peers claiming different prefixes for the same
   round → reject both, flag.
4. Replay of old commits/proposals across sessions and reconnects.
5. Withholding results after prefill (resource theft) → deadlines, slot
   release, reputation cost.
6. Latency-measurement manipulation → scheduler uses its own measurements
   (requester-measured RTT dominates; ADR-013).
7. Coordinator/verifier abuse: malicious verifier accepting garbage →
   auditor re-verification of sampled rounds catches divergence; Byzantine
   collusion assumptions documented per round, not assumed away.
8. Flood: registration, lookup, session-open storms → rate limits (msp-v1
   §3.4) + per-IP caps (Phase-1 acceptance F5–F7).
9. Malformed frames / oversized payloads at every P2P entry point.
10. Prompt-retention attempts by serving peers (logging assertions on both
    sides; redaction gates from phase acceptance index).

## Threat-model addendum (cooperative adversaries)

Add to `docs/threat-model.md` adversary set when Phase D begins: **H.
Byzantine swarm participant** (fabricates proposals, equivocates on prefixes,
colludes with other participants or a verifier, withholds work, attempts
prompt extraction/retention). Mitigations: per-message prefix/profile/
generation-param binding, random redundant verification, audit epochs,
receipt evidence, quarantine scoring — each priced for performance cost per
the revision's requirement to measure security overhead.
