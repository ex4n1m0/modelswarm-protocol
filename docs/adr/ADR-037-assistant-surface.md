# ADR-037: The assistant surface — a user-facing setup helper and expanded advisory role behind the chat wall

Status: **Proposed (2026-10-13) — paste-ready for the Integrator; acceptance
per the ADR log process. Owner-directed; the directive is quoted verbatim in
§1. Authored against HEAD `1e371ed` (advisory wiring not yet in tree — the
first implementation session lands against ADR-033 revision 2 AS AMENDED by
this ADR). Extends ADR-033 (Accepted, revision 2) by recorded amendment
(§4): the four §1 invariants stand; the §2 non-goal "No user-facing chat" is
superseded exactly and only by the creation of this ADR's SEPARATE assistant
channel, while its other half — the swarm chat box never routes to the
advisory API — is made a permanent structural wall (§3.1). The §2 suggestion
vocabulary, §4 config table, §6 gap vocabulary, and §9.2 item set extend as
enumerated in §4. ADR-036's domain (the user-content pin carrier, family
sessions) is untouched. `protocol/msp-v1.md`, `protocol/messages.proto`,
`catalog/schema.json`, `catalog/schema-v2.json`, and `protocol/vectors/`
stay byte-identical — ZERO protocol wire fields, so the
TS-tracker-vs-Rust-client wire-compat CI job and the guards frozen-surface
pins are unaffected.** Owner: Protocol Architect (contract, this ADR, the
ADR-033 revision note, vocabulary/artifact amendment rows). Implementation:
Windows Product Engineer (advisory client core per ADR-033 §9.4, then the
assistant surface — §7), with Protocol, Security, and Test and Release
review. No implementation lands before both gate seats sign the §6
checklist; no assistant traffic before the chat-wall tests are green in CI.

## 1. The owner directive, and the wall that survives it

Verbatim, 2026-10-13 night: **"We can keep this design for the api key so
that it doesn't spend so many tokens, however i don't mind to make it a
central part of the solution as long as the main rule of not responding
chat questions is solid, we can make this api key talk to the user about
the recommended model for his machine, help with anything related with the
installation and troubleshooting, and take a more active role in the
management, to make this solution feel and be intelligent from its core,
i will support these costs no problem at all."**

The directive funds four things: keep the token-cheap digest design
(ADR-033 §2–§4 discipline — kept byte-for-byte); add a user-facing
assistant that talks about model fit, installation, and troubleshooting;
expand the advisory role in management; and pay the costs — all bounded by
ONE solid rule the owner states as the condition: **the advisory API never
answers chat questions.** This ADR makes that rule architectural (§3.1)
and then spends the owner's authorization on exactly the four funded
things (§3.2–§3.4). The two ADR-033 framing conditions survive verbatim:
the endpoint is **not part of the decentralized swarm**, and the digest
plane remains **few-tokens by design**; the assistant plane is
owner-funded and separately capped (§3.4).

## 2. Context

ADR-033 revision 2 (Accepted; implementation gate open) specifies an
owner-deployment, content-blind, advisory-only, fully-fallback-capable
management plane: typed-accumulator digests, one value-typed egress gate
plus the `Redactor` scrub net, timer-driven cycles, `advisory.json` config,
a pure injectable client core, and the shadow-measured promotion bar. What
it deliberately excluded — "No user-facing chat" — excluded two very
different things at once: (a) answering SWARM chat questions with the
cloud model (a trust and content catastrophe, permanently forbidden
here), and (b) ANY user-facing conversational surface (a setup helper that
never touches swarm content). The owner's directive accepts (a) is
forbidden forever and funds (b). Splitting them requires a wall that is
structural, not behavioral, because the failure mode of a "we promise not
to" seam is exactly the accretion ADR-033's Context warned about. The
shipped product already has every deterministic ingredient the assistant
explains: the M1 hardware profile
(`crates/modelswarm-resource/src/profile.rs` — `HardwareProfile`:
`logical_cpus`, `total_ram_bytes`, `gpu: Option<GpuGeometry{name,
total_vram_bytes}>`, zero = honest unknown), the catalog hardware rows
the picker derives (`hardware_requirements` → `download_mb`, `ram_mb_est`,
`crates/modelswarm-desktop/src/app.rs:1352`), the deterministic recommender
(`fit_and_score` → `gpu_full | gpu_partial | ram_only` plus score,
`app.rs:1424`), the top-6 shortlist (`app.rs:1879-1910`, owner directive
2026-10-07), the census join (`app.rs:1912-1933`), the honest degrade
vocabularies (`FamilyStatus` reasons, `app.rs:170-204`; the
`family.engage` ladder, `app.rs:3746`; probe/pairing verdicts and
`pairing_detail` labels, ADR-034 revision note; governor verdict labels
`ram_ceiling | vram_ceiling | ram_unobservable | vram_unobservable |
vram_total_unknown | weights_not_positive`, gap `kv_unmodeled`, and
enforcement labels, `crates/modelswarm-resource/src/governor.rs`; the
effective msp-v1 §6.5 error registry; the ADR-033 §6 gap vocabulary), and
the ADR-023 HF search/request surface. The assistant's job is to EXPLAIN
these — never to replace them.

