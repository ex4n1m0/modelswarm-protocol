# ADR-032: Batched speculative verification (one engine pass per draft window)

Status: **Proposed (2026-10-10) — draft for review, NOT accepted.** Nothing in
this ADR is implementable until it is accepted. Owner: Protocol Architect.
Required reviewers before any implementation: Security Engineer, Test and
Release Engineer (AGENTS.md ownership map). Named consultees: Runtime Engineer
(§4 engine boundary, ADR-031 gate), Scheduler Scientist (§4 cost-model
correction — flagged as a Protocol/Runtime sign-off question in
`docs/reviews/handoff-scheduler-scientist-2026-10-10.md`).

Evidence base (all committed): `experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/ANALYSIS.md`
· `experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10/ANALYSIS.md` (the 9.6
pass-2 published negatives) · ADR-031 + the P16 spike record
(`docs/reviews/handoff-runtime-engineer-2026-10-09.md` §P16) ·
`docs/verification/e0-determinism-2026-10-07.md` · ADR-007/013/024/026/029/030 ·
`protocol/msp-v1.md` (effective §6.5 registry per ADR-028 §5) ·
`protocol/msp-cooperative-v1.md` · the shipped `SpecMessage` vocabulary
(`crates/modelswarm-session/src/spec.rs:206-339`).

## Context

9.6 pass 2 published the wire-side negative: **2,460 engaged speculative
completions** (946 loopback+injected-delay; 1,982 over a real two-machine QUIC
+ Noise swarm with ADR-026 leases; 6,720 join rows total, zero failures), and
**no engaged arm beat the fastest eligible single**. Engaged cells lost
1.38–1.94× (LAN) with best engaged median 1.071× (the parity corner: one
round, window+1 vs window verifier tokens) and worst 4.257×. The cause is
quantified, not anecdotal: the msp-v1 whole-request wire verifies a draft
window in **window+1 sequential engine steps plus a full prefix re-post per
round**, while the frozen cost model charges `VERIFY_BATCH_STEPS = 1.5`
decode steps per round (`crates/modelswarm-bench/src/lib.rs:78`, the ADR-013
batch-verify ENGINE model) — realized/predicted **2.8–8.3×** across engaged
cells, i.e. the model undercharges by ~(window+1)/1.5 (≈6× at w8, ≈11× at
w16). With a wire-true verification term, every engagement in both runs
would have been gate-blocked; the production rule-6 guard would have aborted
911/946 (loopback) and 1,857/1,982 (LAN) engaged completions at round 1. The
conclusion already on record in both ANALYSIS files and the scheduler
handoff: **on the msp-v1 whole-request wire, adaptive sizing must not engage
speculation; speculative wins require batch verification at the engine.**

