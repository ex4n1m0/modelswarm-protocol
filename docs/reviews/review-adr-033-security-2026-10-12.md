# Review: ADR-033 advisory plane — Security Engineer §9.1 (implementation gate)

Reviewer: Security Engineer (independent blocking seat). Routed per ADR-033 §9.1.
Date: 2026-10-12 · Verdict: **APPROVE WITH REQUIRED CHANGES**
Reviewed object: `docs/adr/ADR-033-advisory-plane.md` (status **Accepted —
option B**, commit `be4199a`, main @ `be4199a`).

Scope note: DESIGN review of the advisory plane's redaction and injection
surface before any code exists (the ADR's own framing: §9.1/§9.2 "remain the
implementation gate"). Nothing here judges implementation — there is none —
and nothing reopens the owner's option-B decision. The verdict below gates
the START of implementation: the required changes are contract-text deltas
the Protocol Architect must record as an ADR-033 amendment (its own
amendment-ready style) before the first advisory-client commit lands.

Grounding read: `docs/threat-model.md` (§4.3 local node, §5 non-guarantees);
ADR-033 in full; the shipped Redactor (`crates/modelswarm-telemetry/src/redact.rs`);
the telemetry emitter (`crates/modelswarm-telemetry/src/lib.rs`); every
telemetry call site the node/desktop emit today (`serving.rs`, `lib.rs`,
`engine.rs`, `executor.rs`, `measure.rs`, `artifact.rs`, `catalog.rs`,
`verifydrafts/{composed,serve,client}.rs`, `desktop/app.rs` family + shadow
+ chat.swarm events); `modelswarm-eligibility/src/lease.rs` (`from_wire` error
text); `modelswarm-session/src/{verifydrafts,negotiation}.rs` (`session_id`
provenance); `modelswarm-scheduler/src/shadow.rs` (env-tunable targets);
`modelswarm-bench/src/{divergence,shadow_join}.rs`; `protocol/msp-v1.md`
§6.4/§6.5; ADR-023 + `catalog/candidate-profiles/` row shapes; the guards
job in `.github/workflows/rust.yml`.

Verdict counts: **0 blockers · 11 required changes · 5 advisories.**

No blocker: the four invariants (§1) are the right invariants, the option-C
rejection is correct, zero protocol wire bytes change, and every finding
below is fixable by recorded contract text without architecture change. But
implementation MUST NOT start before RC-1…RC-11 are recorded: three of them
(RC-2, RC-10, RC-9) are exactly the class of ambiguity that becomes a leak
or an exfil path at implementation time when an implementer "makes it work".

---

## 1. The structural redaction gate (§3.2) — is the composition airtight?

**Ruling: the composition is sound and fail-closed BY DESIGN, but the ADR
under-specifies three things that decide whether the shipped system matches
the design: the peer-reference form (RC-1), the typed-aggregation/vocabulary
discipline (RC-2), and a single testable choke point (RC-3). The Redactor,
alone, is NOT a content gate — the ADR must stop letting §3.2 read as if it
were one half of a two-factor content defense.**

What I verified:

- **Order and failure mode are correct.** A positive type allowlist followed
  by the shipped `Redactor`, per-field drop-and-count, request suppressed
  when zero fields survive (`empty_after_redaction`), byte cap on the body:
  this is fail-closed in the right direction. No field type in the allowlist
  (integer, bool, ISO-8601 UTC timestamp, frozen-vocab enum string,
  16-lowercase-hex prefix) can carry prompt or completion content: 16 hex
  chars cap any string at 16 chars of constrained charset, and enums are
  value-matched against frozen vocabularies.
- **The Redactor's actual semantics** (`redact.rs`): forbidden NAMES drop
  (`FORBIDDEN_FIELDS`, redact.rs:12–36); `scrub_content` removes `hf_`/`sk-`
  runs ≥ 8 token-ish chars, `Bearer ` runs, `eyJ` JWT heads, and hex runs
  ≥ 32 (redact.rs:76–132); values over 256 chars truncate-with-marker
  (redact.rs:66–69). Critically, the crate's own test
  `prompt_smuggled_under_allowed_name_is_truncated_not_dropped`
  (redact.rs:189–196) documents that **free text under an allowed name
  passes, truncated to 256 chars**. So invariant (a)'s "structurally, not by
  policy" claim rests entirely on the allowlist being value-typed and
  exhaustive over what the node emits. That is achievable — but it is ONE
  gate, not two, and the ADR should say so (RC-2).

**RC-1 (required change) — the 16-hex peer-reference claim does not match
the PeerId encoding.** §3.2: "hex digest prefixes of exactly 16 lowercase
hex characters (peer ids and profile ids render truncated to 16 chars …)".
PeerIds are base58btc (`12D3Koo…`, every `peer`/`pick`/`production_peer`/
`probe_peer` field emitted today), never hex; profile ids are
`msp1:<64 hex>`. As written, every peer-reference field fails the hex gate
and is dropped: the digest silently loses its core correlation value, and
the predictable failure mode is an implementer loosening the gate to "make
peer correlation work". Contract delta for §3.2, replace the parenthetical
with: "(peer references render as the first 16 lowercase hex characters of
SHA-256 of the full PeerId string; profile references render as the 16 hex
characters following the `msp1:` prefix; the raw base58 PeerId string is
never emitted)". Hash-truncation (rather than base58 truncation) is
recommended so the emitted form is uniform, charset-constrained, and cannot
itself be crafted to look like another type.

