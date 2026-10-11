# ADR-032 VerifyDrafts family — implementation plan

Status: plan of record for implementing the accepted VerifyDrafts family
(ADR-032 revision 2, accepted 2026-10-11, owner decision option B).
Author: Protocol Architect. Binding contract for implementers: ADR-032
revision 2 (`docs/adr/ADR-032-batched-speculative-verification.md`).
Where this plan and the ADR appear to conflict, the ADR wins and the
conflict is escalated to the architect seat; items where revision 2 needs
a follow-up amendment are flagged in §6, never silently interpreted.

Grounding (all read, main @ 4be7436): `crates/modelswarm-session/src/lib.rs`
(`compute_prefix_hash`, `SessionStateMachine::apply`, `RejectReason`),
`crates/modelswarm-session/src/spec.rs` (`SpecMessage`, `verify_commit_signature`,
`default_sampling_params_hash`, `WINDOW_MAX`, phase D/E flows),
`crates/modelswarm-types/src/canonical.rs` (canonical JSON, `FloatRejected`),
`crates/modelswarm-identity/src/{canonical,installation,timestamp}.rs`
(`canonical_json -> Option`, `installation_id_for`, `new_nonce`),
`crates/modelswarm-runtime/src/lib.rs` (`SamplingParams`,
`SamplingParams::default` = temperature 1.0 / top_p 1.0 / top_k 40 /
seed None), `crates/modelswarm-runtime/src/capi/mod.rs` (`verify_drafts`,
`VerifyOutcome { accepted, bonus: Option<u32> }`),
`crates/modelswarm-scheduler/src/shadow.rs:384-394` (option-A term landed),
`protocol/msp-v1.md` §2.2/§6.4/§6.5(+ADR-028 §5 effective registry)/§6.6,
`protocol/msp-cooperative-v1.md` (§1 note, §2 SessionOffer, §3 SessionAccept,
§7 vector obligation), `protocol/vectors/lease-hubkey-1.json` +
`apps/tracker/scripts/make-lease-vector.mjs` (generator precedent),
`crates/modelswarm-eligibility/src/lease.rs:585-612` (consumer precedent),
`.github/workflows/rust.yml` (`check` job = per-push venue).

Venue interpretation (pinned, from ADR-032 §7 matrix): the per-push rows
are gated only on "this ADR's acceptance" — the wire contract, admission
state machine, fixtures, and mock-verifier tests land in normal CI now.
The ADR status line "the ADR-031 productionization gate remains the hard
dependency for anything runnable" means the REAL-ENGINE execution path
(batched `verify_drafts` in serving traffic, item 2 exactness pins,
adapter cohorts): those wait for ADR-031 §5 gate items 1–5 plus their
`docs/verification/` entry. Flagged as F-5 in §6 for confirmation at
kickoff; do not block steps 1–4 on ADR-031.

Dependency order of this plan: §1 types → §2 fixtures → §3 verifier →
§4 coordinator → cross-boundary items in §5. Step 0 (option A) is already
landed and only needs test confirmation (§5.2).

---

## 1. Step 1 — type + serialization layer

### 1.1 Home: `crates/modelswarm-session`, new modules

New files (both architect-owned paths):

- `crates/modelswarm-session/src/verifydrafts.rs` — the two message
  structs, byte-rule helpers, admission types (§3), coordinator types (§4).
- `crates/modelswarm-session/src/negotiation.rs` — `SessionOffer`,
  `SessionAccept`, `SessionReject` wire structs.

Justification for the session crate over `modelswarm-types`:
`protocol/msp-cooperative-v1.md` §1 names
`crates/modelswarm-session/src/spec.rs` as the Rust home of the
cooperative message vocabulary; `modelswarm-types/src/lib.rs` is restricted
to msp-v1-frozen fundamentals. The family additionally needs
`SigningKey`/`VerifyingKey` (via `modelswarm-transport`),
`InstallationIdentity`/`installation_id_for` (`modelswarm-identity`), and
`compute_prefix_hash` (this crate) — all already session-crate imports.

New dependency: `modelswarm-types` (for `canonical_json` returning
`Result<_, CanonicalError>` with `FloatRejected`). ADR-032's evidence base
cites `crates/modelswarm-types/src/canonical.rs` explicitly. Do NOT switch
existing `spec.rs` paths off `modelswarm_identity::canonical_json` in this
plan (no cross-cutting edits); new code MUST use
`modelswarm_types::canonical_json`. Add one parity unit test asserting
both implementations produce identical bytes for every family payload
(they differ only in error type today; the test makes any future
divergence a build failure, not an interop break).

### 1.2 Message structs (ADR-032 §1 field-for-field)

Serde rules encoded once, here:

- All fields snake_case. Deserialization is a strict two-pass: parse to
  `serde_json::Value`, then assert the object's key set EQUALS the
  declared field set (missing key or unknown key ⇒ schema failure ⇒
  `bad_frame` per §2 table row "schema invalid / unknown fields"), then
  `serde_json::from_value` into the struct. This is what enforces
  present-as-null (`divergence_point` and the EOG `correction`, §1 rule 4
  and flag F-2) — serde alone cannot distinguish missing from null on
  `Option` fields.
- Integer-only fields: any value that arrives as a JSON float fails
  `from_value` into `u32`/`u64` fields ⇒ `bad_frame`; the signing side is
  additionally guarded because `canonical_json` rejects floats
  (`FloatRejected`, msp-v1 §2.2 / ADR-032 §1 "verify_pass_ms" paragraph).
- `#[serde(deny_unknown_fields)]` is NOT relied on (the two-pass key-set
  check above is the enforcement; it also produces the exact failure the
  admission table names).

