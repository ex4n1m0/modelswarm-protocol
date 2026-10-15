# ADR-033: The advisory plane — an owner-funded cloud reasoning endpoint as a non-authoritative management brain

Status: **Accepted (revision 2, 2026-10-12; originally accepted 2026-10-12
— owner decision, option B).** The owner accepted with the same message
that reported the first live two-peer swarm on v0.2.21. Both gate reviews
returned approve-with-required-changes and every required change is folded
into revision 2 (amendment note below): Security RC-1..RC-11
(`docs/reviews/review-adr-033-security-2026-10-12.md`, `86c6c77`) and
Test and Release TR-RC-1..TR-RC-10
(`docs/reviews/review-adr-033-testrelease-2026-10-12.md`, `8bb2e91`) —
the implementation gate is OPEN. The §3.5 key rotation, evidenced per
§9.2(9) under `docs/verification/`, remains a precondition for the first
production digest cycle and for any tagged release carrying advisory
wiring. Owner: Protocol Architect (contract); implementation routing per
§9.4.

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

**Amendment note (revision 2, 2026-10-12).** Folds both implementation-gate
reviews: `docs/reviews/review-adr-033-security-2026-10-12.md` (commit
`86c6c77`: 0 blockers, required changes RC-1..RC-11, advisories AD-1..AD-5)
and `docs/reviews/review-adr-033-testrelease-2026-10-12.md` (commit
`8bb2e91`: 0 blockers, required changes TR-RC-1..TR-RC-10, advisories
TR-AD-1..TR-AD-5). Both seats ruled approve-with-required-changes with this
amendment as the landing condition; with it recorded, both approvals stand.
Nothing here reopens the owner's option-B decision or any §1 invariant —
the four invariants are unchanged; this revision tightens mechanics only.
Changed from revision 1: §3.2 is rewritten around ONE value-typed content
gate with the `Redactor` as a secondary scrub net, pinned peer/profile
reference rendering (SHA-256 16-hex, RC-1), a frozen in-repo vocabulary
artifact `protocol/advisory-egress-v1.json` carrying the full
stringly-field reduction table (RC-2, RC-11), and typed in-process
accumulators as the only aggregation source (RC-4); §3.3 pins https-only,
no-redirect, hostname-displayed transport (RC-9); §4 gains the
post-serialization re-scan before CALL (RC-3), the pure injectable-core
precondition (TR-RC-4, TR-AD-2), the dedicated `advisory.json` config file
with corrected citations (TR-RC-8), the rationale rendering contract
(RC-5, AD-3), the frozen per-target clamp table with absent-target
`requires_adr` (RC-7), the `cycle_id` echo check and generator pin (RC-8),
the non-finite-hint classification (TR-AD-1), and re-pinned A2/A3
invariants (TR-RC-1, TR-RC-2); §5 is rewritten with the trace-keyed join
shape and (trace, hint) event cardinality (TR-RC-5), explicit
dual-execution outcome measurement (RC-6), the machine-readable promotion
verdict artifact with provenance, dispersion, worst-cell, and
fastest-single-baseline requirements plus CI replay (TR-RC-6), and
promotion-evidence integrity preconditions (TR-RC-10); §6 pins the `late`
gap firing condition and failed-call cap accounting (TR-RC-4); §9.2 is
rewritten as items (1)–(9) with the frozen fixture inventory and
completeness tie (TR-RC-3), the verbatim A2 mechanism (TR-RC-1), the A3
carve-out and shipped corpora (TR-RC-2), and the execution-venue matrix
whose standing statement is that the provider round-trip NEVER runs in CI
(TR-RC-7, TR-AD-4); §9.3 records the gate outcome; §9.4 records the added
routing obligations. Review interactions are reconciled as ONE mechanism
each, recorded here: **(R-1)** Security RC-2's vocabulary artifact and
TR-RC-3's fixture inventory are one two-sided mechanism — the contract
artifact `protocol/advisory-egress-v1.json` (Protocol Architect) is the
allowlist's vocabulary of record, the committed fixture inventory under
`tests/fixtures/advisory-egress/` (Test and Release) is its test mirror,
and the guards completeness tie diffs the source emit-site vocabulary
against BOTH — a new event name without both a manifest row and a
vocabulary row is a red build. **(R-2)** Security RC-7's clamp table,
TR-RC-6's promise-metric pin, and TR-RC-4's item-(4) sequencing are one
artifact — `protocol/advisory-clamp-table-v1.json`, rows
`target → [min, max] → source of truth → {promise_metric, direction}`,
pinned in §4; §9.2(4) is written against it and sequenced after it.
**(R-3)** Security RC-6 rules HOW advisory-arm outcomes are measured
(explicit dual execution, never attribution of realized deterministic-arm
outcomes) and TR-RC-6 rules WHAT the checker computes from; both land in
the single §5 rewrite. **(R-4)** Security AD-2's emitter escaping fix
(Integrator-owned `modelswarm-telemetry`) is NOT a precondition for
advisory egress — RC-4's typed accumulators remove JSONL from the digest
path entirely — but IS a precondition for any promotion decision, whose
evidence is built from telemetry files (TR-RC-10); both routings are in
§9.4. **(R-5)** Security RC-9's transport contract is CI-tested through
mock-transport doubles per push (TR-RC-7 venue matrix) and confirmed
against the live endpoint only in the owner-local venue. **(R-6)** the
ADR-internal `late`-vs-`timeout` contradiction TR-RC-4 found is resolved
in §6: `late` is a diagnostic-only gap fired at discard time, permitted
only after a `timeout` gap for the same cycle. Advisories folded into
normative text: AD-3 (display pairing) and the RC-8 generator ride §4;
TR-AD-1 rides §4's malformed table; TR-AD-3 rides §9.2(2); TR-AD-4 and
TR-AD-5 ride §9.2's standing statements and the release checklist.
Advisories routed, not folded: AD-1 (guards pattern breadth — Test and
Release widens the CI pattern or the owner verifies the exact key shape
at §3.5 rotation) and AD-4 (provider retention terms — the owner records
the check before the first production digest cycle); AD-2 routes per
(R-4). The frozen protocol surfaces (`protocol/msp-v1.md`,
`protocol/messages.proto`, `catalog/schema.json`, `protocol/vectors/`)
remain byte-identical; the two new `protocol/*.json` artifacts are
advisory-plane contract files, not wire surface, and the wire-compat CI
job is unaffected.

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
value-typed allowlist — the content gate — followed by the shipped
`Redactor` as a secondary scrub net (not itself a content gate: free text
under an allowed name passes it truncated — revision 2, RC-2's ruling); a
field failing the allowlist is dropped and counted, never sent.

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
3. **Catalog curation proposals** (`catalog_note` — revision 2, RC-11):
   `catalog_note` digests carry structured fields only — request counts,
   dedupe-cluster sizes keyed by `artifact_sha256` prefixes, quant enums,
   byte histograms — plus `display_name` values drawn ONLY from the SIGNED
   catalog and candidate rows already pinned in-repo (owner-controlled
   strings). The ADR-023 community `note` field, free-text `display_name`
   from the request queue, and URLs (`licenseEvidenceUrl` included) never
   enter any digest under any future revision without a new ADR. (If
   structured triage later needs to name a repo, a typed bounded identifier
   `hf_repo` — charset `[A-Za-z0-9._/-]{1,100}` — may be added to the §3.2
   vocabulary artifact by amendment; the 280-char `note` and URLs stay
   excluded either way.) Proposed for owner review. The plane never writes
   to the tracker, the catalog envelope, or any signed surface; the owner
   uses the existing §3.5 admin flow.
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

**3.2 Request shape (the redacted metadata digest) — revision 2.** A cycle
aggregates **typed in-process accumulators only** — counters, histograms,
and snapshot structs updated at emit time in the producing components; log
sinks (JSONL or otherwise) are NEVER parsed as advisory input (RC-4: the
emitter escapes only `\` and `"` today,
`crates/modelswarm-telemetry/src/lib.rs:203-205`, so a forged log line
could otherwise skew counts, timings, and verdict tallies). The
aggregation input set is itself an enumerated event-type allowlist that
EXCLUDES `advisory.*` rows — the plane's own telemetry never feeds the
next cycle's digest. Accumulated state reduces to aggregates only: counts,
integer min/median/max ms, EWMA snapshots rendered as integers (rounded
ms), verdict tallies, roster health rolls; the honest sentinels
`none`/`unmeasured`/`unknown` map to null on every timing field.

**The content gate is ONE gate: the positive value-typed allowlist.** The
shipped `Redactor` is a secondary scrub net, not a content gate — free
text under an allowed name passes it truncated (`redact.rs`'s own test
`prompt_smuggled_under_allowed_name_is_truncated_not_dropped`);
content-blindness therefore rests on the allowlist being value-typed and
exhaustive over what the node emits, and §9.2(1) enforces exactly that.
Each emitted `(name, value)` field must match one of:

- integers and booleans;
- ISO-8601 UTC timestamps;
- enum strings VALUE-matched against `protocol/advisory-egress-v1.json` —
  the frozen in-repo vocabulary artifact (revision 2; Protocol-owned;
  changed only by ADR-033 amendment). Its initial content is the committed
  vocabularies verbatim — the effective msp-v1 §6.5 registry as amended by
  ADR-028 §5, `CancelRound` reasons and `served_by`/verdict enums from the
  cooperative namespace, `finishReason` values, the §6 gap vocabulary, the
  §2 suggestion types and confidence levels, and the per-site reduction
  code sets below — COPIED, not referenced: drift in the source documents
  never changes the egress vocabulary, and vice versa;
- peer and profile references, rendered as exactly 16 lowercase hex
  characters: peer references are the first 16 hex characters of SHA-256
  over the full base58btc PeerId string; profile references are the 16 hex
  characters following the `msp1:` prefix; the raw base58 PeerId string is
  never emitted (RC-1 — hash-truncation keeps the emitted form uniform,
  charset-constrained, and uncraftable as another type; 16 hex is a
  correlation aid, not an integrity anchor — integrity stays local and
  cryptographic).

Every stringly field the node emits today has a pinned reduction rule
(RC-2; the artifact carries this table mechanically):

| Emitted field (site) | Shape today | Reduction rule for the digest |
|---|---|---|
| `error`, `detail` (gateway/engine/executor/family/chat.swarm/net.metrics sites) | `e.to_string()` — OS/HTTP text, can embed file paths | drop by default; a site's errors enter only via a per-site frozen code set mapped before aggregation |
| `code` on `serving.session_ended` (`crates/modelswarm-node/src/serving.rs:114`) | composite despite the code comment: `recv: {transport-error}` (`:314`), `invalid_lease: {why}` (`:366`; `why` can carry serde fragments of a crafted `capability_token`) | match the prefix before the first `:` against a frozen code set; no match ⇒ drop+count |
| `deadline_ms`/`max_tokens` on `serving.clamped` (`serving.rs:381-396`) | composite `"X->Y"` | decompose into `*_requested`/`*_clamped` integer pairs, or drop |
| float-formatted timings (`probe_p50_ms`, `ttft_ewma_ms`, `total_ewma_ms`, `pick_predicted_ms`) | `"{:.1}"/"{:.3}"` strings | accumulators emit integers (rounded ms); sentinels → null |
| `decision` on `sched.shadow.decision` | stringified JSON blob | decompose or drop |
| `session` (`verifydrafts.*`) | opaque requester-generated string (peer-chosen bytes) | drop (correlate by peer reference + integer round) |
| `addr` (`chat.swarm`, `listener.bound`) | peer-registered multiaddr | drop |
| local constants (`backend`, `base`, `file`, `port`, `gateway_addr`, `version`, `name`, `preset`) | local disclosure strings | drop, or a tiny pinned per-field vocabulary in the artifact |

After the allowlist, the shipped `Redactor`
(`crates/modelswarm-telemetry/src/redact.rs`) runs as the secondary net:
forbidden field names drop the value, `scrub_content` removes `sk-` runs,
JWTs, Bearer headers, and hex runs ≥ 32, and values are capped at 256
chars. A field failing the allowlist is dropped and counted; a request
with zero surviving fields is not sent (gap `empty_after_redaction`). The
request body is bounded by `advisory.max_request_bytes`. Free-form text
never reaches the digest: attacker-influenceable strings reduce to the
frozen codes above before aggregation. The system prompt remains a fixed,
repo-versioned template (no secrets, no user content) that itself states
the metadata-only contract; the allowlist runs regardless of the prompt —
the structural gate, not prompt compliance, is the defense.

**3.3 Provider pinning.** Endpoint URL and model id are recorded in local
config at setup time from the provider console (`advisory.endpoint_url`,
`advisory.model` — owner-stated model string `DeepSeek-V4.1-Flash` recorded
at setup). Neither is hardcoded in source; provider model ids are not frozen
by this ADR and may be changed in config without an ADR. A config with either
missing is inert (gap `no_config`).

**Transport pinning (revision 2, RC-9).** The client rejects any
`advisory.endpoint_url` that is not `https://` (gap `no_config`); the HTTP
client does not follow redirects — a 3xx response is `http_error`, and the
request (including the Authorization header) is never re-sent to any other
host; the configured hostname is shown in the operator UI at every cycle.
Mock-transport coverage of these behaviors is per-push CI (§9.2 venue
matrix); live-endpoint confirmation is owner-local only.

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

**One egress choke point (revision 2, RC-3).** Exactly one function
constructs the request body, and before CALL the FINAL serialized body is
re-verified: it is re-parsed and every value must re-match the §3.2
allowlist, and the body bytes pass `scrub_content`. Any failure ⇒ gap
`empty_after_redaction`; nothing is sent. §9.2(1) attacks this single seam.

**Pure injectable core (revision 2, TR-RC-4/TR-AD-2 — a design
precondition, not an implementation detail).** The state machine is
implemented as a pure `step(state, input) -> (state, effects)` over an
injected clock and an injected transport double; §9.2 items (2)/(3) and
the RC-9 mock tests are unrunnable without them.

**Cap accounting and window (revision 2, TR-RC-4).** Every CALL transition
consumes one unit of `max_calls_per_day` regardless of outcome — caps count
calls initiated, not successes, so a flapping endpoint cannot retry toward
the cap for free. The daily window is the UTC calendar day: a flood
straddling midnight UTC legally earns a fresh allowance — intended, and
pinned by test, not discovered by it.

**Config (revision 2, TR-RC-8: a dedicated local file
`<data_dir>/advisory.json` — NOT `config.json`, whose two-field desktop
writer (`crates/modelswarm-desktop/src/app.rs:222-227` parse,
`:617-625`/`:905-913` wholesale rewrite) silently deletes unknown keys on
every tracker/profile change; the cited `lib.rs:146-186` range is
`default_data_dir` plus the migration list — `crates/modelswarm-node/src/lib.rs:154-164`
— not a config parser. The client reads `advisory.json` and never writes
it; the JSON form is one flat object whose keys are the table's dotted
suffixes (`enabled`, `endpoint_url`, …); unknown keys are tolerated on
read. Defaults frozen by this ADR, tunable without an ADR):**

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
`cycle_id` is generated as the first 16 hex characters of a fresh
`identity::new_nonce()` (32-hex output,
`crates/modelswarm-identity/src/timestamp.rs:32-41` — RC-8's generator
pin), and a response whose `cycle_id` does not echo the in-flight cycle's
is discarded whole (malformed-handling table). `rationale` renders as
plain text only: no markdown or HTML interpretation, no clickable or
auto-linkable URLs, a per-suggestion display cap of 2,000 characters, and
the untrusted-provenance label rendered adjacent to the text, always
(RC-5). The display renderer pairs any peer-naming advisory claim with
the local tallies it claims to summarize, so provider fabrication is
visible at a glance (AD-3).

**Malformed/adversarial response handling (fail-closed, first-failure-wins):**

| Injected condition | Outcome |
|---|---|
| transport error / HTTP non-2xx | gap `http_error`, no retry this cycle |
| deadline exceeded | gap `timeout`; a late response is a no-op discard that additionally records the diagnostic gap `late` (§6 firing condition — TR-RC-4) |
| response > `max_response_bytes` | gap `response_oversized` |
| invalid JSON / unknown `schema` | gap `schema`, whole body discarded |
| unknown suggestion `type` | that suggestion discarded and counted; siblings parse |
| numeric hint out of range | clamped to the per-target range recorded in the frozen clamp table (`protocol/advisory-clamp-table-v1.json`, §4 — RC-7); a target absent from the table is treated as `requires_adr=true` — no improvised ranges at implementation time; `clamped=true` recorded |
| hint names a frozen constant | recorded `requires_adr=true`; never applicable by config |
| rationale contains instruction-like text | display-only by construction; no parse path exists |
| response `cycle_id` ≠ current cycle | gap `schema`, whole body discarded (RC-8) |
| non-finite hint value (NaN/Inf, however encoded) | that suggestion discarded and counted; siblings parse (TR-AD-1 — JSON cannot carry non-finites; the parser is strict) |
| duplicate `cycle_id` / response after cycle closed | no-op discard (gap already recorded) |

**The frozen clamp table (revision 2, RC-7 + TR-RC-6/R-2).**
`protocol/advisory-clamp-table-v1.json` ships with exactly these rows; any
other hint target is `requires_adr=true`:

| target | [min, max] | source of truth | promise_metric (direction) |
|---|---|---|---|
| `MSP_SHADOW_QUEUE_ADVERTISED_WEIGHT` | [0.0, 1.0] | `crates/modelswarm-scheduler/src/shadow.rs:77-82` (module contract: weights clamped to [0, 1]) | `ttft_p50_ms` (down) |
| `MSP_SHADOW_QUEUE_MEASURED_WEIGHT` | [0.0, 1.0] | `crates/modelswarm-scheduler/src/shadow.rs:80-85` | `ttft_p50_ms` (down) |
| `MSP_SHADOW_QUEUE_CAP_MS` | [1000, 120000] | default 120 000 mirrors the msp-v1 §6.4 `deadlineMs` ceiling; the floor mirrors its 1 000 ms minimum (`shadow.rs:83-85`) | `ttft_p50_ms` (down) |

**Invariants (test obligations attach in §9.2):**
**(A1)** content-blind egress (§3.2, structural — one value-typed
allowlist, exhaustively tied to the emit-site inventory); **(A2)** no
authority — structurally, the suggestion type is not a dependency of any
production decision crate, enforced by the three-layer mechanism of §9.2(6)
(cargo-metadata graph over the frozen nine-crate decision set, source-scope
grep allowlist, shared-schema exclusion via `types` in the set — TR-RC-1);
because §9.4 hosts the client in node/desktop, the node seam is covered by
the grep allowlist unless the client lands as a dedicated crate or a
`modelswarm-node::advisory` module, which layer 1 then guards directly;
**(A3)** full function without it — a disabled/absent/errored plane leaves
every production DECISION output byte-identical, and the telemetry stream
byte-identical after filtering out events whose name starts with
`advisory.` (the carve-out §6 itself requires — TR-RC-2); **(A4)**
measured-before-trusted + disclosed (§5, §3.6); **(A5)** bounded cadence
and cost — caps/interval/threshold enforced pre-call, fail-closed to
DORMANT, timer-driven only, UTC-day window, failed calls consume cap
budget; **(A6)** key hygiene (§3.4) plus the §3.3 transport pinning
(https-only, no redirects — RC-9).

### 5. The measurement harness (shadow-layer extension)

- **Shadow rows and the join shape (revision 2, TR-RC-5).** For every
  realized decision point in a cycle's window where a `tuning_hint` or
  de-rank hint was live, the client emits a `sched.shadow.advisory`
  telemetry row: `trace`, `deterministic_pick`, `advisory_pick` (the
  counterfactual the pure planner would choose under the CLAMPED hint —
  computable locally, no network), `agree`, `hint_type`, `clamped`,
  `requires_adr`, `cycle_id`. The join is BY `trace` ALONE, exactly as
  `shadow_join.rs` implements it (one decision per trace); `cycle_id` is
  grouping metadata that buckets joined rows into cycles for the promotion
  artifact — never a join key. Cardinality: advisory contexts attach as a
  Vec per trace (two hints may be live in one cycle; one hint spans many
  traces), and ONE EVENT = ONE (trace, hint) PAIR — the promotion
  denominator counts pairs, never traces, so hint multiplicity cannot
  inflate it. The LEFT-join-on-realized discipline extends verbatim:
  every realized row survives; decision-only rows are counted and visibly
  reported as dropped; ADVISORY-only rows (a hint was live, the trace
  never realized) are counted and visibly reported as dropped (new stat
  `advisory_only_dropped`); the honest sentinels map to null
  (`advisory_pick=none` → null, `agree=unknown` → null); and null-pick
  rows are EXCLUDED from the promotion denominator — "differ" requires
  both arms non-null.
- **Outcome measurement (revision 2, RC-6).** A non-applied hint has no
  realized outcomes. The advisory arm's outcomes are measured by EXPLICIT
  DUAL-EXECUTION under this harness — never by attributing realized
  deterministic-arm outcomes to differing advisory picks (an attribution a
  patient hostile peer can spoof by behaving-while-measured on the steered
  arm). The evidence records the measurement window with its adversarial
  caveat: a peer measured while behaving may not behave after promotion —
  the deterministic local policy (EWMAs, engage gate, divergence ledger)
  remains the runtime defense after any promotion.
- **Promotion criteria (normative).** `triage_summary`, `digest`, and
  `catalog_note` NEVER influence behavior — display-only forever.
  `tuning_hint` may influence production ONLY through an owner-edited
  recorded config change (runtime-tunable parameters, landing as file
  state — never unrecorded environment state, TR-RC-8) or an ADR (frozen
  constants), and only after published measurement: at least **100 joined
  differing events** ((trace, hint) pairs where both arms are non-null and
  the picks differ), a **strict-majority win for the advisory arm on the
  target's pinned promise metric** (the `promise_metric` + `direction`
  recorded per target in `protocol/advisory-clamp-table-v1.json`,
  TR-RC-6), measured against the **fastest eligible single host at that
  moment** — which host was fastest and its measured value are recorded
  per window, baseline-selection method named, never an average — and a
  **non-regressing worst-cell median**, where a CELL is (profile ×
  capacity class); medians alone never suffice, single-run windows never
  suffice, and the worst cells are reported as visibly as the wins. If the
  evidence does not meet the bar, the honest negative is published in the
  harness artifacts under the same discipline as ADR-032's 9.6 records.
