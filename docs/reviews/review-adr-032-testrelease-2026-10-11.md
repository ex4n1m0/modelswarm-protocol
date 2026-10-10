# Review: ADR-032 (batched speculative verification) — Test and Release seat — 2026-10-11

Reviewer: Test and Release Engineer (release gate seat). Reviewed object:
`docs/adr/ADR-032-batched-speculative-verification.md` (Proposed, drafted at
`35ee671`) — **testability and gate plan only** (§7 items 1–5, §8 invariants
I1–I7, §2 admission order, evidence citations, CI consequences). Protocol
semantics, option choice, and the §4 cost-model correction are other seats'
calls; I flag them only where they block testability. Adjacent to
`docs/reviews/review-guard-calibration-2026-10-10.md` (same evidence chain,
same artifacts). **No code was changed by this review** — docs only.

## Verdict: APPROVE WITH REQUIRED CHANGES

The decision direction is testable and the §7 list is the right shape, but
the ADR cannot be **accepted** with four blockers unresolved: one headline
evidence number does not trace to any committed artifact, the §2
rejection-code mapping names a code that does not exist in the effective
§6.5 registry, the item-1 golden-vector set is not enumerated (so "before
any second implementation reads the namespace" has no artifact to point
at), and the gate tests have no stated execution venue (which jobs, which
features, which workflow — the exact class of mismatch that broke
`cargo test --workspace` at `a8b630f` yesterday). All four are fixable in
the ADR text before owner acceptance; none require re-opening the design.

Findings register: **4 blockers, 3 required changes, 4 advisories**
(§6). Evidence spot-checks: 5/6 trace (§1).

## 1. Evidence cross-check (citations vs committed artifacts)

Requested spot-checks (3) plus two independent arithmetic checks I ran
while reading the Context section:

| ADR claim | Committed source | Result |
|---|---|---|
| guard would have aborted **1,857/1,982** LAN engaged completions at round 1 (and 911/946 loopback) | `experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/ANALYSIS.md` finding 4 ("The production rule-6 guard would have aborted 1,857/1,982"); loopback 911/946 at `…LOOPBACK…/ANALYSIS.md` (finding on round-1 aborts); independently re-derived in my 2026-10-10 review §1 | **TRACES** |
| worst engaged loss **4.257×** | same LAN ANALYSIS, aggregate section: "best engaged median ratio **1.071** (inj0ms-high-w16…), worst **4.257** (inj0ms-geo700-w16)" | **TRACES with labeling defect** — 4.257 is the worst engaged-arm **cell median** vs the prompt-best single, not a per-row worst; the ADR's "Engaged cells lost 1.38–1.94× … and worst 4.257×" invites a per-row reading (advisory A1) |
| P16: k=8 window verified in one 9-row batched decode, **33–36 ms**, **1.45×** 2-round loopback, token-exact vs HTTP | `docs/reviews/handoff-runtime-engineer-2026-10-09.md` §P16 measured table: "verify alone **33–36 ms (one 9-row batched decode)**", "**181.9 ms (1.45×)**", exactness asserted (tokenize parity, 32-tok greedy stream identity, verify outcomes == HTTP ground truth, reject-path rollback exact) | **TRACES with caveat** — single-run, single-machine loopback under dev-session load, per ADR-031 §Measured's own environment paragraph; fine as motivation, must not migrate into any claim doc without the repeat/P50 protocol ADR-031 §5 assigns to pass 2 (advisory A2) |
| **6,720 join rows total**, zero failures | LAN 4,800 (`…LAN…/ANALYSIS.md` aggregate) + loopback 1,920 (loopback ANALYSIS; both re-derived 2026-10-10) | **TRACES** (4,800 + 1,920 = 6,720) |
| **"2,460 engaged speculative completions (946 loopback+injected-delay; 1,982 … LAN)"** — Context ¶1 | **NOTHING.** `grep -rn "2,460\|2460"` over `docs/`, `experiments/`, `protocol/` matches **only ADR-032 itself**; and the ADR's own parenthetical sums to 946 + 1,982 = **2,928**, not 2,460 | **DOES NOT TRACE — blocker B1** |