```rust
// crates/modelswarm-session/src/verifydrafts.rs (sketch — field names,
// types, and serde behavior are normative; private helpers elided)
pub const COOPERATIVE_PROTOCOL_VERSION: &str = "msp-cooperative-v1";
pub const WINDOW_CAP: u32 = 32;                      // ADR-032 §1 hard cap
pub const DEADLINE_CLAMP_MIN_MS: u32 = 1_000;        // §2 clamp, not reject
pub const DEADLINE_CLAMP_MAX_MS: u32 = 120_000;      // msp-v1 §6.4
pub const WAIT_COMMIT_GRACE_MS: u64 = 250;           // §8 RC-5(b)
pub const VERIFY_PASS_CLOCK_NOISE_MS: u64 = 25;      // §1 cross-check
pub const TEARDOWN_VIOLATION_THRESHOLD: u32 = 2;     // §2 RC-3 tail

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyDrafts {
    pub session_id: String,
    pub protocol_version: String,        // must equal COOPERATIVE_PROTOCOL_VERSION
    pub seq: u64,                        // per (session, sender), §2 RC-7
    pub round: u64,                      // strictly monotone per session, §8 RC-5(c)
    pub profile_id: String,              // "msp1:<64 hex>"
    pub parent_prefix_hash: String,      // bare 64-char lowercase hex (§1 rule 1)
    pub generation_params_hash: String,  // bare 64-char lowercase hex
    pub window_tokens: Vec<u32>,         // length 1..=min(WINDOW_CAP, batch_window_max)
    pub proposer_id: String,
    pub nonce: String,                   // single-use per session
    pub deadline_ms: u32,                // clamped server-side to [1000, 120000]
    pub sender: String,                 // installation_id_for(verifying key), §1 rule 5
    pub signature: String,               // base64 ed25519 (STANDARD alphabet)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyDraftsResult {
    pub session_id: String,
    pub protocol_version: String,
    pub seq: u64,                        // VERIFIER's own counter (§2 RC-7)
    pub round: u64,
    pub accepted_prefix_len: u32,        // <= len(window_tokens) (§2 RC-3 (d))
    pub divergence_point: Option<u32>,   // PRESENT as null on full acceptance
    pub correction: Option<u32>,         // replacement / bonus; null only at EOG (F-2)
    pub new_prefix_hash: String,         // bare 64-char lowercase hex
    pub verify_pass_ms: u32,             // whole ms, >= 0 only is asserted (A4)
    pub nonce_echo: String,
    pub sender: String,
    pub signature: String,
}
```

Wire encoding: these serialize as the BARE objects of ADR-032 §1 — they
are NOT new `SpecMessage` variants (no external tag), matching the ADR's
JSON examples. They ride the same `Session::send_json`/`recv_json` path
as `SpecMessage` (ADR-018 staging), each as one JSON frame.

### 1.3 Byte-rule helpers (ADR-032 §1 rules 1–5, each one function)

1. `fn is_bare_hash(s: &str) -> bool` — 64 chars, `[0-9a-f]` only. The
   `"sha256:<…>"` prefixed form is rejected (§1 rule 1: the prefix is hub
   `bodyDigest` prose, never this wire). Applied to
   `parent_prefix_hash`, `generation_params_hash`, `new_prefix_hash`.
