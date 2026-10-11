# ADR-032: Batched speculative verification (one engine pass per draft window)

Status: **Accepted (2026-10-11, owner decision — option B, with option A
already live in code).** Both required reviews returned approve-with-
required-changes and every required change is folded into revision 2 (see
the amendment note). Implementation is authorized under the §7 gates; the
ADR-031 productionization gate remains the hard dependency for anything
runnable. Owner: Protocol Architect.

**Decision record (2026-10-11).** The owner accepted option B with the
directive: the main goal is benefits from a swarm regardless of the
engineering path — if benefits cannot be achieved one way, find other
ways; the engineering exercises themselves may surface other creative
"many is better than one" benefits, and accuracy benefits are welcome even
when slower. Scope note: this acceptance commits the batched-verify family
under the §3 contracts; the accuracy-benefits exploration is a parallel
research direction, and any mode that CHANGES outputs (rather than
verifying them) requires its own ADR and preset disclosure — the
lossless `speculative_exact` contract is not weakened by this acceptance.

**Amendment note (revision 2, 2026-10-11).** Folds
`docs/reviews/review-adr-032-testrelease-2026-10-11.md` (commit `10de665`:
blockers B1–B4, required R1–R3, advisories A1–A4) and
`docs/reviews/review-adr-032-security-2026-10-11.md` (commit `5ad5215`:
required RC-1..RC-8, advisories A-1..A-5). Changed from revision 1: the
Context headline number is corrected to the traceable 2,928 and loss figures
are relabeled as cell medians (B1, A1); round-level rejects are pinned as
cooperative-namespace `CancelRound` reasons, not §6.5 codes, and every
admission stage now has a decided failure string (B2, A-5, R1); the float
`verify_ms` is replaced by integer `verify_pass_ms` with a measured
cross-check and per-verifier EWMA (RC-2 — name chosen per task delegation;
the review's literal `verify_ms_ms` was not adopted, rationale in §1); a
coordinator audit path and honest receipt labels for verifier-claimed
numbers are added (RC-1); commit-must-reproduce-result admission is added in
WAIT_COMMIT (RC-4); the state machine pins one-outstanding-verify,
deadline+250 ms WAIT_COMMIT bound, and gap-tolerant round monotonicity
(RC-5); hash wire forms and byte preimages, null-vs-omission rules, and
per-sender `seq` semantics are byte-pinned (RC-6, RC-7); the engine
disclosure gains `batch_window_max` (RC-8); the golden-vector fixture set is
enumerated and committed at acceptance (B3); §7 gains an execution-venue
matrix and the invariant test paths, including the I2 coordinator property
(B4, R2, R3); advisories A-1..A-4 and A2–A4 are folded (audit-void wording,
deadline clamp + session-budget enforcement, no-payload-echo, `SessionReject`
shape + offer/accept vector extension, P16 single-run label, `≥ 0`-only
value pins). Two reviews' points of divergence and their resolutions are
recorded in the handoff addendum
(`docs/reviews/handoff-protocol-architect-2026-10-10.md`, revision 2).

Evidence base (all committed): `experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/ANALYSIS.md`
· `experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10/ANALYSIS.md` (the 9.6
pass-2 published negatives) · ADR-031 + the P16 spike record
(`docs/reviews/handoff-runtime-engineer-2026-10-09.md` §P16) ·
`docs/verification/e0-determinism-2026-10-07.md` · ADR-007/013/024/026/029/030 ·
`protocol/msp-v1.md` (effective §6.5 registry per ADR-028 §5) ·
`protocol/msp-cooperative-v1.md` · the shipped `SpecMessage` vocabulary and
commit machinery (`crates/modelswarm-session/src/spec.rs:206-339`,
`crates/modelswarm-session/src/lib.rs:100-260` — `compute_prefix_hash`,
`apply`, round reject reasons) · `crates/modelswarm-types/src/canonical.rs`
(canonical JSON, float rejection).

## Context

9.6 pass 2 published the wire-side negative: **2,928 engaged speculative
completions** — 946 on loopback+injected-delay (1,920 join rows) and 1,982
over a real two-machine QUIC + Noise swarm with ADR-026 leases (4,800 join
rows), 6,720 join rows total, zero failures (both ANALYSIS files) — and
**no engaged arm beat the fastest eligible single**. Engaged LAN cells lost
1.38–1.94× by cell median; the best engaged median ratio was 1.071× (the
parity corner: one round, window+1 vs window verifier tokens,
inj0ms-high-w16) and the worst engaged-arm **cell median** was 4.257×
(inj0ms-geo700-w16), both versus the prompt-best independently-executed
single. The cause is quantified, not anecdotal: the msp-v1 whole-request
wire verifies a draft window in **window+1 sequential engine steps plus a
full prefix re-post per round**, while the frozen cost model charges
`VERIFY_BATCH_STEPS = 1.5` decode steps per round
(`crates/modelswarm-bench/src/lib.rs:78`, the ADR-013 batch-verify ENGINE
model) — realized/predicted **2.8–8.3×** across engaged cells, i.e. the
model undercharges by ~(window+1)/1.5 (≈6× at w8, ≈11× at w16). With a
wire-true verification term, every engagement in both runs would have been
gate-blocked; the production rule-6 guard would have aborted 911/946
(loopback) and 1,857/1,982 (LAN) engaged completions at round 1. The
conclusion already on record in both ANALYSIS files and the scheduler
handoff: **on the msp-v1 whole-request wire, adaptive sizing must not engage
speculation; speculative wins require batch verification at the engine.**

The engine-side complement exists and is measured: ADR-031's in-process
C-API adapter (feature `modelswarm-runtime/capi-adapter`, OFF by default)
verifies a k=8 window in **one 9-row batched `llama_decode`** (33–36 ms,
token-exact against HTTP ground truth, single KV rollback at first
rejection), giving **1.45×** on a 2-round speculative loopback run with a
real model. Label per ADR-031's own environment paragraph and review A2:
single-run, single-machine, loopback, under dev-session load — motivation
evidence only; the repeat/P50 protocol ADR-031 §5 assigns to pass 2 governs
any claim-bearing reuse. Tree verification is not expressible through the
public C API; linear verify is the documented fallback. What does not exist
is the protocol contract that lets a cooperative session ask a remote peer
to verify a window in one engine pass — the shipped wire vocabulary carries
per-round whole requests only. This ADR specifies that contract and decides
where it lives.

## Decision

### 1. Batched verification semantics (the contract, not the implementation)

A two-message family is added to the **msp-cooperative-v1** namespace
(§5 decides the home):

**`VerifyDrafts`** — coordinator → verifier. Canonical JSON per msp-v1 §2.2,
snake_case:

```json
{
  "session_id": "<echoed from SessionOffer>",
  "protocol_version": "msp-cooperative-v1",
  "seq": 12,
  "round": 3,
  "profile_id": "msp1:<64 hex>",
  "parent_prefix_hash": "<bare 64-char lowercase hex>",
  "generation_params_hash": "<bare 64-char lowercase hex>",
  "window_tokens": [<u32 token ids, length 1..=WINDOW_CAP>],
  "proposer_id": "12D3Koo…",
  "nonce": "<single-use per session>",
  "deadline_ms": 2500,
  "sender": "12D3Koo…",
  "signature": "base64 ed25519 over canonical JSON with signature omitted entirely"
}
```

**`VerifyDraftsResult`** — verifier → coordinator. The `seq` values are
illustrative and per-sender (RC-7): the result carries the VERIFIER's own
counter, not a continuation of the coordinator's.

```json
{
  "session_id": "<echoed>", "protocol_version": "msp-cooperative-v1",
  "seq": 4, "round": 3,
  "accepted_prefix_len": 9,
  "divergence_point": null,
  "correction": 12345,
  "new_prefix_hash": "<bare 64-char lowercase hex>",
  "verify_pass_ms": 35,
  "nonce_echo": "<echoed>", "sender": "<verifier peer id>",
  "signature": "base64 ed25519"
}
```

Semantics (normative):

- The verifier executes **ONE batched engine pass** over
  `[prefix_last, window_tokens…]` with logits on every row, using the
  engine's own sampler chain under the pinned generation parameters, and
  accepts the longest prefix the target model itself would have sampled. On
  the first rejection the verifier rolls its KV back to that position and
  emits the replacement token in `correction`; when the whole window holds,
  `correction` is the bonus (lookahead) token. This is exactly the shipped
  `SpecMessage::VerificationResult` commitment discipline —
  `committed = window_tokens[..accepted_prefix_len] ++ [correction]` always
  holds — with the engine pass count as the only change. `TreeVerifyRequest`
  / `TreeVerifyResult` stay reserved for a future tree ADR (ADR-031 §4); in
  this family `window_tokens` is exactly one branch (linear batch verify).
- `divergence_point` is the 0-based index of the first rejected position,
  `null` on full acceptance; invariant: on rejection
  `divergence_point == accepted_prefix_len`. The result returns the LENGTH
  and the divergence point, not the accepted tokens — both sides already
  hold the window; the actual ids travel in the subsequent `PrefixCommit`.
- `correction` is always present for a non-empty window (replacement or
  bonus). An empty `window_tokens` is malformed (there is nothing to verify;
  the one-lookahead-token case is a window of the proposer's last greedy
  token).
- `verify_pass_ms` (RC-2, replacing revision 1's float `verify_ms`) is the
  verifier's measured engine-pass wall time in WHOLE milliseconds (u32,
  rounded to nearest ms; test pins assert `≥ 0` only, never measured values
  — review A4). It MUST be an integer: floats are forbidden in signed
  canonical payloads (msp-v1 §2.2; the canonical JSON implementation rejects
  any float — `crates/modelswarm-types/src/canonical.rs`, `FloatRejected`),
  so a fractional field makes `VerifyDraftsResult` unsignable. The rename
  from `verify_ms` is deliberate: a stale float-bearing implementation fails
  deserialization instead of silently coexisting. The requester maintains a
  **per-verifier** EWMA fed by the round latency it measures itself (send
  `VerifyDrafts` → receive `VerifyDraftsResult`), using `verify_pass_ms`
  only as a lower-bound sanity term. Cross-check (normative):
  `verify_pass_ms` ≤ observed round wall time + 25 ms clock noise, or the
  claim is recorded as `verify_ms_lie` telemetry, the verifier is de-ranked,
  and the engage gate discounts it as unverified. Under-reporting cannot be
  detected from timing alone (RTT/queue dominate); it is bounded by the
  existing realized-loss guard and acceptance-collapse telemetry. The EWMA
  is per-verifier: a cohort-wide blend would let one lying peer de-rank
  honest peers — forbidden.
- **Hard caps (frozen):** `WINDOW_CAP = 32` tokens per `VerifyDrafts`, and
  the msp-v1 §6.4 frame limit (256 KiB) bounds the message. A window longer
  than `min(WINDOW_CAP, the peer's batch_window_max)` is `payload_too_large`
  BEFORE any engine work (RC-8). This ceiling does not redefine the
  coordinator's adaptive sizing (shipped `WINDOW_MAX = 16` today — an upper
  bound, not a floor).