- **The promotion verdict artifact (revision 2, TR-RC-6).** The promotion
  checker is a `modelswarm-bench` component (Scheduler, §9.4) consuming
  the joined shadow rows and the dual-execution outcome records, emitting
  a machine-readable verdict JSON — `{windows[], differing_events,
  per_metric{wins,losses}, majority, worst_cells[{cell, n, median_delta,
  dispersion}], provenance{…}}` — COMMITTED under `docs/verification/`
  with the promotion entry (metadata-only, already redacted). The
  provenance block records the full standing release set: hardware, model
  profile, runtime build, prompt corpus, network conditions,
  baseline-selection method, repetitions, variance (per-cell n, median,
  and a dispersion measure — MAD or p10/p90). A per-push CI test re-runs
  the checker over the committed artifact and asserts the recorded
  verdict.
- **Promotion-evidence integrity (revision 2, TR-RC-10).** The telemetry
  emitter's control-character escaping fix (Security AD-2, routed to the
  Integrator in §9.4) is a PRECONDITION for any promotion decision — the
  join parses telemetry JSONL, and forged lines inflate the advisory arm's
  win count. Every contributing window's join stats are disclosed in the
  verdict artifact; an `unparsable_lines > 0` window is visibly flagged
  and disqualifying unless justified in the entry; the promotion entry
  pins the SHA-256 of each source telemetry file it was computed from.