Also verified adjacent citations: `crates/modelswarm-session/src/spec.rs`
`SpecMessage` vocabulary at the cited lines (206–339) exists as described;
`protocol/msp-cooperative-v1.md` §1 carries the Proposed-status
cross-reference paragraph and its §5 forbidden list (msp-v1 and
messages.proto byte-identical) is respected by the ADR's additive design;
`protocol/vectors/lease-hubkey-1.json` exists and is byte-pinned from both
`crates/modelswarm-eligibility/src/lease.rs` (golden test, `verify_at`
anchor) and `apps/tracker/tests/crypto-vectors.test.ts` — the precedent
item 1 must follow.

## 2. §7 items 1–5 — completeness and executability

### Item 1 (cross-language golden vectors) — RIGHT IDEA, UNDERSPECIFIED (blocker B3)

"Golden vectors for the family … before any second implementation reads the
namespace" is the correct ordering, but as written there is no committed
fixture set to be first. Acceptance must commit to the enumerated set
(modeled on `lease-hubkey-1.json`: fixture-only signing keys recorded in the
file — never production; `verify_at` time anchor so deadline/expiry checks
are deterministic; wire form + `canonical_json_sha256` byte-pin; expected
outcome with the **exact** effective-registry code). Proposed set:

| Fixture | Pins |
|---|---|
| `verifydrafts-1-accept.json` | Full acceptance, w=8: `VerifyDrafts` canonical-JSON bytes + sha256; signature preimage = signature-absent canonical form; `VerifyDraftsResult` canonical bytes + signature + sha256; `accepted_prefix_len == len(window_tokens)`, `divergence_point: null`, `correction` = bonus token; `new_prefix_hash` value **and the byte-exact chaining construction** (see below); `nonce_echo` |
| `verifydrafts-2-reject.json` | Mid-window rejection: `accepted_prefix_len = k < len`, `divergence_point == k`, `correction` = replacement token, chain hash recomputed |
| `verifydrafts-3-boundary.json` | `WINDOW_CAP` edge: len 32 accepted (cap-legal); len 33 → `payload_too_large` **before any engine work** (pairs with the §7.1 DoS shape) |
| `verifydrafts-4-eos.json` | EOS surfacing in the accepted path (item 2 asserts EOS; a wire fixture for the shape is cheap and prevents second-implementation drift on eos serialization) |
| `verifydrafts-malformed.json` | Array: empty `window_tokens`; bad signature; wrong `profile_id`; duplicate nonce; duplicate seq; non-monotone seq (code per B2/R1); stale/deadline-past (with `verify_at`); unknown session (code per B2). Each entry carries `expected_reject` |

Two things only the fixture process will force, and the ADR should decide
them **before** acceptance rather than in a test-review later:

- **The chaining construction is not byte-specified.** "`new_prefix_hash`:
  sha256 of `parent ‖ accepted ‖ [correction]`" leaves open the encoding of
  token ids (u32 LE? varint? ASCII decimal?), whether `parent` is the raw
  32 bytes or the `sha256:`-prefixed string, and the separator. Two
  implementations will drift here with high probability — this is exactly
  the class the lease vector's `canonical_json_sha256` exists to prevent.
  The fixtures must pin one construction and the ADR must state it.
- **"Cross-language" needs a named consumer set.** Today the family's only
  implementation language is Rust (`modelswarm-session`), and `apps/
  modelswarm-sim` is also Rust — there is no TS surface for this family
  (unlike the lease, where the TS tracker issues tokens). The commitment
  should read: fixtures are implementation-neutral; the Rust golden test
  lands with the family; any future implementation in any language must
  reproduce identical canonical bytes, signatures, and outcomes from the
  same files. If the owner wants a live second consumer, the sim (T/R
  owned) is the natural one — say so.

### Item 2 (exactness pins) — correct tests, NO runnable venue stated (blocker B4)

"Batched-verify committed stream == single-mode decode … (the P16 pin set,
re-run on the productionized adapter)" — as written this cannot be a
standing gate, because:

- The P16 pins live in `crates/modelswarm-runtime/tests/capi_loopback.rs`,
  which is `#![cfg(feature = "capi-adapter")]` **and** `#[ignore =
  "set MSP_LLAMA_SERVER and MSP_REAL_GGUF"]` — they need a staged pinned
  engine + real GGUF. GitHub-hosted check-job runners never stage engines
  (see the TAURI_CONFIG note in `rust.yml`).