**Wire byte-pinning (RC-6, normative).** Each rule below is a
cross-implementation signature-parity break if left open — the bug class the
golden-vector rule exists to prevent:

1. **Hash wire form:** all hashes in this family are BARE lowercase 64-hex
   (msp-v1 §1: all digests are lowercase hex). The `"sha256:<…>"`
   notation is descriptive prose only and MUST NOT appear on the wire (the
   prefixed form is the hub `bodyDigest` convention; the P2P commit-hash
   convention — shipped `compute_prefix_hash` / `GENESIS_PREFIX_HASH` — is
   bare hex).
2. **Prefix-hash preimage:** `new_prefix_hash = hex(sha256(
   raw32(parent_hash_hex_decoded) ‖ token_ids as little-endian u32 each ))`
   over `(parent ‖ accepted prefix ‖ [correction])` — exactly the shipped
   `compute_prefix_hash` rule (`crates/modelswarm-session/src/lib.rs:134-146`).
   Genesis = hex(sha256 of the empty string) =
   `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
3. **`generation_params_hash` preimage (new canonical encoding, defined
   here):** sha256 over the canonical JSON (msp-v1 §2.2) of the pinned
   sampling-parameter object `{"seed":<u32|null>,"temperature":<num>,
   "top_k":<u32>,"top_p":<num>}` with `seed` null when unset. The shipped
   `default_sampling_params_hash` hashes a Rust `Debug` string —
   implementation-specific; it remains the OLD vocabulary's internal value
   and is NOT this family's wire preimage.
4. **Signature preimage:** canonical JSON of the message with the
   `signature` field OMITTED ENTIRELY (not null, not empty string). All
   other declared fields — including a null-valued `divergence_point` — are
   PRESENT in the canonical form. Floats are forbidden in signed payloads.
5. **Sender binding:** `sender` MUST equal the installation id derived from
   the verifying key AND the QUIC-authenticated remote PeerId of the
   connection (msp-v1 §6.6; the discipline `verify_commit_signature`
   already applies to commits).

### 2. Binding, replay, and malformed-message rules

Every state-changing message in the family binds: `protocol_version`,
`session_id`, `seq` (strictly monotone per **(session, sender)** — RC-7),
`round`, `profile_id`, accepted-prefix hash (`parent_prefix_hash`),
generation-parameter hash, sender identity, single-use `nonce`,
`deadline_ms`, and Ed25519 signature. This closes, for the new family, the
gap ADR-029 §4 recorded for the old vocabulary (`PrefixCommit` omits
version/nonce/deadline); migrating the remaining `SpecMessage` variants to
this binding set belongs to the full freeze ADR, not this one.

**`seq` semantics (RC-7, normative):** each sender maintains its own
per-session counter, incremented by exactly 1 on each state-changing
message that sender emits; `seq` is strictly monotone per (session,
sender). A `VerifyDraftsResult` therefore does NOT continue the
coordinator's counter — the verifier's first result carries the verifier's
own next `seq` (hence the §1 examples: request `seq: 12`, result `seq: 4`).
A message whose `seq` ≤ the last seen `seq` from that sender is dropped as
`replayed_request` before any other check.

**Verifier admission order** (deterministic, testable; **first-failure-wins**
is the property-test assertion — inject conjunctions, assert the earliest
code): frame size → schema → signature → session binding → session alive →
seq/nonce → profile → round/parent → window cap → session budget →
deadline → engine dispatch (deadline RE-CHECKED immediately before dispatch,
after any queue wait — A-2). The complete partition (R1):

| Injected condition | Outcome |
|---|---|
| frame > 256 KiB | `payload_too_large` |
| schema invalid / unknown fields | `bad_frame` |
| signature invalid | `bad_frame` (unauthenticated = malformed, fail-closed) |
| `session_id` unknown or ≠ the stream's admitted session | `bad_frame` |
| session torn down (e.g. after `profile_mismatch`) | `cancelled_by_peer`, stream closed |
| lease expired mid-session | `expired_token` |
| `seq` == last seen from sender (duplicate) | `replayed_request` |
| `seq` < last seen from sender (decrease, fresh value) | `replayed_request` |
| `nonce` reuse (any `seq`) | `replayed_request` |
| `profile_id` mismatch | `profile_mismatch` + session teardown |
| `round` ≤ last handled round | `CancelRound(reason="round_mismatch")`, no state change |
| `parent_prefix_hash` ≠ current committed hash | `CancelRound(reason="parent_mismatch")`, no state change |
| `VerifyDrafts` while a round is outstanding, nonce repeats | `replayed_request` |
| `VerifyDrafts` while a round is outstanding, fresh nonce | `bad_frame` (never queued — RC-5) |
| `len(window_tokens)` > `min(WINDOW_CAP, batch_window_max)` | `payload_too_large`, before engine work |
| session budget would be exceeded | `overloaded` (budget exhausted) |
| deadline past at admission | `deadline_exceeded` |
| any conjunction of the above | the earliest code in this table's order |

`deadline_ms` is clamped server-side to `[1000, 120000]` (clamp, not reject —
the msp-v1 §6.4 discipline; A-2). Rejection frames and logs carry codes,
counts, and hashes ONLY — `window_tokens`, `correction`, and any token ids
never appear in rejections, receipts' human-readable fields, or logs: token
ids ARE completion content in encoded form (privacy rule 5; the shipped
`serving.rs` `wire_kind` discipline, extended to this family — A-3).

**Round-level abandons are cooperative-namespace messages, not §6.5 codes
(B2 / A-5, decided).** The effective §6.5 registry (msp-v1 §6.5 + ADR-028
§5: `bad_frame`, `invalid_lease`, `executor_error` added;
`replayed_request`/`overloaded` renames) contains no chain-position code,
and none is added: frame- and session-level rejects map onto the effective
registry with **no registry change**; round-level abandons are
`CancelRound { session_id, round, reason }` with the frozen reason
vocabulary **`parent_mismatch | round_mismatch | commit_mismatch`** (§6.5
codes are reserved for frame/session-level errors). Revision 1's "no new
codes are needed" claim was false as written; the corrected claim is: no
§6.5 registry amendment is needed, and the exact reject strings are the
table above plus this vocabulary — decided, testable.

**Result admission at the coordinator (RC-3, normative).** A
`VerifyDraftsResult` is applied only when ALL hold: (a) the signature
verifies under the session's pinned verifier key, `sender` derives from that
key AND equals the QUIC-authenticated remote PeerId (msp-v1 §6.6); (b)
`nonce_echo` EQUALS the outstanding request's `nonce` (equality check —
pinned here); (c) `session_id`, `round`, `seq`, `profile_id`, and
`parent_prefix_hash` equal the outstanding request's; (d)
`accepted_prefix_len ≤ len(window_tokens)`, and on rejection
`divergence_point == accepted_prefix_len`; (e) `new_prefix_hash` equals the
locally recomputed hash of `(parent ‖ accepted prefix ‖ [correction])` under
the §1 byte rule. The first valid result consumes the request's nonce; any
later result carrying a consumed nonce, and any result with no outstanding
request, is dropped with no state change (a replayed genuine signed result
is thereby undetectable-to-harmless). A second protocol violation by the
same peer within one session tears the session down with an honest reason
(the threshold is two).

**Backward compatibility with v0.1 rule 6 (ADR-007) — unchanged and
outermost.** Draft tokens are never emitted to the local client; the
emission point is the `PrefixCommit` of verified tokens. "First output
token" for retry purposes is therefore the first *committed* token surfaced
by the gateway: before it, the gateway may fail over to another eligible
peer per ADR-007; after it, the stream ends with an honest interruption.
Inside a cooperative session, the fallback primitive is the existing
`SingleDecode` (exact by construction). The family never touches msp-v1
§6 single-stream messages.

### 3. Eligibility, determinism, and the lossless contract

- **Backend pairing (E0 + ADR-024 forward constraint).** E0 proved greedy
  determinism CPU-identical across thread counts and Vulkan-identical across
  GPU vendors, with CPU ≠ Vulkan. `SessionOffer` gains the additive field
  `verifier_backend: "cpu" | "vulkan"` (the requester's own backend — under
  the project rule the requester hosts exact P too); `SessionAccept` gains
  the additive disclosure `"engine": { "backend": "cpu" | "vulkan",
  "batch_verify": true | false, "batch_window_max": <u32, 1..=WINDOW_CAP> }`
  (RC-8). `batch_window_max` is the largest window this engine verifies in
  one pass; `batch_verify: true` REQUIRES an honest `batch_window_max` (a
  peer that cannot honor invariant I4 must declare `batch_verify: false`).
  The coordinator MUST NOT send `window_tokens` longer than the peer's
  `batch_window_max`; the verifier rejects longer windows with
  `payload_too_large` before any engine work. A peer whose backend differs,
  or that cannot batch-verify (§4), signals it and the requester falls back
  to single — mandatory, never silent. The reject path has a wire shape
  (A-4): **`SessionReject { session_id, protocol_version, reason }`** with
  the frozen reason vocabulary `preset_unavailable | backend_mismatch |
  batch_verify_unsupported | budget_infeasible`. The frozen §5
  `EligibilityLease` signed shape is NOT touched
  (`protocol/vectors/lease-hubkey-1.json` byte-pins it); backend rides the
  cooperative negotiation, which is the layer where ADR-024's "backend MUST
  become part of verifier-class eligibility" is enforceable without a
  frozen-field edit. Because these additive fields change the canonical
  forms of two frozen shapes, the `protocol/vectors/` golden vectors for
  `SessionOffer`/`SessionAccept` MUST be extended (and `SessionReject`
  vectors created) in the same gate as the family fixtures (A-4, §7 item 1).
  A tracker-side backend column for cohort pre-filtering is a possible
  future ADR, not this one.
- **What misreporting can and cannot do (A-1 wording).** The committed
  stream is always the verifier engine's own continuation; a lying proposer
  can only get drafts rejected. A peer misreporting its backend or
  `batch_verify` capability therefore degrades acceptance economics (wasted
  rounds, recorded in telemetry for de-ranking) AND voids auditability for
  that peer (the audit path below pairs on the disclosed backend). Output
  remains the verifier engine's own continuation — never unverified garbage
  — but the continuation basis silently shifts to the undisclosed backend;
  de-ranking follows measured acceptance collapse and audit divergence.
  Cross-machine CPU determinism remains E0's open sub-question and still
  gates any real-draft pass 3 (scheduler handoff 2026-10-10).
- **Verifier authority is checkable, not assumed (RC-1).** The committed
  stream's authority is the verifier-signed result, and a false result is
  NOT detectable from the signature or the hash chain alone (chain checks
  catch malformed results, not false-but-self-consistent ones). The
  coordinator MUST therefore run an audit path: within the E0-pinned
  backend pairing of this session, the coordinator locally re-executes a
  sampled fraction of committed windows with its own engine for the same
  profile — at minimum the first committed window of the session and one
  window after every acceptance/divergence transition — and compares
  token-for-token against the committed stream. On any divergence it
  records a `verifier_divergence` telemetry event, switches the session to
  the `SingleDecode` fallback for the remainder, and marks the receipt
  outcome. The audit path is sound within a backend family (E0 finding:
  re-execution spot-checks are sound within a backend family, not across);
  CPU cross-machine audit is gated on the same open E0 sub-question that
  already gates real-draft pass 3. Exactness pins remain engine-side tests;
  the audit path is the wire-side check the lossless label requires.
  **Receipt honesty:** counters granted from this family are
  verifier-claimed numbers inside counter-signed receipts — the ADR-030
  grant rules govern WHO may increment counters; they do not independently
  measure the claimed values. This is an honest-label statement in the
  msp-v1 §5 `verified_capacity` sense and MUST be documented as such at the
  receipt intake.
- **The lossless contract is unchanged and batch verify adds no
  approximation.** `speculative_exact` (ADR-013) means: greedy — token-equal
  to the verifier engine's own single-mode continuation (bit-parity by
  construction; P16 asserted exactly this against HTTP ground truth).
  Accepted length is a performance property, never a correctness property.
  ADR-029/031 preset disclosures stay linear-verify-bounded: `verified`/
  `maximum` claims must not imply tree-verify latency or acceptance until a
  future ADR says otherwise.

### 4. Engine requirement boundary (mixed swarms, honestly)

- Batched verify requires the **in-process C-API adapter** (ADR-031, feature
  OFF by default, productionization gate §5 items 1–5: shape-stable
  batching, `spawn_blocking`, per-handle KV RAM accounting, Vulkan
  in-process test, CI gate) **or an equivalent engine API surface** offering
  batched decode with per-row logits, KV rollback, and the engine's own
  sampler chain. The pinned llama.cpp HTTP server — today's only production
  adapter — **cannot batch-verify**; its per-token request/reply path is
  precisely the structural loss pass 2 measured.
- **Mixed swarms under the project rule.** Adapter peers and HTTP peers
  host the same exact profile P with identical swarm identity (ADR-031 §2:
  same pinned binaries, same `canonical_build_hash`; `llama.cpp-capi` is a
  run-manifest/telemetry name per ADR-019 honesty rules). An HTTP peer
  retains full `fast`/single eligibility and MAY act as proposer (a greedy
  window via streaming decode); it can NEVER act as batch verifier and must
  declare `batch_verify: false` in `SessionAccept.engine`.
- **The fastest-single guarantee is unchanged and class-blind.** The
  comparator is the fastest eligible single regardless of adapter class.
  **Companion correction (required regardless of this ADR's fate):** the
  acting planner's verification term must be engine-true — the 1.5-step
  batch term applies only to peers that declared `batch_verify`; every
  other cohort is charged window+1 sequential verifier tokens plus the
  per-round prefix re-post (the pass-2 never-engage conclusion, enforced
  rather than advised). This scheduler-crate change needs Scheduler +
  Protocol sign-off and precedes any light-up (it is option A below, and B
  includes it).

### 5. Options considered

| Option | Content | Cost |
|---|---|---|
| **A — status quo, hardened** | Keep the sequential wire; adopt the wire-true verification term so the engage gate blocks engagement on HTTP-only cohorts; publish the never-engage guidance as the permanent v0.x wire answer | Zero protocol surface; speculative mode is dead on this wire; the honest negative becomes final for the current wire; scheduler term change only |
| **B — batched-verify family (RECOMMENDED)** | §1–§4: `VerifyDrafts` family in msp-cooperative-v1, backend pairing + `batch_window_max`, engine adapter path, wire-true gate term (includes A) | New frozen messages + golden vectors + Security review surface; depends on ADR-031's productionization gate passing; mixed-swarm disclosure discipline; no tree verify (linear only) |
| **C — full msp-v2** | Redesign the wire around engine-proximate cooperative inference | Renegotiates every frozen surface; breaks msp-v1/messages.proto byte-identity for no additional capability — B is expressible additively; rejected |

**Recommendation: B, with A's term correction landing first as the standing
guard** (it protects production the day it lands and is independently
correct). B lives entirely in the additive msp-cooperative-v1 namespace:
msp-v1 and `protocol/messages.proto` stay byte-identical (the §5
forbidden-list in `protocol/msp-cooperative-v1.md` is respected); a v2 is
unwarranted while single mode works and only the verify path is structurally
capped.

### 6. Non-goals held

- **No KV-cache transfer.** KV state never leaves the verifier process; the
  wire carries token ids and hashes only (AGENTS.md hard constraint 8
  untouched).
- **No token-level distributed inference.** Cooperation stays at round/
  window granularity with whole-block messages; no pipeline parallelism, no
  layer splitting, no cross-peer token interleaving inside a pass.
- **Hub stays content-blind.** The family rides P2P libp2p streams; the hub
  sees only receipt digests (ADR-030); `VerifyDrafts` payloads are never
  logged (redaction rules unchanged); rejection frames echo codes only (A-3).
- **No economics beyond ADR-030**: `verified_tokens` counters from
  counter-signed receipts only, labeled verifier-claimed (§3); no payment
  surface.

### 7. Review and gate requirements (before any implementation)

**Security Engineer attack surfaces (the §4 enumerated list from the
review, folded):** (1) batch-size bounds — `WINDOW_CAP = 32` +
`batch_window_max` + 256 KiB frame limit, all checked before engine work;
(2) replay of verify requests — `(session_id, nonce)` single-use, per-sender
monotone `seq`, deadline enforcement, result-side admission per §2 (RC-3);
(3) acceptance-length spoofing by a hostile proposer — verifier-signed
result is the only authority, commits chain on prefix hashes, ReceiptV2
counters increment only from counter-signed verifier numbers (ADR-030 §3);
(4) **hostile verifier** — false-but-hash-consistent results detected by the
§3 audit path (RC-1), `commit_mismatch` by RC-4; (5) **`verify_pass_ms` as a
statistical weapon** — inflation caught by the measured cross-check
(`verify_ms_lie`), deflation bounded by realized-loss guard; per-verifier
EWMA prevents cohort poisoning (RC-2); (6) WAIT_COMMIT hold / pending-slot
flooding — RC-5; (7) signature parity — float rejection, bare-hex, hash
preimages, null-vs-omission, per-sender seq (RC-6, RC-7); (8) engine
batch-bound mismatch (RC-8); (9) session-budget exhaustion at the verifier
(A-2, invariant I8); (10) payload echo in rejection frames (A-3); (11)
negotiation-shape drift — `SessionReject` + vector extension (A-4). The
reproduction sketches in Security review §6 are the seed corpus for these
tests.

**Golden-vector fixture set (B3 — committed at acceptance, modeled on
`protocol/vectors/lease-hubkey-1.json`: fixture-only signing keys recorded
in the file, never production; `verify_at` time anchor so deadline checks
are deterministic; wire form + `canonical_json_sha256` byte-pin; expected
outcome with the EXACT code or CancelRound reason):**

| Fixture | Pins |
|---|---|
| `verifydrafts-1-accept.json` | Full acceptance, w=8: canonical-JSON bytes + sha256 of `VerifyDrafts`; signature preimage (signature-omitted canonical form); `VerifyDraftsResult` canonical bytes + signature + sha256; `accepted_prefix_len == len(window_tokens)`, `divergence_point: null` (present-as-null), `correction` = bonus token; `new_prefix_hash` value AND the byte-exact §1 chaining construction; `nonce_echo` |
| `verifydrafts-2-reject.json` | Mid-window rejection: `accepted_prefix_len = k < len`, `divergence_point == k`, `correction` = replacement token, chain hash recomputed |
| `verifydrafts-3-boundary.json` | Cap edge: len 32 accepted when `batch_window_max ≥ 32`; len 33 → `payload_too_large` before any engine work (pairs with the deadline-DoS shape) |
| `verifydrafts-4-eos.json` | EOS surfacing in the accepted path (prevents second-implementation drift on eos serialization) |
| `verifydrafts-malformed.json` | Array, each entry with `expected_reject`: empty `window_tokens`; bad signature; wrong `profile_id`; duplicate nonce; duplicate seq; non-monotone seq; stale/deadline-past (with `verify_at`); unknown session; torn-down session; lease-expired; window over `batch_window_max`; budget-exhausted |
| `session-offer-2.json` / `session-accept-2.json` / `session-reject-1.json` | Extended canonical forms of the additive fields (`verifier_backend`; `engine {backend, batch_verify, batch_window_max}`; `SessionReject`) — A-4 |

Consumer-neutrality rule: fixtures are implementation-neutral; the Rust
golden test lands with the family; any future implementation in any
language must reproduce identical canonical bytes, signatures, and outcomes
from the same files. If the owner wants a live second consumer,
`apps/modelswarm-sim` (T/R-owned) is the natural one.

**Execution-venue matrix (B4 — which job can ever run each item):**

| Test | Venue | Features/config | Gated on |
|---|---|---|---|
| Item 1 golden vectors (all fixtures) | per-push, `rust.yml` check job | default features | this ADR's acceptance |
| Item 3 admission-order/replay/reorder/stale/cross-profile/cap/malformed property suite | per-push | default features | acceptance |
| Item 5 divergence-point + boundary property | per-push | default features | acceptance |
| I2 coordinator property (hostile fake verifier), `SessionAccept` honesty, `verify_pass_ms` cross-check | per-push | default features | acceptance |
| Item 4a HTTP-never-engage + fastest-single-wins decision logic | per-push, `modelswarm-scheduler` | default features | option A wire-true term landing |
| Item 4b mixed-swarm fallback scenarios | per-push, `apps/modelswarm-sim` (T/R-owned; NOT bench quic-runner-only modules) | default features | 4a + the §3 negotiation fields |
| Item 2 exactness pins (incl. the I4 one-pass decode-count assertion) | manual dispatch, `real-model-e2e.yml` EXTENDED WITH A NEW CAPI STEP (or owner-local run + `docs/verification/` record) | `capi-adapter` + staged pinned engine + real GGUF | ADR-031 gate items 1–4 passing + their `docs/verification/` entry |

Two standing statements so nobody blocks a release on a job that does not
exist: **item 2 and the adapter-cohort half of item 4 will never go green in
per-push CI** (GitHub runners never stage engines); and the capi feature
clippy/test job that ADR-031 gate item 5 required **already exists in
`rust.yml` on every push** (satisfied since the 2026-10-08 review ruling).
Pin reruns are owed on every `runtime-pins.json` engine bump (token
exactness is build-sensitive), every ADR-031 gate-item change, and every new
backend — each with a `docs/verification/` entry; per ADR-031 §5.5 the
honest negative is published if exactness ever breaks. When the family
carries real traffic, the `MSP_LIVE=1`-style live-wire harness extension
obligation applies (`crates/modelswarm-node/tests/live_tracker.rs`
precedent). Invariant coverage: I3/I5/I6/I7 + I2 + the I4 disclosure half
are wire tests (per-push); I1 and the I4 one-pass half are engine-side-only
(item 2 venue).

### 8. Verifier round state machine (per session; reviewer-facing)

```text
session open (ADR-026 lease gate passed, SessionOffer accepted / SessionReject honest)
  └─ PREFILLED ──VerifyDrafts(round n)──► VALIDATE ──ok──► VERIFY (ONE engine pass)
        ▲                                     │                │
        │                    frame/session-level reject:        ├─ VerifyDraftsResult
        │                    bad_frame / replayed_request /     │    (accepted_len,
        │                    profile_mismatch(+teardown) /       │     divergence, correction,
        │                    expired_token / overloaded /        │     verify_pass_ms)
        │                    payload_too_large /                 ▼
        │                    deadline_exceeded               WAIT_COMMIT ──PrefixCommit(== result)──► COMMITTED(n) ──► PREFILLED
        │                                     │                  │        (chains validly but ≠ result → CancelRound
        │                    round-level: CancelRound             │         reason="commit_mismatch"; round abandoned — RC-4)
        │                    reason=parent_mismatch|              │ timeout: request deadline_ms + 250 ms fixed grace
        │                    round_mismatch (no state change)     │ → abandon round, discard pending result, → PREFILLED (RC-5)
        └────────── SingleDecode (exact fallback) ◄──────────────┤ measured gate breach / audit divergence / budget end
                                   │                              └─ second protocol violation by same peer → teardown
                                   └─► ReceiptV2 (two-phase, ADR-030; counters labeled verifier-claimed)
