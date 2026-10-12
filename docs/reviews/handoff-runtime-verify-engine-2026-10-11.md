# Handoff — Runtime Engineer, production BatchVerifyEngine (2026-10-11 task, run 2026-10-12)

Scope: ADR-032 §1/§4's REAL engine implementation behind the EXISTING
off-by-default `modelswarm-runtime/capi-adapter` feature —
`CapiVerifyEngine`, the session-scoped batched-verify engine over the
productionized C-API adapter (ADR-031 gate PASSED, commits `48536d2`/
`30258fe`), asserted through the architect's
`modelswarm_session::verifydrafts::BatchVerifyEngine` trait. Nothing
production-visible changes; the flag stays OFF until the family lights up.
Verification record with every number:
`docs/verification/capi-verify-engine-2026-10-11.md`.

## 1. Changed files (paths) and why

- `crates/modelswarm-runtime/src/capi/verify.rs` (NEW) — the engine:
  `CapiVerifyEngine` (one adapter + one handle + the verifier's own
  committed continuation under a tokio Mutex — the wire never re-sends the
  prefix, so the engine tracks it), `verify_window` = the ADR-032 §1
  semantics over `LlamaCppCapi::verify_drafts` (ONE batched pass; first
  rejection → KV rollback at that position + replacement as correction;
  full acceptance → bonus token; `correction == None` exactly at EOG with
  a prefix-only commit and a fail-closed post-EOG refusal), `prefill`
  (PREFILLED root), `capabilities()` from the adapter's REAL geometry
  (`batch_window_max` = the construction-pinned verify window the
  load-time warmup sizes; `batch_verify: true`; backend from the
  configured bundle, `VerifyBackend::of_bundle` detection), and the
  pre-engine-work refusals (empty window malformed; window >
  `min(WINDOW_CAP, batch_window_max)` → `413 payload_too_large`; honest
  propagation of the adapter's `429 kv_budget_exceeded`, committed prefix
  untouched on any error). Also `BatchVerifyEngineMirror` — the clearly
  labeled MINIMAL LOCAL MIRROR of the architect's trait (see §4 gap), and
  4 unit tests (backend labels/detection, WINDOW_CAP pin vs the ADR).
- `crates/modelswarm-runtime/src/capi/mod.rs` — `pub mod verify;` +
  re-exports (`CapiVerifyEngine`, `VerifyBackend`, `VerifyEngineCaps`,
  `BatchVerifyEngineMirror`). Additive only.
- `crates/modelswarm-runtime/src/lib.rs` — one name added to the existing
  feature-gated re-export list.