- The manual-dispatch `real-model-e2e.yml` stages a model + engine but
  currently runs only `modelswarm-types real_artifact` and
  `modelswarm-node real_engine` — **it has no capi step**. Nobody re-runs
  the P16 pins today except the owner, locally.
- The pins are only meaningful **after** ADR-031 productionization gate
  items 1–4 pass (shape-stable batching, `spawn_blocking` migration,
  per-handle KV accounting, Vulkan in-process): items 1–3 change the very
  call shapes and threading the pins assert through. Gate item 5 (capi
  clippy/test in CI) is **already satisfied** — `rust.yml` has run the
  capi-adapter feature clippy+test steps on every push since the
  2026-10-08-lease-break review ruling (workflow comment cites it).

Required statement in the ADR (the CI ordering dependency, explicit):

1. **Per-push, default features** (`rust.yml` check): items 1, 3, 5 — pure
   wire/data-structure tests, no engine. These must live in default-feature
   test targets so `cargo test --workspace` (the exact job `a8b630f`
   repaired yesterday) runs them; they depend on **nothing** from ADR-031.
2. **Per-push, capi-adapter feature** (existing steps): compile coverage of
   adapter-integrated family code plus any engine-free adapter unit tests.
3. **Option A first**: item 4's HTTP-never-engage half turns green in
   default-feature scheduler tests as soon as the wire-true term lands
   (it is a pure decision-logic change); the ADR already sequences A before
   B light-up — the gate list should say item 4 splits on that boundary.
4. **ADR-031 gate items 1–4 pass + `docs/verification/` entry** → then item
   2's pins are authored/re-run against the productionized adapter, in
   `real-model-e2e.yml` extended with a capi step (manual dispatch —
   consistent with the standing manual-dispatch-only rule) or owner-local
   with a `docs/verification/` record. **Item 2 and the adapter-cohort
   half of item 4 will never go green in per-push CI; the ADR must say so**
   so nobody blocks a release waiting on a job that does not exist.

### Item 3 (replay/reorder/stale/cross-profile/cap/malformed/admission order)

Executable once B2 (codes) and R1 (order ambiguities) are fixed; otherwise
complete. This is the item that catches the three historical wire bugs the
`MSP_LIVE=1` harness exists for (envelope casing, runtime shape, peerId) —
when the family reaches a live wire, the harness extension obligation
applies too (`crates/modelswarm-node/tests/live_tracker.rs` precedent:
extend on any wire-surface change).

### Item 4 (engage-gate with engine-true term)

Testable in `modelswarm-scheduler` **default features** (pure planning
logic over declared capabilities) for the HTTP-never-engage and
fastest-single-wins cases; the mixed-swarm fallback case needs a cohort
fixture harness — name the home (the sim is T/R-owned and default-feature;
the bench quic-runner path is owner-gated and must NOT hold logic tests,
see `a8b630f`).

### Item 5 (divergence-point consistency property)

Executable as a default-feature property test over synthesized results +
fixtures 1/2. Complete as written; fold the boundary assertions
(`accepted_prefix_len ≤ len(window)`, cap edge) into the same property.

## 3. §8 invariants I1–I7 — coverage matrix

| Inv | Wire-testable (items 1/3/5, default CI) | Engine-side only | Test path in ADR |
|---|---|---|---|
| I1 committed stream == engine's own continuation | — | yes (real engine, pinned params) | item 2 (venue per B4; manual dispatch) |
| I2 every committed token passed a `VerifyDraftsResult` or `SingleDecodeResult` | yes — coordinator state-machine property vs a hostile fake verifier/proposer, no engine | — | **NONE — missing (R2)** |
| I3 chain gap-free, reorder-rejecting | yes | — | items 3 + 5 |
| I4 exactly one engine pass per `VerifyDrafts` | disclosure half ("cannot honor → must not declare `batch_verify`") is wire-testable via SessionAccept honesty; the one-pass half needs a decode-count assertion (instrumented adapter or a counter inside the item-2 pin) | one-pass half: yes | **partial — disclosure half unassigned; one-pass half has no stated venue (R2)** |
| I5 never-worse engage rule, margin ≥ 0.05, mandatory fallback | yes (planner logic) | — | item 4 |
| I6 session-scoped single-use nonces, monotone seq | yes | — | item 3 |
| I7 cross-profile isolation | yes | — | item 3 |