## 3. Decision

### 3.1 The chat wall (normative; invariants W1–W6)

**(W1) NO ROUTE, EVER.** The swarm chat box — the desktop "Test box"
surface (`send_chat`/`try_swarm_chat`, `crates/modelswarm-desktop/src/app.rs`)
and every path it can reach: the msp-v1 §6 single-serving stream, ADR-036
family sessions, the loopback gateway (`127.0.0.1:11435`), the local
fallback — NEVER calls the advisory API, never sends chat content to it,
and never renders advisory output as or inside a chat reply. No
completion-request path anywhere in the workspace may reach the advisory
client. This is the owner's "main rule," made structural.

**(W2) TWO ENTRY CLASSES, ONE CLIENT, ENUMERATED CALL SITES.** The
advisory client (ADR-033 §9.4: `crates/modelswarm-node::advisory` or a
dedicated crate with Integrator sign-off) has exactly two entry classes:
the timer-driven digest cycle (ADR-033 §4) and the user-triggered
assistant turn (§3.7 here). The complete call-site list is pinned by
test; a third call site requires an ADR-037 amendment. No call path
exists in any request, serving, scheduling, negotiation, gateway, or
telemetry-emit path (ADR-033 invariant A5, extended to cover the
assistant command: it is user-triggered UI, never an internal trigger,
never a hot-path hook, never a chat-turn side effect).

**(W3) DATA WALL.** The assistant request builder may read exactly two
sources: the assistant conversation store and the typed context-block
builders (§3.5). It is structurally unable to read `chat_log`, `ChatTurn`,
`chat-delta` events, serving frames, gateway request/response bodies,
receipt payloads, or ADR-036 carrier data (`prompt_tokens`,
`pin_commitment`). The digest cycle keeps the ADR-033 §3.2 typed
accumulators. Both planes share the one egress choke point and the
post-serialization re-scan (ADR-033 §4, RC-3), extended per §3.5 below.

**(W4) UI WALL.** The assistant is its own panel: separate section,
distinct visual treatment, its own input box, its own event namespace
(`assistant-*` webview events; the chat transcript renders only
`chat-delta`-fed turns). Assistant output never renders inside the chat
transcript; chat input never feeds the assistant; the chat box never
displays advisory output; no autocomplete, suggestion chip, or
completion of chat input may originate from the assistant plane.

**(W5) THE WALL OUTRANKS FEATURES.** Any future desire to let the
assistant see or shape chat content (summarize a conversation, suggest
prompts, prefill chat input, triage a chat failure inline) is FORBIDDEN
in v1 and may exist only under a future ADR that the owner signs naming
the exact exception. Until then, a feature request is not a waiver.

**(W6) NO SILENT SUBSTITUTION.** Assistant absence, failure, or refusal
changes nothing about chat behavior (ADR-033 invariant (c) extended):
same peers, same scheduler picks, same gateway semantics, same replies.

### 3.2 The user-facing assistant channel (new surface)

A desktop-app panel, shipped to every install in its DEGRADED static form
and cloud-backed exactly where a key is locally configured (§3.6). Three
jobs, each grounded in what the product already computes:

**(a) Hardware-aware model recommendation help.** The context block
carries the M1 hardware profile and the picker's per-profile rows
(`display_name` from the signed catalog/candidate rows — owner-controlled
strings per ADR-033 RC-11; quant method/bits; `download_mb`;
`ram_mb_est`; fit label; `recommended` flag; census peer count/chip). The
API may rank, explain, and reword IN THE ASSISTANT PANEL — why this
machine's shortlist looks the way it does, what fit tier means, what the
quant tradeoffs are. The deterministic recommendation stays the fallback
AND the authority of record: `fit_and_score`, the top-6 `recommended`
membership, fit labels, and card order are computed exactly as shipped
and are not inputs to any advisory mutation (there is no mutation path —
§6 pins them byte-identical with the plane on and off). If the
assistant's ranking disagrees with the deterministic shortlist, both are
shown and the disagreement is labeled as advice. The assistant may reason
up to the 90% RECOMMENDATION bar the governor already documents
(`RECOMMENDATION_BAR_CEILING_PERCENT = 90`,
`crates/modelswarm-resource/src/governor.rs:48-53`); the governor's
enforced 70% ceiling (`ENFORCED_CEILING_PERCENT`) decides actual
admission, and the assistant must not contradict it.