- `crates/modelswarm-runtime/tests/capi_verify.rs` (NEW) — the REAL-trait
  test: implements `modelswarm_session::BatchVerifyEngine` for
  `CapiVerifyEngineAsTrait(Arc<CapiVerifyEngine>)` (the dev-dependency
  cycle is the only legal way to bind the real trait to a runtime type —
  §4), a non-ignored `VerifyEngineCaps → EngineDisclosure` mapping pin
  (runs in CI's capi job), and the env-gated `#[ignore]`d real-engine test
  (full-window accept + bonus, mid-window reject + continuation parity vs
  HTTP ground truth, BOTH EOG sub-cases (bonus-EOG and replacement-EOG)
  with prefix-only commit + post-EOG refusal, over-window-cap refusal with
  zero engine work, honest `kv_budget_exceeded`, and the I4
  one-engine-decode-count assertion — all through the trait call).
- `crates/modelswarm-runtime/Cargo.toml` — ONE dev-dependency added:
  `modelswarm-session` (the legal-cycle compile proof). No feature or
  dependency otherwise changed.
- `Cargo.lock` — the dev-dep's lock entry (and nothing else; see §7).

NOT changed: `modelswarm-session` (architect-owned; instruction 2 forbade
it and nothing required it — the trait binds from the test target), the
adapter's engine paths (`ffi.rs`, `verify_drafts`), `runtime-pins.json`,
`scripts/`, any workflow, any other crate.

## 2. Exact commands run and outcomes (owner-local, Windows 11)

Gates:

- `cargo fmt --all --check` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- `cargo clippy --workspace --all-targets --features
  modelswarm-runtime/capi-adapter -- -D warnings` — PASS.
- `cargo test --workspace` — **459 passed / 0 failed / 6 ignored** (the
  stated baseline, exact match).
- `cargo test --workspace --features modelswarm-runtime/capi-adapter` —
  **474 passed / 0 failed / 10 ignored** (pre-change workspace would be
  468/0/9; crate-level pre/post with the change stashed: runtime+capi
  32/0/3 → 38/0/4, delta = exactly +6 passed/+1 ignored from this
  change: 4 unit + 1 doctest + 1 mapping test, +1 env-gated ignored).

Real-engine (env-gated `#[ignore]`d; staged pinned CPU bundle +
engine-vulkan + SmolLM2-135M-Instruct Q4_K_M, sha256-verified at test
start; exact env vars in the verification doc):

- `cargo test -p modelswarm-runtime --features capi-adapter --test
  capi_verify -- --ignored --nocapture` — **1/1 PASS, twice** (pre- and
  post-format, identical): every pin in §1 of the verification doc.
- `cargo test -p modelswarm-runtime --features capi-adapter --test
  capi_loopback -- --ignored --nocapture` — **1/1 PASS** (no regression).
- `cargo test -p modelswarm-runtime --features capi-adapter --test
  capi_vulkan -- --ignored --nocapture` — **1/1 PASS** (engine-vulkan
  bundle, within-backend exactness).
- `capi_batching` not re-run (measurement harness; no measured path
  changed). No `llama-server` left running after any run.

## 3. Test evidence

See §2 and `docs/verification/capi-verify-engine-2026-10-11.md` (the full
pin table with observed values, the 36-token EOS-prompt negative, and the
ground-truth construction rule for EOG).

## 4. The trait-gap finding (the requested report item)

`modelswarm_session::verifydrafts::BatchVerifyEngine` is NOT implementable
for a type in the `modelswarm-runtime` LIB as today's graph stands:
`modelswarm-session` already depends on `modelswarm-runtime` (it imports
`RuntimeError`/`SamplingParams` from it), and implementing the trait for a
runtime type requires naming it — a reverse package dependency, which
Cargo rejects as cyclic. This is a graph fact, not a trait-shape problem:
the trait itself binds perfectly once both are visible (proven by the
newtype impl in `tests/capi_verify.rs`, compiled and executed). Resolution
options for the integrator/architect: (a) the serve loop's crate
(`modelswarm-node`, which already depends on both) hosts the impl —
`tests/capi_verify.rs`'s `CapiVerifyEngineAsTrait` + `to_disclosure` are
the drop-in pattern; (b) an Architect ADR re-homing the trait (a leaf
crate, or `modelswarm-runtime` itself). Until either lands, the lib ships
the labeled mirror `BatchVerifyEngineMirror` (identical method shapes;
`VerifyEngineCaps` mirrors `EngineDisclosure` field-for-field). Related
small gap: `EngineDisclosure`/`Backend` are session-crate types the
runtime cannot name — hence the mapping function in the test, which is
also the serve-loop reference.

## 5. Assumptions

- The engine's committed-prefix discipline: after a verify, committed =
  `old ++ accepted ++ [correction]` (no correction token at EOG), and the
  next window continues from there — this is exactly the `PrefixCommit`
  preimage `compute_prefix_hash` covers, and it keeps every warm verify at
  ONE engine decode call (KV delta = the not-yet-decoded correction
  token, re-decoded inside the next batch). Pinned by the I4 and
  continuation-parity assertions.
- The `finished` (post-EOG) refusal is my fail-closed addition: ADR-032
  says correction-None is the EOG outcome; a serve loop that keeps
  verifying after it is buggy, and the engine now says so instead of
  sampling past an EOG. `prefill` clears it (fresh session root).
- `VerifyBackend` is supplied by configuration (the caller knows which
  bundle it staged); `of_bundle` detects it from bundle contents
  (ggml-vulkan.dll presence; `None` on a non-bundle directory — fail
  closed, unit-tested).
- KV-cap semantics inherited unchanged from the adapter: enforcement
  happens at sync boundaries (documented as "a sync/verify target longer
  than the cap fails"); one verify's batch may transiently exceed a
  deliberately-lowered cap by up to window+1 tokens before the NEXT round
  refuses (the context-window bound is enforced separately). Pre-existing
  behavior, now exercised through the trait; flagged as an observation,
  not changed.
- The EOS-prompt choice is pinned by the test; a different model/prompt
  pair must re-satisfy the asserted 1..=31 pre-EOG band (the failure
  message says exactly that).

## 6. Unresolved risks

- The dev-dep on `modelswarm-session` slightly widens runtime's test
  build graph (build-time only; no lib-level cycle; CI green in both
  feature configs). It should be REMOVED if option 4(b) (trait re-home)
  lands and the lib can host the impl directly.
- Cross-machine CPU determinism remains E0's open sub-question; nothing
  here changes it. Exactness pins are build-sensitive and re-run on every
  engine bump (ADR-032 §7 standing rule).
- The serve loop must still supply PeerId/authentication context, signing,
  WAIT_COMMIT polling, and budget-ledger deltas (ADR-032 §7 RC-2
  checklist) — this engine covers only the engine-side contract.

## 7. Concurrent-workspace note (for the integrator)

Another seat's serve-loop WIP is present in this tree
(`crates/modelswarm-node/src/verifydrafts/` untracked +
node `Cargo.toml`/`lib.rs` modifications). It was NOT touched, and it is
NOT part of my commit: I staged only the six files in §1. `Cargo.lock`
initially carried BOTH seats' entries; I stashed their files, regenerated
the lock so it contains ONLY my dev-dep entry, committed, then restored
their tree and rebuilt the lock locally so their WIP keeps compiling.
Verify at merge time that the node seat's commit carries their own lock
entries.

## 8. Suggested next task for the integrator

The serve-loop seat should land its wiring against the REAL engine: host
`CapiVerifyEngineAsTrait`-style impls in `modelswarm-node` (the §4
pattern), feed the gate's `dispatch_outstanding` with it, and decide the
4(a)-vs-4(b) trait re-home with the Protocol Architect. Runtime's
readiness statement: the engine is exact through the architect's trait on
the pinned CPU bundle (and the adapter on Vulkan), one-pass (I4),
KV-bounded, and fail-closed on caps/EOG/post-EOG misuse — the
`capi-adapter` flag itself stays OFF until the family's light-up gates
(item-2 exactness-pin reruns included) pass.
