# Review: ADR-032 batched speculative verification — Security Engineer (blocking seat)

Reviewer: Security Engineer (independent). Routed by the Integrator.
Date: 2026-10-11 · Verdict: **APPROVE WITH REQUIRED CHANGES**
Reviewed object: `docs/adr/ADR-032-batched-speculative-verification.md`
(status Proposed, NOT accepted; commit `35ee671`, main @ `a8b630f`).

Scope note: this is a DESIGN review of the proposed contract. No production
implementation of `VerifyDrafts`/`VerifyDraftsResult` exists; nothing here
judges code, and nothing in this review accepts the ADR (owner decision
follows Security + Test and Release reviews).

Grounding read: `docs/threat-model.md`; ADR-032 (especially §2, §3, §7, §8);
`protocol/msp-v1.md` §2.2/§6.4/§6.5/§6.6 (effective registry per ADR-028 §5);
ADR-026 (lease gate), ADR-029 (cooperative namespace freeze terms),
ADR-030 (ReceiptV2), ADR-013/024/031 (exactness contract, backend pairing,
engine boundary); `protocol/msp-cooperative-v1.md`; E0 determinism record
(`docs/verification/e0-determinism-2026-10-07.md`);
`docs/reviews/handoff-protocol-architect-2026-10-10.md`; shipped
`crates/modelswarm-session/src/spec.rs` (SpecMessage vocabulary, sign/verify
discipline), `crates/modelswarm-session/src/lib.rs` (`compute_prefix_hash`,
commit gate), `crates/modelswarm-types/src/canonical.rs` (canonical JSON,
float rejection), `crates/modelswarm-node/src/serving.rs` (session admission
order as implemented).

Verdict counts: **0 blockers · 8 required changes · 5 advisories.**
No blocker: nothing makes the design irredeemable — every required change is
a contract-text delta, and the ADR's option-A wire-true scheduler term is
independently correct and should land regardless.

## 1. Answers to the six enumerated questions

**Q1 — Window/batch bounds and `verify_ms`.**
`WINDOW_CAP = 32` + the 256 KiB frame limit is sufficient per request: one
engine pass over ≤ 33 rows (~33–36 ms measured, P16), and the cap check
precedes engine work. The residual DoS is *rate and queue*, not size: a
hostile coordinator can stream sequential junk rounds, and nothing in the
ADR pins "one outstanding verify per session" or a bound on the WAIT_COMMIT
wait (RC-5, A-2). `verify_ms` is the right quantity (it is exactly the
engine-true term pass 2 proved missing) but the wrong type and an unchecked
self-report: as a float it is **unsignable** under the frozen canonical-JSON
rule (`modelswarm_types::canonical_json` returns `FloatRejected` for any
float — the shipped signer can never produce the proposed
`VerifyDraftsResult` preimage), and as a self-report the ADR's
"telemetry-de-rank" answer does not hold on its own — de-ranking fires only
*after* wasted rounds, while the engage gate consumes the self-report
*before* engaging, and if the EWMA were cohort-blended one lying verifier
could de-rank honest peers. Required: integer milliseconds + a
coordinator-measured cross-check (measured round wall time upper-bounds
`verify_ms`; a claim above the measured bound is provably false) + a
per-verifier EWMA (RC-2).

**Q2 — Replay/reorder.**
The request side is sound: `(session_id, nonce)` single-use, strictly
monotone `seq`, deadline, and cross-session/cross-profile replay fails the
signed `session_id`/`profile_id`/parent-hash bindings (verified against the
shipped binding discipline). Three gaps: (a) **result-side** — the
contract never pins the coordinator's acceptance rules for
`VerifyDraftsResult`; `nonce_echo` is echoed but no *equality check* is
normative, `seq` semantics are ambiguous (§1 shows request `seq: 12` and
result `seq: 13`, while §2 says "per sender per session" — a shared counter
and per-sender counters cannot both be true), and a replayed *genuine*
signed result is undetectable unless the coordinator enforces
nonce/round/hash matches (RC-3, RC-7); (b) the repeat-offense teardown
threshold is unspecified (RC-3); (c) `new_prefix_hash` must be recomputed
locally by the coordinator, byte-exactly (RC-6).

