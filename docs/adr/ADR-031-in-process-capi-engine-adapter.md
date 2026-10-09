# ADR-031: In-process llama.cpp C-API engine adapter (research path)

Status: Accepted (2026-10-09) · Amends ADR-019 — this is the ADR its §3
research-adapter slot required ("requires its own ADR before implementation").
Research-path capability under ADR-021's production/research split: the
production HTTP `LlamaCppAdapter` (ADR-019 §2) is unchanged and remains the
only serving engine. Evidence base: the P16 spike (main `7d47847`;
`docs/reviews/handoff-runtime-engineer-2026-10-09.md` §P16 — the sole content
source for every measured claim below) and
`experiments/reports/PASS-1-LOOPBACK-DRYRUN.md` (9.6 pass 1, the empirical
justification).

Owners: Runtime Engineer (adapter, FFI envelope, pins). Reviewers: Protocol
Architect (identity semantics), Security Engineer (loading/FFI surface),
Test and Release Engineer (gates).

## Context

9.6 pass 1 (loopback + injected delay, synthetic executor — NOT LAN, NOT a
product claim) returned its expected negative as a first-class result: no
cooperative arm beat the independently executed fastest single, and the 13
engaged runs realized **1.07×–3.36× the fastest single's elapsed time**
(slower). The structural cause was measured and named: every wire round
re-posts the whole prefix over the HTTP request/reply path.
`docs/implementation/dependency-graph.md` therefore orders "engine adapter
(P16, in-process C-API spike) BEFORE any lossless-mode speedup expectation":
speculative speedup over the per-token HTTP adapter is structurally ≤ 0, and
llama.cpp HTTP cannot tree-verify.

The P16 spike tested whether the capability surface cooperative modes need
(KV rollback, per-row logits, llama.cpp's own sampler chain) already exists
in the artifacts every node ships. It does: `llama.dll` / `libllama.so` /
`libllama.dylib` ride in the pinned per-OS engine bundles, each sha256-pinned
in `runtime-pins.json` `platforms{}.files`. ADR-021's Option-B rejection
(no llama.cpp fork) constrains the design; ADR-024 supplies the identity
precedent (a local execution detail, not a swarm-visible runtime change).

## Decision

### 1. Research-path adoption; production untouched

- The adapter lands behind the feature `modelswarm-runtime/capi-adapter`
  (`dep:libloading`), **OFF by default**. The node never enables it; the
  production HTTP adapter, `runtime-pins.json`, and every serving path are
  unchanged. This decides ADR-019 §3's research-adapter slot on the
  research side of ADR-021's split (alongside its other research paths,
  Options A/C) — with no fork, no new endpoints, no new binaries.
- Every experiment record produced through it carries
  `runtime.name = "llama.cpp-capi"` (ADR-019 honesty rules; run-manifest
  runtime identity), the same way mock records carry `"mock"`.

### 2. Identity: a local execution detail (ADR-024 precedent)

- **No new artifacts.** The adapter runtime-loads the SAME pinned binaries
  the HTTP server uses, from the existing hash-verified per-OS bundles
  (`llama.dll` windows-x64 CPU and Vulkan variants — identical sha;
  `libllama.so` linux-x64; `libllama.dylib` macos). Zero pin changes; the
  no-redistribution rule is untouched.
- `canonical_build_hash` semantics are unchanged: `EngineIdentity` reports
  name `llama.cpp-capi` (telemetry/run-manifest disambiguation only), the
  same version tag and the same canonical_build_hash as the HTTP adapter.
  Swarm identity is identical across HTTP/C-API, exactly as ADR-024 treats
  the Vulkan backend; disclosure is local. This name never feeds the
  msp-v1 §6.1 handshake `runtime_name` (which stays `llama.cpp` per
  ADR-024).