**RC-2 (required change) — pin the typed-aggregation discipline and one
frozen vocabulary artifact.** §3.2's phrase "enum strings drawn only from
frozen vocabularies (the effective §6.5 registry, `CancelRound` reasons,
`served_by` values, verdict enums)" under-specifies the hard part: the
allowlist must match field VALUES against a vocabulary, and that vocabulary
must live in ONE versioned in-repo artifact frozen by ADR-033 amendment —
not "whatever §6.5 currently says", because §6.5 drifts with every protocol
amendment and drift silently changes the egress vocabulary. Inventory of
what the node emits TODAY that fails every allowlisted type, with the
reduction rule each needs (this table is the contract delta for §3.2's
"attacker-influenceable strings … reduce to frozen-vocabulary codes"
sentence — the ADR states the principle but never the table):

| Emitted field (site) | Today's shape | Reduction rule for the digest |
|---|---|---|
| `error`, `detail` (`gateway.failed`, `net.metrics.persist_failed`, `engine.spawn_failed`, `engine.job_assign_failed`, `family.fallback`, `chat.swarm.error`, `net.metrics.store_unavailable`) | `e.to_string()` — OS/HTTP error text; can embed file paths | drop from the digest by default; if a site's errors are wanted, per-site frozen code sets mapped before aggregation |
| `code` on `serving.session_ended` (serving.rs:114) | NOT frozen despite the code comment claiming it: values include `recv: {transport-error}` (serving.rs:314), `invalid_lease: {why}` (serving.rs:366) where `why` can carry `BadWireLease(format!("json: {e}"))` — serde_json error fragments EMBED attacker-chosen bytes from a crafted `capability_token` (lease.rs:240–258) | match the prefix before the first `:` against a frozen code set; no match ⇒ drop+count. Also see AD-2 (local-log injection) |
| `deadline_ms`/`max_tokens` on `serving.clamped` (serving.rs:381–396) | composite strings `"4294967295->120000"` | decompose into two integer fields (`*_requested`, `*_clamped`) or drop |
| `probe_p50_ms`, `ttft_ewma_ms`, `total_ewma_ms`, `pick_predicted_ms` (`net.rtt`, `sched.shadow.*`) | float-formatted strings `"{:.1}"`/`"{:.3}"` | aggregation emits integers (rounded ms); the honest sentinels `none`/`unmeasured`/`unknown` map to null (§5 already says so — extend to all timing fields) |
| `decision` on `sched.shadow.decision` (app.rs:2128–2131) | stringified JSON blob | decompose or drop |
| `session` (`verifydrafts.*`) | opaque REQUESTER-generated string (negotiation.rs:190–192) — peer-chosen bytes | drop from the digest (correlate by peer-reference + integer round) or hash like RC-1 |
| `addr` (`chat.swarm`, `listener.bound`) | multiaddr from tracker lookup (peer-registered data) | drop |
| `backend` (`family.engine_ready`), `base` (tracker URL), `file`, `port`, `gateway_addr`, `version`, `name`, `preset` | local constants/disclosure strings | drop, or pin tiny per-field vocabs in the RC-2 artifact |
| `rationale`-like free text | n/a today (response-side) | never enters the digest — response side, see RC-5 |