- **No automatic promotion path may exist in code.** The advisory output's
  only consumers are the shadow ledger and the display renderer; the
  suggestion payload's `hint` map is not publicly readable outside its
  sanctioned consumers — the advisory module exposes only (i) a
  Display-rendered summary type (operator UI, the RC-5 rendering contract)
  and (ii) a ledger-record type whose fields are the already-clamped,
  already-typed values (TR-RC-9's compiler layer: production code has no
  API through which to read a hint value to apply). Promotion is a human
  edit with a `docs/verification/` entry recording the measured evidence.
  This is invariant A4's structural form.

### 6. Failure, degradation, and cost bounding

Timeout, quota, HTTP, schema, and redaction failures all degrade the same
way: the advice for that cycle is silently absent from the operator's view,
a `advisory.cycle.gap` telemetry event records the reason (frozen vocabulary:
`disabled | no_key | no_config | cap_reached | interval |
empty_after_redaction | timeout | http_error | quota | response_oversized |
schema | late`), and NOTHING else changes — invariant (c) in operation.

Revision 2 (TR-RC-4) pins the firing condition of the twelfth value:
`late` fires exactly at the discard of a response arriving after its cycle
closed — a diagnostic-only event permitted only following a `timeout` gap
for the same cycle; every failure edge produces exactly one primary gap.
Failed calls consume daily-cap budget (§4). Cost
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

