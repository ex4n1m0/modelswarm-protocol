# ADR-013: Execution-mode registry and scheduler cost model v2

Status: Accepted (Phase A, 2026-10-04) · Extends architecture.md §10;
freezes the telemetry vocabulary that `experiments/schemas/` implements.

## Context

The single-stream protocol (msp-v1 §6) is one execution mode among five the
revision defines. Mode selection must be explainable, honest against the
fastest eligible single host, and automatically reversible.

## Decision

### Mode registry (only `single` is default-on; others experimental flags)

| Mode | Peers | Correctness contract |
|---|---:|---|
| `single` | 1 | Ordinary target-model generation (msp-v1 §6 as-is) |
| `hedged` | 2–3 | One complete stream kept; others cancelled; no duplicate output |
| `speculative_exact` | 2–8 | Exact target distribution (greedy: token-equal; sampled: distribution-tested) |
| `search_verified` | 4–64 | Changes generation behavior by design; only for tasks with objective verifiers; labeled approximate |
| `map_reduce` | var | Application-specific aggregation; never labeled lossless |

Approximate/semantic acceptance (DSD-style relaxation) is **not** part of
any lossless mode and requires its own future ADR after `speculative_exact`
is complete and benchmarked.

### Cost model v2 (frozen formula; coefficients measured, never invented)

```text
predicted_swarm_completion
  = prefill_cost
  + proposal_cost
  + verification_cost
  + synchronization_cost
  + expected_rollback_cost
  + failure_risk_penalty

engage cooperative mode only when:
predicted_swarm_completion × (1 + confidence_margin)
  < predicted_fastest_single_completion
```

- `confidence_margin` starts at **0.15** (conservative), tunable, must be
  ≥ 0.05; the value used is recorded in every run record.
- Inputs: prompt/output token counts, measured RTT/jitter/loss/bandwidth
  (requester-measured EWMA dominates), queue depth, historical acceptance
  rate per profile, verification cost per token, session failure history.
- Re-evaluation: after prefill and every bounded round window; fallback to
  `single` is mandatory when measured acceptance or synchronization crosses
  the predicted budget.

### Telemetry vocabulary (frozen; schema in experiments/schemas/)

Per session: mode, peers, TTFT, per-token time, completion time, prompt/
output tokens, fastest-single prediction **and** actual, cooperative
prediction **and** actual, proposal window size, accepted tokens/round,
acceptance rate, rollback count, bytes per accepted token, coordinator/
proposer/verifier compute time, RTT/jitter/loss/relay status, failure and
fallback reason, exactness result.

Negative results are first-class: every experiment artifact records them;
the Scheduler Scientist role must publish them alongside wins.

## Consequences

+ One formula, honest comparator, automatic fallback — testable end to end.
+ Modes can't silently change correctness contracts.
− Real coefficients arrive only with Phases C–D benchmarks (by design).
