# ADR-036: Family user-content sessions — the rule-8 pin carrier under the owner's swarm-together directive

Status: **Proposed (2026-10-13) — paste-ready for the Integrator; acceptance
per the ADR log process. Owner-directed; the directive is quoted verbatim in
§1. Authored against HEAD `6234929` (all CI green).** Lands ADR-034 rule 8's
reserved wire vocabulary in its owner-directed user-content form: family
(cooperative verification) sessions MAY carry the requesting user's
conversation, as token ids, via a signed session-open pin carrier, and
serving peers participate BY DEFAULT. Amends the frozen
`protocol/msp-cooperative-v1.md` §1/§5 with one narrow carve-out (exact
texts in §4). `protocol/msp-v1.md` and `protocol/messages.proto` remain
byte-identical; the frozen §5 `EligibilityLease` signed shape is untouched.
Owner: Protocol Architect (contract, carrier types, reject-reason
vocabulary, golden vectors, namespace amendments —
`crates/modelswarm-session`, `protocol/`). Implementation: Windows Product
Engineer (negotiate/serve wiring, per-session engine composition, both UI
disclosures, the default-on carriage setting — `crates/modelswarm-node`,
`crates/modelswarm-desktop`), sequential after the Protocol seat's carrier
types land. Required reviewers per the ownership map: Security Engineer and
Test and Release Engineer — the implementation gate; no carrier traffic
before both sign off (§6).

## 1. The owner directive, and the two constraints that survive it

Verbatim, 2026-10-13: **"the idea of a swarm is that all agents work
together, privacy is not a concern at this moment."**

This selects Option 2 of `docs/reviews/adr034-rule8-options-2026-10-13.md`
(the rule-8 consent/commitment carrier) and relaxes the privacy-driven gates
that analysis recorded:

- **Serving-peer carriage is DEFAULT ON.** The options analysis specified a
  default-OFF setting ("carry other users' conversations through family
  verification"); the owner's posture inverts it. A stock install with the
  family gates held (`MSP_LISTENER=1` + C-API bundle + hosting on) serves
  user-content carrier sessions with no additional configuration. The
  opt-OUT toggle survives as a permitted product surface; its refusal is
  honest (§3.2).
- **The consent-attack-surface and N-party-exposure threat-model deltas are
  downgraded per the owner's explicit posture, not designed around.**
  Adversary B's scope under default-on carriage is restated honestly in
  `docs/threat-model.md` — every cohort member reads the conversation; the
  owner accepts this (§6). No consent-friction mechanism is introduced to
  compensate.
- **The requester-side disclosure is an always-visible honest statement,
  not a friction gate.** No modal, no per-conversation click-through; the
  statement of what crosses the wire sits permanently on the chat surface
  while the mode is elected (§3.1). The project's honesty discipline stands:
  the disclosure must never imply the content is unreadable to receivers,
  encrypted beyond transport, or faster.

**Two constraints survive unchanged and bound everything below:**

1. **The HUB stays content-blind.** `modelswarm.deepflux.space` never
   carries prompts, completions, or their token-id form — Vercel is the
   control plane only (AGENTS.md hard constraint 1; ADR-001). The carrier
   rides P2P QUIC + Noise sessions exclusively. This is architectural, not
   a privacy posture; it has NO exception in this ADR.
2. **Counts-only telemetry; no token ids in logs or events.** Redaction
   from logs is engineering hygiene, not peer-privacy: local logs and
   diagnostics are artifacts users inspect and export, and nothing
   content-bearing may outlive a session in them. ADR-032 §2's ruling
   stands verbatim — "token ids ARE completion content in encoded form" —
   and now binds a family wire surface that actually carries them (§3.2
   hygiene rules; §6 redaction gates).

## 2. Context — the block being lifted, and why a new ADR

Three rules currently keep user content off the family wire: ADR-034 rule 5
("it does not smuggle user prompts onto the family wire — content-blindness
and the family's no-prompt-on-wire design stand"), ADR-034 rule 8 (which
reserves the commitment carrier as owner-gated future vocabulary), and
`protocol/msp-cooperative-v1.md` §5's forbidden-list bullet ("Prompt/
completion content anywhere in this namespace — forbidden"). The chat engage
ladder encodes the block as the frozen label `blocked_content_contract`
(`crates/modelswarm-desktop/src/app.rs`, `family_engage_reason`). The
measurement fact behind the block's speed half is settled science on this
repo (§3.4); the privacy half is what the owner's directive now overrides.

**Why a new ADR and not an ADR-034 amendment:** (1) ADR-034 rule 8 itself
routes this vocabulary to "its own ADR and gate" — amending it in place
would contradict its own reservation; (2) the change amends the frozen
msp-cooperative-v1 namespace, and the options analysis (appendix item 1)
already ruled that a frozen-namespace amendment requires its own ADR; (3)
ADR-034 is a shipped v0.2.23 contract whose label vocabularies are
telemetry-frozen — its decision text stays byte-stable and takes only the
cross-reference revision note provided in §4.3, the established
ADR-032-revision-note pattern.

## 3. Decision

### 3.1 The carriage contract (default-on; honest disclosures on both ends)

**Serving side.** Carriage of other users' conversations through family
verification is ON by default (§1). The serving desktop's family status
line carries an always-visible disclosure while family serving is enabled:

> "This computer carries other users' conversations (as token ids it can
> read) through family verification. On by default — you can turn it off,
> and requesting peers are refused honestly when you do."