**Review outcome (2026-10-12, revision 2).** Completed as
`docs/reviews/review-adr-033-security-2026-10-12.md` (`86c6c77`): APPROVE
WITH REQUIRED CHANGES — 0 blockers, RC-1..RC-11, advisories AD-1..AD-5 —
all required changes folded by revision 2. The rotation-rule sign-off on
the RULE stands; the EXECUTION sign-off is deferred to the RC-10 evidence
artifact (§9.2(9)); until that entry exists, the first production digest
cycle must not run.

**9.2 Test and Release Engineer — the shadow-promotion and degradation
gates (revision 2; folds TR-RC-1..TR-RC-10, TR-AD-1..TR-AD-5, and the
Security review's derived test obligations).** The enumeration:

(1) **Redaction property suite** over a frozen, committed, versioned
    fixture inventory — `tests/fixtures/advisory-egress/` with one fixture
    row set per emitted event type (every field carrying an adversarial
    value) plus the manifest `advisory-egress-inventory.json` of
    `{event, field, type}` triples (TR-RC-3). The adversarial value set is
    enumerated and frozen: prompt-shaped text; control characters incl.
    newline/CR/tab; base58 PeerIds; serde parse-error fragments via a
    crafted `capability_token`; composite `"X->Y"` clamp strings;
    float-formatted timings; `sk-`/JWT/Bearer/hex blobs; overlong strings;
    unicode/RTL; NaN/Inf where numeric parsing occurs. The suite asserts:
    the serialized digest never contains the raw adversarial bytes; no
    emitted field contains a base58 PeerId (peer references are exactly
    `^[0-9a-f]{16}$` SHA-256 prefixes — RC-1); the serde-fragment vector
    never appears and the frozen-code reduction fires (RC-2); corrupting
    one field post-gate in a test double stops the request (RC-3); a
    forged extra JSONL line injected into the log sink leaves digest
    contents unchanged (RC-4); the rationale renderer renders markdown, a
    URL, and 10 000 chars as plain, unlinked, capped text (RC-5). The
    inventory walks the accumulator→digest seam per §3.2 (typed
    accumulators, never parsed JSONL as input).
(2) **State-machine property**: a golden edge→gap-value mapping plus a
    property sweep over the pure injected-core `step()` (§4) — every
    failure edge lands in DORMANT with exactly one PRIMARY gap event and
    zero decision diff; `late` appears only in the timeout-then-late
    sequence and is counted separately; a well-formed response with a
    foreign `cycle_id` is discarded whole (RC-8). A golden enum test pins
    the 12-value gap vocabulary — a new gap value without an amendment
    note is a red build (TR-AD-3).
(3) **Cadence/cap property** on the virtual clock: for any adversarial
    event-arrival schedule, calls ≤ `max_calls_per_day` per UTC calendar
    day (midnight rollover pinned as intended) and inter-call spacing ≥
    `min_interval_ms`; failed calls consume budget; newline-forged
    telemetry floods never move digest cadence past the caps.
(4) **Suggestion-clamp property** written against
    `protocol/advisory-clamp-table-v1.json` (sequenced after that artifact
    lands — TR-RC-4's dependency ruling): for every target t in the table,
    for every numeric v (negative, zero, huge, NaN, Inf): the recorded
    value lies in [min_t, max_t] with `clamped=true` when v is out of
    range; t absent from the table implies `requires_adr=true` and never
    an improvised bound; non-finite values are discarded per the §4
    malformed table (TR-AD-1).
(5) **Shadow-join golden rows** in the `shadow_join.rs` test style
    (deterministic in-crate string fixtures), extended with: two hints
    live on one trace; a hint live but not applicable
    (`advisory_pick=none` → null, excluded from the denominator); an
    advisory row whose trace never realized (counted and visibly dropped
    as `advisory_only_dropped`).
(6) **Dependency-graph test for A2** — the three-layer mechanism
    (TR-RC-1), verbatim. Layer 1 (graph; a new step in the `guards` job —
    `cargo metadata --no-deps` compiles nothing and takes seconds; jq is
    already a guards dependency): assert that no member of the frozen
    decision-crate set (`modelswarm-{scheduler, session, eligibility,
    gateway, runtime, transport, tracker-api, store, types}` — `types`
    included deliberately: the shared-schema crate is the sneaky path to
    universal visibility; `bench` exempt as the sanctioned shadow-ledger
    consumer; node/desktop host the client and are covered by layer 2)
    declares a normal dependency on an advisory crate, via
    `cargo metadata --format-version 1 --no-deps` piped through jq
    selecting packages by name test and checking `.dependencies[].name`.
    Dev-dependencies are tolerated for test-fixture reuse only and
    recommended against in scheduler/session. Layer 2 (source scope): the
    suggestion type name (`AdvisorySuggestion`) may appear only in the
    advisory module, `modelswarm-bench`, the desktop display renderer, and
    `*/tests/` — enforced by a grep over `crates/ apps/ --include='*.rs'`
    filtered against that allowlist, erroring on any hit outside it. Layer
    2 also catches a re-declared lookalike type inside a decision crate,
    which the dep graph cannot see. Layer 3 (schema) is covered by layer 1
    including `types`.
(7) **The A3 proof**: identical outputs defined as (i) decision outputs
    byte-identical AND (ii) the telemetry stream byte-identical after
    filtering out events whose name starts with `advisory.` (the carve-out
    §6 itself requires — TR-RC-2). The corpus is the SHIPPED one, not a
    new one: (a) the scheduler/shadow decision property inputs; (b) the
    seeded session admission + coordinator suites
    (`crates/modelswarm-session/tests/`, the
    `verifydrafts_admission_properties.rs` fixture discipline); (c)
    `apps/modelswarm-sim` mixed-swarm pinned-verdict scenarios (the
    ADR-032 §7 item-4b per-push precedent); (d) the bench corpora
    (`corpus.rs`, `engage_gate.rs`, `divergence.rs`); (e) the item-(1)
    fixture corpus as the accumulation layer's input distribution.
    Mechanics: run each corpus twice — plane absent vs plane enabled with
    an always-erroring transport double — and assert both (i) and (ii);
    seeds recorded per the ADR-032 fixture discipline.
(8) **The promotion-gate test** as a layered mechanism (TR-RC-9): the
    compiler layer (§5's opaque hint payload — Display summary +
    clamped/typed ledger-record only), the link layer (item 6), and the
    drift layer — a per-push golden test pins (i) the eleven §4 config
    defaults to the ADR-table values and (ii) the frozen clamp-table
    contents, so any value change without an ADR-033 amendment is a red
    build (the only green path to a changed tunable is the recorded
    amendment plus the promotion entry with its verdict artifact).
(9) **Rotation evidence** (RC-10): a `docs/verification/` entry records
    the §3.5 key rotation as executed (date, provider console action, new
    key entered directly into the local key file; no key material or
    derivation), cross-referenced from the ADR-033 status line; the first
    production digest cycle and any tagged release carrying advisory
    wiring require this entry (TR-AD-5 release checklist).

**Execution-venue matrix (revision 2, TR-RC-7 — the ADR-032 §7 B4 style):**

| §9.2 item | Venue | Mechanism | Gated on |
|---|---|---|---|
| (1) redaction property suite | per-push, `rust.yml` check job | frozen fixture inventory + inventory-walking unit tests; no network | this revision |
| (2) state-machine property | per-push | pure `step()` core, injected failure doubles, virtual clock | this revision |
| (3) cadence/cap property | per-push | virtual clock, adversarial arrival schedules | this revision |
| (4) clamp property | per-push | property sweep over the frozen clamp table | `protocol/advisory-clamp-table-v1.json` |
| (5) join golden rows | per-push, `modelswarm-bench` | in-crate string fixtures, `shadow_join.rs` test style | this revision |
| (6) dependency-graph (A2) | per-push, `guards` job (new steps) | `cargo metadata` + jq + grep allowlist (item 6) | this revision |
| (7) A3 proof | per-push | double-run of shipped corpora + `advisory.*`-filtered telemetry diff | this revision |
| (8) promotion-gate | per-push | golden-pinned defaults/clamp table + opaque-payload compile discipline + (6) | this revision |
| promotion verdict replay | per-push, `modelswarm-bench` (once any verdict artifact exists) | checker re-derives the committed verdict | first promotion decision |
| provider round-trip (real DeepSeek HTTPS call: build → send → parse → record, incl. the §3.3 https-only/no-redirect behavior against the live endpoint) | **OWNER-LOCAL ONLY — never CI, never manual-dispatch-with-key** | `#[ignore]`d tests gated on `MSP_ADVISORY_E2E=1` AND the key file present (`MSP_ADVISORY_KEY_FILE` / platform default); evidence = `docs/verification/` entry | §3.5 rotation evidence (item 9) |

Standing statements: **the provider round-trip never runs in CI** — (a)
the key's custody scope is local client config only (§3.4); a CI secret is
a second custody location the owner authorization does not cover, and the
provider key is NEVER added as a CI secret, even "for one smoke test"
(TR-AD-4); (b) a per-push external-SaaS call puts a third party's latency
and uptime inside the release gate; (c) it spends owner money on every
push. A green CI badge therefore implies nothing about live provider
interop; that evidence lives exclusively in owner-local
`docs/verification/` records (the ADR-032 §7 item-2 and
`MSP_LIVE=1`/`MSP_REAL_GGUF` ignored-test precedents). Mock-transport
coverage of §3.3 (https-only, no redirect, no Authorization on redirect)
IS per-push CI and MUST be. An unauthenticated manual-dispatch
reachability probe is permissible CI-side without the key but is optional,
not a gate.

**9.3 Owner acceptance (recorded).** The owner accepted option B on
2026-10-12 (decision record above) — the stale "is Proposed" wording is
corrected by this revision. Both §9.1/§9.2 gate reviews returned
approve-with-required-changes on 2026-10-12 and every required change is
folded by revision 2 — the implementation gate is OPEN. The funded-key
authorization covers bring-up; the §3.5 rotation, with the §9.2(9)
evidence entry, remains a precondition for the first production digest
cycle and for any tagged release carrying advisory wiring.

**9.4 Implementation routing (ownership map).** Contract and this ADR:
Protocol Architect. Shadow join extension + promotion analysis
(`crates/modelswarm-bench`): Scheduler Scientist, with Protocol and T/R
review. Advisory client, config, daemon composition (v1 inside
`crates/modelswarm-node`/desktop; a dedicated crate requires Integrator
sign-off): Windows Product, with Protocol, Security, and T/R review. The
Redactor is reused read-only from `modelswarm-telemetry` (Integrator-owned;
no changes required). All hands run the standard `cargo fmt --check`,
`clippy -D warnings`, `test --workspace` gate before handoff.


Revision-2 routing additions: the frozen contract artifacts
`protocol/advisory-egress-v1.json` and
`protocol/advisory-clamp-table-v1.json` are Protocol-Architect-owned,
created by the first advisory implementation session under this revision's
authority, then frozen (changes only by ADR-033 amendment). The
`tests/fixtures/advisory-egress/` inventory, the guards completeness-tie
step, the item-(6) A2 steps, and the optional frozen-surface hash-pinning
in guards (TR-AD-5) are Test and Release. The advisory client lands as a
pure injectable core (clock + transport doubles — TR-AD-2) in
`crates/modelswarm-node::advisory` or a dedicated crate (Integrator
sign-off for the latter), reading `<data_dir>/advisory.json` only
(TR-RC-8); the desktop `config.json` writer is untouched. The telemetry
emitter control-character escaping fix (Security AD-2) is routed to the
Integrator (`modelswarm-telemetry`) — a promotion-decision precondition
(§5), not an advisory-egress precondition (§3.2 typed accumulators). The
promotion checker and verdict artifact are Scheduler
(`crates/modelswarm-bench`), with Protocol and T/R review.

## Revision note (2026-10-15, owner directive): the provider-key era ends

The owner ruled, verbatim: **"how about we remove this deepseek key
completely, and make the helper use the hosted model when hosted."**
Amended, exactly and only as follows.

**The provider-key era ends.** The funded-key decision record above is
history: no provider key exists anywhere in the product, and the
content-egress-to-provider surface is DELETED with it, not defaulted
off. Concretely, in the implementation of record
(`crates/modelswarm-node/src/advisory/`):

- The timer-driven digest cycle — the whole §4 state machine, its typed
  accumulators (RC-4's `accum` module), its runner, and the §5
  shadow-promotion consumer path — is deleted. Its only reason to exist
  was the metadata-egress-to-provider brain; with no provider there is
  no digest. Invariant (c) — full function without the plane — becomes
  the ONLY mode: the deterministic local policy set (frozen cost model,
  EWMAs, engage gate, divergence ledger, rule-6 guard) is all there is,
  and A3's filtered-stream discipline now pins the assistant plane's
  rows alone.
- The transport's provider legs are deleted: `ReqwestTransport` (the
  provider HTTP client), the §3.3 https-endpoint arming
  (`is_https_endpoint`, `endpoint_url`), and the entire §3.4 key
  surface — `MSP_ADVISORY_KEY_FILE`, the config `key_file` row, and the
  platform-default key path. No key resolution code exists; the
  guards `sk-<32 hex>` tracked-file scan remains as the standing
  backstop it always was. The RC-9 transport pinning survives in its
  loopback form (below).
- `advisory.json` collapses to what remains real: the ADR-037 §3.4
  `assistant.*` rows plus the enable posture. Old keys (the digest
  caps, the provider rows) are ignored and preserved on disk, never
  fatal, never rewritten. The §4 config table's digest-plane rows are
  RETIRED; the ADR-037 §3.4 assistant table is the whole table.
- The §6 gap vocabulary is the 12-value post-collapse set:
  `no_key`, `interval`, and `quota` are REMOVED as unemittable (their
  emitting surfaces are gone), and `not_hosting` is ADDED as the honest
  static-floor reason (the assistant's model backing is the machine's
  own hosted model; without hosting there is no backing). A further
  value still needs an ADR-037 amendment.

**The egress contract artifact** (`protocol/advisory-egress-v1.json`)
is synced mechanically in the same change, per the R-1 two-sided tie
(the count legitimately changed): the `advisory.cycle.*` and
`advisory.suggestion.recorded` emit-site rows are removed (their emit
sites are deleted), the five former `feeds_digest` classifications
become `excluded`, `aggregation_event_allowlist` is retired-empty,
`plane_telemetry_events` carries the two `assistant.turn.*` rows,
`plane_enum` is `assistant`, the `reduction_rules` section is removed
(its consumer was the deleted accumulator gate), and `advisory_gap` is
the 12-value set above. The fixture inventory under
`tests/fixtures/advisory-egress/` regenerated via the committed
scanner; the tie verifies 87/87/87 (was 90/90/90).

**What is NOT amended:** the §1 invariants stand retargeted to the
local seam — the plane's only egress is the assistant request body to
the machine's OWN loopback gateway (the one value-typed context gate
still defends it, W3 unchanged); advisory-only/no-authority is
unchanged (nothing applies anything, and the hint payload stays
opaque outside its sanctioned consumers); measured-before-trusted is
moot-but-standing for any future surface. The clamp table
(`protocol/advisory-clamp-table-v1.json`) is unchanged. The frozen
protocol surfaces remain byte-identical. The ADR-037 chat wall is
unchanged and now also structurally stronger: the node daemon
composition wires NO advisory machinery at all — the assistant plane
is constructed by the desktop against the running node's own gateway,
and the layer-2 pin asserts the transport executes from exactly one
site (the assistant turn runner).

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

## Revision note (2026-10-13, ADR-037): the assistant surface and the expanded advisory role

The owner directive of 2026-10-13 ("…as long as the main rule of not
responding chat questions is solid… make this api key talk to the user
about the recommended model for his machine… take a more active role in
the management… i will support these costs") extends the plane via
ADR-037. Amended, exactly and only as follows. §2's non-goal "No
user-facing chat" is superseded SOLELY by the creation of ADR-037's
separate assistant channel under its chat wall (ADR-037 §3.1, W1–W6);
the other half of that non-goal — the swarm chat box and every
completion path never reaching this plane — is PERMANENT and now
structurally pinned (ADR-037 §6). Invariant (a) gains one named
exception: the user's own assistant-channel text, under ADR-037 §3.5's
disclosure, sourcing (W3), and scrub rules; it is unchanged for every
swarm/serving surface. The §2 suggestion vocabulary extends with
`peer_pattern_note | catalog_presentation_hint` (display-only). The §4
config table gains the nine `assistant.*` rows (ADR-037 §3.4);
`max_calls_per_day` 24 remains the DIGEST-plane cap. The §6 gap
vocabulary gains `rate_limited | request_oversized` (14 values). §9.2
gains items (10)–(14) (ADR-037 §4.4). The four invariants (a)–(d), the
§5 promotion bar, and the §3.2–§3.4 mechanics are otherwise unchanged;
the §3.5 rotation is deferred by standing owner directive until the
owner calls for it (ADR-037 §3.6 records the accepted risk).