Additionally: the aggregation INPUT set must itself be an enumerated
allowlist of event types that excludes `advisory.*` rows (the plane's own
telemetry must not feed the next cycle's digest — a self-referential loop
adds nothing and complicates the A1/A3 proofs).

**RC-3 (required change) — one egress choke point, re-scanned after
serialization.** §3.2/§4 should pin: (a) exactly one function constructs the
request body, and (b) the FINAL serialized body is re-verified before send —
re-parse it and assert every value matches an allowlisted type, and run
`scrub_content` over the wire bytes. This costs nothing, makes invariant A1
testable end-to-end even if a future field sneaks past the per-field gate,
and gives §9.2's redaction property suite a single seam to attack. Contract
delta for §4 (RECORD/CALL transition): "before CALL, the serialized request
body is re-parsed and every value must re-match the §3.2 allowlist, and the
body bytes pass `scrub_content`; any failure ⇒ gap `empty_after_redaction`,
nothing is sent."

**RC-4 (required change) — the aggregation source is typed in-process
accumulators, not parsed log files.** §3.2 says "a cycle aggregates local
telemetry accumulated since the last successful cycle" without saying from
where. The JSONL log stream is NOT a trustworthy aggregation source: the
telemetry line emitter escapes only `\` and `"` (`lib.rs:203–205`), so a
value containing a newline forges subsequent log lines, and attacker-chosen
bytes demonstrably reach log values today (RC-2's serde-fragment path; the
opaque `session_id`). Egress stays fail-closed either way, but forged lines
skew counts, timings, and verdict tallies — i.e., they inject into the
advice the operator reads (the §9.1 INJECT surface), and they pollute the
A3 "decision functions byte-identical" evidence. Contract delta for §3.2:
"aggregation reads typed in-process accumulators only; log sinks are never
parsed as advisory input." (The escaping defect itself is AD-2 — telemetry
is Integrator-owned; I do not edit it here.)

## 2. The prompt-injection surface — can a crafted metadata pattern steer a suggestion past display/shadow-record?

**Ruling: under the frozen schema as written, no. Every path from a
suggestion to effect terminates in the shadow ledger or the display
renderer; there is no authority consumer, and A2/A5 make that checkable.
The ADR's mitigation claims reproduce against the schema — with four
caveats that must be closed before implementation, because the residual
channels are exactly the ones the ADR leaves implicit: the human display
channel, the promotion-measurement definition, the clamp table, and
response binding.**

Verified against §4: suggestion `type` vocabulary is frozen to four values;
unknown types drop (siblings parse); numeric hints clamp with
`clamped=true`; frozen-constant hints record `requires_adr=true` and "can
never be applied by config alone"; `rationale` has "no parse path"; the
client is timer-driven only with no call site in any request/serving/
scheduling/negotiation/gateway path (A5); promotion is a human config edit
with a `docs/verification/` entry and "no automatic promotion path may exist
in code" (§5). Hostile peers control metadata values (counts, timings,
verdict tallies, peer references, CancelRound reasons, session ids) that
survive into digests — the model can be steered to emit a `tuning_hint`
that names a target and a value — but the value is clamped, the target must
be runtime-tunable or config-recorded, and the effect is one shadow row.
That is bounded. The holes:

**RC-5 (required change) — the `rationale` rendering contract is missing.**
`rationale` is the one field that crosses from the provider to a human, and
§4 pins only a provenance label. The human is the sole authority channel
(advice becomes action only through operator edits), so the human channel
needs the same structural discipline as the machine channel. A provider
(or a metadata pattern the model echoes) can author a phishing line —
"rotate the advisory key now via https://deepseek-rotator.example/…" — and
if the renderer ever interprets markdown/HTML or makes URLs clickable, the
display-only claim is hollow. Contract delta for §4's suggestion-schema
paragraph, append: "`rationale` renders as plain text only: no markdown or
HTML interpretation, no clickable or auto-linkable URLs, a per-suggestion
display cap of 2,000 characters, and the untrusted-provenance label
rendered adjacent to the text, always."