**(b) Installation and troubleshooting help.** The context block carries
the daemon's honest status as LABELS + COUNTS + TYPED FACTS: the
`FamilyStatus` state and off-reason, the `family.engage` ladder label of
the last turn, probe/pairing verdicts, governor verdict labels and gap
flags, hosting/serving status labels, effective §6.5 error-code counts,
the ADR-033 gap vocabulary counts, `finishReason` tallies, `capacityClass`
enum, app version and OS platform. NEVER raw log files, never JSONL
lines (the RC-4 discipline extended: stringly `detail` text stays local
and reduces per the ADR-033 §3.2 table; the static corpus explains labels
deterministically). The assistant explains what a label means and what to
try next; it cannot change the state it explains.

**(c) Honest capability bounds.** The fixed, repo-versioned system prompt
(ADR-033 §3.2, extended) states the contract to the model: your context
is machine/setup metadata and this conversation; say "I don't know" when
unsure; never invent product behavior, version numbers, settings, or
protocol rules; your advice about protocol and product decisions is
advisory — the app's deterministic rules decide. Structurally: every
assistant reply that names a product state renders ADJACENT to the local
status facts it discusses (the ADR-033 AD-3 display-pairing rule extended
from peer claims to product-state claims, so fabrication is visible at a
glance), and replies render plain-text-only with the untrusted-provenance
label (§3.7 rendering contract).

### 3.3 The expanded management role (enumerated; all advisory-never-authoritative)

The ADR-033 §2 suggestion-type vocabulary extends, frozen by this ADR,
from `triage_summary | tuning_hint | catalog_note | digest` to add
exactly two display-only members:

1. **`peer_pattern_note` — peer-pattern anomaly summaries.** Per
   16-hex-prefix tallies (the RC-1 rendering) of divergence verdicts, gap
   codes, reject-code distributions, acceptance/EWMA snapshots, and
   repeat-offender correlation across cycles — the "which peers keep
   diverging and when" view the operator currently assembles by hand.
   Display-only.
2. **`catalog_presentation_hint` — model-catalog presentation hints.**
   Explanations and emphasis for the picker (e.g. why a profile tops this
   machine's shortlist, what a census count means for swarm availability).
   Display-only, and structurally toothless: these hints cannot alter
   `recommended` membership, fit labels, or card order (the census-join
   order rules of `app.rs:1912-1933` stand); catalog CURATION proposals
   remain `catalog_note` under the ADR-033 §2.3 owner-review flow.

3. **The assistant channel as the interactive management surface.** The
   digest surfaces remain timer-driven and rendered in the operator view;
   the assistant answers interactive follow-up about them. Swarm-health
   digests extend the existing `digest` class with governor verdict
   distributions and capacity-class census — no new type needed.

**Promotion bar unchanged (normative).** `tuning_hint` remains the only
behavior-influencing class and its ADR-033 §5 promotion criteria (≥100
joined differing (trace, hint) pairs, strict-majority win on the pinned
promise metric against the fastest eligible single host, non-regressing
worst-cell median, human promotion with a `docs/verification/` verdict
artifact) are byte-unchanged. The new classes are display-only forever.
`tuning_hint` is DIGEST-PLANE ONLY: an assistant reply can never carry
one (§3.7 malformed table) — a provider conversation must not be able to
mint tuning hints, full stop. Shadow-measured-before-trusted, honest
negatives published, deterministic fallbacks always live: all unchanged.

### 3.4 Budget and cadence (the new cap shape)

The digest plane's ADR-033 §4 caps are UNCHANGED (`max_calls_per_day` 24,
`min_interval_ms` 900000, `events_threshold` 200, timer-driven) — the
"doesn't spend so many tokens" design the owner said to keep. The
assistant plane gets its own owner-funded budget with a conversational
burst allowance, same discipline:

| Key (`advisory.json`) | Default | Meaning |
|---|---|---|
| `assistant.enabled` | `false` | master switch; false or key-absent = static-help mode (§3.6) |
| `assistant.max_calls_per_day` | `300` | hard per-UTC-day cap, assistant turns |
| `assistant.max_calls_per_hour` | `60` | burst bound: ≥1 min spacing sustained; bounds runaway loops |
| `assistant.max_turns_per_conversation` | `40` | per-conversation cap; idle reset below |
| `assistant.idle_reset_ms` | `900000` | 15 min idle closes the conversation (fresh turn budget next time) |
| `assistant.history_turns_sent` | `12` | context discipline: last N assistant turns sent |
| `assistant.max_request_bytes` | `49152` | per-request body cap (48 KiB) |
| `assistant.max_completion_tokens` | `1024` | provider-side output bound per turn |
| `assistant.request_timeout_ms` | `45000` | per-call deadline |

Rules carried over verbatim from ADR-033 §4: failed calls consume budget
(calls initiated, not successes); caps are enforced pre-call and fail
closed; the day window is the UTC calendar day (midnight rollover
intended and pinned); no in-turn retry — the next user turn is the
retry. Worst-case daily envelope is computable from the caps and nothing
else: ≤ 324 calls (300 + 24), ≤ 307,200 assistant completion tokens +
49,152 digest completion tokens, ≤ 15.2 MB + 0.4 MB request bytes per
day. Defaults are frozen by this ADR and tunable without one
(config-recorded values, golden-pinned per §4.4); the owner funds the
envelope. The AGENTS.md stop-point on paid cloud resources remains
satisfied by the ADR-033 decision record's provider authorization
(DeepSeek, this bounded scope); a second provider, a different pricing
class, or shipping the owner's key to other installs re-opens the owner
gate.

### 3.5 What may be sent (the privacy contract)

**Digest plane: UNCHANGED** (ADR-033 §3.2, verbatim — typed accumulators,
the value-typed allowlist, the vocabulary artifact, the Redactor net,
byte caps).

**Assistant plane sends exactly:**
1. **What the user types into the assistant channel.** Said plainly:
   this is a conversation about the user's machine and setup; the text
   goes to the provider. It is the ONE deliberate exception to the
   digest plane's no-free-form-text rule: user-authored in the assistant
   channel, disclosed at the surface before the first call (§3.6),
   structurally sourced only from the assistant conversation store (W3),
   scrubbed by the Redactor net (keys, JWTs, Bearer, hex runs ≥ 32), and
   byte-capped. It is NOT swarm chat content and cannot become it (W1,
   W4).
2. **The typed context block**: hardware profile fields (integers; GPU
   name string bounded to the engine's device-listing text; honest
   zero/null sentinels), picker rows (§3.2a; `display_name` strings drawn
   only from the signed catalog and candidate rows), status labels and
   counts (§3.2b, frozen vocabularies only), app facts (version, OS
   platform, backend label, `capacityClass` enum), and the last
   `history_turns_sent` assistant turns.

**NEVER sent, either plane:** chat-box content (`chat_log`, `ChatTurn`),
serving prompts or completions, token ids, ADR-036 carrier payloads
(`prompt_tokens` arrays, `pin_commitment`, any content-derived hash), raw
base58 PeerIds (16-hex prefixes only), log files or JSONL lines, free
`detail` strings, file paths beyond the frozen vocabulary, secrets,
keys, bearer material, HF tokens. The context block passes the same
value-typed allowlist discipline as the digest (its field set joins
`protocol/advisory-egress-v1.json` — §4.2); the post-serialization
re-scan at the choke point verifies the context block re-matches the
allowlist, the user text passes `scrub_content`, canary swarm-chat bytes
are absent, and the body is within `max_request_bytes` — else nothing is
sent.

**The hub stays content-blind; the provider stays outside the swarm.**
The API call originates client-side from the desktop/node process with
the locally configured owner key; `modelswarm.deepflux.space` is never in
the path, receives nothing about these calls, and changes by zero bytes
(no tracker surface, no hub change of any kind). DeepSeek is not the
hub, not a peer, holds no keys, signs nothing, admits no one, and has no
routing or protocol authority (ADR-033 invariant (b), unchanged). The
plane's own telemetry (`assistant.*` events) is counts and labels only
and is excluded from digest aggregation exactly as `advisory.*` is.

### 3.6 Failure posture, key handling, disclosure

**Full function without the key — no dead UI.** The assistant panel ships
in every install. Without a key, with `assistant.enabled=false`, or on
any provider failure, it degrades to the STATIC HELP CORPUS: a
repo-versioned, app-bundled, label-keyed set of honest help entries
(Windows Product authors; every `FamilyStatus` off-reason, governor
verdict label, engage-ladder label, probe verdict, and picker fit tier
has a static entry — coverage is testable against the frozen
vocabularies) plus the deterministic recommendation, which is always
live in the Models picker regardless. The panel states which mode it is
in and why, using the gap vocabulary — never a blank box, never an error
wall, never a spinner that hides absence.