So: I3/I5/I6/I7 are covered as wire tests; I1 is covered but engine-side
(and only meaningful post-ADR-031-gate); **I2 has no test path at all in
the ADR's list, and I4 is half-covered** — both are cheap default-feature
tests (I2 is the invariant that actually stops a compromised coordinator
from laundering unverified tokens into a committed stream; it should not
ship untested because it looks engine-ish).

## 4. §2 admission order — property-testable as specified?

Almost. The linear order (frame → schema → signature → session/lease →
seq/nonce → profile → parent-hash → cap → deadline) is deterministic under
single-failure injection, and the ordering itself resolves multi-failure
cases implicitly — but the ADR should state **first-failure-wins** as the
assertion (property tests should inject conjunctions and assert the
earliest code). Three gaps block a clean property suite:

1. **`parent_mismatch` is not a registry code** (blocker B2). The effective
   §6.5 registry (`protocol/msp-v1.md` §6.5 + ADR-028 §5 amendments
   `bad_frame`/`invalid_lease`/`executor_error`) contains nothing expressing
   a chain-position rejection; grep confirms `parent_mismatch` appears
   nowhere in `protocol/`. The ADR's "no new codes are needed" is false as
   written: either name an existing code (none fits cleanly), or the ADR
   must carry the one-line registry amendment through the ADR-028 §5
   mechanism it itself cites. A test cannot assert a string that has not
   been decided.
2. **The seq/nonce → code partition is ambiguous** (R1). "Duplicate
   nonce/seq → `replayed_request`" vs "reordered round →
   `parent_mismatch`-class" overlap: a `seq` strictly lower than last but
   never seen (not a duplicate), same round — which code? A replayed
   message with a *fresh* seq? Round-number vs seq disagreement? The
   property suite will hit all three in hour one; the mapping needs the
   exhaustive table (seq dup / seq decrease / nonce dup / round mismatch /
   round reorder × code).
3. **The session/lease stage has no failure branch** (R1/B2). The order
   lists "session/lease (ADR-026 gate already passed at session open)" but
   assigns no code for: unknown session, session torn down after a
   `profile_mismatch` (the ADR says tear-down happens — what do subsequent
   messages get?), lease expiry mid-session. The malformed-fixture array
   (B3) needs these codes.

Clock injection for the deadline stage: fixtures' `verify_at` anchor is the
established mechanism (lease-hubkey-1) — advisory that the ADR need not
specify it, but the fixture set must carry it.

## 5. Standing CI obligations accepted with this ADR (maintenance cost)

