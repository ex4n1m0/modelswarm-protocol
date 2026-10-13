# ADR-034 rule 8 — pre-decision options analysis (2026-10-13)

Author: Protocol Architect. Purpose: the owner-gated decision record
behind ADR-034 rule 8 — whether to lift the block that keeps user chat
content out of family (cooperative verification) sessions. The owner has
asked for "family by default"; rule 5 of ADR-034 currently blocks chat
content from the family wire by design, and rule 8 is the recorded
commitment path through which that block could be lifted. This document
is written for an owner decision brief: plain language first, concrete
descriptions of what data goes where; the formal ADR/gate requirements
are collected in the appendix.

## The question, in one paragraph

Today, chat works by sending your conversation to ONE other computer (or
serving it on your own), and the "family" feature — several computers
checking each other's model outputs — only ever works on a fixed neutral
sentence that everyone can compute independently. The owner wants to know:
should we let real user conversations ride the family sessions too? Doing
that requires the computers doing the checking to actually receive the
conversation, which the current design deliberately forbids.

## What crosses the wire today (be concrete)

**Chat, today:** your message text, your temperature setting, and the
reply stream cross ONE encrypted connection — either to your own local
engine (never leaves the machine) or to one remote hosting peer chosen
from the roster. The remote peer's engine sees the full conversation; it
processes it and streams tokens back; nothing is stored at either end,
and the hub (modelswarm.deepflux.space) only ever sees routing facts
(who hosted, timings, outcomes) — never text. This is disclosed at
onboarding: "the serving peer — another person's computer — can
technically read any prompt it serves" (`docs/threat-model.md` §5.2).