- **One bundle per process, fail-closed.** `ggml_backend_load_all_from_path`
  registration is process-global (OnceLock). Exactly one pinned engine
  bundle per process is today's invariant; a second adapter construction
  naming a DIFFERENT bundle dir SHALL fail closed rather than silently keep
  the first registration. (Records the spike's unresolved-risk check.)

### 3. FFI safety envelope (terms from the spike)

- **Fail-closed symbol resolution**: every required symbol is resolved by
  name at construction; a missing export fails construction — never a
  partial or fallback adapter.
- **ABI canaries**: struct mirrors are derived from the pinned tag's public
  `include/llama.h` (`llama_model_params` 80 B, `llama_context_params`
  160 B, `llama_batch` 56 B), layout-pinned by unit tests; constructor and
  `load()` run default-params sanity and `n_ctx` write/read-back canaries.
  A tag bump is a new pinned build and re-derives the mirrors by rule.
- **`ggml_backend_load_all_from_path(<bundle dir>)` is mandatory** before
  model load (b11407 fails with "no backends are loaded" otherwise).
- **Unsafe confinement**: `unsafe` exists only in
  `crates/modelswarm-runtime/src/capi/ffi.rs`, enabled by a crate-local
  `[lints]` table (`unsafe_code = "deny"` with one module-scoped allow);
  the workspace `forbid` stands for every other crate. CI-green condition:
  no `unsafe` outside that file.
- **`spawn_blocking` before any concurrent use**: current decode steps
  block the async executor for ~one token of compute (~7.5 ms measured,
  loopback) — acceptable for single-arm research/bench runs only; moving
  to `spawn_blocking`-driven channels is a productionization requirement,
  not hygiene.
- **Open item — per-handle KV RAM accounting**: contexts share one model;
  the marginal cost per handle is its KV cache (~90 MiB at ctx 4096 for
  SmolLM2-135M; scales with model). Winjob accounts only process totals
  and `RuntimeMetrics` has no memory fields — must land before concurrent
  handles (gate item below).

### 4. Tree verification is not expressible; linear verify is the fallback

The tree attention-mask construction lives inside `llama_speculative`
(C++); it is not reachable through the public C API, and reaching it would
require the fork ADR-021 rejected — that rejection stands. The primitives
exist (`seq_cp`, multi-sequence batches), so branch-per-sequence trees are
plausible follow-up research requiring their own ADR. Until then the
documented fallback is the implemented **linear verify**: one batched
`llama_decode` of `[prefix_last, draft…]` with logits on every row,
accept-while-llama.cpp's-own-sampler-chain-would-sample, single KV rollback
at the first rejection — greedy results are the server's results
(bit-parity by construction). ADR-029 tie: `verified`/`maximum` preset
disclosures must not imply tree-verify latency or acceptance; every
lossless-mode claim while this ADR governs is linear-verify-bounded and
says so.

### 5. Productionization gate and next consumers

Nothing leaves the research feature flag — no bench default, no serving
path, no trait integration of `verify_drafts` (it stays an inherent
method; the frozen `InferenceRuntime` trait changes only via ADR) —
until the gate items below pass; passing gate items 1–5 lands with a
`docs/verification/` entry per the repo convention. Until then:

1. **Shape-stable batching**: decodes that alternate batch shapes each pay
   a ~10–13 ms `graph_reserve`-class cost in b11407 (measured; it ate the
   growing-prefix win). Uniform output counts per phase — what
   llama-server's own pipeline does — plus its own re-measurement.
2. **`spawn_blocking`-driven channels** (§3) before any concurrent use.
3. **Per-handle KV RAM accounting** (§3 open item).
4. **Vulkan variant in-process** tested before any GPU claim (untested in
   the spike; same registration path expected, ADR-024 auto-fit semantics
   live in device selection).
5. The capi-feature clippy/test gate joins CI (today it is a manual,
   feature-on gate), and the honest negative is published if the measured
   wins do not survive items 1–3.

Next consumer: the Scheduler Scientist's **9.6 pass 2** (dependency-graph
edge: engine adapter → 9.6 pass 2 → lossless-mode light-up), loopback then
owner-gated LAN, with the speculation harness consuming `verify_drafts`.
Comparator unchanged: the independently executed fastest eligible single.
Pass 2 owns the repeat/P50 protocol — the spike's numbers are single-run.

## Measured (honest) results

**Environment: single machine, single run, loopback — Windows 11 dev
machine, CPU-only (24 logical CPUs) under dev-session load; pinned llama.cpp
b11407, bundle sha256-verified against `runtime-pins.json` windows-x64;
SmolLM2-135M-Instruct Q4_K_M (100.6 MiB), ctx 4096, greedy; both arms
back-to-back against one warm engine each. NOT LAN. NOT a product claim.
Load caveat: this same machine recorded 4.3–4.9 ms/tok on a quieter day
(P1); here both arms ran ~7.5 ms/tok — relative comparisons hold,
absolutes are noisy. Exactness pins are pass/fail assertions and
load-independent.**

| Measurement (1217-tok prompt class) | HTTP adapter | C-API in-process |
|---|---|---|
| Exactness | — | tokenize parity (1217 tok), 32-tok greedy stream token-identical, detokenize + eos agreement, verify outcomes == HTTP ground truth, reject-path rollback exact (all asserted) |
| Repeated identical prefill ×5 | 9.3–11.1 ms/round (warm slot cache) | **0.14–0.16 ms/round** (delta = 0) |
| Verify (speculative, k=8) | rounds 83.8/180.2 ms of per-position prefix re-posts | rounds 91.8/90.1 ms like-for-like; verify alone **33–36 ms (one 9-row batched decode)** |
| 2-round speculative total | 264.0 ms | **181.9 ms (1.45×)** — no prefix ever re-posted or re-evaluated |

Honest negatives, first-class:

1. **Shape churn**: growing-prefix rounds that alternated batch shapes
   cost 12.1–13.9 ms (the ~10–13 ms `graph_reserve`-class penalty) versus
   0.15 ms on shape-stable delta-0 rounds — decision §5.1 exists because
   of this row.
2. **The HTTP arm degraded itself**: round 2 ran 84→180 ms as interleaved
   request patterns thrashed the server's single slot cache — an
   independent loopback reproduction of the 9.6 pass-1 structural finding
   (per-round prefix re-post) with no injected delay at all.

Together with pass 1's 1.07×–3.36× engaged-run slowdown (loopback +
injected delay, synthetic executor), these rows are the empirical case for
this ADR: the structural cost the wire cannot avoid is the cost the
in-process KV removes — at research-posture safety terms, honestly labeled.

## Consequences

+ ADR-019 §3's open research-adapter decision is now made — no fork, no
  new artifacts, no pin changes — and cooperative research gains logits
  access, KV rollback, and batched linear verification behind the frozen
  trait.
+ 9.6 pass 2 has its instrument; every lossless-mode positive claim now
  waits on pass-2 records against the fastest eligible single, not on the
  structurally capped HTTP path.
+ Swarm identity, pin verification, and the honesty-label machinery are
  unchanged (ADR-024 precedent applied to a second execution detail).
− Research-path only: nothing user-facing changed, and the production HTTP
  adapter still carries the per-round prefix re-post on cooperative wire
  paths until pass 2 decides otherwise.
− Real engineering precedes any default-on use (shape-stable batching,
  `spawn_blocking`, KV RAM accounting, CI gate), and the single-run
  loopback numbers must be re-measured under the pass-2 protocol.
− No tree verification: `verified`/`maximum` expectations stay
  linear-verify-bounded until a follow-up ADR says otherwise.
