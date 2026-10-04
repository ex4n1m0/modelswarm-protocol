# ADR-009: Revised positioning — micro-swarm cooperative inference

Status: Accepted (Phase A, 2026-10-04) · Supersedes the marketplace-first
framing of `docs/build-plan.md`; amends nothing in the core eligibility rule.

## Context

The competitive-learnings review (`docs/competitive-revision-plan.md`) shows
the tracker-plus-marketplace concept is not differentiating by itself:
p2ptokens already ships Rust/libp2p/Tauri peers that serve and consume
inference behind a ratio economy with signed co-receipts and a content-blind
coordinator. What no audited system does is cooperatively accelerate **one**
user's request across exact-profile replicas with lossless correctness
contracts and honest fastest-single fallback.

## Decision

ModelSwarm's technical identity is:

> Network-aware micro-swarms of machines hosting the same immutable model
> profile, cooperating to accelerate or improve **one request** through
> exact speculative decoding, verified token-tree exploration, parallel
> solution search, and automatic fastest-host fallback.

The exit-gate differentiation paragraph (each clause testable, tests named):

1. Membership is keyed to a manifest-derived exact profile, proven by
   randomized chunk challenges + live inference challenges
   [test: mismatched profile/hash/revision peer is excluded].
2. A request executes in a micro-swarm of ≤ 8 peers selected by measured
   topology from a population of any size [test: cohort stays bounded
   regardless of registry size].
3. Cooperative modes are lossless speculative decoding / verified trees with
   explicit correctness contracts [test: greedy golden equality; sampled
   distribution tests].
4. Cooperation engages only when the explainable cost model predicts beating
   the measured fastest eligible single host, with automatic fallback at the
   break-even point [test: fallback triggers on injected RTT/acceptance
   collapse].

Consequences: generic ratio-economy, payments, and public marketplace remain
non-goals; credits stay profile-specific research policy (ADR-012). The
single-peer path remains the always-available default and the fallback.

## Alternatives rejected

- Marketplace-first positioning (original plan): not differentiating vs
  p2ptokens; demoted.
- Layer pipelines (Petals-style): different optimization target; rejected as
  default architecture by prior-art analysis.