An opted-out peer refuses a pin carrier with the honest labeled reject
`content_carriage_refused` (§3.2) — never a silent drop, never a
misleading label. Note the honest boundary of what the opt-out protects:
it prevents this peer from PROCESSING the content (no prefill, no engine
work); it cannot prevent RECEIVING the bytes of a session that was already
dialed, because no roster or wire field carries the carriage posture
(adding one rides its own ADR, per ADR-034's revision note).

**Requester side.** A family-verified chat session opens ONLY by explicit
user election (the verified-chat product toggle; Windows Product names the
surface). Election is required for ANY user-content carrier session — no
automatic path sends a user conversation to any peer beyond today's
single-serving path. While elected, the chat surface carries an
always-visible disclosure:

> "Family verification ON: your conversation — in a form the receiving
> computers can read — goes to every verification peer this turn (N). It
> is slower than single-peer chat, not faster. The hub
> (modelswarm.deepflux.space) never sees any of it. Turn it off any time."

N is the actual cohort size for the turn (bounded by the preset cohort cap
and the session budget's `max_endpoints`; 1 in the shipped configuration).
Both disclosures are normative UI contract: always visible while active,
plain language, no modal, no one-time click-through, no
security-through-tokenization implication, the honest cost stated, the
hub-blind fact stated. Both ship with the implementation and are asserted
in tests (§6).

For calibration: the msp-v1 §6 single-serving path ALREADY transmits the
full conversation text to one serving peer (privacy.md §2; threat-model
§5.2). The carrier's delta is the RECIPIENT SET (the cohort, not one peer)
and the FORM (token ids, not text) — not a new content class on the wire.
Token ids are not encryption: a peer hosting the same exact profile holds
the tokenizer and reads them trivially, and the disclosures say so.

### 3.2 The pin carrier (wire form and semantics)

**Position and precedent.** The carrier follows the `WireSessionLease`
precedent (ADR-032 §2 F1): a session-open frame, consumed by the
session-open reader BEFORE any engine work and before the serve loop
starts, never reaching the family frame router, parsed STRICT. The open
sequence is fixed-order:

```text
[lease carrier]?  →  [pin carrier]?  →  SessionOffer  →  SessionAccept | SessionReject
```

The lease carrier remains the FIRST session-open frame when presented
(F1's "immediately before the SessionOffer" is amended, wording only, to
"first at session open; the pin carrier, when present, is the frame
immediately before the offer" — the F1 discipline itself is unchanged).
The disclosure-minimizing rationale: the tiny lease precedes the
content-bearing carrier, so lease refusal happens before the coordinator
has sent the conversation. An offer WITHOUT a preceding pin carrier
remains valid and opens a NEUTRAL session (ADR-034 rules 1–5 govern the
root, unchanged). A pin carrier without a preceding lease carrier is valid
(roster-fed coordinators). Any other order is a `bad_frame`-class session
end.

**Wire form.** Canonical JSON per msp-v1 §2.2 (floats forbidden); a BARE
object with exactly these fields, `deny_unknown_fields`:

```json
{
  "session_id": "<opaque unique string, generated by the coordinator>",
  "protocol_version": "msp-cooperative-v1",
  "seq": 1,
  "round": 0,
  "profile_id": "msp1:<64 hex>",
  "generation_params_hash": "<bare 64-char lowercase hex>",
  "prompt_tokens": [<u32 token ids, length >= 1>],
  "pin_commitment": "<bare 64-char lowercase hex>",
  "nonce": "<single-use per session>",
  "deadline_ms": 2500,
  "sender": "<coordinator installation id>",
  "signature": "base64 ed25519 over canonical JSON with signature omitted entirely"
}
```

| Field | Type | Meaning |
|---|---|---|
| `session_id` | string | MUST equal the subsequent `SessionOffer`'s (mismatch `bad_frame`); echoed by accept/reject |
| `protocol_version` | string (frozen constant) | Binds the carrier to the namespace version (ADR-029 review condition) |
| `seq` | u64 ≥ 1 | The coordinator's per-(session, sender) counter (ADR-032 §2 RC-7); the carrier ESTABLISHES the coordinator's baseline; every later coordinator `VerifyDrafts` carries `seq` strictly greater |
| `round` | u32, always 0 | The reserved session-open phase marker; in carrier sessions every `VerifyDrafts` round is ≥ 1 (strict monotonicity over the session's handled rounds, the carrier's 0 included) |
| `profile_id` | `msp1:<64 hex>` (ADR-032 §1 form) | The coordinator's pinned exact profile; MUST equal the offer's; compared against the serving side's pinned profile → `SessionReject` `profile_mismatch` (the G3 discipline, now triggerable at the carrier — the honest-early close of the G3 gap) |
| `generation_params_hash` | bare 64-hex (ADR-032 §1 rule-3 preimage) | MUST equal the offer's; compared against the serving side's pinned params → `params_mismatch` |
| `prompt_tokens` | [u32], length ≥ 1 | The user's conversation rendered through the profile's pinned chat template (ADR-025 application point) and pinned tokenizer (the ADR-034 rule-3 discipline), as token ids; bounded by the msp-v1 §6.4 256 KiB frame limit. The serving side does NOT re-derive or re-tokenize — no text crosses, the ids ARE the contract |
| `pin_commitment` | bare 64-hex | `hex(sha256(prompt_tokens rendered as little-endian u32 each))` — the ADR-034 rule-3 pin identity, NO parent (ADR-032 §1 rule-2 encoding). It binds the ENGINE ROOT, not the chain: the prefix-hash chain still starts at `GENESIS_PREFIX_HASH` (ADR-032 §1 rule 2 unchanged); the commitment never becomes the first round's `parent_prefix_hash`. This is the likeliest second-implementation drift and is byte-pinned by the §6 vectors |
| `nonce` | string | Single-use per session; enters the session nonce ledger; any later frame reusing it is `replayed_request` |
| `deadline_ms` | u32 | Server-side clamp `[1000, 120000]` (ADR-032 §2); bounds the open-phase work (parse, verify, prefill); re-checked immediately before prefill dispatch |
| `sender` | string | MUST equal the installation id derived from the signing key AND the QUIC-authenticated remote PeerId (ADR-032 §1 rule 5; msp-v1 §6.6); mismatch `bad_frame` |
| `signature` | base64 ed25519 | Over the canonical JSON of the carrier with `signature` omitted entirely (ADR-032 §1 rule 4) |

Unlike the lease carrier (one field, an unchanged token, no new signed
shape), the pin carrier is a NEW signed protocol-level shape and therefore
lands as a real namespace member: the type lives in
`crates/modelswarm-session/src/negotiation.rs` beside `SessionOffer`/
`SessionReject` (protocol-owned, visible to default-feature tests and any
second implementation), with golden vectors in `protocol/vectors/` (§6).

**Serving-side admission at the carrier (first-failure-wins, mirroring the
ADR-032 §2 order; the property-test assertion is the earliest outcome):**

| Injected condition | Outcome |
|---|---|
| frame > 256 KiB | `payload_too_large` (transport, before parse) |
| schema invalid / unknown fields / envelope tag / non-bare-hex commitment / non-u32 or empty `prompt_tokens` / wrong forms | `bad_frame`-class session end |
| signature invalid / `sender` ≠ authenticated PeerId / `protocol_version` wrong | `bad_frame` |
| carriage setting OFF | `SessionReject` reason `content_carriage_refused` |
| `profile_id` ≠ pinned profile | `SessionReject` reason `profile_mismatch` |
| `generation_params_hash` ≠ pinned params | `SessionReject` reason `params_mismatch` |
| `pin_commitment` ≠ sha256(`prompt_tokens` LE-u32) | `SessionReject` reason `pin_mismatch` |
| prefill infeasible (context window / governor ceiling, ADR-035 D11) | `SessionReject` reason `budget_infeasible`, rendered honestly as "conversation does not fit this peer" |
| prefill engine failure | `executor_error`-class honest session end |
| deadline past at admission (clamped) | `deadline_exceeded` |

The `SessionReject` reason vocabulary extends by exactly two members, both
G3-class (negotiation-level cooperative-namespace vocabulary; the §6.5
effective registry is untouched): **`pin_mismatch`** — reserved by ADR-034
rule 8 for commitment mismatches at open, now landing; the string is
deliberately identical to the frozen probe verdict label
`family.probe.pairing` (ADR-034 revision note) so one vocabulary names one
fact everywhere — and **`content_carriage_refused`** — the opted-out
peer's refusal. The second reason exists because the honesty discipline
the owner kept demands it: a refusal is not a mismatch, and labeling a
refusal `pin_mismatch` would put a lie on the wire. The six existing
canonicalizations in `session-reject-1.json` are unchanged bytes; the two
new reasons extend the file (the G3 precedent).

**Engine lifecycle.** A carrier session's family engine is constructed
per-session from `prompt_tokens` through the existing F2 seam
(`FamilyEngineSource::Capi { prompt_tokens }` / `capi_family_source`,
`crates/modelswarm-node/src/verifydrafts/engine_source.rs`) — the
conversation is the PREFILLED root at engine construction, exactly as the
neutral pin is today. The composed neutral family engine (ADR-034 rules
1–5) continues to serve carrier-less sessions and is NOT modified; the
expert file continues to rule the neutral pin only. The carrier session's
engine is torn down at session end, which bounds KV residency (§6
threat-model row); admission is governed by the resource governor's 70%
ceiling (ADR-035, gate D11) — a conversation the peer cannot fit is
refused honestly, never silently degraded. The composition detail (swap
vs. a second resident engine) is Windows Product + Resource Engineer
coordination, recorded as a consequence (§7), not contract.

**Carrier sessions are lossless-verification sessions only.** The elected
chat mode rides the `verified` preset tier's family (`VerifyDrafts` +
the audit path; `speculative_exact` per ADR-013/032 §3 — the committed
stream is the verifier engine's own continuation under pinned params,
unchanged by the carrier, which only changes the ROOT). Approximate tiers
(`deep`/`maximum` composition) carrying user content are NOT authorized by
this ADR (§5).

**Hygiene rules for the carrier (the ADR-032 §2 discipline, now
binding real content).** `prompt_tokens` are content: never logged, never
telemetered, never echoed in rejections, never in receipts, never in the
webview (the serving webview shows status facts only). Rejection frames
echo codes and counts only. The pin commitment MAY appear in LOCAL status
and diagnostics on both ends (it is the pairing identity, exactly as the
neutral pin hash is per ADR-034 rule 6a) but is content-derived, so —
unlike the neutral pin hash, whose publication ADR-034 rule 7 authorized
because it hashes a public constant — it NEVER enters telemetry: family
telemetry stays counts and labels only (token COUNT yes; arrays, ids, and
content-derived hashes never). KV residency is bounded by session
teardown; the cache-cold discipline (ADR-032 §3) stands.

### 3.3 Engagement: the gate stays outermost; the mode is elected, never automatic

The engage gate and the fastest-single comparator are UNTOUCHED and remain
the outermost rules for automatic engagement: the frozen arithmetic
(`should_engage_cooperative`, `DEFAULT_CONFIDENCE_MARGIN` 0.15, 0.05 reject
floor), the never-worse rule, and the mandatory fallback to single are
byte-stable. This ADR creates NO automatic speed-up path. Two paths exist
post-ADR-036:

- **The automatic path (unchanged ladder).** Without an explicit election,
  user content still does not ride family sessions. The frozen label
  `blocked_content_contract` remains in the vocabulary (telemetry compat)
  with a NARROWED meaning: it fires when the gate would otherwise engage
  but the user-content carrier path is not elected or not locally
  available (the conversation's carrier form exceeds the §6.4 256 KiB
  frame limit, or the conversation is not renderable under the pinned
  template/tokenizer) — never as a blanket content block. Truncating a
  conversation to fit is FORBIDDEN: truncation changes the root, breaks
  the commitment, and would fail verification silently.
- **The elected path (user-triggered, the probe's sibling).** The user
  elects family-verified chat (§3.1); the coordinator opens a carrier
  session per verifying peer; the honest cost is displayed. This path is
  user-triggered — like ADR-034 rule 6b's probe confirmation round, it
  does not ride the engage gate, because the user has explicitly accepted
  the measured cost the gate exists to protect them from. It is opt-in
  verification/co-production, never an automatic speed-up: the committed
  stream is the verifying peer's engine's own continuation — the peers
  co-produce and cross-check the answer.

### 3.4 The honest performance statement (do not soften)

**This does NOT make chat faster.** The project rule guarantees the
requester hosts the same exact profile, so the requester's own engine still
drafts every committed word — the drafting cost is local and unavoidable,
and the verifying peers CHECK, they do not relieve it. The 9.6 pass-2
measurement settled this across 2,928 engaged speculative completions
(`experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/ANALYSIS.md`,
`experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10/ANALYSIS.md`): no
engaged arm beat the fastest eligible single; engaged LAN cells lost
1.38–1.94× by cell median; the single best engaged median ratio was 1.071×
(the parity corner). The engage gate would essentially never fire for chat
speed — correctly, because the win is not there to be had. The benefit on
offer is VERIFICATION and CO-PRODUCTION: a chat answer cross-checked by
peers hosting the same exact profile, slower by design, with divergence
honestly labeled. Any UI, release note, or document that presents this
mode as a speed feature is a defect.

**What this OPENS (recorded, not scheduled):** the cooperative-plan
research track (`docs/cooperative-plan.md`) now has a real-prompt venue
the neutral `FAMILY_PROMPT_V1` path could never provide — real prompt
distributions, real lengths, real acceptance statistics for multi-proposer
token trees (phase P4) and cross-machine speculative decoding on actual
conversations. This is recorded as the future direction ONLY: nothing is
scheduled, no crate or agent is opened, and any start rides the
cooperative plan's own prerequisite-gate audit and P0 discipline plus its
scientific rules (fastest-eligible-single comparator; negative results
published as visibly as positive ones).

## 4. Frozen-namespace and document amendments (exact texts)

All four edits are mechanical; the Integrator lands them with the carrier
types. Each is a shared-schema change under ADR authority — exactly this
ADR's.

**4.1 `protocol/msp-cooperative-v1.md` §1, third paragraph.** Replace:

> This namespace carries no prompt or completion content beyond what the
> §6 stream already carries; the hub never sees any of it.

with:

> This namespace carries no prompt or completion content beyond what the
> §6 stream already carries, with ONE narrow exception (ADR-036): the
> user-content session-open pin carrier, which carries the requesting
> user's conversation as token ids to the verifying peers of an explicitly
> elected family session (serving-side carriage default-on); the hub never
> sees any of it — unconditionally.

**4.2 `protocol/msp-cooperative-v1.md` §5, fourth bullet.** Replace:

> - Prompt/completion content anywhere in this namespace — forbidden
>   (content-blind).

with:

> - Prompt/completion content anywhere in this namespace — forbidden,
>   with the ONE narrow exception of ADR-036's user-content session-open
>   pin carrier (explicitly elected, carriage-default-on; serving peers
>   refuse honestly when opted out). The hub-blind rule has NO exception.

**4.3 `protocol/msp-cooperative-v1.md` §1 — append the ADR-036 freeze
record:**

> **ADR-036 freeze record (2026-10-13):** family sessions may carry user
> conversation content as token ids via a session-open PIN CARRIER —
> coordinator → serving peer, after the `lease` carrier (when presented)
> and immediately before the `SessionOffer` — a strict bare object
> (`deny_unknown_fields`) binding the full ADR-032 §2 set (`session_id`,
> `protocol_version`, `seq`, `round: 0`, `profile_id`,
> `generation_params_hash`, `nonce`, `deadline_ms`, `sender`,
> `signature`) plus `prompt_tokens` and `pin_commitment`, where
> `pin_commitment = hex(sha256(prompt_tokens as little-endian u32 each))`
> (the ADR-034 rule-3 construction, no parent; the prefix chain still
> starts at `GENESIS_PREFIX_HASH`). The `SessionReject` reason vocabulary
> extends to `pin_mismatch | content_carriage_refused` (the G3 class).
> The discriminant keys `pin_commitment` and `prompt_tokens` join the
> ADR-032 §2 rule-(i) enumerated family discriminant keys. The carrier is
> consumed by the session-open reader before the serve loop and never
> reaches the frame router. Full contract, gates, and the owner directive
> live in ADR-036.

**4.4 ADR-034 — append this revision note:**

> ## Revision note (2026-10-13, ADR-036): the rule-8 carrier lands, in its
> owner-directed user-content form
>
> The owner directive of 2026-10-13 ("the idea of a swarm is that all
> agents work together, privacy is not a concern at this moment") selected
> Option 2 of `docs/reviews/adr034-rule8-options-2026-10-13.md`: ADR-036
> lands rule 8's reserved carrier as the USER-CONTENT pin carrier (the
> conversation as token ids plus the rule-3 commitment), with serving-side
> carriage DEFAULT ON, extending the `SessionReject` vocabulary with
> `pin_mismatch | content_carriage_refused`. Rule 5's closing statement is
> superseded for carrier sessions: family sessions MAY carry user
> conversation content via the ADR-036 carrier under its election and
> disclosure contract. Rules 1–5 stand UNCHANGED for carrier-less
> (neutral-root) sessions; the expert file continues to rule the neutral
> pin. Rule 6b's E0 caveat extends verbatim to carrier sessions: divergence
> labels mean context-or-backend divergence class, never proof that a
> specific peer lied. Rule 7's protocol-parameter analysis does NOT
> transfer to user-content commitments (content-derived: local
> status/diagnostics only, never telemetry). The `family.engage` label
> `blocked_content_contract` remains frozen vocabulary with a narrowed
> meaning: it fires when the user-content carrier path is not elected or
> not locally available, never as a blanket content block.

**Disjointness (ADR-032 §2 rule (i), asserted).** The carrier's
discriminant key `pin_commitment` and its payload key `prompt_tokens` are
disjoint from the frozen ten §6 `WireMessage` variant tags (`handshake`,
`handshake_ack`, `inference_request`, `token_delta`, `usage`,
`stream_error`, `cancelled`, `completed`, `cancel`, `control`) and from
every existing family discriminant key (`window_tokens`,
`accepted_token_ids`, `preset`, `budget`, `verifier_backend`, `engine`,
`reason`, `nonce_echo`, `code`, `lease`). Both keys join the rule-(i)
enumerated set; the per-push disjointness test
(`crates/modelswarm-session/tests/verifydrafts_vocabulary_disjointness.rs`)
extends its `FAMILY_DISCRIMINANT_KEYS` and builds the carrier shape from
the real session-crate type in the same change — the collision-test
obligation rule (i) attaches.

## 5. What this ADR does NOT authorize

1. **Hub carriage of content.** Prompts, completions, and their token-id
   form never reach `modelswarm.deepflux.space`; receipts reach the hub as
   counters and digests only. No exception exists or is contemplated.
2. **Automatic engagement without the gate.** The engage gate, the
   fastest-single comparator, the margins, and the mandatory fallback are
   untouched and outermost; no automatic speed-up path is created; user
   content crosses the family wire only under explicit election.
3. **Any change to the frozen §6 tags or the `EligibilityLease` shape.**
   `protocol/messages.proto` and `protocol/msp-v1.md` stay byte-identical
   (asserted by the existing guards and the wire-compat CI job);
   `protocol/vectors/lease-hubkey-1.json` keeps byte-pinning the lease;
   the §6.5 effective registry is untouched (the two new reasons are
   cooperative-namespace G3-class vocabulary).
4. **Receipts carrying content.** ReceiptV2 stays verifier-claimed
   counters under the ADR-030 grant rules — no token ids, no commitments,
   no content-derived values in receipts.
5. Additionally, in the same spirit: no KV-cache transfer (AGENTS.md
   constraint 8 — token ids for the peer's OWN prefill are prompt
   carriage, not KV transfer, and nothing more is authorized); no change
   to the neutral `FAMILY_PROMPT_V1` path or the expert-file seam; no
   approximate cooperative tiers (`deep`/`maximum`) carrying user content;
   no roster or wire field for carriage posture; no cooperative-plan
   scheduling (§3.4 records direction only).

## 6. Gate requirements (right-sized for the relaxed posture)

**Golden vectors** (ADR-032 §7 fixture discipline: fixture-only signing
keys recorded in the file, never production; `verify_at` time anchor; wire
form + `canonical_json_sha256` byte-pin; expected outcome with the EXACT
code; consumer-neutral — any second implementation must reproduce
identical canonical bytes, signatures, and outcomes):

| Fixture | Pins |
|---|---|
| `pin-carrier-1-accept.json` | Full accept: canonical-JSON bytes + sha256; signature preimage (signature-omitted canonical form); the LE-u32 commitment construction byte-exact; `round: 0` + the seq-baseline/nonce-ledger seeding; offer-consistency binding; accept outcome |
| `pin-carrier-2-mismatch.json` | `pin_commitment` ≠ sha256(`prompt_tokens` LE-u32) → `SessionReject` reason `pin_mismatch`, reject canonical form byte-pinned |
| `pin-carrier-malformed.json` | Array, each with `expected_reject`: unknown fields; envelope tag; non-bare-hex commitment; non-u32 token; empty `prompt_tokens`; wrong `protocol_version`; bad signature; `sender` ≠ PeerId; carriage-off → `content_carriage_refused`; context-overflow → `budget_infeasible` |
| `session-reject-1.json` (extension) | The two new reasons' canonicalizations; the six existing entries unchanged bytes |

Property/pin tests (session crate, per-push): strict-schema first-failure
order; carrier-sessions' first `VerifyDrafts` round ≥ 1; `seq` monotone
over the carrier baseline; carrier nonce single-use (reuse →
`replayed_request`); the carrier consumed before the serve loop and never
routed; the disjointness extension (§4); the fixed open-sequence order
(lease → pin → offer; deviations `bad_frame`).

**Redaction tests.** Family frames join the prompt-redaction test surface
(the phase discipline: every code path that first touches prompt text
lands a redaction gate with it — the carrier is that path for the family
wire): carrier token ids asserted absent from node logs, telemetry,
rejection frames, receipts, the hub DB, and the webview; the counts-only
assertion for family events extends to the carrier fields (token COUNT
permitted; arrays and content-derived hashes never); KV residency bounded
by session teardown asserted at the engine seam.

**Threat-model updates (`docs/threat-model.md`, honest restatement under
the owner's posture):** adversary B's scope — under default-on carriage,
EVERY cohort member of an elected family-verified chat receives the full
conversation as token ids and can read, log, or repurpose it; the owner
accepts this (directive, 2026-10-13); remaining mitigations stated without
overstatement (transport encryption only, no hub visibility, invite-only
population, teardown-bounded KV, honest disclosures, no
confidential-computing claim). A §4.2 row for the carrier exposure; a
KV-residency row (verifier engine KV holds the conversation until session
teardown; the cache-cold discipline and the routed dribble-residence
lifetime-ceiling advisory apply). The consent-UX attack surface is
downgraded per the owner posture (no consent modal exists to phish; the
residual is a user tricked into opting out, or a peer misrepresenting its
posture — the refusal label is honest either way). §5.2's non-guarantee
extends: the serving peer reads every prompt it serves, and every
verifying peer of an elected family-verified chat reads the conversation
it verifies.

**`docs/privacy.md` §2 update + onboarding reword (pre-recorded
obligations, now due for this mode).** The "Planned change (follow-up
track, gated)" paragraph is discharged for this mode: record the carrier
recipient set (every cohort member of the elected session, bounded by the
preset cohort cap and `budget.max_endpoints` — 1 in the shipped
configuration — plus the requester's own machine; token-id form), and
reword the onboarding disclosure to the §3.1 texts. The remaining
cooperative modes stay gated with the paragraph's obligation standing for
them.

**E0 caveat (standing, extended).** Divergence labels in carrier sessions
— `pin_mismatch` at open (a pairing fact) and in-session
`verifier_divergence`/acceptance collapse — mean CONTEXT-OR-BACKEND
DIVERGENCE CLASS, never proof that a specific peer lied; cross-machine CPU
determinism remains E0's open sub-question
(`docs/verification/e0-determinism-2026-10-07.md`) and gates any exactness
claim arising from cross-peer verification. The lossless
`speculative_exact` contract is unchanged (ADR-032 §3).

**Review seats.** Security Engineer (adversary matrix for the first
content-bearing family surface: the carriage-refusal path, the
commitment-oracle analysis — the serving peer already holds the payload,
so the commitment check is self-consistency and discloses nothing beyond
it; replay/reorder at open; the disclosure contract) and Test and Release
Engineer (fixtures, redaction suite, default-on posture tests — a fresh
install carries a carrier session with no extra configuration; the opt-out
refusal path; disclosure visibility on both ends; ladder-label stability)
review the implementation handoff BEFORE any carrier traffic. CI venue:
the vectors and property suites ride the existing per-push
`rust.yml` verifydrafts feature steps; the tracker surface is untouched,
so the TS-tracker-vs-Rust-client wire-compat job is unaffected;
`messages.proto` byte-identity is asserted by the existing guards.

## 7. Consequences

+ The owner's swarm-together posture gets a real wire path: family
  verification of actual conversations, co-produced and cross-checked by
  peers hosting the same exact profile — and ADR-034 rule 8's original
  goal is achieved for both session classes: a pairing mismatch is a
  session-open reject, never a first-round observation.
+ The neutral path, the probe, the lease carrier, the frozen msp-v1
  surfaces, and the engage gate are all untouched; the change is
  additive to the cooperative namespace with its own vectors.
− One more content-bearing surface to defend forever: the family wire is
  no longer vacuously content-free, so every redaction claim must be
  TESTED, not implied — the permanent per-push cost ADR-032 §7 already
  enumerated, now with real teeth.
− N-party conversation exposure is real, accepted by the owner, and must
  be stated plainly in the threat model, privacy model, onboarding, and
  both UI disclosures — never softened (§3.4's performance honesty
  discipline applies to the privacy statements too).
− Resource posture: carrier sessions construct a per-session engine
  (conversation root) alongside the serving sidecar; the ADR-035 governor
  owns admission, and the Resource Engineer coordination queued by
  ADR-034's resource note extends to this surface.

## 8. Evidence base (all committed)

`docs/reviews/adr034-rule8-options-2026-10-13.md` (the options analysis
this ADR executes) · ADR-034 (rules 1–8, the 2026-10-13 revision note's
frozen label vocabularies, the resource note) · ADR-032 (§1 rules 1–5,
§2 binding set + F1/F2/F3 freeze records + rule (i), §3/G3, §7 fixture
discipline and venue matrix, §8 state machine) · ADR-026 (the lease gate
the open sequence rides), ADR-029/030/031, ADR-035 (governor admission) ·
`protocol/msp-cooperative-v1.md` (§1/§5 as amended), `protocol/msp-v1.md`
(§2.2, §6.4, §6.6), `protocol/messages.proto` (byte-identical) ·
`docs/privacy.md` §2, `docs/threat-model.md` §3/§4.2/§5.2,
`docs/verification/e0-determinism-2026-10-07.md` · the 9.6 pass-2
negatives (`experiments/raw/PASS-2-ENGAGE-{LAN,LOOPBACK}-2026-10-10/
ANALYSIS.md`) · the implementation seams:
`crates/modelswarm-session/src/negotiation.rs` (`SessionOffer` v3,
`SessionReject`, `SessionRejectReason`),
`crates/modelswarm-node/src/verifydrafts/client.rs` (`negotiate` — the
lease-carrier send the pin carrier follows),
`crates/modelswarm-node/src/verifydrafts/serve.rs` (`read_session_open`,
the honest-reject path), `crates/modelswarm-node/src/verifydrafts/mod.rs`
(`WireSessionLease` — the precedent),
`crates/modelswarm-node/src/verifydrafts/engine_source.rs`
(`FamilyEngineSource::Capi { prompt_tokens }` — the prefill seam),
`crates/modelswarm-desktop/src/app.rs` (`family_engage_reason`,
`compose_family_serving`, `family_probe`), and
`crates/modelswarm-session/tests/verifydrafts_vocabulary_disjointness.rs`
(the rule-(i) collision test this ADR extends).

## Revision note (2026-10-15, owner directive "everything on by design"): the requester election is ALWAYS ON — the toggle is retired

Owner ruling (verbatim): "why do we have a button to turn on Family?
this should always be on, its the sole porpuse of this project, try
to reduce the number of switches, everywhere, assume everything is
on by design."

Changes to §3.3's election semantics:

- The requester-side election toggle (`family_verified_chat`
  persisted key, `set_family_verified_chat` IPC,
  `verified-chat-switch` UI) is REMOVED. Election is unconditional:
  every swarm chat turn attempts the carrier-session verification
  path first, with the existing honest pre-first-token fallback to
  the plain swarm path (the `elected_fell_back` label — exercised
  live by the owner's cross-NAT C-machine test on 2026-10-15).
- The §3.1 requester disclosure is PERMANENTLY visible (no longer
  gated on election). The frozen text drops its final sentence
  ("Turn it off any time.") — there is no toggle to turn off; the
  remaining text is byte-identical.
- The serving-side carriage default (ON) and its opt-out toggle are
  UNCHANGED (they are the other party's consent surface), as are the
  wire contracts, the pin discipline, and both §3.1 texts' remaining
  content.
- The automatic engage ladder (§3.3's no-content mode) is superseded
  at runtime — the always-elected path runs instead. The ladder code
  and its stable-label vocabulary remain in place this release
  (compiled, probe-exercised, `last_engage_reason`-fed for the
  ADR-037 assistant context); its retirement is a recorded follow-up
  to be reviewed with the relay adoption batch.
- Old configs carrying `family_verified_chat` are preserved verbatim
  (unknown keys are never deleted) and the key is simply never read
  again.
- The retired event leaves the advisory egress surfaces in the same
  change: `settings.family_verified_chat` is removed from
  `protocol/advisory-egress-v1.json` and the frozen fixture inventory
  (the R-1 two-sided tie requires artifact == source == inventory; a
  stale row with no emit site fails the guards job — the tie caught
  exactly this on run 38673888323).
