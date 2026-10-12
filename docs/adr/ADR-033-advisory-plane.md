# ADR-033: The advisory plane — an owner-funded cloud reasoning endpoint as a non-authoritative management brain

Status: **Proposed (2026-10-12) — the owner decision to accept follows; Security
and Test and Release reviews are required before any implementation lands
(§9).** Owner: Protocol Architect (contract); implementation routing per §9.4.

**Decision record (2026-10-12).** The owner approved the direction and funded
it with a provider key: "a cloud API that I pay not to be part of the
decentralized swarm, but to manage it — use that API brain for decision-making
on protocol decisions that don't require many tokens." Provider confirmed:
**DeepSeek** (DeepSeek-V4.1-Flash available; 1M context — irrelevant to the
digest sizes here, which are byte-capped far below it). The key lives OUTSIDE
the repository, in local client config only (`%USERPROFILE%\ModelSwarm-advisory-key.txt`
pattern on the owner machine; per-install config on clients); the guards
pattern-scan now blocks any `sk-<32 hex>` token from the repo (`.github/workflows/rust.yml`,
guards job, step "No provider API keys are committed", `1549678`). This
authorization satisfies the AGENTS.md stop-point on paid cloud resources for
exactly this provider and exactly this bounded scope (§6); any additional
provider, model, or deployment class re-opens the owner gate.

This ADR is amendment-ready in the ADR-032 style: revision notes append here;
normative text changes only through a recorded amendment.