**RC-6 (required change) — the promotion bar measures an outcome the design
never measures.** §5's shadow rows define `advisory_pick` as "the
counterfactual the pure planner would choose under the CLAMPED hint —
computable locally" — a PICK, not an outcome — yet the promotion criteria
demand "a strict-majority win for the advisory arm on the pinned promise
metric". A non-applied hint has no realized outcomes, so as written an
implementer must either (a) attribute realized deterministic-arm outcomes
to the advisory arm when the picks differ — which is spoofable: a patient
hostile peer performs well-while-measured on the arm the hint steers toward
(bait-and-switch; the same measurement-window weakness the earn-chain
reputation work tracks), then degrades after the human promotes the hint —
or (b) quietly apply hints to measure them, violating "no automatic
promotion path may exist in code". The backstop is sufficient ONLY if the
measurement is defined and honest. Contract delta for §5, replace the
promotion-criteria sentence's win clause with: "a strict-majority win for
the advisory arm on the pinned promise metric, where the advisory arm's
outcomes are measured by explicit dual-execution under the §5 harness
(never by attributing realized deterministic-arm outcomes to differing
advisory picks), and the evidence records the measurement window with its
adversarial caveat: a peer measured while behaving may not behave after
promotion — the deterministic local policy (EWMAs, engage gate, divergence
ledger) remains the runtime defense after any promotion."

**RC-7 (required change) — "clamped to the existing config range" is
undefined for several legal hint targets.** §6.4 gives ranges for
requester-facing parameters (`deadlineMs` [1 000, 120 000], `maxTokens`
[1, 2048], `temperature` [0, 2], `top_p` (0, 1], `top_k` [1, 200]) — but
the runtime-tunable scheduler knobs a `tuning_hint` naturally targets are
env-var weights today (`MSP_SHADOW_QUEUE_ADVERTISED_WEIGHT`,
`MSP_SHADOW_QUEUE_MEASURED_WEIGHT`, `MSP_SHADOW_QUEUE_CAP_MS`,
shadow.rs:79–85) with NO declared min/max. Clamping bounds magnitude, not
quality — a within-range-but-worse value is legal (the task's "clamps to a
WORSE value" case) and is acceptable only because the shadow/promotion
backstop exists — so the backstop's inputs must be well-formed. Contract
delta for §4's malformed-handling table, replace the "numeric hint out of
range" row with: "numeric hint out of range ⇒ clamped to the per-target
range recorded in the frozen clamp table (target → [min, max] → source of
truth, frozen by ADR-033 amendment); a target absent from the table is
treated as `requires_adr=true` — no improvised ranges at implementation
time; `clamped=true` recorded."

**RC-8 (required change) — bind the response to the cycle.** The suggestion
schema carries `cycle_id`, and the malformed table handles "duplicate
`cycle_id` / response after cycle closed", but nothing requires the client
to CHECK that a response's `cycle_id` equals the in-flight cycle's. Without
the echo check, a stale or replayed provider response attaches suggestions
to the wrong cycle's ledger and shadow rows. Contract delta for §4's
malformed-handling table, add row: "response `cycle_id` ≠ current cycle ⇒
gap `schema`, whole body discarded". Also pin the generator (the
`identity::new_nonce()` primitive returns 32 hex; the schema says 16 hex —
define `cycle_id` as the first 16 hex of a fresh nonce, or a dedicated
16-hex generator; either way, frozen by the same amendment).

Advisory on the same surface: a hostile provider can slander an honest peer
in a `triage_summary` ("peer ab12… diverges constantly") — bounded by
display-only, but the operator should not have to trust the provider's
arithmetic. See AD-3.

## 3. Key handling (§3.4/§3.5)

Verified the ADR's claims:

- **The guards scan exists and matches.** `.github/workflows/rust.yml`, job
  `guards`, step "No provider API keys are committed" (rust.yml:141–147):
  `git ls-files -z | xargs -0 grep -rlE 'sk-[0-9a-f]{32}'` with the
  workflow file itself excluded (it contains the pattern). This matches the
  DeepSeek key shape the ADR states (sk- + 32 hex) — see AD-1 for the
  lowercase-only caveat.
- **No telemetry path.** `sk-` runs ≥ 8 chars are scrubbed by
  `scrub_content` under ANY field name (redact.rs:80–93) — broader than the
  guards pattern.
- **No payload path.** No allowlisted type can carry a key: `sk-…` matches
  no enum, no ISO timestamp, no integer/bool, and fails the 16-lowercase-
  hex form; RC-3's post-serialization re-scan adds a second net.