**Family, today:** family sessions carry only counts, hashes, and windows
of token ids that continue a FIXED PUBLIC SENTENCE (ADR-034's
`FAMILY_PROMPT_V1`). No user text, ever. The redaction rules go further:
even those token ids are treated as content and are banned from logs,
rejections, receipts, and telemetry (ADR-032 §2: "token ids ARE
completion content in encoded form"). This is what the block protects.

## The decisive measurement fact (why speed cannot be the reason)

The entry rule of the whole system is that your own computer must be
hosting the exact same model in order to use the swarm. That fact has a
hard consequence for the family idea: in cooperative verification, every
committed word is first DRAFTED by your own computer and then CHECKED by
the other one. So the family can never produce words faster than your
own computer types them — and if the other computer is faster, then just
asking it directly (which is what chat already does today) is faster
than asking it to check your typing. This is not speculation; the 9.6
pass-2 measurement tested it 2,928 times: no cooperative attempt beat
the fastest eligible single machine; typical cooperative attempts ran
1.38-1.94x SLOWER; the single best corner was 1.07x (parity). The
standing product rule ("beat the fastest eligible single host or fall
back") means the engage gate would essentially never fire for chat speed
— correctly, because the win is not there to be had.

So the honest question is not "how do we get faster chat through the
family" — it cannot be faster. The question is whether the family's
OTHER benefit — cross-checking, i.e., catching wrong or tampered answers
— is worth putting user conversations on a multi-peer wire. ADR-032's
own decision record says accuracy benefits are welcome even when slower;
that is the real prize on offer.

## Option 1 — Keep the block (status quo + finish the groundwork)

Chat keeps using today's single-peer path (which the math above says is
already the latency optimum). The family keeps doing what it does now:
pairing confirmation, capability discovery, health probing, honest
labels — all on the neutral sentence. Groundwork already shipped
(capability cache, engage ladder, self-check) stays in place, dormant
but honest.

- **What changes for the user:** nothing. Same speed (the optimum), same
  exposure (one peer or local), same disclosures.
- **New risks:** none.
- **Engineering cost:** zero mandatory. One recommended small task (no
  ADR needed — it is local measurement only): feed a family-round timing
  EWMA from the probe's existing round, so the engage ladder's
  `gate_unmeasured` becomes measurable and the owner can see, in
  telemetry, how often the gate would engage (expected: never — which is
  itself decision-useful evidence).

## Option 2 — The consent commitment path (lift the block via rule 8)

What ADR-034 rule 8 actually reserved: a session-open carrier (like the
existing lease carrier) carrying a prompt COMMITMENT, plus a
`pin_mismatch` session-reject reason. Applied to user content, the
workable design is: at family session open, the coordinator sends the
user's conversation rendered as token ids plus a bare-hex hash
commitment; the verifying peer (which has explicitly opted in) prefills
its engine with those tokens and verifies drafted windows against them;
a hash mismatch is a session-open reject instead of a first-round
failure.

- **What crosses the wire that does not today:** the user's ENTIRE
  conversation, as token ids, to EVERY verifying peer in the session
  (the cohort, bounded by the session budget) — plus drafts and
  corrections derived from it. For that session, "the family wire is
  content-blind" stops being true; it becomes "the hub stays blind, the
  peers see content."
- **What changes for the user:** their conversation goes to N computers
  instead of 1. The UI must say so before it ever happens, and the
  serving peers must have individually opted in (a default-OFF setting
  on the serving desktop: "carry other users' conversations through
  family verification").
- **Privacy/threat-model deltas vs `docs/threat-model.md`:** adversary
  B (malicious serving peer that records prompts) multiplies from one
  observer per request to every cohort member; user content takes up
  residence in verifier engine KV caches; the redaction surface grows
  (family frames, currently guaranteed content-free vacuously, become
  content-bearing and must join every prompt-redaction gate); consent
  UX becomes an attack surface (peers pressured or tricked into opting
  in). The hub-blind property survives — non-negotiable in any version
  of this option.
- **Prompt-redaction rules that must hold:** token ids remain
  first-class content (they already are, per ADR-032 §2) — so family
  frames join the same redaction regime as msp-v1 request bodies: never
  logged, never in telemetry (the counts-only rule for family events
  stays exactly as is), never in receipts, never echoed in rejections;
  verifier KV residency must be bounded by the existing session teardown
  and the cache-cold discipline.
- **Required ADR text and gates (summary; details in the appendix):**
  amend `protocol/msp-cooperative-v1.md` §5 (its forbidden list says
  "Prompt/completion content anywhere in this namespace — forbidden"),
  which is a frozen-namespace constraint and needs its own ADR; land
  ADR-034 rule 8's carrier with the token-id payload plus commitment
  hash and the reserved `pin_mismatch` reason; amend ADR-034 rule 5's
  blanket statement; update `docs/privacy.md` §2 and reword onboarding
  (both are ALREADY pre-recorded obligations: "this document must be
  updated with the exact recipient set per mode and the onboarding
  disclosure reworded before any cooperative request is served"); add
  threat-model rows; golden vectors for the carrier; consent-default-off
  tests; Security and Test and Release reviews.
- **Engineering cost:** moderate-to-large — a new frozen wire family
  member with strict schema and vectors, redaction-gate extensions,
  consent settings and UI on both ends, three document updates, and the
  permanent maintenance of one more content-bearing surface.
- **Honest benefit statement:** for SPEED, none (see the measurement
  fact; the engage gate would still refuse to fire without a measured
  predicted win, and there is no win to measure). The deliverable
  benefit is VERIFICATION: the ability to have a chat answer
  cross-checked by peers, slower by design, with divergence honestly
  labeled. If that is what the owner wants from "family by default,"
  this option is the only wire path that can provide it — there is no
  third wire.

## Option 3 — Hybrid: conversations stay single-peer; the CHECK moves local (recommended accuracy path)

Keep chat exactly where it is (msp-v1 single serving — the fastest-single
optimum). Keep family sessions neutral-only (pairing, health,
capability). Add the accuracy benefit where it is actually cheap: the
requesting desktop, which by the project rule is ALREADY hosting the
same exact model, spot-checks the remote answer it received — re-run a
sampled window of the reply through its own local engine, cache-cold
(the exact machinery that already shipped as the coordinator self-check
and the RC-1 audit path), and surface "answer verified locally" or
"divergence detected" on the reply.

- **What crosses the wire that does not today:** nothing. Zero new
  exposure — the conversation still touches exactly two machines
  (requester + one serving peer).
- **What changes for the user:** an optional badge on replies saying the
  answer was (or was not) reproduced by their own engine; slightly more
  local compute when enabled.
- **Quantified benefit and cost:** catches the realistic bad-serving-
  peer behaviors (wrong tokens, divergent or mismatched builds — threat
  model adversary B) at a measured cost of roughly one batched local
  pass per audited window (the light-up exactness record measured
  12-14 ms per pass on the pinned CPU bundle). Honest limits, stated
  up front: at temperature 0 the check is exact; at higher temperatures
  the check must compare likelihoods, not equality — a weaker,
  detection-oriented check that would need its own small ADR if built.
  Divergence means "context-or-backend divergence class" (E0: CPU vs
  Vulkan differ; cross-machine CPU determinism is the recorded open
  question), never a claim that a specific peer lied — same honest
  caveat ADR-034 rule 6b already carries.
- **Engineering cost:** small — reuses shipped machinery; mostly UI and
  a sampling policy. One small ADR if the non-greedy (likelihood) check
  is included; the greedy check is an execution detail of the existing
  local audit seam.
- **Why this is technically superior for the accuracy goal:** it spends
  zero new privacy to buy the one benefit the family could actually
  sell, and it checks the peer that SERVED the answer (the machine that
  could have cheated), rather than recruiting additional machines and
  exposing the conversation to each of them.

## Comparison

| | Option 1 (keep blocked) | Option 2 (consent carrier) | Option 3 (local audit) |
|---|---|---|---|
| User-visible change | none | conversations go to N peers, with disclosures and opt-ins | optional "verified locally" badge |
| New exposure | none | full conversation to every cohort peer + their KV | none |
| Chat speed benefit | none (already optimal) | none (measured; gate will not fire) | none (adds a little local work) |
| Accuracy benefit | none | cross-peer verification, slower by design | local cross-check of the serving peer's answer |
| ADR burden | none | frozen-namespace amendment + rule-5/8 landing + privacy/threat/onboarding | one small ADR only if the non-greedy check is built |
| Engineering cost | zero (optional EWMA task) | moderate-large | small |

## Recommendation

Option 1 now, with the probe-sourced EWMA so the ladder becomes
measurable. Option 3 as the accuracy follow-up when the owner wants
checked answers — it is the best benefit-per-risk on the table and
aligned with the standing directive that accuracy benefits are welcome
even when slower. Option 2 only if the owner specifically wants
MULTI-PEER verification of chat conversations and explicitly accepts
N-party conversation exposure in exchange for no speed gain; if chosen,
it rides its own ADR with the appendix gates, default-off consent on
both ends, and the hub-blind and redaction rules held absolutely.

## Appendix — formal requirements if option 2 is chosen

1. **ADR amending `protocol/msp-cooperative-v1.md` §5** (frozen-namespace
   constraint: "Prompt/completion content anywhere in this namespace —
   forbidden"): a narrow carve-out for a consent-gated user-prompt
   session-open carrier only; the forbidden list otherwise stands; the
   hub-blind rule stands unconditionally; disjointness of the carrier's
   discriminant key from the frozen §6 tags and family discriminant keys
   asserted per ADR-032 §2 rule (i).
2. **ADR-034 rule-8 landing text:** the carrier's strict schema (bare
   object; token-id list; bare-64-hex commitment = the ADR-032 §1 rule-2
   LE-u32 construction with no parent; session binding fields per the
   §2 binding set), the reserved `pin_mismatch` `SessionReject` reason
   (negotiation-level vocabulary, the G3 class), and the rule-5
   amendment replacing the blanket block with the consent contract.
   Wire changes ride the wire-compat CI job; `messages.proto` remains
   untouched (the family is JSON-on-QUIC).
3. **Golden vectors** (fixture keys recorded, `verify_at` anchored, per
   the ADR-032 §7 fixture discipline): carrier accept, commitment
   mismatch → `pin_mismatch`, strict-schema failures.
4. **Consent gates:** serving-peer setting (default OFF) refusing the
   carrier with an honest labeled reject; requester-side disclosure tied
   to the recipient-set count; cohort exposure bounded by the session
   budget's `max_endpoints`.
5. **Redaction gates:** family frames join the prompt-redaction test
   surface; family telemetry stays counts-only (the existing no-arrays
   assertion extends); verifier KV residency bounded by session
   teardown; receipts continue to carry verifier-claimed counters only
   (ADR-030 grant rules).
6. **Documents:** `docs/privacy.md` §2 recipient-set update and
   onboarding reword (pre-recorded obligations); `docs/threat-model.md`
   adversary-B and KV-residency rows; E0 cross-machine determinism
   remains the gate on any exactness CLAIM arising from cross-peer
   verification (labels say divergence-class, not proof of lying).
7. **Reviews:** Security (adversary matrix for the new content-bearing
   surface) and Test and Release (fixtures, redaction, consent tests)
   before any traffic; the engage gate's never-worse rule and
   fastest-single comparator remain outermost — engagement still
   requires a measured predicted win, so option 2's honest operating
   mode is opt-in verification, never automatic speed-up.
