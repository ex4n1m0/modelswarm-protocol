# Phase E Acceptance Tests — Multi-Proposer Candidate Trees

Gate (revision): four-peer mode beats two-peer mode on at least one declared
workload without violating correctness — **or** it is archived as a negative
experiment and P3/D-mode retained. Evidence → `docs/verification/phase-e.md`.

- **E1** Branch assignment is deterministic and non-overlapping: proposer i
  receives a reproducible branch/seed set from (session_id, round, roster);
  same inputs → same assignment.
- **E2** Candidate trie assembly is bounded: max nodes and max branches per
  round enforced; exceeding the bound prunes (lowest-score first), never
  grows unbounded.
- **E3** Duplicate token prefixes across proposers are deduplicated before
  verification (counted as `duplicate_work` in records).
- **E4** Verification: batch/tree verification used when the runtime
  declares `tree_attention`/`batch_verify`; otherwise graceful sequential
  fallback. Both paths produce identical accepted prefixes (tested).
- **E5** Straggler drop: a proposer missing the round deadline is excluded
  without blocking the verification round; the session continues from the
  remaining candidates.
- **E6** Losing branches are cancelled immediately after commit; capacity
  released (no leaked slots).
- **E7** Correctness: greedy tree mode remains token-identical to plain
  decoding across the D1 corpus; sampled mode passes the same distribution
  contract as D2.
- **E8** Records: breadth/depth, acceptance, verification cost, duplicate
  work, bytes per accepted token, wall time per ADR-13 schema.
- **E9** The comparative gate runs against the fastest-single and best
  two-peer records (mock backend = TEST-ONLY labels; real claim needs the
  research runtime per phase-d §D8 posture).