**RC-9 (required change) — the HTTP transport seam is the exfil path the
ADR missed.** §3.3 pins the endpoint URL in local config and §9.1(3) claims
"TLS to the config-pinned endpoint" — but nothing requires the CLIENT to
enforce that, and two standard HTTP behaviors defeat the pin: (a) a
non-`https://` `advisory.endpoint_url` (fat-fingered or after a config
compromise) sends the Authorization header — the KEY — plus the digest in
cleartext or to an arbitrary scheme handler; (b) HTTP redirect following
(the default in most clients): a compromised or misconfigured provider
answers 307 and the client re-sends the request INCLUDING the
Authorization header to an arbitrary host — key + fleet metadata
exfiltration with one response. Config-write on the owner box is near
game-over already (threat model §5.5), but this turns a one-line local
config edit into a silent, structured exfil primitive, and the fix is two
lines of contract. Contract delta for §3.3, append: "the client rejects any
`advisory.endpoint_url` that is not `https://` (gap `no_config`); the HTTP
client does not follow redirects (a 3xx response is `http_error`); the
configured hostname is shown in the operator UI at every cycle." (Display
of the hostname also gives the operator a fighting chance against the
repointing-after-compromise case the review was asked to flag.)

**Rotation sign-off (§3.5, as instructed by §9.1).** I sign off the RULE
and its scope: §3.5 is correctly normative — chat-transit key treated as
transcript-exposed, bring-up/wiring use only, rotation in the provider
console with direct local entry before the first production digest cycle
and before the next tagged release, rotation event recorded as a date with
no key material. I CANNOT sign off the EXECUTION from this repository, by
construction: the key never enters the repo (that is the design working),
so no repo artifact proves rotation happened. To make "rotated" checkable
rather than asserted, RC-10 below adds the evidence artifact to the gate.
Until that artifact exists, the precondition stands unmet and the first
production digest cycle must not run.

**RC-10 (required change, process) — rotation evidence artifact.** Contract
delta for §9.2's gate list, append item (9): "a `docs/verification/` entry
records the §3.5 key rotation as executed (date, provider console action,
new key entered directly into the local key file; no key material or
derivation), cross-referenced from the ADR-033 status line; the first
production digest cycle and any tagged release carrying advisory wiring
require this entry."

## 4. The provider-trust boundary — hostile/compromised provider

**Ruling: bounded, with RC-5/RC-6/RC-7/RC-8 in place.** A hostile
provider's reachable effects under the ADR: display-only output for three
of four classes; clamped, `requires_adr`-flagged, shadow-only suggestions
for the fourth; and telemetry rows about its own suggestions. No lease,
admission, verification, receipt, catalog, or tracker state is reachable
(invariant b, structurally testable via A2's dependency-graph test — a
strong obligation, keep it). Cost abuse is properly capped and pre-call
enforced: 24 calls/day (UTC), 900 s spacing, 200-event threshold, 16 KiB
request cap, 64 KiB response cap, 2 048 completion tokens, timer-driven
only, no in-cycle retry — a hostile peer flooding telemetry events can
raise cadence up to the cap and no further. The one quality gap is
provider slander via summary text (AD-3) and the promotion-measurement
spoilage path (RC-6) — both closed above.

## 5. What the ADR missed entirely (independent pass)

**RC-11 (required change) — the `catalog_note` input set contradicts
invariant (a).** §2.3 has the plane summarize "the ADR-023 request queue and
the `catalog/candidate-profiles/` review rows". The useful content of those
surfaces is exactly what §3.2 cannot emit: ADR-023 requests carry
`display_name` (bounded free text) and a 280-character community `note`
(free text — the canonical prompt-injection carrier); candidate rows carry
`display_name` and `licenseEvidenceUrl` (a URL — an injection and
data-encoding carrier). As written, §2.3 is either useless (a structured-
only digest of sha256 clusters and quant histograms cannot tell the
provider WHAT it is triaging — the provider cannot even name the model) or
violates invariant (a) the moment an implementer "makes it work" by
passing display names. This must be decided in the contract, not in code
review. Contract delta for §2.3, replace the class description with:
"`catalog_note` digests carry structured fields only — request counts,
dedupe-cluster sizes keyed by `artifact_sha256` prefixes, quant enums, byte
histograms — plus `display_name` values drawn ONLY from the SIGNED catalog
and candidate rows already pinned in-repo (owner-controlled strings). The
community `note` field, free-text `display_name` from the request queue,
and URLs never enter any digest under any future revision without a new
ADR." (If the Architect prefers a typed "bounded identifier" form for
`hf_repo` — charset `[A-Za-z0-9._/-]{1,100}` — that is an acceptable
alternative to record in the RC-2 vocabulary artifact; the 280-char `note`
and URLs remain excluded either way.)

