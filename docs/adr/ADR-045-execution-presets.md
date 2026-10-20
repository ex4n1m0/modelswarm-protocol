# ADR-045: Execution presets (Fast / Balanced / Deep / Verified / Maximum)

Status: **Accepted (owner approval 2026-10-20 — tier table + disclosure
lines + Fast default approved verbatim).** Owner: Integrator (design),
Protocol Architect seat consulted on contract impact. Required seat reviews
(Security, Test and Release) run against this accepted text before the P1
merge; findings fold as amendment notes.

## Context

The owner's master-prompt cross-check (2026-10-09, `zcode-master-prompt.md`
85%-alignment note) named five execution presets — **Fast / Balanced / Deep /
Verified / Maximum** — as a missing deliverable. M10's build-order step 11 is
"final preset tiers." Two standing owner rulings constrain the design:

1. **Family verification is always on, no toggle** (2026-10-13/15): peers
   hosting the same exact model cross-check every reply; the product reduces
   switches rather than adding them.
2. **Output-changing modes need their own ADR + preset disclosure**
   (ADR-032 acceptance note, 2026-10-11): a preset may change WHERE and HOW
   a reply is computed/checked only with honest labeling.

What exists in the shipped product (v0.2.41): best-single-host turn path
(local engine, remote fastest-peer pull, or cloud key), family verification
cross-check by same-model peers (ADR-032 light-up tier, live since v0.2.21),
cloud cross-model cross-check proven live 2026-10-18 (ADR-042 mixed
economy), and the ADR-031 composed in-process speculative path compiled in
(`verifydrafts-capi`) but dormant in turns (planner is shadow-mode per R0).

The 9.6 pass-2 loopback verdict (2026-10-19,
`experiments/reports/PASS-2-LOOPBACK-CAPI.md`) informs the design: the
composed in-process path beats the fastest-single shape 11.2–12.6× P50 at
every window, wire-form speculation is a measured dead end (~2× slower), and
window size barely moves the composed path at high acceptance — so tiers
differentiate on VERIFICATION DEPTH and COHORT, not on speculation window.

## Decision

Five presets form a strictly upward gradient — **no tier removes the family
verification baseline** (ruling 1); higher tiers add depth:

| Preset | Turn path | Verification | Disclosure line (UI, verbatim intent) |
|---|---|---|---|
| **Fast** (default) | best single host by measured speed (local → remote → cloud) | family verification (all same-model peers, parallel) | "One host answers; every same-model peer checks it. The everyday speed path." |
| **Balanced** | Fast path, plus the composed in-process speculative engine when eligible (ADR-031/032 contracts, engages only when the shadow planner's live prediction says it beats the fastest single host — else silent fallback to Fast) | family verification, unchanged (speculation is lossless: `speculative_exact`, verified every round) | "Same checks as Fast; your own machine drafts ahead and verifies in one pass when that is measurably faster — otherwise it steps aside." |
| **Deep** | Balanced, plus full verification cohort (every available same-model verifier, not the fastest one) and disagreement escalation (any verifier reject ⇒ full recompute + slow-path re-check) | family verification with all hands | "Every same-model peer verifies every reply; any disagreement forces a clean recompute." |
| **Verified** | Deep, plus cross-model cross-check (cloud peer on the ADR-042 roster re-answers a canary form and the match is recorded on the receipt) | family + cross-model | "Everything Deep does, plus a second, different model family sanity-checks the answer; the receipt shows it." |
| **Maximum** | Verified, plus digest re-verification of the serving artifact and a full audit record (which peer, which engine hash, every verify outcome — counts and hashes only, never content) | family + cross-model + audit | "The whole swarm checks everything, and the receipt proves it. Slowest by design." |

Rules pinned by this ADR:

- **Presets are client-side policy.** No new wire fields in `msp-v1.md`; a
  preset composes existing behaviors (fastest-single selection, family
  verification roster, speculative engage/fallback, cross-check, receipts).
  The one contract-visible addition is a receipt label naming the preset and
  its engaged components (ADR-032 RC-1 honest-label pattern).
- **Lossless invariant unchanged:** every tier's outputs are verified
  token-for-token; no tier samples differently. Speed differences come from
  transport/composition, never from looser generation.
- **Fallback is the product behavior:** Balanced/Deep engage extra machinery
  only when the live prediction (planner out of shadow for the engage
  decision only) says it wins; the M10 exit rule (beat the fastest eligible
  single host or fall back) is enforced per turn, per preset.
- **Default = Fast** (today's behavior, relabeled honestly); the picker is
  one control in Settings with the table above as hover/tap disclosure —
  consistent with the reduce-switches ruling (no per-chat nagging).
- **Naming and disclosure lines ship exactly as approved by the owner**;
  any change re-runs this ADR's owner review.

## Implementation phases (honest scope)

1. **P1 (mechanical, no behavior change):** preset enum + picker + disclosure
   lines + receipt label. Fast and Verified/Maximum's non-speculative parts
   map to existing behavior; Deep's full-cohort verification widens the
   existing roster fan-out.
2. **P2:** planner engage-decision out of shadow for the composed path
   (Balanced) with per-turn fallback; gated on the ADR-032 §5 engagement
   checklist.
3. **P3 (research-linked):** any output-influencing additions (e.g.
   best-of-N) require their own ADR per ruling 2 — explicitly out of scope
   here.

## Consequences

- The five presets become the user-visible execution vocabulary; the
  internal mode names (light-up tier, wire-contract tier) stay internal.
- The always-on family ruling is honored verbatim: there is no
  verification-off tier.
- Receipts grow a preset label (schema-additive, receipts-v2 compatible).
- Test gates: per-preset golden turn fixtures (label + engaged components),
  fallback property test (Balanced never slower than Fast beyond noise),
  disclosure-string snapshot test.

## Reviews

- Owner: **APPROVED 2026-10-20** (tier table, disclosure lines, Fast default).
- Security: in flight (receipt label surface, cross-check canary form).
- Test and Release: in flight (fixtures, snapshot gates).