Evidence base (all committed): ADR-032 (the shadow/advisory precedent set —
honest negatives, measured-before-trusted, untrusted-input wording) · the
shipped deterministic local brain: the frozen cost model + EWMAs
(`crates/modelswarm-scheduler/src/shadow.rs`, `EWMA_ALPHA`), the engaged-loss
engage gate (`crates/modelswarm-bench/src/engage_gate.rs`), the divergence
ledger (`crates/modelswarm-bench/src/divergence.rs`,
`SUSPEND_AFTER_DIVERGENCES = 1`), the rule-6 observed-loss guard · the shadow
measurement harness (`crates/modelswarm-bench/src/shadow_join.rs` — the
LEFT-join-on-realized discipline) · the structural redactor
(`crates/modelswarm-telemetry/src/redact.rs` — `FORBIDDEN_FIELDS`,
`scrub_content`, the `sk-` scrub) · `protocol/msp-v1.md` §5 (EligibilityLease),
§6 (P2P stream), §7 (receipts) · ADR-023 (the owner's catalog request queue)
· `catalog/candidate-profiles/` (the M3 catalog-hardening rows) · AGENTS.md
hard constraints 1, 5, 8 and the coordination/stop-point rules · the v0.2.21
ship state (ADR-032 §7 light-up record: the local fallbacks cited in
invariant (c) are live production code, not plans).

## Context

Every decision the shipped system makes is deterministic and local: the
frozen ADR-013 cost model picks the micro-swarm, per-peer EWMAs carry RTT and
acceptance, the engage gate blocks profiles whose realized engaged attempts
do not beat the fastest-single prediction, the divergence ledger suspends a
peer after its first greedy divergence, and the rule-6 guard aborts within
request. None of this needs help, and none of it is weakened here.

What is expensive today is not deciding — it is *looking*: an operator
triaging a stream of divergence events, deciding which tuning knob to try
next, triaging the ADR-023 community model-request queue and the
`catalog/candidate-profiles/` review rows, and assembling a fleet health
digest. These are metadata-native, token-cheap, attention-expensive tasks —
exactly the class the owner named. The owner has funded a cloud reasoning
endpoint for them, with two non-negotiable framing conditions baked into the
funding: the endpoint is **not part of the decentralized swarm**, and it is
for **decisions that do not require many tokens**.

The danger of adding any reasoning service to a trustless, content-blind
system is that it becomes a trusted party and a content leak by accretion.
This ADR exists to make the opposite path the only path: the plane is
specified so that removing it, disabling it, or having it fail changes
nothing except the absence of advice.

## Decision

### 1. The four invariants (normative — every one is load-bearing)

**(a) CONTENT-BLIND FOREVER.** The advisory plane sees metadata only:
timings, counts, hashes and hash prefixes, divergence verdict flags, roster
health summaries, acceptance EWMAs, and frozen-vocabulary reason codes. It
never sees prompts, completions, token ids, chat messages, or model files —
token ids are completion content in encoded form (the ADR-032 §2 discipline).
This extends AGENTS.md hard constraint 5 from storage and logs to the
advisory egress channel, and it is enforced **structurally, not by policy**:
every field leaving the process for the advisory endpoint passes the §3.2
allowlist plus the shipped `Redactor`; a field failing either gate is dropped
and counted, never sent.

**(b) ADVISORY-ONLY, NO AUTHORITY.** The plane may suggest: de-rank hints,
tuning proposals, triage summaries, catalog curation proposals. Leases,
verification, admission, and every correctness decision stay local and
cryptographic (msp-v1 §5 lease verification; ADR-026 session-open gate;
ADR-032 §2 admission; ADR-030 receipts). The plane signs nothing, admits no
one, holds no keys, and has no installation identity in the swarm. It is not
a peer. No advisory output may mutate protocol, lease, admission,
verification, receipt, catalog, or tracker state — enforced structurally
(invariant A2, §4).

**(c) FULL FUNCTION WITHOUT IT.** Every advisory input has a deterministic
local fallback that is already shipped: the engage gate, the EWMAs, the
divergence ledger, the frozen cost model, the rule-6 guard. An API outage
means zero behavior change beyond losing the advice: the gap is recorded as
telemetry (§6) and nothing else happens. This is testable by construction —
the advisory output is not an input to any production decision function.

**(d) MEASURED BEFORE TRUSTED + DISCLOSED.** Suggestions land in the existing
shadow harness (§5) and may influence production ONLY where measured to beat
the deterministic local policy — the same honest-negative discipline as
ADR-032 (9.6 pass 2 published its negatives; so does this plane). Any peer
whose telemetry feeds the plane gets an opt-in disclosure: metadata-only,
named provider, revocable (§3.5). In v1 the only data controller is the
operator (§3.1), so the disclosure rule is standing but not yet exercised
against third parties.

### 2. Scope of decisions it may touch (v1 — enumerated and bounded)

The plane may produce exactly four suggestion classes. The suggestion-type
vocabulary is FROZEN by this ADR: `triage_summary | tuning_hint |
catalog_note | digest`.

1. **Anomaly triage** (`triage_summary`): summarizing divergence-detection
   events (`divergence-check/v1` verdicts, `verifier_divergence`, gap and
   reject-code distributions) into operator digests. Output is display-only.
2. **Tuning suggestions** (`tuning_hint`): window/cohort/preset and
   scheduler-tuning hints, validated by the shadow layer before any effect
   (§5). Hints may target only parameters that are runtime-tunable or
   config-recorded today; a hint naming a frozen constant is recorded with
   `requires_adr: true` and can never be applied by config alone.
3. **Catalog curation proposals** (`catalog_note`): summaries of the ADR-023
   request queue and the `catalog/candidate-profiles/` review rows, proposed
   for owner review. The plane never writes to the tracker, the catalog
   envelope, or any signed surface; the owner uses the existing §3.5 admin
   flow.
4. **Operator management digests** (`digest`): fleet health, roster
   composition, and honest-negative summaries. Display-only.

**Explicit non-goals (normative).** No inference participation — the plane is
never in the serving path and is NOT a swarm peer; it cannot host, serve,
verify, or propose drafts. No key or custody roles. No automatic writes to
the tracker or any signed object. No user-facing chat. No per-request calls
on the hot path — calls are batch/digest-cadence only ("few tokens" by
design; §4 trigger gates). No economics surface beyond ADR-030's existing
discipline. Nothing in `protocol/msp-v1.md`, `protocol/messages.proto`, or
`catalog/schema.json` changes: the advisory plane introduces ZERO protocol
wire fields, so the wire-compat CI surface is untouched.

### 3. Trust and privacy mechanics

**3.1 Where the client lives (v1).** The advisory client is a component of
the operator's own deployment — the owner's desktop/node daemon composition —
and only that. It is not shipped enabled to community peers: `advisory.enabled`
defaults to `false`, and an installation without a key file is inert by
construction. The v1 privacy surface is therefore the operator's own fleet
metadata egressing to the operator's own paid provider, over TLS. Extending
advisory inputs to non-operator peers' telemetry is a future revision with
its own ADR and owner gate.

**3.2 Request shape (the redacted metadata digest).** A cycle aggregates
local telemetry accumulated since the last successful cycle into aggregates
only — counts, min/median/max ms, EWMA snapshots, verdict tallies, roster
health rolls. Each emitted `(name, value)` field must pass BOTH gates:

- a **positive type allowlist**: integers, booleans, ISO-8601 UTC timestamps,
  enum strings drawn only from frozen vocabularies (the effective §6.5
  registry, `CancelRound` reasons, `served_by` values, verdict enums), and
  hex digest prefixes of exactly 16 lowercase hex characters (peer ids and
  profile ids render truncated to 16 chars — correlation aids, not integrity
  anchors; integrity stays local and cryptographic); and
- the shipped **`Redactor`** (`crates/modelswarm-telemetry/src/redact.rs`):
  forbidden field names drop the value, `scrub_content` removes `sk-` keys,
  JWTs, Bearer headers, and hex runs ≥ 32, and values are capped at 256
  chars.

A field failing either gate is dropped and counted; a request with zero
surviving fields is not sent (gap `empty_after_redaction`). Free-form text is
never emitted: attacker-influenceable strings (error messages, filenames)
reduce to frozen-vocabulary codes before they can reach the digest. The
request body is bounded by `advisory.max_request_bytes`. The system prompt is
a fixed, repo-versioned template (no secrets, no user content) that itself
states the metadata-only contract; the redaction gate runs regardless of the
prompt — the structural gate, not prompt compliance, is the defense.

**3.3 Provider pinning.** Endpoint URL and model id are recorded in local
config at setup time from the provider console (`advisory.endpoint_url`,
`advisory.model` — owner-stated model string `DeepSeek-V4.1-Flash` recorded
at setup). Neither is hardcoded in source; provider model ids are not frozen
by this ADR and may be changed in config without an ADR. A config with either
missing is inert (gap `no_config`).

**3.4 Key handling.** The key is read from a local file whose path resolves
env-first (`MSP_ADVISORY_KEY_FILE`), then config (`advisory.key_file`), then
the platform default (`%USERPROFILE%\ModelSwarm-advisory-key.txt` on Windows,
`~/.modelswarm/advisory-key.txt` elsewhere): single line, trimmed,
owner-readable only. The key never enters the repository — the guards
pattern-scan (`sk-[0-9a-f]{32}` over tracked files) makes a tracked key a
build failure by definition — never enters telemetry or logs (the Redactor's
`sk-` scrub), and never appears in a request payload (the allowlist has no
key-bearing field type).

**3.5 The standing key-hygiene and rotation rule (normative).** The current
key was pasted in chat once during funding. A key that has transited a chat
channel is treated as exposed in a transcript: it may be used for wiring and
bring-up validation ONLY, and MUST be rotated — in the DeepSeek provider
console, entering the new value DIRECTLY into the local key file — before
the plane's first production digest cycle, and in any case before the next
tagged release carrying the wiring. Chat is never a key channel; the only
sanctioned path from console to config is direct local entry. The rotation
event is recorded in the operator log as a date, with no key material or
derivation.

**3.6 Disclosure rule (standing).** Any peer whose telemetry feeds the plane
receives an opt-in disclosure stating: what is shared (redacted metadata
only), with whom (the named provider, DeepSeek), and how to revoke (disable
the plane or exclude the peer's telemetry). In v1 this binds the operator's
own peers; it becomes externally load-bearing only if §3.1 is ever extended,
which requires a new ADR.

### 4. The advisory client: state machine, config, malformed handling

**Why no protocol binding set applies.** The advisory plane exchanges NO
protocol messages: it has no peer id, no session, no `seq`, no nonce, no
signature surface, and it is not a participant in msp-v1 §6 or the
msp-cooperative-v1 family. Its only seam is an HTTPS call to the provider.
This is deliberate and permanent for v0.x: any future revision that let
advisory output touch protocol state would create a state-changing influence
channel and would need the full binding treatment (protocol version, session,
seq, round, profile id, prefix hash, params hash, sender, nonce, deadline,
signature) — invariant (b) exists so that revision is never proposed.
Because there is no wire surface, there are no cross-implementation parity
concerns and no protocol golden vectors; the fixtures owed are the local test
vectors enumerated in §9.2.

**State machine (per operator installation):**

```text
DORMANT ──cycle timer fires AND enabled AND key present AND endpoint+model
   ▲      configured AND daily cap > 0 AND min_interval elapsed AND
   │      new telemetry events ≥ events_threshold──► COLLECT
   │                                                    │ aggregate since last cycle
   │                                                REDACT (allowlist + Redactor)
   │                                                    │ drops counted
   │         gap(empty_after_redaction) ◄── zero fields survived ──┤
   │                                                    │ ≤ max_request_bytes
   │                                                 CALL (HTTPS, request_timeout_ms)
   │         gap(timeout | http_error | quota | response_oversized)┤
   │                                                    ▼
   │                                                 PARSE (strict suggestion schema)
   │         gap(schema) ◄── body unparseable / unknown schema ────┤
   │                                                    ▼
   │                                                 RECORD
   │                                                    │ advisory.cycle.sent +
   │                                                    │ advisory.suggestion.recorded;
   │                                                    │ suggestions → shadow ledger +
   │                                                    │ operator display ONLY
   └──────────────── COOLDOWN (min_interval_ms) ◄─────────────────┘
```

Cycles are **timer-driven only** — no call site exists in any request,
serving, scheduling, negotiation, or gateway path (invariant A5). There is no
in-cycle retry; the next cycle is the retry.

**Config (local `config.json` alongside the existing per-install config,
`crates/modelswarm-node/src/lib.rs:146-186` layout; defaults frozen by this
ADR, tunable without an ADR):**

| Key | Default | Meaning |
|---|---|---|
| `advisory.enabled` | `false` | master switch; false or key-absent = inert |
| `advisory.endpoint_url` | unset | provider endpoint, recorded at setup |
| `advisory.model` | unset | provider model id, recorded at setup |
| `advisory.key_file` | platform default (§3.4) | key file path |
| `advisory.max_calls_per_day` | `24` | hard daily cap, UTC days |
| `advisory.min_interval_ms` | `900000` | minimum spacing between calls |
| `advisory.events_threshold` | `200` | new events required to trigger |
| `advisory.request_timeout_ms` | `30000` | per-call deadline |
| `advisory.max_request_bytes` | `16384` | request body cap |
| `advisory.max_response_bytes` | `65536` | response body cap |
| `advisory.max_completion_tokens` | `2048` | provider-side output bound |

**Suggestion schema (local vocabulary, frozen by this ADR; the
self-describing-JSONL precedent of `divergence.rs` — not a frozen protocol
schema):** `{"schema":"advisory-suggestion/v1","cycle_id":"<16 hex>",
"suggestions":[{"type":"<frozen vocabulary §2>","target":"<16-hex profile
prefix | \"fleet\">","hint":{…existing tunable/clamped parameters only…},
"confidence":"low|medium|high","rationale":"<display-only free text,
never parsed>"}]}`. `rationale` is rendered to the operator with a visible
"advisory — untrusted, metadata-derived" provenance label and is NEVER
interpreted as instructions — the prompt-injection posture for §9.1.

**Malformed/adversarial response handling (fail-closed, first-failure-wins):**

| Injected condition | Outcome |
|---|---|
| transport error / HTTP non-2xx | gap `http_error`, no retry this cycle |
| deadline exceeded | gap `timeout`; a late response is a no-op discard |
| response > `max_response_bytes` | gap `response_oversized` |
| invalid JSON / unknown `schema` | gap `schema`, whole body discarded |
| unknown suggestion `type` | that suggestion discarded and counted; siblings parse |
| numeric hint out of range | clamped to the existing config range (msp-v1 §6.4 clamp discipline); `clamped=true` recorded |
| hint names a frozen constant | recorded `requires_adr=true`; never applicable by config |
| rationale contains instruction-like text | display-only by construction; no parse path exists |
| duplicate `cycle_id` / response after cycle closed | no-op discard (gap already recorded) |

**Invariants (test obligations attach in §9.2):**
**(A1)** content-blind egress (§3.2, structural); **(A2)** no authority —
structurally, the suggestion type is not a dependency of any production
decision crate (dependency-graph test); **(A3)** full function without it —
disabled/absent/errored plane leaves every production decision byte-identical;
**(A4)** measured-before-trusted + disclosed (§5, §3.6); **(A5)** bounded
cadence and cost — caps/interval/threshold enforced pre-call, fail-closed to
DORMANT, timer-driven only; **(A6)** key hygiene (§3.4).

### 5. The measurement harness (shadow-layer extension)

- **Shadow rows.** For every realized decision point in a cycle's window
  where a `tuning_hint` or de-rank hint was live, the client emits a
  `sched.shadow.advisory` telemetry row: `trace`, `deterministic_pick`,
  `advisory_pick` (the counterfactual the pure planner would choose under
  the CLAMPED hint — computable locally, no network), `agree`, `hint_type`,
  `clamped`, `requires_adr`. The join extension follows the shipped
  LEFT-join-on-realized discipline of `shadow_join.rs` exactly: every
  realized row survives, decision-only rows are counted and visibly
  reported as dropped, correlation is by `trace`/`cycle_id`, and the
  honest sentinels (`none`/`unmeasured`/`unknown`) map to null.
- **Promotion criteria (normative).** `triage_summary`, `digest`, and
  `catalog_note` NEVER influence behavior — display-only forever.
  `tuning_hint` may influence production ONLY through an owner-edited config
  change (runtime-tunable parameters) or an ADR (frozen constants), and only
  after published measurement: at least **100 joined realized events where
  the advisory arm and the deterministic arm differ**, a **strict-majority
  win for the advisory arm on the pinned promise metric** (versus the
  fastest eligible single — never an average baseline, per the competitive
  revision plan), and a **non-regressing worst-cell median**; medians alone
  never suffice and the worst cells are reported as visibly as the wins. If
  the evidence does not meet the bar, the honest negative is published in
  the harness artifacts under the same discipline as ADR-032's 9.6 records.
- **No automatic promotion path may exist in code.** The advisory output's
  only consumers are the shadow ledger and the display renderer; promotion
  is a human edit with a `docs/verification/` entry recording the measured
  evidence. This is invariant A4's structural form.

### 6. Failure, degradation, and cost bounding

Timeout, quota, HTTP, schema, and redaction failures all degrade the same
way: the advice for that cycle is silently absent from the operator's view,
a `advisory.cycle.gap` telemetry event records the reason (frozen vocabulary:
`disabled | no_key | no_config | cap_reached | interval |
empty_after_redaction | timeout | http_error | quota | response_oversized |
schema | late`), and NOTHING else changes — invariant (c) in operation. Cost
is bounded by the §4 config caps (`max_calls_per_day` default 24, byte caps,
`max_completion_tokens` 2048); the caps are enforced locally before every
call and fail closed to DORMANT. The owner pays; the AGENTS.md stop-point on
paid cloud resources is satisfied by the §Decision-record authorization for
this provider and scope only — a second provider, a model change with a
different pricing class, or non-owner deployments re-open the owner gate.

### 7. Options considered

| Option | Content | Cost |
|---|---|---|
| **A — no brain (status quo)** | Pure local heuristics (all shipped); the operator reads raw telemetry and decides alone | Zero new surface; triage/curation toil stays manual; the funded key goes unused |
| **B — the advisory plane as specified (RECOMMENDED for v1)** | §1–§6: owner-deployment-only, content-blind, advisory-only, full-function-without-it, measured-before-trusted | One local client component + shadow extension + new review surface; honest-negative publication burden; standing key hygiene |
| **C — protocol-level elected brain** | A swarm-elected reasoning peer with protocol-visible authority (elected coordinator for triage/tuning) | REJECTED for v0.x: reintroduces a trusted party into a trustless design, needs new wire surface and key/custody roles, creates a corruption/election attack class, and contradicts invariant (b) by construction |

**Recommendation: B**, owner-deployment-only for v1, with the §9 reviews as
the landing gate. A remains the permanent fallback — and the proof that B is
safe is that A is always one config flip away.

### 8. Non-goals held

- **No inference participation**: the plane never joins a swarm, never
  serves, verifies, or drafts; it is not a peer and has no installation
  identity (AGENTS.md constraint 1 untouched — the plane cannot grant,
  dilute, or vouch for eligibility).
- **Content-blindness preserved end to end**: hub, logs (constraint 5), and
  now the advisory egress channel — all metadata-only, structurally
  redacted.
- **No speculative DHT, blockchain, payments, KV-cache transfer, or
  token-level distributed inference** (constraint 8) — the advisory plane is
  none of these and adds no economics surface.
- **No cooperative-protocol surface changes**: no `msp-cooperative-v1`
  family byte, no ADR-032 migration touched; the plane observes
  cooperative telemetry only through already-redacted events.
- **No frozen-surface edits**: `protocol/msp-v1.md`, `protocol/messages.proto`,
  `catalog/schema.json`, and `protocol/vectors/` stay byte-identical; the
  wire-compat CI job is unaffected.

### 9. Review and gate requirements (before any implementation lands)

**9.1 Security Engineer — the metadata-only redaction surface (enumerate
learn/inject).** LEAK surface: timing histograms (fleet activity inference),
roster counts (swarm size/composition), truncated digests/prefixes
(correlation only — no content recoverable), divergence verdicts + peer
prefixes (which peers misbehave), aggregate volumes (operator usage
patterns). Countermeasures to verify: aggregation-before-egress, 16-char
prefix truncation, frozen-vocabulary strings, byte caps, v1 owner-only
deployment (the operator is the data controller of their own fleet's
metadata), §3.6 disclosure. INJECT surface: (1) prompt injection via
metadata — hostile peers influence field values (counts, codes, timings)
that survive into digests and try to steer the model's suggestions;
mitigated by frozen suggestion schema + clamps + display-only rationale +
the untrusted-provenance label; (2) adversarial suggestions — out-of-range
values clamped, frozen-constant hints `requires_adr`-flagged, shadow-only
recording; (3) provider compromise or MITM — TLS to the config-pinned
endpoint, and the same clamps bound a hostile provider's suggestions;
(4) key exfiltration — never in repo (guards scan), telemetry (Redactor),
or payload (allowlist); (5) cost abuse — caps bound daily spend and no
call path exists without the key; (6) provider-side traffic analysis — the
provider sees the digest by design; that is the named, disclosed service.
The Security review must reproduce the injection sketches against the
allowlist gate and sign off the rotation-rule execution (§3.5).

**9.2 Test and Release Engineer — the shadow-promotion and degradation
gates.** (1) Redaction property suite: for every telemetry event type the
node emits, prompt-shaped payloads injected into every field never appear in
the serialized request; forbidden names drop; `sk-`/JWT/Bearer/hex-blob
values scrub. (2) State-machine property: every failure edge in §4 lands in
DORMANT with exactly one gap event and zero decision diff. (3) Cadence/cap
property: adversarial trigger floods never exceed `max_calls_per_day` or
`min_interval_ms`. (4) Suggestion-clamp property: every numeric hint is
recorded clamped into the existing range; `requires_adr` flags frozen
constants. (5) Shadow-join extension golden rows in the `shadow_join.rs`
test style (LEFT join, visible drops, sentinel mapping). (6) The
dependency-graph test for A2: no production decision crate depends on the
suggestion type; the advisory output's only consumers are the shadow ledger
and the display renderer. (7) The A3 proof: decision functions with the
plane enabled-but-erroring versus disabled produce identical outputs across
the property corpus. (8) The promotion-gate test: no code path applies a
`tuning_hint` — promotion is a human config edit with a
`docs/verification/` entry.

**9.3 Owner acceptance.** This ADR is Proposed; the owner accepts or rejects
option B. The funded-key authorization covers bring-up; the §3.5 rotation is
a precondition for the first production digest cycle.

**9.4 Implementation routing (ownership map).** Contract and this ADR:
Protocol Architect. Shadow join extension + promotion analysis
(`crates/modelswarm-bench`): Scheduler Scientist, with Protocol and T/R
review. Advisory client, config, daemon composition (v1 inside
`crates/modelswarm-node`/desktop; a dedicated crate requires Integrator
sign-off): Windows Product, with Protocol, Security, and T/R review. The
Redactor is reused read-only from `modelswarm-telemetry` (Integrator-owned;
no changes required). All hands run the standard `cargo fmt --check`,
`clippy -D warnings`, `test --workspace` gate before handoff.

## Consequences

+ The operator gets token-cheap triage, tuning proposals, curation
  summaries, and digests over metadata the deployment already holds, with
  the swarm's trustless and content-blind properties provably intact: no
  authority, no content, full function without it, measurement before
  trust, and disclosure.
+ The system's honesty discipline extends cleanly: every advisory claim is
  shadow-measured against the deterministic policy, negatives are published,
  and promotion is a human act with recorded evidence.
− One more local component to maintain and review (redaction gate, state
  machine, cadence caps, shadow extension) — forever, like every frozen
  surface.
− A standing operational dependency on an external paid provider for
  operator convenience — acceptable only because its absence is a no-op by
  construction; any drift toward hot-path or authority use is a defect
  against invariants (a)–(c) and must be treated as such.
− Key hygiene is now a permanent operator obligation (rotation rule §3.5;
  the guards scan is the backstop, not the defense).
