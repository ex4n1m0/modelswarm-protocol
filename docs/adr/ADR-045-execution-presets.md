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
| **Fast** (default) | best single host by measured speed (local → remote → cloud) | family verification (all same-model peers, parallel; the per-turn label shows the verifier count — "checked by 0 peers" is visible, never vacuous) | "One host answers; every same-model peer checks it. The everyday speed path." |
| **Balanced** | Fast path, plus the composed in-process speculative engine when eligible (ADR-031/032 contracts, engages only when the shadow planner's live prediction says it beats the fastest single host — else silent fallback to Fast) | family verification, unchanged (speculation is lossless: `speculative_exact`, verified every round) | "Same checks as Fast; your own machine drafts ahead and verifies in one pass when that is measurably faster — otherwise it steps aside." |
| **Deep** | Balanced, plus full verification cohort (every available same-model verifier, not the fastest one) and disagreement escalation (any verifier reject ⇒ full recompute + slow-path re-check) | family verification with all hands | "Every same-model peer verifies every reply; any disagreement forces a clean recompute." |
| **Verified** | Deep, plus cross-model cross-check: a different-family peer on the ADR-042 serving path receives the turn's user message and draft answer (bounded) and returns a verdict (`confirmed`/`disagree`/`inconclusive`); when that peer hosts a cloud profile, the content transits the third-party provider under the HOSTING peer's key (disclosed above — this is the egress S-3 requires at the point of choice). The cross-check rides exclusively the lease-gated ADR-042 serving path; it never touches the ADR-033 advisory/assistant plane, and the requesting app opens no provider client | family + cross-model | "Everything Deep does, plus a second, different model family sanity-checks the answer; the receipt shows it." |
| **Maximum** | Verified, plus digest re-verification of the serving artifact and a full audit record (which peer, which engine hash — `canonical_build_hash` per ADR-019 — every verify outcome; counts and hashes only, never content; token ids are content per ADR-032 §2; the audit record is local by default, anything egressing is enum/count/hex only) | family + cross-model + audit | "The whole swarm checks everything, and the receipt proves it. Slowest by design." |

Rules pinned by this ADR:

- **Presets are client-side policy in P1.** No new wire fields anywhere:
  no `msp-v1.md` changes, and NO new ReceiptV2 fields — the existing
  `ReceiptV2.preset` slot (ADR-029, currently the honest literal
  `"unspecified"`) is populated ONLY on paths where the preset reached the
  serving peer via the negotiated `SessionOffer.preset`
  (msp-cooperative-v1 §2, P2 work), and then carries exactly the five-value
  enum; engaged components ride the existing `mode_id` under the frozen
  msp-cooperative-v1 §4 vocabulary. The P1 label is a requester-side
  annotation on the locally displayed turn (the reply meta line), composed
  from the requester's own ground truth. This resolves both the field-name
  collision (T/R B1) and the tracker zod-strip digest trap (T/R B2) by
  construction — nothing new transits the hub in P1.
- **Enum semantics supersede ADR-029 §6 / msp-cooperative-v1 §6**
  (Security S-1): the five NAMES are unchanged, but their semantics are
  the ADR-045 table's (all lossless, upward verification depth) — the
  mirror table's "deep = approximate, never lossless" reading is
  superseded. The mirror edit in `protocol/msp-cooperative-v1.md` §6 is
  routed to the Protocol Architect seat. The label vocabulary is frozen:
  exactly the five enum values, bounded length, counts and hashes only —
  never content, including token ids (S-5); intake-side enum/length
  validation of the transit field is routed to the Tracker Engineer as a
  companion ADR-030/045 amendment note (never a silent P1 change).
- **Lossless invariant unchanged:** every tier's outputs are verified
  token-for-token; no tier samples differently. Speed differences come from
  transport/composition, never from looser generation.
- **Fallback and honest non-engagement labeling apply to EVERY tier**
  (Security S-6): any component that did not engage this turn (family
  cohort 0, cross-check refused/undialable, speculation stepped aside) is
  shown as not-engaged with its honest reason; a preset label never claims
  an engagement that did not occur.
- **Default = Fast** (today's behavior, relabeled honestly); the picker is
  one control in Settings with the table above as hover/tap disclosure —
  consistent with the reduce-switches ruling (no per-chat nagging).
- **Naming and disclosure lines ship exactly as approved by the owner**;
  any change re-runs this ADR's owner review.

## Implementation phases (honest scope)

1. **P1 (mechanical, ALL tiers label-only — T/R R4):** preset enum +
   picker + owner-approved disclosure lines + the per-turn reply-meta
   label + persistence. NO behavior of any tier is wired — family
   verification and the ADR-042 cross-check run unchanged for every tier;
   Deep's cohort widening and everything heavier ride P2. P1 gates: the
   two-layer disclosure-sync test (Rust consts pinned byte-exact + an
   `include_str!` HTML-contains test binding `ui/index.html`, the picker
   IDs, and the IPC name) plus a config persistence test (selection
   survives restart; unknown stored value fails closed to fast). Rides the
   next release train with no version bump of its own (T/R R6).
2. **P2 (behavior):** planner engage-decision out of shadow for the
   composed path (Balanced) with per-turn fallback; Deep's full-cohort
   fan-out with session-count/receipt-cohort/load tests; the
   `SessionOffer.preset` negotiation so the serving peer can honestly sign
   the existing `ReceiptV2.preset` slot (msp-cooperative-v1 contract work,
   Protocol Architect; deploy-order: any preset-carrying cooperative
   session ships only after the counterpart parser supports it). P2 gates:
   per-preset golden turn fixtures asserting the REAL engaged set
   (including non-engagement reasons per the every-tier fallback rule),
   and the fallback property test with a PRE-REGISTERED bound (concrete
   epsilon or named statistical test over N repetitions; fastest eligible
   single host as baseline, full provenance per the house performance
   rule — T/R R5).
3. **P3 (research-linked):** any output-influencing additions (e.g.
   best-of-N) require their own ADR per ruling 2 — explicitly out of scope
   here.

## Consequences

- The five presets become the user-visible execution vocabulary; the
  internal mode names (light-up tier, wire-contract tier) stay internal.
- The always-on family ruling is honored verbatim: there is no
  verification-off tier.
- No receipt schema change (see the pinned rules above): the existing
  `preset` slot is used only where negotiation delivered the value.
- Verified/Maximum disclose the cross-model egress at the point of choice
  (S-3) and double receipt volume against the ADR-041/042 balance windows
  (S-12 advisory — a fairness note, not a cost-protection goal per the
  owner's standing ruling).
- Advisories tracked pre-P2: the Verified match record shape (verdict
  enum + checker profile id + counts/hashes only; the verifier's free-text
  reason never rides receipts/telemetry/logs — S-8), the untrusted-reason
  display discipline (S-11), and the audit-record locality pins (S-9,
  folded into the Maximum row).

## Amendment note (revision 2, 2026-10-20)

Folds the Test and Release review (**APPROVE-WITH-CHANGES**: blockers B1
field-name collision and B2 zod-strip digest trap — both resolved by the
no-new-fields mechanism above — plus R1 gate phasing, R2 two-layer sync
test, R4 P1 label-only, R5 pre-registered fallback bound, R6 train-riding,
R7 feature-combo surface) and the Security review (**APPROVE-WITH-CHANGES**:
S-1 frozen-enum supersession recorded + mirror edit routed to the Protocol
Architect; S-2/S-3 the canary wording replaced with the accurate
cross-check egress disclosure + the advisory-plane wall sentence; S-4/S-5
receipt mechanism pinned to the existing field via SessionOffer
negotiation, vocabulary frozen, intake validation routed to the Tracker
Engineer; S-6 every-tier honest non-engagement labeling; S-7 the Maximum
line's "proves" → "records" one-word correction PENDING the owner's
re-approval — the shipped line stays the approved text until then;
S-8..S-12 advisories tracked above).

## Reviews

- Owner: **APPROVED 2026-10-20** (tier table, disclosure lines, Fast
  default). Revision 2's Turn-path accuracy corrections (S-2/S-3) ride the
  approved table's structure; the S-7 one-word line change is queued for
  owner re-approval, not applied.
- Security: **APPROVE-WITH-CHANGES (2026-10-20)** — S-1..S-7 folded in
  revision 2; S-1's mirror edit routed to the Protocol Architect, S-5's
  intake validation routed to the Tracker Engineer.
- Test and Release: **APPROVE-WITH-CHANGES (2026-10-20)** — B1/B2/R1/R2/R4
  through R7 folded in revision 2.