Enumerated so the owner sees what "one more frozen surface to defend"
(translated from the ADR's own consequence) costs per release:

1. **Per-push compile+run (default features, `rust.yml` check job)**:
   golden-vector tests (canonical bytes, `canonical_json_sha256`
   re-derivation, signature preimages), admission-order/replay property
   suite, divergence-point property, SessionAccept honesty tests, I2
   coordinator property — every push, every PR, forever.
2. **Per-push (capi-adapter feature job, already exists)**: compile
   coverage of adapter-integrated family code.
3. **Vector drift discipline**: any change to the family's field set,
   canonical form, or chaining construction requires an ADR **and**
   regenerated fixtures; the sha256 pins make silent drift a hard test
   failure. This is the point of B3 — the set must exist before it can be
   defended.
4. **Manual-dispatch pin reruns (`real-model-e2e.yml`, needs a new capi
   step)**: exactness pins re-run on (a) every `runtime-pins.json` engine
   bump — token-exactness is build-sensitive, a llama.cpp bump can flip
   logits and the pins are the tripwire; (b) every ADR-031 gate-item
   change (batching shape work, `spawn_blocking` migration, Vulkan
   in-process); (c) every new backend. Each rerun owes a
   `docs/verification/` entry, and per ADR-031 §5.5 the honest negative is
   published if exactness ever breaks.
5. **Engage-gate suite in `modelswarm-scheduler`** (default features) once
   option A lands, plus mixed-swarm fallback scenarios in the sim
   (T/R-owned) — per-push.
6. **Live-wire harness extension**: when the family carries real traffic,
   `MSP_LIVE=1`-style wire checks extend to it (the three-bug history is
   the standing reason).

## 6. Findings register

**Blockers (resolve in the ADR text before owner acceptance):**

- **B1 — untraceable headline number.** Context ¶1's "2,460 engaged
  speculative completions" matches nothing committed and contradicts its
  own parenthetical (946 + 1,982 = 2,928). Fix to 2,928 (both ANALYSIS
  files) or drop the aggregate. Evidence-base discipline is a release
  gate; an accepted ADR with an untraceable number in paragraph one fails
  my seat's standard.
- **B2 — rejection-code mapping names a non-existent code.** `parent_mismatch`
  is absent from the effective §6.5 registry; "no new codes are needed" is
  false; the session/lease admission stage has no failure code. Item 3 and
  the §2 property suite are untestable until the exact strings are decided.
- **B3 — item 1 fixture set not enumerated.** Commit at acceptance to the
  named fixture set (§2 table above), the `lease-hubkey-1` format (fixture
  keys, `verify_at`, `canonical_json_sha256`, exact expected codes), the
  byte-exact `new_prefix_hash` chaining construction, and the
  consumer-neutrality rule (Rust golden test now; any language must
  reproduce identical bytes/signatures/outcomes).
- **B4 — no execution venue / CI ordering matrix.** State per item: default
  per-push job (1/3/5 + item-4 HTTP half, gated only on option A), capi
  feature job (compile + engine-free), manual-dispatch real-model-e2e with
  a NEW capi step (item 2 + item-4 adapter half, gated on ADR-031 gate
  items 1–4 + verification entry). Without this, acceptance signs off a
  gate plan whose centerpieces cannot run anywhere CI can see — the exact
  self-reported-success pattern this seat exists to reject, and the
  feature-venue mismatch class that broke workspace tests at `a8b630f`.

**Required changes (before implementation; can be folded into the
acceptance edit):**

- **R1 — §2 partition table for property tests**: seq-duplicate /
  seq-decrease / nonce-duplicate / round-mismatch / round-reorder → code;
  first-failure-wins assertion; session-not-found / torn-down /
  lease-expired-mid-session branches.
- **R2 — missing invariant tests**: I2 coordinator property (hostile fake
  verifier; default features, no engine) and I4's two halves (disclosure
  honesty as a wire test; one-pass as a decode-count assertion inside the
  item-2 pin or an instrumented adapter). Mark I1/I4-one-pass
  engine-side-only in §7 so the venue matrix (B4) covers them.
- **R3 — item 4 test placement**: `modelswarm-scheduler` default features
  for the decision logic; name the mixed-swarm scenario home (sim
  recommended, T/R-owned, default features; not bench quic-runner-only
  modules — `a8b630f`).

**Advisory:**

- **A1** — label 4.257× as "worst engaged-arm cell median" (the ANALYSIS's
  own framing); "worst 4.257×" invites a per-row reading the artifacts do
  not support.
- **A2** — the P16 33–36 ms / 1.45× numbers are single-run loopback under
  dev-session load (ADR-031 §Measured says so itself); fine as motivation,
  repeat/P50 protocol before any claim-bearing reuse.
- **A3** — include the cap boundary (32 ok / 33 `payload_too_large`) and an
  EOS-path fixture in the B3 set — cheap, high drift value.
- **A4** — `verify_ms` pins: assert `≥ 0` only; asserting measured values
  invites flaky gates.

## Gates (this review)

Docs-only session; no production or test code touched.

- `cargo fmt --all --check` — clean (proves no code moved).
- `git status` — only this review file added for commit; the working tree
  otherwise matches `a8b630f` (the tree whose `cargo test --workspace`
  default-features run was verified green at `a8b630f`'s commit: 51
  targets ok). Per the 2026-10-10 gate lesson, no default-feature surface
  was modified, so the workspace test suite is untouched by construction.