```

**WAIT_COMMIT admission (RC-4, normative):** a `PrefixCommit` for round n
is applied only if `accepted_token_ids == window_tokens[..accepted_prefix_len]
++ [correction]` of the verifier's own outstanding `VerifyDraftsResult` for
round n AND `new_prefix_hash` equals that result's `new_prefix_hash`. A
commit that chains validly from the parent but does not reproduce the
result is rejected (`CancelRound` reason `commit_mismatch`) and the round is
abandoned. Without this rule invariant I2 is unenforceable: the shipped
`apply()` path verifies hash chaining only.

**State-machine pinning (RC-5, normative).** (a) At most ONE outstanding
`VerifyDrafts` per session: a `VerifyDrafts` received while a round is in
VALIDATE / VERIFY / WAIT_COMMIT is rejected — `replayed_request` if the
nonce repeats, `bad_frame` otherwise — and is NEVER queued for engine work.
(b) WAIT_COMMIT is bounded by the outstanding request's `deadline_ms` plus
a fixed 250 ms grace; expiry abandons the round, discards the pending
result, and returns to PREFILLED — the verifier never holds an uncommitted
round for a vanished coordinator. (c) Round numbers are strictly monotone
per session; a round abandoned without a commit leaves a gap, and the next
`VerifyDrafts` must carry `round` > last handled round with
`parent_prefix_hash` equal to the current committed hash. Contiguity is NOT
required — the shipped `round == state.round() + 1` check forbids gaps and
MUST NOT be inherited by this family. (d) PREFILLED accepts `VerifyDrafts`
only when no round is outstanding. (e) The verifier enforces the effective
`SessionAccept` budget on this family (`max_output_tokens` cumulative
committed, `max_endpoint_seconds` cumulative), refusing `VerifyDrafts` that
would exceed it (`overloaded`, budget exhausted) — otherwise a hostile
coordinator burns unbounded engine seconds inside one admitted session.

Invariants: **(I1)** the committed stream equals the verifier engine's own
continuation under pinned params (engine-side test only); **(I2)** every
committed token passed a `VerifyDraftsResult` or `SingleDecodeResult` —
enforced verifier-side by RC-4 and coordinator-side by RC-3 + the RC-1
audit path, tested as a default-feature coordinator property against a
hostile fake verifier/proposer; **(I3)** the prefix-hash chain is gap-free
(every commit's parent equals the last committed hash) and
reorder-rejecting, while round NUMBERS may gap after abandonment; **(I4)**
exactly one engine pass per `VerifyDrafts` — disclosure half wire-tested
via `batch_verify`/`batch_window_max` honesty, one-pass half asserted as a
decode count inside the item-2 pin; **(I5)** never-worse engage rule with
the engine-true term (ADR-013 margin ≥ 0.05) and mandatory fallback;
**(I6)** session-scoped single-use nonces and per-sender strictly monotone
`seq`; **(I7)** cross-profile isolation by `profile_id` + parent-hash
binding; **(I8)** session-budget enforcement at the verifier (§8e).

## Consequences

+ The pass-2 conclusion becomes actionable: the wire gains the one
  capability its published negative proved is required, without touching a
  single frozen msp-v1 byte, and every reject path has a decided, testable
  string with an enumerated fixture set and an honest venue matrix.
+ The lossless contract, content-blindness, project rule, and fastest-single
  comparator all survive unchanged; capability discovery is honest
  (`batch_verify: false` peers are first-class singles, never fake
  verifiers); verifier authority is auditable, not assumed.
− Depends on ADR-031's productionization gate; until it passes, this ADR
  authorizes nothing runnable, and the wire-true term (option A) is the only
  production-visible change.
− Linear verify only: no tree acceptance; multi-proposer diversity gains
  wait on a future tree ADR (pass-2 finding: extra proposers cost traffic
  without raising acceptance in a linear world).
− One more frozen surface to defend: per-push golden-vector, admission
  property, and invariant tests (forever), plus manual-dispatch pin reruns
  on every engine bump — the maintenance cost the T/R review enumerated.