Advisories (no contract change required to start implementation; record as
amendment notes or tracker items):

- **AD-1 — guards pattern breadth.** `sk-[0-9a-f]{32}` is lowercase-hex
  only; if DeepSeek mints keys with uppercase or non-hex characters, the
  scan silently passes them. Widen the CI pattern (e.g. `sk-[0-9a-zA-Z]{20,}`)
  or verify the exact key shape against the provider console during §3.5
  rotation. The Redactor scrub is already broader (any `sk-` run ≥ 8).
- **AD-2 — telemetry control-char escaping (Integrator-owned handoff).**
  `Telemetry`'s `escape()` handles only `\` and `"` (lib.rs:203–205); a
  newline in any value produces invalid JSONL and enables line forging.
  Attack value reachability is demonstrated (RC-2's serde-fragment path;
  opaque `session_id`). Egress is unaffected (fail-closed), but fix the
  emitter to escape `\n`, `\r`, `\t` and control chars; route to the
  Integrator (modelswarm-telemetry) — suggested owner of the change.
- **AD-3 — display advisory claims next to their local backing counts.**
  Cheap defense against provider slander: the triage/digest renderer should
  pair any peer-naming advisory claim with (a link to) the local tallies it
  claims to summarize, so fabrication is visible at a glance.
- **AD-4 — provider retention terms.** §3.6 discloses what/whom/revoke but
  not retention: record that the owner checked DeepSeek's API
  data-retention/training-use terms for digest inputs. Metadata-only keeps
  the harm bounded regardless, so advisory only.
- **AD-5 — key memory hygiene and env-path note.** Crash dumps/swap can
  carry the key (owner machine; threat model §5.5 accepts local
  compromise); `MSP_ADVISORY_KEY_FILE` env-first resolution is likewise a
  local-process influence on key location — accepted under §5.5, noted for
  the record. No v1 action.

## 6. Test obligations to attach (feeds §9.2)

The §9.2 list is right; add these derived from the findings above so the
gate covers them executably:

1. (RC-1) digest property: no emitted field contains a base58 PeerId; peer
   references are exactly `^[0-9a-f]{16}$` SHA-256 prefixes.
2. (RC-2) adversarial redaction corpus built from the RC-2 table — for
   every row, the emitted digest contains the reduced form and never the
   raw string; specifically the serde-fragment vector: a crafted
   `capability_token` whose parse error embeds attacker text must never
   appear in any digest field (and the frozen-code reduction fires).
3. (RC-3) the serialized-body re-scan: corrupt one field post-gate in a
   test double and assert the request is not sent.
4. (RC-4) aggregation source: with a forged extra JSONL line injected into
   the log sink, digest contents are unchanged.
5. (RC-5) rationale renderer: a rationale containing markdown, a URL, and
   10 000 chars renders as plain, unlinked, capped text.
6. (RC-7) clamp table: hints for table-absent targets record
   `requires_adr=true`; in-range hints clamp to table bounds.
7. (RC-8) response binding: a well-formed response with a foreign
   `cycle_id` is discarded whole.
8. (RC-9) transport: non-https endpoint ⇒ `no_config`/inert; a 3xx
   response ⇒ `http_error`, no follow, no re-send with Authorization.
9. (§9.2(3) extension) newline-forged telemetry floods never move digest
   cadence past the caps.

## 7. What I could not verify (and why it does not block)

- The key file's existence, permissions, or rotation state — outside the
  repo BY DESIGN; gated by RC-10 instead.
- The actual DeepSeek key shape — AD-1 covers the residual.
- Anything about the advisory client's implementation — none exists; this
  review is the gate it must pass first.

## 8. Gate statement

Implementation of the advisory plane (any commit under §9.4's routing)
MUST NOT land until RC-1…RC-11 are recorded as an ADR-033 amendment by the
Protocol Architect. The §3.5 rotation (evidence per RC-10) remains a
precondition for the first production digest cycle. With the amendment
recorded, this review's approval stands; no re-review is required for the
amendment itself unless it deviates from the deltas above, in which case
return it to this seat.

— Security Engineer, 2026-10-12