2. `new_prefix_hash` preimage: REUSE the shipped
   `crate::compute_prefix_hash` unchanged (lib.rs:134–157 —
   `hex(sha256(raw32(parent_hex) ‖ token ids as LE u32))` over
   `(parent ‖ accepted prefix ‖ [correction])`. Genesis =
   `GENESIS_PREFIX_HASH` (spec.rs:117). No new hash function.
3. `pub fn batch_generation_params_hash(p: &SamplingParams) ->
   Result<String, ParamsHashError>` — NEW canonical encoding per §1 rule
   3: sha256 over canonical JSON of
   `{"seed":<null|integer>,"temperature":<integer>,"top_k":<integer>,
   "top_p":<integer>}`. Rendering: integral f32 values (1.0) render as
   bare integers (`1`); a non-integral value returns
   `ParamsHashError::NonIntegral` and fails closed (the shipped
   canonicalizer would reject the float anyway). The shipped
   `default_sampling_params_hash` (Debug-string) is untouched and remains
   the OLD vocabulary's internal value — it is NOT this family's
   preimage. Numeric-rendering width questions are flag F-1 (§6).
4. `fn signing_preimage<T: Serialize>(msg: &T) -> Result<String,
   CanonicalError>` — canonical JSON of the message with the `signature`
   field OMITTED ENTIRELY (not null, not empty); every other declared
   field — including null-valued `divergence_point`/EOG `correction` —
   PRESENT. Implementation: `serde_json::to_value`, remove the
   `"signature"` key, `modelswarm_types::canonical_json`.
   `fn sign(msg, identity) -> Result<String, SpecError>` and
   `fn verify_signature(msg_value, expected_key, claimed_sender)` follow
   the exact discipline of `spec.rs::verify_commit_signature` (base64
   STANDARD, 64-byte `Signature::from_slice`, sender-binding check
   FIRST): `installation_id_for(expected_key) == sender` (§1 rule 5,
   msp-v1 §6.6). The QUIC-authenticated remote-PeerId equality is
   enforced by the serve loop, which is the only place that knows the
   stream's peer id.
5. Signature alphabet: base64 STANDARD (matching every existing
   cooperative signature; NOT base64url — that is the hub lease token
   convention).

### 1.4 Negotiation shapes (ADR-032 §3; msp-cooperative-v1 §2/§3)

No Rust `SessionOffer` exists today (only scheduler shadow references),
so `negotiation.rs` defines all three fresh, byte-matching the frozen
doc plus the ADR-032 additive fields:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend { Cpu, Vulkan }          // ADR-032 §3 vocabulary

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionBudget {
    pub max_endpoints: u32,               // >= 1
    pub max_endpoint_seconds: u64,        // >= 1
    pub max_output_tokens: u32,           // >= 1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionOffer {                 // msp-cooperative-v1 §2 + ADR-032 §3
    pub session_id: String,
    pub protocol_version: String,         // "msp-cooperative-v1"
    pub preset: Preset,                   // fast|balanced|deep|verified|maximum
    pub budget: SessionBudget,
    pub verifier_backend: Backend,        // ADDITIVE (ADR-032 §3); required in the
}                                         // new canonical form pinned by session-offer-2.json

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineDisclosure {             // ADR-032 §3 (RC-8)
    pub backend: Backend,
    pub batch_verify: bool,
    pub batch_window_max: u32,            // validated 1..=WINDOW_CAP
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionAccept {
    pub session_id: String,               // echoed
    pub protocol_version: String,         // echoed
    pub preset: Preset,                   // echoed
    pub budget: SessionBudget,            // effective, clamped DOWN only
    pub engine: EngineDisclosure,         // ADDITIVE (ADR-032 §3)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionReject {                // ADR-032 §3 (A-4)
    pub session_id: String,
    pub protocol_version: String,
    pub reason: SessionRejectReason,
}
```

Frozen vocabularies as enums with `Display`/`as_str` (the
`RejectReason`/`FallbackReason` precedent):

- `SessionRejectReason`: `preset_unavailable | backend_mismatch |
  batch_verify_unsupported | budget_infeasible` (ADR-032 §3).
- `CancelRoundReason` (THIS family only): `parent_mismatch |
  round_mismatch | commit_mismatch` (ADR-032 §2). The existing
  `SpecMessage::CancelRound` "proposal_failed" string belongs to the old
  vocabulary and is not reused here.
- Frame-level reject codes map 1:1 onto the effective §6.5 registry
  strings (ADR-032 §2: no registry change): `bad_frame`,
  `replayed_request`, `expired_token`, `cancelled_by_peer`,
  `profile_mismatch`, `payload_too_large`, `overloaded`,
  `deadline_exceeded`.

Validation pins: `batch_verify: true` REQUIRES `batch_window_max` in
`1..=WINDOW_CAP` (§3 — a peer that cannot honor I4 declares
`batch_verify: false`); guidance for incapable peers:
`batch_verify: false, batch_window_max: 1` (any in-range value is
schema-valid; the meaning only attaches to `batch_verify: true`).
Negotiation messages carry no signature of their own — they ride the
handshake-authenticated session (msp-cooperative-v1 §1; msp-v1 §6.6
PeerId binding), exactly like today's `SpecMessage` non-commit variants.

### 1.5 Step-1 tests (in `verifydrafts.rs` `#[cfg(test)]`)

- Round-trip + exact `serde_json::to_value` shape for both messages
  (the `spec_message_tags_are_snake_case_and_round_trip` precedent).
- Present-as-null: serializing a full-acceptance result emits
  `"divergence_point":null`; a result with the key absent fails the
  two-pass parse (`bad_frame` path).
- Signature preimage: signature verifies; flipping any signed field
  breaks it; `"sha256:"`-prefixed hash fails `is_bare_hash`.
- `batch_generation_params_hash`: stable, 64 hex, integral default
  renders integers; non-integral returns `NonIntegral`.
- Canonicalizer parity: `modelswarm_identity::canonical_json` and
  `modelswarm_types::canonical_json` agree byte-for-byte on family
  payloads.

## 2. Step 2 — fixture generator + committed golden vectors

### 2.1 House precedent (state it, follow it)

`protocol/vectors/lease-hubkey-1.json` was produced by a checked-in
deterministic generator (`apps/tracker/scripts/make-lease-vector.mjs`:
fixed fixture key recorded in-file, fixed fields, no clock/randomness,
`canonical_json_sha256` self-pin, trailing-newline JSON output) and is
consumed by an `include_str!` golden test
(`crates/modelswarm-eligibility/src/lease.rs:585-612`: serde form equals
recorded `fields`, signature verifies under the recorded public key,
time checks anchored at the recorded `verify_at`). Deviation here, with
rationale: the generator is RUST, not Node —
`crates/modelswarm-session/examples/gen-verifydrafts-vectors.rs`, run as
`cargo run -p modelswarm-session --example gen-verifydrafts-vectors`
from the repo root. Reasons: (a) the ADR names the Rust golden test as
the consumer and there is no second implementation yet, so generating
with the real `canonical_json`/`SigningKey`/`compute_prefix_hash` code
makes fixture-vs-consumer byte-parity structural rather than hoped-for
(this is precisely the failure class the 2026-10-08 lease interop break
showed); (b) the Node path depends on `apps/tracker`'s `@noble/ed25519`
install, and a `protocol/` artifact must not depend on the tracker app;
(c) `apps/tracker/scripts/` is Tracker-Engineer-owned. Consumer
neutrality (ADR-032 §7) is preserved: fixtures record canonical bytes,
signatures, and sha256 pins; any future implementation (TS, sim) must
reproduce them from the files. Extending
`apps/tracker/scripts/validate-vectors.mjs` to these files is a later,
Tracker-owned task.

### 2.2 Generator algorithm (exact)

Determinism rules: fixture-only Ed25519 seeds recorded in every file
(coordinator seed `[0x11; 32]`, verifier seed `[0x22; 32]` — the
`[9u8;32]`/`0x07` fixture-key style of ADR-020/lease-hubkey-1; never
production); fixed token ids, ids, nonces; the ONLY time value is the
recorded `verify_at` (`"2026-10-12T00:00:00Z"`) which anchors every
deadline check; output files written with `JSON.stringify`-equivalent
pretty JSON + trailing newline (Rust: `serde_json::to_string_pretty` +
`'\n'`). The generator takes no arguments and is idempotent — rerunning
must leave `git diff` empty (asserted in CI via the golden test, and
manually at handoff).

Per fixture: build the struct → `serde_json::to_value` → remove
`signature` → `canonical_json` (assert Ok — no floats) → sign preimage
bytes with `SigningKey::from_bytes(&seed)` → base64 STANDARD → insert
`signature` → record, for BOTH `VerifyDrafts` and `VerifyDraftsResult`
where present:

```json
{
  "name": "verifydrafts-1-accept",
  "description": "...ADR-032 §7 fixture table row 1...",
  "coordinator_seed_hex": "11".repeat(32),
  "coordinator_public_key_hex": "...",
  "coordinator_installation_id": "...",       // installation_id_for(pubkey)
  "verifier_seed_hex": "22".repeat(32),
  "verifier_public_key_hex": "...",
  "verifier_installation_id": "...",
  "verify_at": "2026-10-12T00:00:00Z",
  "engine": { "backend": "cpu", "batch_verify": true, "batch_window_max": 32 },
  "request":  { "fields": { "...full VerifyDrafts incl. signature..." },
                "canonical_json": "<signature-omitted canonical bytes>",
                "signature": "<base64>",
                "canonical_json_sha256": "<sha256 of the omitted-signature form>" },
  "result":   { "fields": { "...": "..." }, "canonical_json": "...", "signature": "...",
                "canonical_json_sha256": "..." },
  "expected": { "outcome": "accepted",
                "accepted_prefix_len": 8, "divergence_point": null,
                "correction": 0, "committed": [ 0 ],
                "new_prefix_hash": "<value AND construction recorded>" }
}
```

(`"correction": 0` / `"committed": [ 0 ]` above are placeholders for the
fixture's fixed token ids — the generator writes the real values.)

`new_prefix_hash` is computed with the real `compute_prefix_hash` over
`(parent ‖ accepted ‖ [correction])`; the fixture also records the
inputs so the consumer test (and any second implementation) can rebuild
it independently of our function.

Fixture set (ADR-032 §7 table, exact filenames):

| File | Content pins |
|---|---|
| `protocol/vectors/verifydrafts-1-accept.json` | w=8 full acceptance; `divergence_point: null` present-as-null; `correction` = bonus; chain hash + construction; `nonce_echo`; seq pair (request 12, result 4 — the §1/RC-7 illustration made normative bytes) |
| `protocol/vectors/verifydrafts-2-reject.json` | mid-window rejection at k<w; `divergence_point == accepted_prefix_len == k`; `correction` = replacement; recomputed chain hash |
| `protocol/vectors/verifydrafts-3-boundary.json` | len 32 accepted with `batch_window_max = 32`; plus the len-33 case as an admission input (`payload_too_large` before engine work — pairs with the deadline-DoS shape) |
| `protocol/vectors/verifydrafts-4-eos.json` | EOG inside the accepted path; `correction: null` present-as-null (flag F-2); commit covers the accepted prefix only |
| `protocol/vectors/verifydrafts-malformed.json` | ARRAY of cases, each `{ "name", "wire": <raw JSON or framing note>, "context": {...pre-state...}, "expected_reject": "<exact code or CancelRound reason>" }`: empty `window_tokens`; bad signature; wrong `profile_id`; duplicate nonce; duplicate seq; non-monotone seq; deadline-past relative to `verify_at`; unknown session; torn-down session; lease-expired; window over `batch_window_max`; budget-exhausted. Malformed inputs that the structs cannot express are stored as RAW JSON (the admission API must accept `Value`) |
| `protocol/vectors/session-offer-2.json`, `session-accept-2.json`, `session-reject-1.json` | extended canonical forms (`verifier_backend`; `engine{backend,batch_verify,batch_window_max}`; `SessionReject` with each reason variant pinned once — the file records one primary case plus a `reasons` list of all four canonical serializations) |

Addition beyond the ADR table (justified, no ADR needed — it vectors
already-frozen shapes): also generate `session-offer-1.json` and
`session-accept-1.json` (base ADR-029 forms without the additive
fields). msp-cooperative-v1 §7 already requires a SessionOffer canonical
vector "before any second implementation reads this namespace"; none
exists; creating it now gives the "-2" numbering meaning and pins the
base shape the additive fields extend.

### 2.3 Consumer-side golden test

`crates/modelswarm-session/tests/verifydrafts_vectors.rs`
(per-push, default features): `include_str!` each fixture; for every
message: (1) recompute canonical bytes from `fields` and assert equality
with `canonical_json` AND `canonical_json_sha256`; (2) verify the
signature under the recorded public key and assert
`installation_id_for(pubkey) == sender`; (3) run the §3 admission gate
with a mock engine and a clock fixed at `verify_at`, asserting the exact
expected code / CancelRound reason / accepted state transition; (4) for
results, recompute `new_prefix_hash` from the recorded construction and
assert equality. This test is the CI guard that the committed fixtures
and the implementation still reproduce each other byte-for-byte.

## 3. Step 3 — verifier-side admission + state machine

### 3.1 Module layout and engine abstraction

All in `verifydrafts.rs`. Engine access is a trait so the whole family
is testable with a mock verifier (ADR-032 §4 boundary; nothing here
requires the capi adapter):

```rust
#[async_trait::async_trait]
pub trait BatchVerifyEngine: Send + Sync {
    /// ONE batched engine pass over [prefix_last, window...] with logits
    /// on every row, the engine's own sampler chain under `params`
    /// (ADR-032 §1). Returns (accepted_len, correction).
    async fn verify_window(&self, window: &[u32], params: &SamplingParams)
        -> Result<(u32, Option<u32>), RuntimeError>;
    fn capabilities(&self) -> EngineDisclosure;
}
```

The production implementation (capi `verify_drafts` wrapped in
`spawn_blocking`) is OUT of this plan's scope: it is ADR-031-gated
engine work (Runtime Engineer, §5.4). The clock is injected
(`now_ms: u64` parameter on every admission call) so `verify_at`-anchored
fixture tests and property tests are deterministic.

### 3.2 The gate (pure state machine)

```rust
pub enum GateState { Prefilled, Validating, Verify, WaitCommit, Committed }
```

```text
            SessionOpen (ADR-026 lease gate; SessionOffer accepted / SessionReject honest)
                                 │
                                 ▼
   ┌─────────► PREFILLED ──VerifyDrafts(round n)──► VALIDATE ──ok──► VERIFY (ONE pass)
   │             ▲  │                                │                  │
   │             │  │ frame/session-level reject      │           VerifyDraftsResult
   │             │  │ bad_frame / replayed_request /  │                  │ (accepted_len,
   │             │  │ profile_mismatch(+teardown) /   │                  │  divergence,
   │             │  │ expired_token / overloaded /    ▼                  │  correction,
   │             │  │ payload_too_large /          WAIT_COMMIT ──PrefixCommit(== result)──► COMMITTED(n) ──► PREFILLED
   │             │  │ deadline_exceeded                │     chains validly but != result → CancelRound "commit_mismatch"
   │             │  round-level: CancelRound           │       (round abandoned — RC-4)
   │             │  parent_mismatch | round_mismatch   │ timeout: deadline_ms + 250 ms → abandon, discard, → PREFILLED (RC-5)
   │             │                                   │
   │             └── SingleDecode fallback ◄─────────┤ measured gate breach / audit divergence / budget end
   │                                                └─ second protocol violation by same peer → teardown
   └─ COMMITTED(n): hash chain advances; round numbers may gap (RC-5(c))
```

(ASCII rendering of ADR-032 §8; that diagram is normative.)

State carried by `VerifyDraftsGate`: session binding (session_id,
profile_id, generation_params_hash), committed hash +
`last_handled_round` (these diverge under gaps), outstanding request
(round, nonce, deadline_ms, window_tokens, result), consumed-nonce set
(session-scoped), `last_seq` per sender, budget ledger (§3.5), per-peer
violation counters, lease expiry, `GateState`.

### 3.3 Admission order (ADR-032 §2, first-failure-wins)

Executed exactly in this order; the property suite asserts that any
CONJUNCTION of injected conditions yields the EARLIEST code in this
list (§2: "the earliest code in this table's order"):

1. frame size > 256 KiB → `payload_too_large`
2. schema (two-pass parse, key set, integer-only, bare-hex, protocol
   version) → `bad_frame`
3. signature (sender-binding FIRST, then ed25519 over the
   signature-omitted canonical form) → `bad_frame` (fail-closed:
   unauthenticated = malformed)
4. session binding (`session_id` unknown or ≠ the stream's admitted
   session) → `bad_frame`
5. session alive? torn down → `cancelled_by_peer`, stream closed
6. seq/nonce: `seq` ≤ last seen from sender (duplicate or decrease) →
   `replayed_request`; nonce reuse at any seq → `replayed_request`
   (checked BEFORE any other semantic state — §2 RC-7: "dropped as
   `replayed_request` before any other check" applies to the seq stage)
7. profile: `profile_id` mismatch → `profile_mismatch` + session
   teardown
8. round/parent: `round` ≤ last handled round →
   `CancelRound("round_mismatch")`, no state change;
   `parent_prefix_hash` ≠ current committed hash →
   `CancelRound("parent_mismatch")`, no state change
9. one-outstanding (RC-5(a)): a `VerifyDrafts` while a round is in
   VALIDATE/VERIFY/WAIT_COMMIT → `replayed_request` if the nonce
   repeats (normally already caught at stage 6), else `bad_frame`;
   NEVER queued (row position: immediately after round/parent, matching
   the §2 table rows 13–14)
10. window cap: `len(window_tokens)` >
    `min(WINDOW_CAP, batch_window_max)` → `payload_too_large`, BEFORE
    any engine work (RC-8); empty window → `bad_frame` (schema)
11. session budget (I8/§8e): would exceed `max_output_tokens` (cumulative
    committed) or `max_endpoint_seconds` (cumulative) → `overloaded`
12. deadline past at admission → `deadline_exceeded`
13. dispatch: `deadline_ms` RE-CHECKED immediately before engine work,
    after any queue wait (A-2) → `deadline_exceeded`

`deadline_ms` is CLAMPED to `[1000, 120000]`, not rejected (§2, msp-v1
§6.4). Rejection frames and logs carry codes, counts, and hashes ONLY —
`window_tokens`, `correction`, and any token ids NEVER appear in
rejections, receipts' human-readable fields, or logs (§2 privacy rule;
the `serving.rs` `wire_kind` discipline; A-3). Assert this in tests by
inspecting the serialized reject frames.

### 3.4 WAIT_COMMIT admission (RC-4) and chain application

A `PrefixCommit` for round n applies only if BOTH:
`accepted_token_ids == window_tokens[..accepted_prefix_len] ++
[correction]` of the verifier's own outstanding result for round n AND
`new_prefix_hash` equals that result's `new_prefix_hash`. A commit that
chains validly from the parent but does not reproduce the result is
`CancelRound("commit_mismatch")` and the round is abandoned.

CRITICAL reuse decision: the family MUST NOT apply commits through the
shipped `SessionStateMachine::apply` — its `round == current + 1` check
forbids gaps (`RejectReason::FutureRound`), and ADR-032 §8 RC-5(c)
states the shipped check "MUST NOT be inherited by this family". The
gate keeps its own chain state and applies the same checks itself:
session/profile/params equality, `round > last committed round`
(strictly monotone, gaps allowed — I3), parent-hash equality,
`compute_prefix_hash` verification, plus a bounded per-round duplicate
memory (`DUPLICATE_MEMORY_ROUNDS` discipline) so replays are remembered
no-ops. `compute_prefix_hash` itself is reused unchanged.

### 3.5 Budget enforcement (I8)

The verifier tracks cumulative committed tokens vs
`budget.max_output_tokens` and cumulative session seconds vs
`budget.max_endpoint_seconds` (started at SessionAccept), refusing a
`VerifyDrafts` that would exceed either with `overloaded` (§8e). The
seconds ledger advances on wall-clock deltas reported by the serve loop
(mocked as fixed in tests).

### 3.6 Step-3 test list (venue: per-push, `rust.yml` `check` job, default features — §7 matrix rows 2/3)

File `crates/modelswarm-session/tests/verifydrafts_admission_properties.rs`
(proptest, the `session_properties.rs` conventions):

1. every §2-table row singly, against the mock engine + fixed clock;
2. first-failure-wins conjunction sweep: for random pairs/triples of
   injected conditions, the emitted code equals the earliest in §3.3
   order;
3. replay: same nonce (any seq), duplicate seq, decreased seq →
   `replayed_request`, no state change (I6);
4. reorder/stale: `CancelRound("round_mismatch")` for round ≤ last
   handled; `CancelRound("parent_mismatch")` for wrong parent; neither
   changes committed state;
5. cross-profile: wrong `profile_id` → `profile_mismatch` + teardown,
   later messages → `cancelled_by_peer` (I7);
6. cap: len 33 / over `batch_window_max` → `payload_too_large` with the
   mock engine asserting ZERO `verify_window` calls (before engine
   work);
7. one-outstanding: fresh-nonce `VerifyDrafts` during
   VALIDATE/VERIFY/WAIT_COMMIT → `bad_frame`, never queued; repeated
   nonce → `replayed_request` (RC-5(a));
8. WAIT_COMMIT: timeout at `deadline_ms + 250 ms` abandons the round,
   discards the pending result, returns to PREFILLED; the NEXT
   `VerifyDrafts` with `round > last handled` and parent = committed
   hash is accepted — gap tolerated (RC-5(b)/(c), I3);
9. RC-4: a hash-valid commit that does not reproduce the outstanding
   result → `CancelRound("commit_mismatch")`; a reproducing commit
   advances; duplicates are remembered no-ops;
10. budget: token/seconds exhaustion → `overloaded` (I8);
11. deadline: past-at-admission and past-at-dispatch (queue wait) →
    `deadline_exceeded`; `deadline_ms` clamping boundaries;
12. reject frames contain no token ids (serialize and inspect);
13. lease expiry mid-session → `expired_token`.

File `crates/modelswarm-session/tests/verifydrafts_divergence_properties.rs`
(§7 row 3): for random windows/mock outcomes: on rejection
`divergence_point == accepted_prefix_len` and
`accepted_prefix_len < len(window)`; on acceptance
`divergence_point == null` and `accepted_prefix_len == len`;
`correction` present except the pinned EOG case; committed set always
`window[..accepted] ++ [correction]`; boundary sweep len ∈ {1, 31, 32,
33} × `batch_window_max` ∈ {1, 8, 32}.

## 4. Step 4 — coordinator side

### 4.1 Result admission (ADR-032 §2 RC-3)

`pub fn admit_result(...) -> ResultAdmission` implementing (a)–(e)
EXACTLY: (a) signature under the session's pinned verifier key AND
`sender` derives from it AND equals the QUIC-authenticated remote
PeerId; (b) `nonce_echo` EQUALS the outstanding request's nonce; (c)
`session_id`, `round`, `seq`, `profile_id`, `parent_prefix_hash` equal
the outstanding request's; (d) `accepted_prefix_len ≤
len(window_tokens)` and on rejection `divergence_point ==
accepted_prefix_len`; (e) `new_prefix_hash` equals the locally
recomputed `compute_prefix_hash` over `(parent ‖ accepted ‖
[correction])`. First valid result consumes the request's nonce; any
later result with a consumed nonce, and any result with no outstanding
request, is DROPPED with no state change (replayed genuine results are
thereby harmless). A SECOND protocol violation by the same peer within
one session tears the session down with an honest reason (threshold
exactly two — §2).

### 4.2 Timing trust: per-verifier EWMA + cross-check (RC-2)

```rust
pub struct VerifierTiming { /* per-verifier, keyed by peer id */ }
impl VerifierTiming {
    pub fn record_round(&mut self, measured_wall_ms: u64);       // requester-measured
    pub fn ewma_ms(&self) -> Option<u64>;
    pub fn cross_check(&mut self, verify_pass_ms: u32, observed_wall_ms: u64)
        -> CrossCheckOutcome;  // Ok | Lie
}
```

The EWMA is fed by the round latency the REQUESTER measures (send
`VerifyDrafts` → receive `VerifyDraftsResult`); `verify_pass_ms` is a
lower-bound sanity term only. Cross-check: `verify_pass_ms ≤ observed
wall + 25 ms` else the claim is recorded as `verify_ms_lie` telemetry,
the verifier is de-ranked, and the engage gate discounts it as
unverified (§1). The EWMA is strictly per-verifier — a cohort-wide
blend is FORBIDDEN (§1). EWMA alpha is not pinned by the ADR: default
`0.25`, configurable, recorded in Appendix A — flag F-4.

### 4.3 Telemetry + audit interfaces (RC-1; node/desktop implement later)

The session crate takes NO `modelswarm-telemetry` dependency (it is a
dev-dependency today; keep the graph unchanged). Define local traits
the node/desktop layers implement:

```rust
pub trait VerifyTelemetry: Send + Sync {
    fn verify_ms_lie(&self, session_id: &str, peer: &str);          // RC-2
    fn verifier_divergence(&self, session_id: &str, peer: &str);    // RC-1
    fn budget_refused(&self, session_id: &str, code: &str);         // I8
}

#[async_trait::async_trait]
pub trait WindowAuditor: Send + Sync {
    /// Re-executes one committed window with the coordinator's own
    /// engine for the same profile and compares token-for-token (RC-1).
    async fn audit_window(&self, prompt_tokens: &[u32],
                          committed_before: &[u32], window_commit: &[u32])
        -> Result<AuditVerdict, RuntimeError>;
}
pub enum AuditVerdict { Match, Divergence { at_index: usize } }
```

Audit-path sampling policy (RC-1, normative): at minimum the FIRST
committed window of the session and one window after every
acceptance/divergence transition. On `Divergence`: emit
`verifier_divergence`, switch the session to the `SingleDecode`
fallback for the remainder, and mark the receipt outcome. Audit is
sound only within the E0-pinned backend pairing of the session (§3).
Fallback primitive: the EXISTING `SpecMessage::SingleDecode` (exact by
construction); ADR-007 retry semantics are unchanged and outermost —
"first output token" means the first COMMITTED token surfaced by the
gateway (§2).

Receipt honesty (RC-1): counters granted from this family are
verifier-claimed numbers inside counter-signed receipts — ADR-030
grant rules govern WHO increments; they do not measure the values.
This MUST be documented at the receipt intake (an honest-label
statement in the msp-v1 §5 `verified_capacity` sense). No new wire
fields (§6 non-goals).

### 4.4 Step-4 tests (venue: per-push, default features — §7 row 4)

`crates/modelswarm-session/tests/verifydrafts_coordinator_properties.rs`:

1. I2 hostile fake verifier: a mock verifier returning
   false-but-hash-consistent results is caught by the `WindowAuditor`
   mock → `verifier_divergence` recorded, session switched to
   SingleDecode, receipt marked;
2. result admission (a)–(e) negative cases (wrong nonce echo, wrong
   round/seq/profile/parent, over-long accepted len, bad recomputed
   hash, wrong key/peer) → dropped, no state change; consumed-nonce
   replays dropped; second violation → teardown;
3. `SessionAccept` honesty: `batch_verify: true` without in-range
   `batch_window_max` is schema-invalid; `batch_verify: false` peers
   are first-class singles, never selected as batch verifiers; backend
   mismatch → `SessionReject("backend_mismatch")`;
4. `verify_pass_ms` cross-check: claimed value above measured + 25 ms →
   `verify_ms_lie` recorded (via a mock `VerifyTelemetry`), verifier
   de-ranked; per-verifier isolation: one lying peer never de-ranks
   another (feed two trackers, assert).

## 5. Step 5 — sequencing, ownership, venues, acceptance

### 5.1 Staging under the read-only architect seat

This plan is the architect's output. Implementation of session-crate
code happens under the architect's path rules
(`crates/modelswarm-session`, `protocol/`); since this seat is
read-only, the Integrator should stage it as PASTE-READY-FIRST
IMPLEMENTATION SESSIONS: one implementation session per step below,
each receiving the corresponding plan section as the binding contract
(byte rules are fully specified above — implementers guess nothing),
each ending with the AGENTS.md handoff file
`docs/reviews/handoff-<agent>-<date>.md` and reviewed by Security +
Test and Release (the ownership-map reviewers for Protocol Architect
outputs). Sequential merges, one branch at a time. Any deviation an
implementer needs is escalated back to the architect seat and recorded
in the ADR log — never improvised.

### 5.2 Sequence and owners

| # | Work | Owner (roster) | Venue | Depends on |
|---|---|---|---|---|
| 0 | Option-A wire-true verification term in the acting planner — LANDED (`crates/modelswarm-scheduler/src/shadow.rs:384-394`, `batch_verify: false` mapping). Remaining: confirm decision-logic test coverage exists (HTTP-never-engage, fastest-single-wins) and add if missing; Scheduler + Protocol sign-off recorded | Scheduler Scientist | per-push, `modelswarm-scheduler` tests | — |
| 1 | §1 types + serialization (`verifydrafts.rs`, `negotiation.rs`, `modelswarm-types` dep) + unit tests | implementation session under Protocol Architect contract (Integrator-staged) | per-push, `rust.yml` `check` | this plan |
| 2 | §2 generator example + all committed fixtures + `verifydrafts_vectors.rs` golden test | same | per-push | 1 |
| 3 | §3 verifier gate + admission/divergence property suites | same | per-push | 1, 2 |
| 4 | §4 coordinator admission + EWMA + telemetry/audit traits + coordinator property suite | same | per-push | 3 |
| 5 | Mixed-swarm fallback scenarios (§7 item 4b) | Test and Release Engineer (`apps/modelswarm-sim`) | per-push, sim | 4a + §1 negotiation fields |
| 6 | Engine-side: capi `verify_drafts` wiring behind the ADR-031 gate; exactness pins incl. the I4 one-pass decode-count assertion (§7 item 2); `docs/verification/` entries; pin reruns on every `runtime-pins.json` bump / gate-item change / new backend | Runtime Engineer + Test and Release | manual dispatch `real-model-e2e.yml` EXTENDED WITH A NEW CAPI STEP (or owner-local run + record); never green in per-push CI | ADR-031 §5 items 1–5 passed |

Standing honesty statements (so nobody blocks a release on a job that
does not exist — §7): item 2 and the adapter-cohort half of item 4b
will NEVER go green in per-push CI; the capi-feature clippy/test job
already runs on every push (`rust.yml` "Clippy (capi-adapter feature)"
/ "Tests (capi-adapter feature)" steps). When the family carries real
traffic, the `MSP_LIVE=1` live-wire harness obligation applies
(`crates/modelswarm-node/tests/live_tracker.rs` precedent).

### 5.3 Acceptance checklist (family declared implemented = wire contract tier)

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
   -- -D warnings`, `cargo test --workspace` green (default features)
   — including the capi-feature CI steps still green.
2. Golden-vector tests (§2.3) green per-push; generator rerun is
   byte-idempotent (`git diff` clean).
3. Admission property suite green incl. the first-failure-wins
   conjunction sweep (§3.6 items 1–13).
4. Divergence/boundary property suite green (§3.6 final paragraph).
5. Coordinator property suite green (I2 hostile verifier, SessionAccept
   honesty, `verify_pass_ms` cross-check, per-verifier EWMA isolation).
6. Scheduler 4a tests green with Scheduler + Protocol sign-off
   recorded in the ADR log.
7. Reject-frame privacy assertion green (no token ids anywhere in
   rejections/logs).
8. Handoff docs per step; Security + T/R review sign-offs recorded.

Light-up tier (real traffic; separately gated, NOT part of the wire
tier): ADR-031 §5 items 1–5 + `docs/verification/` entry; item-2
exactness pins incl. I4 one-pass decode count; sim 4b scenarios; then
the live-wire harness extension.

## 6. Risks / open questions (follow-up amendments, not silent interpretation)

- **F-1 (amendment needed) — `generation_params_hash` numeric
  rendering.** ADR-032 §1 rule 3 pins the preimage object with
  `<num>` for `temperature`/`top_p`, but msp-v1 §2.2 permits
  shortest-round-trip fractions while the shipped canonicalizer
  (`crates/modelswarm-types/src/canonical.rs`) REJECTS floats
  (`FloatRejected`), and `SamplingParams::default()` is
  `temperature 1.0 / top_p 1.0`. This plan pins integral values
  rendered as bare integers and fail-closed `NonIntegral` otherwise,
  and the fixture byte-pins the exact preimage — but a follow-up
  amendment should pin the rendering rule (and the `seed` width: the
  ADR writes `<u32|null>`, the runtime type is `Option<u64>`; this
  plan renders the full integer).
- **F-2 (amendment needed) — `correction` at EOG.** §1 says
  "correction is always present for a non-empty window", but the
  engine's `VerifyOutcome.bonus` is `Option` (`None` only at
  generation end — `crates/modelswarm-runtime/src/capi/mod.rs:105-108`).
  This plan pins: the FIELD is always present; its VALUE is null
  exactly at EOG (mirroring the `divergence_point` present-as-null
  discipline), the commit covers the accepted prefix only, and
  `verifydrafts-4-eos.json` byte-pins it. Revision 3 should confirm.
- **F-3 (confirm at kickoff) — "runnable" scope.** ADR-032's status
  line vs the §7 venue matrix: this plan reads the per-push rows as
  gated only on acceptance (wire contract + mock tests now) and the
  ADR-031 gate as binding the real-engine path only. Confirm with the
  owner at implementation kickoff.
- **F-4 (pin later) — EWMA alpha and de-rank policy details are not
  ADR-pinned.** Default 0.25 here, configurable; the scheduler-facing
  discount semantics need a pinned value before light-up (scheduler
  handoff).
- **F-5 (design note) — chain-state fork.** The family cannot route
  commits through `SessionStateMachine::apply` (gap-intolerant by
  design, RC-5(c)); the gate keeps its own chain state. Unifying both
  belongs to the future SpecMessage-freeze ADR (ADR-032 §2 explicitly
  defers the old vocabulary's version/nonce/deadline binding there).
- **F-6 (minor) — `verifier_backend` optionality.** §3 calls the
  SessionOffer field "additive" yet requires the golden vectors to be
  extended (new canonical form). This plan makes it required in the
  new canonical form (no deployed fleet to be compatible with);
  revision 3 may want to say "required" explicitly.
- **F-7 (risk) — fixture keys are fixture-only.** Generator seeds are
  recorded in-file and MUST never collide with any production identity
  (they cannot: installation ids derive from the recorded public keys).
- **F-8 (risk) — cross-machine CPU determinism remains E0's open
  sub-question and gates any real-draft pass 3 and cross-machine audit
  (§3).** Nothing in the wire tier depends on it; everything in the
  light-up tier does.
- **F-9 (schedule risk) — item 2 depends on ADR-031 gate items 1–5 and
  manual dispatch; token exactness is build-sensitive (pin reruns owed
  on every engine bump).** The honest-negative publication rule
  (ADR-031 §5.5) applies if exactness ever breaks.

## Appendix A — constants (single source: `verifydrafts.rs`)

| Constant | Value | ADR-032 |
|---|---|---|
| `WINDOW_CAP` | 32 | §1 |
| `DEADLINE_CLAMP_MIN_MS` / `MAX_MS` | 1000 / 120000 | §2, msp-v1 §6.4 |
| `WAIT_COMMIT_GRACE_MS` | 250 | §8 RC-5(b) |
| `VERIFY_PASS_CLOCK_NOISE_MS` | 25 | §1 cross-check |
| `TEARDOWN_VIOLATION_THRESHOLD` | 2 | §2 RC-3 tail |
| EWMA alpha (default, config) | 0.25 | not ADR-pinned (F-4) |
| Genesis prefix hash | `e3b0c442…b855` (spec.rs:117) | §1 rule 2 |

## Appendix B — invariant → test traceability (ADR-032 §7/§8)

| Invariant | Enforced by | Test venue |
|---|---|---|
| I1 committed = engine's own continuation | engine-side | item 2, manual (§5.2 row 6) |
| I2 every committed token passed a result | RC-4 (verifier), RC-3 + RC-1 audit (coordinator) | coordinator property suite (per-push) |
| I3 chain gap-free, rounds may gap | gate chain state + RC-5(c) | admission property suite |
| I4 one engine pass per VerifyDrafts | `batch_verify`/`batch_window_max` honesty (wire); decode count (engine) | coordinator property suite; item 2 manual |
| I5 never-worse engage, engine-true term | scheduler term (4a) | scheduler tests |
| I6 single-use nonces, per-sender monotone seq | §3.3 stages 6 | admission property suite |
| I7 cross-profile isolation | profile + parent binding | admission property suite |
| I8 verifier budget enforcement | §3.5 | admission property suite |