**Mid-conversation failure.** The failed turn renders the honest gap
label in the panel; history is retained locally; the next user turn
retries within caps. Nothing else in the app changes (W6, invariant (c)).

**Key handling.** ADR-033 §3.4 unchanged: the outside-repo key file,
env-first resolution, owner-readable only, the guards `sk-[0-9a-f]{32}`
tracked-file scan (`.github/workflows/rust.yml`, guards job, step "No
provider API keys are committed", `1549678`), the Redactor `sk-` scrub,
no key-bearing field type in either plane's payload.

**Rotation — standing owner directive recorded (2026-10-13).** The owner
has directed that the current key NOT be rotated until he says so;
ADR-033 §3.5's rotation and its §9.2(9) evidence precondition are
therefore DEFERRED by owner decision until the owner calls for rotation.
Recorded honestly as an accepted risk: the key transited a chat channel
once; the guards scan is the backstop, not the defense; chat remains a
non-key-channel going forward. The deferral covers THIS key only — any
future key starts clean under the §3.5 rule (console → direct local
entry; rotation before its first production use). No rotation action may
be taken, by any agent, until the owner says so.

**Disclosure and consent (normative UI contract).** Before the FIRST
cloud call on an install, the assistant panel shows a first-use
disclosure that must be acknowledged (the call is blocked until then):

> "This assistant sends what you type here, plus facts about your machine
> and setup (hardware, model list, status labels — never your swarm
> chats), to a cloud service (DeepSeek) using a key configured on this
> machine. It is advice only — it cannot change any decision this app
> makes. You can turn it off any time in Settings."

While any cloud reply is displayed, a persistent provenance chip is
always visible: "Cloud assistant — advice only; this app's rules
decide." In degraded mode the panel states: "Built-in help — the cloud
assistant is off/unreachable (<gap label>)." Disclosure visibility,
first-call gating, and opt-out behavior are test-obligations (§6).
Opt-out is a config flip with zero functional loss.

### 3.7 Assistant-plane client behavior (state machine, schema, malformed handling)

The assistant plane rides the SAME pure injectable client core
(`step(state, input) -> (state, effects)`, injected clock + transport
double — ADR-033 §4, TR-RC-4/TR-AD-2) with its own per-conversation
state machine:

```text
IDLE ──user sends assistant turn AND assistant.enabled AND key AND config
  ▲    AND day+hour caps unspent AND conversation turn cap unspent──► BUILD
  │                                                            │ typed context block
  │                                                            │ + last N turns
  │                                                        SCRUB (allowlist for the
  │                                                            │ context; scrub_content for
  │                                                            │ the whole body; choke-point
  │                                                            │ re-scan, §3.5)
  │   gap(empty_after_redaction | request_oversized) ◄── block failed / body
  │         gap(disabled | no_key | no_config | cap_reached |── oversize even at
  │         rate_limited)                                     │  zero history
  │                                                        CALL (HTTPS, timeout)
  │         gap(timeout | http_error | quota | response_oversized)
  │                                                            ▼
  │                                                        PARSE (strict echo check)
  │         gap(schema) ◄── bad JSON / unknown schema / echo mismatch ──┤
  │                                                            ▼
  │                                                        RECORD (display +
  │                                                            │ assistant.turn.sent |
  └────────────────────────── IDLE (await next user turn) ◄────┘ assistant.turn.gap)
```

**Request/response schema (local, frozen by this ADR; not a protocol
surface).** Request: the fixed repo-versioned system prompt (§3.2c), the
typed context block, and the trailing `history_turns_sent` turns.
Response: `{"schema":"advisory-assistant/v1","conversation_id":"<16
hex>","turn":<u32>,"reply":"<display-only free text>","hints":[{…the
ADR-033 §4 suggestion shape, DISPLAY-ONLY CLASSES ONLY…}]}`.
`conversation_id` is the first 16 hex of a fresh
`identity::new_nonce()` at conversation open (the RC-8 generator pin,
reused); a response whose `conversation_id` or `turn` does not echo the
in-flight request is discarded whole (`gap schema`). Rendering: plain
text only — no markdown or HTML interpretation, no clickable or
auto-linkable URLs, a 4,000-character display cap, the untrusted-
provenance chip adjacent always (the RC-5 contract, extended), and the
AD-3 pairing of product-state claims with local status facts (§3.2c).

**Malformed/adversarial response handling (fail-closed,
first-failure-wins):**

| Injected condition | Outcome |
|---|---|
| transport error / HTTP non-2xx | gap `http_error`, no retry this turn |
| deadline exceeded | gap `timeout`; a late response is a no-op discard additionally recording diagnostic `late` (same firing rule as ADR-033 §6/TR-RC-4) |
| response > `max_response_bytes` (65536, shared) | gap `response_oversized` |
| invalid JSON / unknown `schema` / `conversation_id` or `turn` echo mismatch | gap `schema`, whole body discarded |
| a `hints` entry of class `tuning_hint` | that hint discarded and counted (digest-plane-only rule, §3.3) — the reply still renders |
| unknown hint `type` / non-finite hint value | discarded and counted; siblings parse (ADR-033 §4 table) |
| empty or absent `reply` with no parseable hints | gap `schema`, whole body discarded (no fabricated fallback text) |
| `reply` contains instruction-like text | display-only by construction; no parse path exists |
| `reply` exceeds the display cap | truncated at render; never parsed |

## 4. ADR-033 amendments (exact texts and enumerated extensions)

**4.1 Append this revision note to
`docs/adr/ADR-033-advisory-plane.md` (verbatim):**

> ## Revision note (2026-10-13, ADR-037): the assistant surface and the
> expanded advisory role
>
> The owner directive of 2026-10-13 ("…as long as the main rule of not
> responding chat questions is solid… make this api key talk to the user
> about the recommended model for his machine… take a more active role in
> the management… i will support these costs") extends the plane via
> ADR-037. Amended, exactly and only as follows. §2's non-goal "No
> user-facing chat" is superseded SOLELY by the creation of ADR-037's
> separate assistant channel under its chat wall (ADR-037 §3.1, W1–W6);
> the other half of that non-goal — the swarm chat box and every
> completion path never reaching this plane — is PERMANENT and now
> structurally pinned (ADR-037 §6). Invariant (a) gains one named
> exception: the user's own assistant-channel text, under ADR-037 §3.5's
> disclosure, sourcing (W3), and scrub rules; it is unchanged for every
> swarm/serving surface. The §2 suggestion vocabulary extends with
> `peer_pattern_note | catalog_presentation_hint` (display-only). The §4
> config table gains the nine `assistant.*` rows (ADR-037 §3.4);
> `max_calls_per_day` 24 remains the DIGEST-plane cap. The §6 gap
> vocabulary gains `rate_limited | request_oversized` (14 values). §9.2
> gains items (10)–(14) (ADR-037 §4.4). The four invariants (a)–(d), the
> §5 promotion bar, and the §3.2–§3.4 mechanics are otherwise unchanged;
> the §3.5 rotation is deferred by standing owner directive until the
> owner calls for it (ADR-037 §3.6 records the accepted risk).

**4.2 Contract artifacts.** `protocol/advisory-egress-v1.json` does not
exist at this ADR's writing; its initial content (created by the first
implementation session under ADR-033 rev 2, then frozen) is EXTENDED by
this ADR to also carry: the assistant-plane context-block field names and
types (integers, bounded enum strings, the 16-hex reference form, the
catalog-row `display_name` string class per RC-11, GPU-name string with
its length bound), the `plane` enum (`digest | assistant`) for the
plane's own telemetry rows, the event names `assistant.turn.sent` and
`assistant.turn.gap`, the new suggestion types, and the two new gap
values. The guards completeness tie and the
`tests/fixtures/advisory-egress/` inventory extend symmetrically
(R-1's two-sided mechanism). `protocol/advisory-clamp-table-v1.json` is
UNCHANGED (no new tunable targets are authorized).

**4.3 Gap vocabulary.** 12 → 14 values: `rate_limited` (assistant hourly
burst bound) and `request_oversized` (assistant body over cap even at
zero history). The golden enum test extends; a further value needs an
ADR-037 amendment.

**4.4 §9.2 test-item extensions.** Items (1)–(9) stand; the venue matrix
gains rows for (10)–(14) below; item (8)'s golden pin extends from the
eleven ADR-033 defaults to those eleven PLUS the nine ADR-037 §3.4
defaults; item (7)'s A3 proof corpus extends per §6 here. New items:

- **(10) Chat-wall suite (W1–W4).** Layer 1: the ADR-033 §9.2(6)
  cargo-metadata step already bars the frozen decision-crate set from
  depending on an advisory crate — extend it to assert the advisory
  crate is depended on ONLY by `modelswarm-node`/`modelswarm-desktop`.
  Layer 2 (source scope, directional): the advisory client type names
  must not appear in `crates/modelswarm-gateway`, the node serving and
  verifydrafts modules, or the `send_chat`/`try_swarm_chat` scopes; the
  assistant request-builder module must not reference `chat_log`,
  `ChatTurn`, `chat-delta`, serving-frame, or carrier types. Layer 3
  (behavioral): with the plane enabled and keyed and a counting
  transport double, a full chat turn — success path AND every
  `try_swarm_chat` failure path AND the local fallback — records ZERO
  advisory calls; and the enumerated-call-sites pin (W2).
- **(11) Assistant redaction suite.** Canary discipline: seed `chat_log`
  with a canary conversation and a canary ADR-036-style carrier payload;
  run assistant turns; assert the canary bytes appear nowhere in any
  serialized request (all transport-double captures), the context block
  re-matches the allowlist post-serialization, `scrub_content` holds for
  user text (sk-/JWT/Bearer/hex-32 adversarial inputs), and corrupting
  one context field post-gate stops the request.
- **(12) Assistant cadence/cap property** on the virtual clock: day and
  hour caps, conversation turn cap with idle reset, failed calls consume
  budget, UTC-midnight rollover pinned, adversarial turn-flood schedules
  bounded by the caps, no in-turn retry.
- **(13) Disclosure/consent UI tests.** First cloud call blocked until
  disclosure acknowledged; the §3.6 texts render verbatim; the
  provenance chip on every cloud reply; degraded mode renders static
  help with the gap label; opt-out yields zero calls (counting double)
  and no functional change elsewhere.
- **(14) Assistant A3 extension.** With the assistant plane absent vs
  enabled-with-erroring-transport: picker output byte-identical
  (`recommended` membership, fit labels, card order), engage-gate
  decisions byte-identical, and the telemetry stream byte-identical
  after filtering `advisory.*` and `assistant.*`.

**4.5 §9.4 routing additions.** The assistant surface (panel, IPC
commands `assistant_turn` and `assistant_reset`, context-block builders,
renderer, static-help corpus, disclosure UI) is Windows Product
(`crates/modelswarm-desktop`), with Protocol, Security, and T/R review.
The vocabulary/artifact rows of §4.2 are Protocol. The fixture and CI
work of §4.4 is Test and Release. No other seat's crates change.

## 5. What this ADR does NOT authorize

1. **No serving inference.** The assistant/API never completes, drafts,
   verifies, augments, or reroutes any swarm chat or family session; the
   chat wall (W1–W5) has no exception, and the plane remains not-a-peer.
2. **No automatic protocol authority.** Leases, admission, verification,
   receipts, catalog, tracker state, scheduler picks, the engage gate,
   and the governor are untouched; every advisory output is display-only
   or shadow-measured under the unchanged ADR-033 §5 bar.
3. **No hub changes.** The tracker surface is untouched; the hub stays
   content-blind; calls are client-side with the locally configured key;
   DeepSeek receives no protocol role of any kind.
4. **No content telemetry.** No chat content, prompts, completions,
   token ids, or ADR-036 carrier payloads leave the machine to the
   provider or enter telemetry — counts and labels only, plus the
   disclosed assistant-channel text and typed context block of §3.5.
5. **No key redistribution.** The owner's key stays in local config;
   shipping it inside any install, binary, update, or CI secret is
   forbidden (custody scope unchanged; a second custody location
   re-opens the owner gate).
6. **No third provider and no non-owner telemetry extension** without a
   new ADR and owner gate (ADR-033 §3.1 stands).

## 6. Gate requirements

Before any implementation lands: Security Engineer reviews the wall and
the assistant egress surface (enumerate learn/inject: the assistant
channel as a disclosure-bearing free-text surface; prompt injection via
context labels; provider fabrication of product claims — the AD-3
pairing; the enumerated-call-sites pin; the rotation deferral risk) and
Test and Release reviews the §4.4 item set and fixtures. The chat-wall
suite (§4.4 item 10) must be green in CI BEFORE any assistant surface is
built (§7). Venue: everything per-push in `rust.yml` (check/guards) via
transport doubles and the virtual clock; the provider round-trip
(including a first live assistant conversation) is OWNER-LOCAL ONLY,
never CI, never a CI secret, evidence under `docs/verification/`
(the ADR-033 §9.2 venue matrix, extended). Gates per AGENTS.md:
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
-D warnings`, `cargo test --workspace`, plus the desktop
`tauri-shell` feature runs. No `protocol/`, `catalog/`, or
`crates/modelswarm-{types,session,eligibility}` byte changes; the
wire-compat CI job is unaffected.

## 7. Implementation order (Windows Product seat)

1. **Land the ADR-033 rev 2 client core** (not yet in tree): the pure
   injectable state machine, `advisory.json` reading, key-file
   resolution, https-only/no-redirect transport pinning, digest-plane
   caps and gaps — plus `protocol/advisory-egress-v1.json` and
   `protocol/advisory-clamp-table-v1.json` created with this ADR's
   §4.2 rows, and the ADR-033 §9.2(1)–(9) fixtures. Digest plane only.
2. **Land the chat-wall suite** (§4.4 item 10) in the same change or the
   immediately following one — green before any assistant code exists.
3. **Land the assistant surface**: the panel and its event namespace,
   `assistant_turn`/`assistant_reset` IPC, the context-block builders
   (W3-typed), the response renderer with provenance chip and AD-3
   pairing, the static-help corpus, the first-use disclosure, §3.4 caps,
   `assistant.*` telemetry, and the §4.4 item (11)–(14) tests.
4. **Land the management additions** (`peer_pattern_note`,
   `catalog_presentation_hint`) as a small digest-plane extension riding
   the same client.

**Seat notes.** Resource and Scheduler seats: NO changes are required to
`crates/modelswarm-resource` or `crates/modelswarm-scheduler` — the new
classes are display-only, the assistant READS the M1 `HardwareProfile`
and the governor's published constants, and the 70/90 seam is already
documented in `governor.rs:30-53`. If a future revision wants
assistant-originated tuning proposals, that is `tuning_hint` territory:
shadow harness, Scheduler-owned promotion checker, unchanged bar, own
amendment. Gateway seat: unchanged (the wall's layer-1 test merely
asserts what already holds). Tracker seat: unchanged.

## 8. Consequences

+ The product gains an intelligent, honest core the owner funds: a
  setup assistant grounded in the machine's real profile and the
  catalog's real rows, troubleshooting grounded in the product's own
  honest labels, and richer management digests — with the swarm's
  trustless, content-blind properties not weakened but TESTED harder
  (the wall turns "never sees your chats" from prose into a red/green
  build).
+ Every install keeps full function keyless (static help + the
  deterministic recommendation), so the cloud layer remains a
  removable, revocable enhancement — ADR-033 invariant (c) extended to
  the new surface.
− One more permanent surface to defend: the assistant channel is a
  disclosed free-text egress, and its honesty rests on the wall tests,
  the typed context block, and the rendering contract staying frozen —
  drift here is a privacy defect, not a regression.
− Owner-funded spend is now bounded by caps rather than by stinginess;
  the caps are the contract (any loosening is an ADR-037 amendment with
  the owner's re-authorization).
− The deferred key rotation is a standing accepted risk until the owner
  calls for it; it must be restated in every release checklist that
  carries advisory wiring.

## 9. Evidence base (all committed, HEAD `1e371ed`)

ADR-033 revision 2 (the amended base: §1 invariants, §2 scope and
vocabulary, §3.2 egress gate, §4 client/config/malformed table, §5
promotion bar, §6 gap vocabulary, §9 reviews and venue matrix) ·
`docs/reviews/review-adr-033-security-2026-10-12.md` (`86c6c77`) and
`docs/reviews/review-adr-033-testrelease-2026-10-12.md` (`8bb2e91`) ·
ADR-034 and its 2026-10-13 revision note (the frozen label
vocabularies the assistant explains) · ADR-035
(`crates/modelswarm-resource`, the 70/90 seam, gate D11) · ADR-036
(the carrier payloads this ADR's canary tests must prove absent; not
modified) · ADR-023 (the catalog request surface and IPC-command
precedent) · ADR-012 (`capacityClass`) · ADR-025/011/022 (profile
immutability behind every catalog row) · `protocol/msp-v1.md` §6
(the serving path the wall protects) and §9 (the amendment ledger
pattern) · `docs/threat-model.md` §4.3 and `docs/privacy.md` §2/§5 ·
the shipped deterministic surfaces: `crates/modelswarm-resource/src/profile.rs`
(M1 `HardwareProfile`), `crates/modelswarm-desktop/src/app.rs`
(`hardware_requirements` :1352, `fit_and_score` :1424, the shortlist
:1879-1910, the census join :1912-1933, `send_chat`/`try_swarm_chat`
:4679/:4201, `FamilyStatus` :170-204, `family_engage_reason` :3746,
the config writer pins :6239-6341), `crates/modelswarm-resource/src/governor.rs`
(:30-53, :48-53) · `.github/workflows/rust.yml` (guards; key scan
`1549678`) and `.github/workflows/tracker.yml` (wire-compat,
unaffected) · the owner directives of 2026-10-07/10-12/10-13 cited in
the code above.