The engine-side complement exists and is measured: ADR-031's in-process
C-API adapter (feature `modelswarm-runtime/capi-adapter`, OFF by default)
verifies a k=8 window in **one 9-row batched `llama_decode`** (33–36 ms,
token-exact against HTTP ground truth, single KV rollback at first
rejection), giving **1.45×** on a 2-round speculative loopback run with a
real model. Tree verification is not expressible through the public C API;
linear verify is the documented fallback. What does not exist is the
protocol contract that lets a cooperative session ask a remote peer to
verify a window in one engine pass — the shipped wire vocabulary carries
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
  "parent_prefix_hash": "sha256:<committed prefix this window extends>",
  "generation_params_hash": "sha256:<pinned sampling params>",
  "window_tokens": [<u32 token ids, length 1..=WINDOW_CAP>],
  "proposer_id": "12D3Koo…",
  "nonce": "<single-use per session>",
  "deadline_ms": 2500,
  "sender": "12D3Koo…",
  "signature": "base64 ed25519 over canonical JSON with signature absent"
}
```

**`VerifyDraftsResult`** — verifier → coordinator:

```json
{
  "session_id": "<echoed>", "protocol_version": "msp-cooperative-v1",
  "seq": 13, "round": 3,
  "accepted_prefix_len": 9,
  "divergence_point": null,
  "correction": 12345,
  "new_prefix_hash": "sha256:<hash of parent ‖ accepted ‖ [correction]>",
  "verify_ms": 35.2,
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
- `verify_ms` is the verifier's measured engine-pass time; it feeds the
  requester's EWMA for the engine-true verification term (§4) — the quantity
  pass 2 proved the frozen model was missing.
- **Hard cap (frozen):** `WINDOW_CAP = 32` tokens per `VerifyDrafts`, and
  the msp-v1 §6.4 frame limit (256 KiB) bounds the message. A window longer
  than the cap is `payload_too_large` before any engine work.

### 2. Binding, replay, and malformed-message rules

Every state-changing message in the family binds: `protocol_version`,
`session_id`, `seq` (strictly monotone per sender per session), `round`,
`profile_id`, accepted-prefix hash (`parent_prefix_hash`), generation-
parameter hash, sender identity, single-use `nonce`, `deadline_ms`, and
Ed25519 signature over canonical JSON with the signature field absent. This
closes, for the new family, the gap ADR-029 §4 recorded for the old
vocabulary (`PrefixCommit` omits version/nonce/deadline); migrating the
remaining `SpecMessage` variants to this binding set belongs to the full
freeze ADR, not this one.

Admission order at the verifier (deterministic, testable): frame size →
schema → signature → session/lease (ADR-026 gate already passed at session
open) → seq/nonce → `profile_id` match → `parent_prefix_hash` match →
window cap → deadline. Rejections map onto the **effective msp-v1 §6.5
registry** (ADR-028 §5) — no new codes are needed: malformed → `bad_frame`;
duplicate nonce/seq → `replayed_request`; wrong profile → `profile_mismatch`
(and session tear-down); wrong parent hash or reordered round →
`parent_mismatch`-class reject (session stays open, round is abandoned —
`CancelRound` semantics); oversized window → `payload_too_large`; past
deadline → `deadline_exceeded`. Cross-session and cross-profile commits are
rejected by construction: the prefix-hash chain and `profile_id` binding
make a commit from another swarm unverifiable, not merely unwelcome.

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
  the additive disclosure `"engine": { "backend": "cpu|vulkan",
  "batch_verify": true|false }`. A peer whose backend differs, or that
  cannot batch-verify (§4), rejects the offer with an honest reason and the
  requester falls back to single — mandatory, never silent (ADR-029 §3).
  The frozen §5 `EligibilityLease` signed shape is NOT touched
  (`protocol/vectors/lease-hubkey-1.json` byte-pins it); backend rides the
  cooperative negotiation, which is the layer where ADR-024's "backend MUST
  become part of verifier-class eligibility" is enforceable without a
  frozen-field edit. A tracker-side backend column for cohort pre-filtering
  is a possible future ADR, not this one.
- **What misreporting can and cannot do (state it honestly).** The
  committed stream is always the verifier engine's own continuation; a
  lying proposer can only get drafts rejected. A peer misreporting its
  backend or `batch_verify` capability therefore degrades acceptance
  economics (wasted rounds, recorded in telemetry for de-ranking), never
  output correctness. Cross-machine CPU determinism remains E0's open
  sub-question and still gates any real-draft pass 3 (scheduler handoff
  2026-10-10).
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
  say so in `SessionAccept.engine.batch_verify: false`.
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
| **B — batched-verify family (RECOMMENDED)** | §1–§4: `VerifyDrafts` family in msp-cooperative-v1, backend pairing, engine adapter path, wire-true gate term (includes A) | New frozen messages + golden vectors + Security review surface; depends on ADR-031's productionization gate passing; mixed-swarm disclosure discipline; no tree verify (linear only) |
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
  logged (redaction rules unchanged).
- **No economics beyond ADR-030**: `verified_tokens` counters from
  counter-signed receipts only; no payment surface.

### 7. Review and gate requirements (before any implementation)

**Security Engineer review — attack surfaces enumerated for that review:**

1. **Batch-size bounds**: `WINDOW_CAP = 32` frozen; frame limit 256 KiB;
   admission checks before any engine work (deadline-exhaustion DoS with
   huge windows near deadline is the shape to test).
2. **Replay of verify requests**: `(session_id, nonce)` single-use, `seq`
   strictly monotone per sender, deadline enforcement; replayed →
   `replayed_request`; cross-session/cross-profile replay must fail at the
   profile/parent-hash binding, and repeat offenses tear the session down.
3. **Acceptance-length spoofing by a hostile proposer**: the verifier-signed
   `VerifyDraftsResult` is the only authority; commits chain on prefix
   hashes; ReceiptV2 counters increment only from the counter-signed
   verifier numbers (ADR-030 §3 grant rules). A proposer over-claiming
   acceptance is ignored; repeated junk windows cost the proposer its own
   standing (wasted-round telemetry feeding de-ranking).
4. **Backend/adapter misreporting**: performance-only impact per §3 —
   verify by test, de-rank on measured acceptance collapse.
5. Signature surface: canonical-JSON preimage rules (signature-absent
   form), nonce-echo binding of results to requests.

**Test and Release gate tests to expect:**

1. Cross-language golden vectors for the family (canonical form, signature
   preimages, `VerifyDraftsResult` invariants) in `protocol/vectors/`
   before any second implementation reads the namespace.
2. Exactness pins: batched-verify committed stream == single-mode decode on
   the same engine, token-for-token, including reject-path rollback and EOS
   (the P16 pin set, re-run on the productionized adapter).
3. Replay / reorder / stale / cross-profile / cross-session rejection
   tests; window-cap enforcement; malformed-message drop tests; admission
   order per §2.
4. Engage-gate tests with the engine-true term: HTTP-only cohorts never
   engage; adapter cohorts engage only when predicted ×(1+margin) beats the
   fastest eligible single; mixed swarms fall back when the fastest single
   (either class) wins.
5. Divergence-point consistency property: `accepted_prefix_len ≤
   len(window_tokens)`; on rejection `divergence_point ==
   accepted_prefix_len`; `new_prefix_hash` chains from `parent_prefix_hash`.

### 8. Verifier round state machine (per session; reviewer-facing)

```text
session open (ADR-026 lease gate passed, SessionOffer accepted)
  └─ PREFILLED ──VerifyDrafts(round n)──► VALIDATE ──ok──► VERIFY (ONE engine pass)
        ▲                                     │                │
        │                              reject: bad_frame /      ├─ VerifyDraftsResult
        │                              replayed_request /       │    (accepted_len,
        │                              profile_mismatch /       │     divergence, correction)
        │                              parent_mismatch /        ▼
        │                              payload_too_large /    WAIT_COMMIT ──PrefixCommit──► COMMITTED(n) ──► PREFILLED
        │                              deadline_exceeded        │ (wrong chain → reject, round abandoned)
        └────────── SingleDecode (exact fallback) ◄─────────────┤ measured gate breach / proposer loss
                                   │                              └─ deadline → close, honest reason
                                   └─► ReceiptV2 (two-phase, ADR-030)
```

Invariants: (I1) committed stream == verifier engine's own continuation
under pinned params; (I2) every committed token passed a
`VerifyDraftsResult` or `SingleDecodeResult`; (I3) prefix-hash chain is
gap-free and reorder-rejecting; (I4) exactly one engine pass per
`VerifyDrafts` (a peer that cannot honor I4 must not declare
`batch_verify`); (I5) never-worse engage rule with the engine-true term
(ADR-013 margin ≥ 0.05) and mandatory fallback; (I6) session-scoped
single-use nonces and monotone seq; (I7) cross-profile isolation by
`profile_id` + parent-hash binding.

## Consequences

+ The pass-2 conclusion becomes actionable: the wire gains the one
  capability its published negative proved is required, without touching a
  single frozen msp-v1 byte.
+ The lossless contract, content-blindness, project rule, and fastest-single
  comparator all survive unchanged; capability discovery is honest
  (`batch_verify: false` peers are first-class singles, never fake
  verifiers).
− Depends on ADR-031's productionization gate; until it passes, this ADR
  authorizes nothing runnable, and the wire-true term (option A) is the only
  production-visible change.
− Linear verify only: no tree acceptance; multi-proposer diversity gains
  wait on a future tree ADR (pass-2 finding: extra proposers cost traffic
  without raising acceptance in a linear world).
− One more frozen surface to defend: golden vectors, admission order, and
  the §7 attack surfaces are now standing review obligations.