**Q3 — Hostile proposer.**
Acceptance-spoofing is impossible: the verifier-signed result is the only
authority and the verifier never believes the window (correct as designed).
Junk-window flooding cannot be cost-bounded *before* engine work — verifying
is the only way to know a window is junk — so the honest bound is on rate:
one outstanding verify per session (missing — RC-5), serving-layer admission
(8 sessions / 2 per peer, shipped), and the session budget (unenforced in
the ADR — A-2). Admission order with deadline LAST is right in one sense
(everything after it is engine work), but the check must cover the queue
between admission and engine dispatch, and out-of-range `deadline_ms`
semantics (clamp vs reject) are unpinned (A-2).

**Q4 — Hostile VERIFIER (the ADR leans on verifier authority).**
Not sound today: nothing wire-side detects a wrong
`accepted_prefix_len`/`correction`. The only shipped check is the
coordinator recomputing the hash chain over the *claimed* tokens
(`spec.rs:1642-1647`) — a lying verifier produces a self-consistent hash
over false tokens and passes. Exactness pins are engine-side tests, not
wire-side checks, and the ReceiptV2 counter-signature authenticates *who*
claimed the numbers, not *that the numbers are true* (ADR-030's
"never self-reported" governs who may increment counters; the claimed
values are still the verifier's own). This is a REQUIRED CHANGES item: the
lossless contract's authority must be checkable. The checkable mechanism
already exists in the project's own records — E0's consequence section:
"re-execution spot-checks are sound **within a backend family**" — so a
coordinator audit path (local re-verify of sampled committed windows on its
own engine for the same profile) is sound within the backend pairing §3
already requires, with the CPU cross-machine case gated on the same open E0
item that already gates pass 3. Plus honest labels on the receipt counters.
(RC-1.) Relatedly, the verifier's *own* authority is defensible against the
coordinator only if the verifier rejects commits that do not reproduce its
result (RC-4).

**Q5 — Canonical-JSON signature preimage rules vs msp-v1 §2.2.**
Three parity hazards, all load-bearing for cross-implementation signature
parity: (a) `verify_ms` is a float and floats are forbidden in signed
canonical payloads — the frozen signer rejects the message outright
(RC-2); (b) the ADR writes `"sha256:<…>"` for prefix hashes while the P2P
convention (msp-v1 §1 "all digests are lowercase hex") and the shipped
`compute_prefix_hash`/`GENESIS_PREFIX_HASH` are bare 64-hex — the wire form
must be pinned, and the byte preimage of `parent_prefix_hash`,
`new_prefix_hash`, and `generation_params_hash` is undefined in the ADR
(the shipped generation-params hash is a Rust `Debug` string — not
portable); (c) `divergence_point: null` and the signature-absent form
(omit the field entirely vs null vs empty string) must be pinned, since
either choice changes signature bytes (RC-6).

**Q6 — §8 state machine, invariants I1–I7.**
Three reachable states make invariants unenforceable at the wire:
(a) WAIT_COMMIT has no timeout — a vanished coordinator pins the verifier's
round slot and its KV indefinitely, and the diagram's "deadline → close"
label is unreachable from WAIT_COMMIT (no deadline exists there);
(b) receiving `VerifyDrafts` while a round is pending (VALIDATE/VERIFY/
WAIT_COMMIT) is undefined — the diagram only shows `VerifyDrafts` from
PREFILLED, so a flooding coordinator's second request is either implicitly
queued (engine-pin DoS) or implicitly dropped (cross-implementation
ambiguity); (c) round numbering across abandonment is undefined and the
shipped contiguity check (`round == state.round()+1`) *forbids* gaps — the
ADR's "round abandoned, session stays open" semantics is novel relative to
shipped code (which treats verifier cancellation as session-fatal) and must
be pinned. Additionally I2 ("every committed token passed a
`VerifyDraftsResult`") is unenforceable: the verifier's `apply()` checks
only hash chaining, so a hostile coordinator can commit tokens the verifier
never verified (RC-4, RC-5). I4 ("exactly one engine pass") is
unenforceable for any engine whose physical batch bound is below
`WINDOW_CAP` — the `batch_verify` boolean does not carry a max (RC-8).

## 2. Required changes (contract-text deltas)

### RC-1 — Verifier authority must be checkable: audit path + honest receipt labels (Q4)

Severity: **required-change**. Attack: a verifier returns a false-but-hash-
consistent result (e.g., always full acceptance → the coordinator commits
unverified drafts; or `accepted_prefix_len: 0` + a random `correction` →
garbage reaches the local client) while ReceiptV2 counters still grant it
`verified_tokens`. No wire-side check detects either today.

Where: ADR-032 §3, after the "lossless contract" bullet. Add:

> **Verifier authority is checkable, not assumed.** The committed stream's
> authority is the verifier-signed result, and a false result is NOT
> detectable from the signature or the hash chain alone (chain checks catch
> malformed results, not false-but-self-consistent ones). The coordinator
> MUST therefore run an audit path: within the E0-pinned backend pairing of
> this session, the coordinator locally re-executes a sampled fraction of
> committed windows with its own engine for the same profile — at minimum
> the first committed window of the session and one window after every
> acceptance/divergence transition — and compares token-for-token against
> the committed stream. On any divergence it records a
> `verifier_divergence` telemetry event, switches the session to the
> `SingleDecode` fallback for the remainder, and marks the receipt outcome.
> The audit path is sound within a backend family (E0 finding: re-execution
> spot-checks are sound within a backend family, not across); CPU
> cross-machine audit is gated on the same open E0 sub-question that
> already gates real-draft pass 3. Exactness pins remain engine-side tests;
> the audit path is the wire-side check the lossless label requires.
> Receipt counters granted from this family are verifier-claimed numbers
> inside counter-signed receipts — the ADR-030 grant rules govern WHO may
> increment counters, they do not independently measure the claimed values;
> this is an honest-label statement in the msp-v1 §5 `verified_capacity`
> sense and must be documented as such at the receipt intake.

### RC-2 — `verify_ms`: integer milliseconds + measured cross-check + per-verifier EWMA (Q1, Q5)

Severity: **required-change**. Attack: (a) inflation — a verifier inflates
`verify_ms` to poison the shared cost term and de-rank honest peers /
block engagement (de-ranking weapon); (b) deflation — a verifier under-
reports to bait engagement it cannot deliver, wasting coordinator budget
before the telemetry de-rank ever fires. Additionally the float type is a
signature-parity break (see below).

Where: ADR-032 §1, `VerifyDraftsResult` JSON and the `verify_ms` bullet.
Replace:

> ```json
>   "verify_ms": 35.2,
> ```

with:

> ```json
>   "verify_ms_ms": 35,
> ```

and replace the bullet

> `verify_ms` is the verifier's measured engine-pass time; it feeds the
> requester's EWMA for the engine-true verification term (§4)

with:

> `verify_ms_ms` is the verifier's measured engine-pass wall time in WHOLE
> milliseconds (u32, rounded to nearest ms). It MUST be an integer: floats
> are forbidden in signed canonical payloads (msp-v1 §2.2; the canonical
> JSON implementation rejects any float), so a fractional field makes
> `VerifyDraftsResult` unsignable across implementations. The requester
> maintains a **per-verifier** EWMA fed by the measured round latency it
> observes itself (send `VerifyDrafts` → receive `VerifyDraftsResult`),
> using `verify_ms_ms` only as a lower-bound sanity term. Cross-check
> (normative): `verify_ms_ms` ≤ observed round wall time + 25 ms clock
> noise, or the claim is recorded as `verify_ms_lie` telemetry, the
> verifier is de-ranked, and the engage gate discounts it as unverified.
> Under-reporting cannot be detected from timing alone (RTT/queue dominate);
> it is bounded by the existing realized-loss guard and acceptance-collapse
> telemetry. The EWMA is per-verifier: a cohort-wide blend would let one
> lying peer de-rank honest peers — forbidden.

### RC-3 — Result admission at the coordinator (Q2)

Severity: **required-change**. Attack: a replayed genuine signed result, or
a verifier that double-answers a round, is accepted if the coordinator does
not enforce request/result binding.

Where: ADR-032 §2, after the verifier admission-order paragraph. Add:

> **Result admission at the coordinator (normative).** A
> `VerifyDraftsResult` is applied only when ALL hold: (a) the signature
> verifies under the session's pinned verifier key and `sender` derives
> from that key AND equals the QUIC-authenticated remote PeerId (msp-v1
> §6.6 discipline); (b) `nonce_echo` EQUALS the outstanding request's
> `nonce` (equality check — pinned here); (c) `session_id`, `round`, `seq`,
> `profile_id`, and `parent_prefix_hash` equal the outstanding request's;
> (d) `accepted_prefix_len ≤ len(window_tokens)`, and on rejection
> `divergence_point == accepted_prefix_len`; (e) `new_prefix_hash` equals
> the locally recomputed hash of (parent ‖ accepted prefix ‖ [correction])
> under the byte rule of RC-6. The first valid result consumes the
> request's nonce; any later result carrying a consumed nonce, and any
> result with no outstanding request, is dropped with no state change.
> A second protocol violation by the same peer within one session tears
> the session down with an honest reason (the threshold is two).

### RC-4 — Commit-must-reproduce-result (Q6, I2 enforceability)

Severity: **required-change**. Attack: a hostile coordinator sends a
`PrefixCommit` that chains validly from the parent but does not equal the
verifier's own result (e.g., the proposer's full window after a rejection).
The shipped verifier `apply()` checks only chaining, so it commits tokens
it never verified, its KV continues from them, and the receipts count them
as verified — I2 is unenforceable at the wire.

Where: ADR-032 §8, WAIT_COMMIT annotation + invariants list. Add:

> **WAIT_COMMIT admission (normative):** a `PrefixCommit` for round n is
> applied only if `accepted_token_ids == window_tokens[..accepted_prefix_len]
> ++ [correction]` of the verifier's own outstanding `VerifyDraftsResult`
> for round n and `new_prefix_hash` equals that result's `new_prefix_hash`.
> A commit that chains validly from the parent but does not reproduce the
> result is rejected (`commit_mismatch`, cooperative-namespace
> CancelRound-class) and the round is abandoned. Without this rule I2
> ("every committed token passed a `VerifyDraftsResult`") is
> unenforceable: the shipped apply path verifies hash chaining only.

### RC-5 — State-machine pinning: pending-slot, WAIT_COMMIT timeout, round monotonicity (Q6, Q3)

Severity: **required-change**. Attacks: junk-round flooding through a
missing pending-slot rule; indefinite WAIT_COMMIT hold by a vanished
coordinator; round-number ambiguity across abandoned rounds.

Where: ADR-032 §8, after the invariants list. Add:

> **State-machine pinning (normative).** (a) At most ONE outstanding
> `VerifyDrafts` per session: a `VerifyDrafts` received while a round is in
> VALIDATE / VERIFY / WAIT_COMMIT is rejected — `replayed_request` if the
> nonce repeats, `bad_frame` otherwise — and is NEVER queued for engine
> work. (b) WAIT_COMMIT is deadline-bounded: the outstanding request's
> `deadline_ms` plus a fixed 250 ms grace; expiry abandons the round,
> discards the pending result, and returns to PREFILLED. The verifier
> never holds an uncommitted round for a vanished coordinator. (c) Round
> numbers are strictly monotone per session; a round abandoned without a
> commit leaves a gap, and the next `VerifyDrafts` must carry `round` >
> last handled round with `parent_prefix_hash` equal to the current
> committed hash. Contiguity is NOT required — the shipped
> `round == state.round() + 1` check forbids gaps and MUST NOT be
> inherited by this family. (d) `PREFILLED` accepts `VerifyDrafts` only
> when no round is outstanding.

### RC-6 — Canonical form and hash preimage pinning (Q5)

Severity: **required-change**. Attacks: none hostile — these are
cross-implementation signature-parity breaks, the same class the repo has
already paid for four times ("four shipped wire bugs" golden-vector rule).
Each ambiguity below silently forks signature bytes between Rust and any
second implementation.

Where: ADR-032 §1 (after the semantics bullets) and §2. Add:

> **Wire byte-pinning (normative).**
> (1) **Hash wire form:** all hashes in this family are BARE lowercase
> 64-hex (msp-v1 §1: all digests are lowercase hex). The `"sha256:<…>"`
> notation in this ADR's examples is descriptive only and MUST NOT appear
> on the wire (the prefixed form is the hub bodyDigest convention; the P2P
> commit-hash convention — shipped `compute_prefix_hash` /
> `GENESIS_PREFIX_HASH` — is bare hex).
> (2) **Prefix-hash preimage:** `hex(sha256( raw32(parent_hash_hex_decoded)
> ‖ token_ids as little-endian u32 each ))` — exactly the shipped
> `compute_prefix_hash` rule; genesis = sha256 of the empty string
> (`e3b0c4…`). `new_prefix_hash` = the same rule over
> (parent ‖ accepted prefix ‖ [correction]).
> (3) **`generation_params_hash` preimage:** sha256 over the canonical
> JSON (msp-v1 §2.2) of the pinned sampling-parameter object
> `{temperature, top_p, top_k, seed}` with `seed` null when unset. The
> shipped `default_sampling_params_hash` hashes a Rust description string —
> implementation-specific and NOT the wire preimage.
> (4) **Signature preimage:** canonical JSON of the message with the
> `signature` field OMITTED ENTIRELY (not null, not empty string). All
> other declared fields — including a null-valued `divergence_point` — are
> PRESENT in the canonical form. Floats are forbidden in signed payloads
> (RC-2).
> (5) **Sender binding:** `sender` MUST equal the installation id derived
> from the verifying key AND the QUIC-authenticated remote PeerId of the
> connection (msp-v1 §6.6; the discipline `verify_commit_signature` already
> applies to commits).

### RC-7 — `seq`/`round` counter semantics (Q2, Q5)

Severity: **required-change**. The §1 examples (request `seq: 12`, result
`seq: 13`) imply a shared per-session counter while §2 says "strictly
monotone per sender per session" — the two rules disagree and the
signature covers whatever value is chosen, so both sides must implement
the same rule.

Where: ADR-032 §2, first paragraph. Add:

> **`seq` semantics (normative):** each sender maintains its own per-session
> counter, incremented by exactly 1 on each state-changing message that
> sender emits; `seq` is strictly monotone per (session, sender). The
> `VerifyDraftsResult` therefore does NOT continue the coordinator's
> counter — the verifier's first result carries the verifier's own next
> `seq`. The §1 examples are updated accordingly (request `seq: 12` /
> result `seq: 4`; illustrative, not a shared counter). A message whose
> `seq` ≤ the last seen `seq` from that sender is dropped as
> `replayed_request` before any other check.

### RC-8 — Engine batch bound disclosure (Q6, I4 enforceability)

Severity: **required-change**. I4 ("exactly one engine pass per
`VerifyDrafts`") is unenforceable for any engine whose physical batch bound
is below `WINDOW_CAP = 32`: the `batch_verify` boolean tells the
coordinator nothing about the bound, so a conforming coordinator can
legally send windows the verifier cannot batch in one pass — forcing a
split (I4 violation) or a reject the coordinator could not have avoided.

Where: ADR-032 §3, `SessionAccept` disclosure sentence. Replace:

> `SessionAccept` gains the additive disclosure
> `"engine": { "backend": "cpu|vulkan", "batch_verify": true|false }`.

with:

> `SessionAccept` gains the additive disclosure
> `"engine": { "backend": "cpu|vulkan", "batch_verify": true|false,
> "batch_window_max": <u32, 1..=WINDOW_CAP> }`. `batch_window_max` is the
> largest window this engine verifies in one pass; `batch_verify: true`
> REQUIRES an honest `batch_window_max` (a peer that cannot honor I4 must
> declare `batch_verify: false`). The coordinator MUST NOT send
> `window_tokens` longer than the peer's `batch_window_max`; the verifier
> rejects longer windows with `payload_too_large` BEFORE any engine work.

## 3. Advisories (recommended deltas, not blocking)

### A-1 — §3 misreporting wording is overstated as written

"degrades acceptance economics … never output correctness" is true only
because the committed stream is defined as the verifier engine's own
continuation. A misreported backend silently shifts the continuation basis
away from the disclosed one and voids the RC-1 audit pairing for that
peer. Suggested replacement for the §3 bullet's last clause:

> …therefore degrades acceptance economics (wasted rounds, recorded in
> telemetry for de-ranking) and voids auditability for that peer (the
> RC-1 audit path pairs on the disclosed backend). Output remains the
> verifier engine's own continuation — never unverified garbage — but the
> continuation basis silently shifts to the undisclosed backend;
> de-ranking follows measured acceptance collapse and audit divergence.

### A-2 — Deadline semantics + session budget enforcement

(a) `deadline_ms` range handling is unpinned: pin server-side clamping to
`[1000, 120000]` per the msp-v1 §6.4 discipline (clamp, not reject). (b)
Deadline-last in the admission order is correct — everything after it is
engine work — but the deadline must be RE-CHECKED immediately before engine
dispatch, after any queue wait between admission and dispatch. (c) The
verifier must enforce the effective `SessionAccept` budget on this family
(`max_output_tokens` cumulative committed, `max_endpoint_seconds`
cumulative), refusing `VerifyDrafts` that would exceed it — otherwise a
hostile coordinator burns unbounded engine seconds within one admitted
session. Add these to §2 / §8.

### A-3 — Rejection frames echo codes only

Extend the shipped privacy discipline (`serving.rs` `wire_kind`: variant
names only, peer payloads never logged or echoed) to the family: rejection
frames and logs carry codes, counts, and hashes only — `window_tokens`,
`correction`, and any token ids never appear in rejections, receipts'
human-readable fields, or logs. Token ids ARE completion content in
encoded form (privacy rule 5; AGENTS.md constraint 5). Add to §6.

### A-4 — Negotiation-shape gaps

(a) The additive `SessionOffer.verifier_backend` and
`SessionAccept.engine` fields change the canonical forms of two frozen
shapes — the `protocol/vectors/` golden vectors must be extended in the
same gate (§7.1) before any second implementation reads them. (b) The
"rejects the offer with an honest reason" path has no wire shape: pin a
`SessionReject { session_id, protocol_version, reason }` message (or an
explicit error-frame code) — the namespace currently defines only accept.

### A-5 — `parent_mismatch` is not a §6.5 registry code

§2's "`parent_mismatch`-class reject" invites reading a code that does not
exist (effective §6.5: `handshake_failed`, `incompatible_protocol`,
`profile_mismatch`, `invalid_token`, `expired_token`, `revoked_token`,
`replayed_request`, `overloaded`, `deadline_exceeded`,
`payload_too_large`, `runtime_error`, `cancelled_by_peer`,
`interrupted`). Pin the split: round-level abandons are cooperative-
namespace `CancelRound { reason }` messages with the frozen reason
vocabulary `parent_mismatch | round_mismatch | commit_mismatch`;
§6.5 codes are reserved for frame/session-level errors. Wording delta in
§2 only; no registry change needed.

## 4. Surfaces the ADR's §7 list missed

1. Verifier-truthfulness / result authority checkability (RC-1) — not
   enumerated anywhere.
2. Coordinator-side result admission (RC-3) — §7.5 mentions nonce-echo
   "binding" but pins no check.
3. `verify_ms` as a statistical weapon — §7.1 treats the deadline DoS
   shape but not the honesty-signal weapon (RC-2).
4. WAIT_COMMIT hold / pending-slot flooding (RC-5).
5. Commit-equals-result enforcement (RC-4).
6. Signature-parity pinning: float rejection, bare-hex vs `sha256:`
   prefix, hash preimages (RC-6, RC-7).
7. Engine batch-bound mismatch (RC-8).
8. Session-budget exhaustion at the verifier (A-2).
9. Payload echo in rejection frames (A-3).
10. SessionReject wire shape + vector extension (A-4).

## 5. What the ADR gets right (independently verified sound)

- Cross-session/cross-profile replay genuinely fails the signed
  `session_id` + `profile_id` + parent-hash bindings; the genesis
  `sha256("")` collision across same-profile sessions does not open a
  replay hole because the signature covers `session_id`.
- Verifier-as-only-authority is correct for acceptance spoofing; the
  proposer cannot inflate acceptance.
- Window cap and frame checks precede engine work; one bounded pass per
  window is the right marginal cost.
- Lease gate (ADR-026) inheritance is correct — the family rides sessions
  already gated at open; no new hub surface is created (content-blindness
  and the tracker boundary hold; §6 non-goals are consistent with
  AGENTS.md constraint 5 and ADR-030).
- Backend pairing via additive cooperative-negotiation fields correctly
  avoids editing the byte-pinned `EligibilityLease`
  (`protocol/vectors/lease-hubkey-1.json` untouched) and honors ADR-024's
  forward constraint at the layer where it is enforceable; E0's
  CPU/Vulkan split and the cross-machine CPU gate are stated honestly.
- Option A (wire-true term first) is independently correct and should
  land regardless of B's fate; mixed swarms under the project rule and
  the class-blind fastest-single comparator are consistent with the
  revision invariants.
- The receipt grant discipline is not weakened (counters move only on
  counter-signed receipts) — RC-1's honest-label only makes explicit that
  the granted *values* are verifier-claimed.
- No conflict between `WINDOW_CAP = 32` and the shipped adaptive
  `WINDOW_MAX = 16` (cap is an upper bound; the coordinator's adaptive
  sizing is not redefined here).

## 6. Reproduction sketches (for the §7 gate tests the review obligations require)

- **Lying verifier (RC-1):** verifier returns `accepted_prefix_len = len(window)`,
  `correction` = the proposer's last draft token, `new_prefix_hash`
  self-consistently computed. Coordinator commits; client receives draft
  tokens never verified; receipts grant `verified_tokens`. Expected after
  RC-1: audit divergence detected on the first sampled window, fallback +
  `verifier_divergence` telemetry.
- **`verify_ms` inflation (RC-2):** verifier reports `verify_ms_ms` > the
  coordinator's observed round wall time + 25 ms. Expected: `verify_ms_lie`
  telemetry, engage-gate discount, de-rank.
- **Result replay (RC-3):** verifier re-sends round n's valid signed result
  after round n+1's request. Expected: dropped — `nonce_echo` equality
  fails.
- **WAIT_COMMIT hold (RC-5):** coordinator sends one `VerifyDrafts`, then
  vanishes. Expected: verifier abandons the round at deadline + 250 ms and
  returns to PREFILLED.
- **Commit divergence (RC-4):** coordinator commits the proposer's full
  window after the verifier rejected position k. Expected:
  `commit_mismatch`, round abandoned, no KV advance.
- **Parity (RC-6):** serialize `VerifyDraftsResult` and sign in two
  independent implementations. Expected: identical canonical bytes;
  `verify_ms_ms: 35` integer, bare-hex hashes, null `divergence_point`
  present, `signature` omitted from the preimage.

## 7. Commands run

- `cargo fmt --check` — clean (docs-only change; no code touched).
- `git diff --stat` — single new file under `docs/reviews/`.

No production code was read-into, modified, or approved by this review.
Workspace tests were not re-run: the change is a single documentation
file; the ADR remains Proposed and authorizes nothing runnable.
