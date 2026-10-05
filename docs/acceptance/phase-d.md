# Phase D Acceptance Tests — Exact Two-Peer Speculative Protocol

Gate (revision): greedy outputs match ordinary decoding **exactly** across
the corpus; sampled mode passes predeclared distribution tests; one real
low-RTT configuration beats the fastest single host reproducibly — **or**
the boundary is documented and we stop before Phase E (the plan's own
negative-result exit). Evidence → `docs/verification/phase-d.md`.

## Correctness (hard gates)

- **D1** Greedy equality: for ≥ 50 seeded prompts × 3 window sizes on the
  correctness oracle, `speculative_exact` output token ids == plain
  `decode_stream` output ids, token for token.
- **D2** Lossless sampled mode: rejection-sampling acceptance + recovery
  implemented (Leviathan-style); empirical distribution test over ≥ 10 000
  draws on a 3-token toy distribution (chi-square within bounds, seeded).
- **D3** Rollback: after a rejected window, state provably returns to the
  committed prefix (KV commitment digest equality), 200 randomized cases.
- **D4** Session safety over transport: prefix-hash chain + round rules
  from modelswarm-session enforced end-to-end; disconnect of proposer or
  verifier mid-round yields explicit cancellation/fallback — never
  corruption; reconnect resumes from last committed prefix.

## Protocol mechanics

- **D5** Window sweep machinery (1/2/4/8/16) runs against the oracle;
  acceptance rate + mean acceptance length (vLLM convention:
  1 + accepted/steps) recorded per window in schema-valid records.
- **D6** Fallback: injected acceptance collapse or RTT spike flips the
  planner to `single` mid-session (ADR-013 rule) — observed in records.
- **D7** Two-process E2E on loopback: proposer and verifier are separate
  OS processes via the sim harness; commits signed; receipts two-phase
  (server-signed `completed` + requester `receipt_ack`).

## The performance gate — honest posture

- **D8** The comparator machinery runs (fastest-single prediction + actual
  in every record). On the mock/loopback backend the numbers are
  TEST-ONLY. A real-hardware "beats fastest single" claim requires the
  research runtime (Phase D entry ADR) + real GGUF profile — both stop
  conditions recorded. If those remain unavailable, Phase D closes as
  *correctness-proven, performance-unproven*, which per the revision's own
  success criteria is a valid outcome, and Phase E proceeds only on
  algorithmic grounds (or is archived as the plan allows).
