# Handoff — Runtime Engineer audit (2026-10-09) + P1 implementation record

Branch `audit/master-prompt-2026-10-09` (baseline `730cebf`; main at `63b98e1`
is a CI-only fix). READ-ONLY audit: this file is the only artifact created in
the repo. No `cargo`/`npm` run; test evidence read from the frozen local suite
in `target/audit-logs/` (summary: ALL PASS, 301 tests / 49 suites default,
313 libp2p, 7 desktop tauri-shell). One live engine probe was run against the
installed pinned engine on this dev machine (loopback, hash-verified, killed
afterwards) — details under "Streaming analysis"; it is the only execution and
it touched nothing in the repo.

Scope: `crates/modelswarm-runtime` (trait, llama.cpp adapter, mock),
`runtime-pins.json` consumption/verification, `scripts/`, the HF acquisition
path (`crates/modelswarm-node/src/artifact.rs` — Phase H2 runtime-owned), and
the engine-supervisor seam (`crates/modelswarm-node/src/engine.rs`, interface
review only). M5/M6 items in my domain. ADRs 002/005/019/021/022/024/025 read
and cross-checked against code.

---

## 1. Findings (severity-ranked)

### CRITICAL

**C1 — Streaming is fake end-to-end; client TTFT ≈ full completion time.**
Evidence chain (every layer materializes before emitting):

1. `crates/modelswarm-runtime/src/llamacpp.rs:510-540` — `decode_stream` is a
   loop of `decode_step`; each step is its own `POST /v1/completions` with
   `max_tokens: 1`, `logprobs: true` and the **entire growing prefix** as the
   `prompt` token array (`llamacpp.rs:423-440`). One HTTP round trip per token,
   payload O(prefix length).
2. `crates/modelswarm-node/src/executor.rs:136-195` — `SingleLocalExecutor`
   awaits the complete `decode_stream`, then builds the whole event list and
   returns `stream::iter(events)` — a fully materialized "stream". Note the
   serving path never calls `prefill()` at all; the first `decode_step`
   carries the whole prompt.
3. `crates/modelswarm-gateway/src/http.rs:547-578` — SSE forwards events as
   they arrive, but they all arrive at once after decode finishes (plus one
   `detokenize` round trip per 16-token chunk, `executor.rs:173-181`).
4. `crates/modelswarm-desktop/src/app.rs:1508-1534` — `drain_chat` also
   collects every delta before rendering. The UI is non-incremental even
   against a future true stream.
5. `crates/modelswarm-node/src/serving.rs:366-409` — remote peers receive the
   same end-of-decode burst over QUIC.

Measured cost (this dev machine, CPU, SmolLM2-135M-Instruct Q4_K_M, pinned
engine verified `llama-server.exe` sha256 `2fd854df…8789e` == `runtime-pins.json`
windows-x64 entry, banner `0.5.0-dev build 11407`):

| Path | 16-tok prompt | 1215-tok prompt |
|---|---|---|
| Adapter loop (per-token POST, growing prefix) | 9.11 ms/tok avg; 48 toks = 437 ms; HTTP p50 5.89 ms | 24.73 ms/tok avg; 32 toks = 791 ms; first request 512.8 ms (re-prefill) |
| Native SSE (`stream:true`, same endpoint, `logprobs:true`) | 5.27 ms/tok; total 253 ms; **TTFT 4.0 ms** | 3.3 ms/tok; total 108 ms; TTFT 4.1 ms (warm slot cache) |

Cross-check against published numbers: phase-H's **70.3 tok/s** (Qwen2.5-0.5B,
CPU, dev machine, adapter path) already *includes* the per-token loopback RTT
and prefix re-send (it is measured per `decode_step` elapsed), so it is honest
as a throughput figure — but it is ~55% of the raw engine's llama-bench
126.6 tok/s CPU (ADR-024), and **no TTFT number was ever published**, which is
exactly the dimension fake streaming destroys: the client sees the first token
at ~100% of completion time (437 ms → minutes at 27B scale).
Prefix caching: llama-server's slot cache already absorbs re-prefill after the
first request (measured: only request 1 pays 512.8 ms); the recurring per-token
cost is JSON parse + cache search + request overhead of the re-sent prefix —
it grows with prompt length (9.11 → 24.73 ms/tok from 16 → 1215 tokens), i.e.
O(n²) total work per completion. So "prefill re-billing" is real but small
with a warm cache; the dominant fix is streaming, not caching.
Availability of streaming on b11407: **native SSE works and each chunk's
`logprobs.content[]` entry carries `id`, `bytes`, `token`, `logprob`,
`top_logprobs`** — exact token-id recovery (including EOS via `id`, the
Qwen2.5-7B `<|im_end|>` fix) is fully compatible with `stream:true`. The
recorded reason for non-streaming (ADR-025: exact-token path needs the raw
`/v1/completions` endpoint) is true but was never a constraint against
`stream:true` on that same endpoint.

Fix (time-boxed, ranked by latency win per master prompt §11 — connection
reuse + prefill are the sanctioned levers; both are subsumed by (a)):
- (a) **True streaming adapter** (~2-3 days incl. tests): override
  `decode_stream` with one `POST /v1/completions {stream:true, logprobs:true}`,
  parse SSE incrementally, recover ids per chunk exactly as today
  (`id` first, `bytes` cross-check, fail-closed on disagreement), stop at EOS.
  Expected (measured-class): TTFT 437 ms → ~4 ms warm cache; ITL 9.1 → 3.9 ms
  on the probe model; removes O(prefix) per-token cost entirely.
- (b) **Incremental executor** (~1 day after (a)): `SingleLocalExecutor`
  emits `ExecutorEvent`s as tokens arrive (the `ExecutorStream` trait type
  already supports it; only the implementation changes) —
  detokenize-per-chunk already batches 16.
- (c) **Incremental desktop UI** (~0.5 day, Windows Product Engineer's file):
  replace `drain_chat` with Tauri event emission per delta.
- (d) Keep the current one-token loop only as the `propose()` implementation
  (window is small; scheduler-facing, gated).

### HIGH

**H1 — Prefix work is re-billed per token** (`llamacpp.rs:423-427`): every
`decode_step` re-serializes, re-parses and re-searches the whole prefix;
O(n²) bytes per completion. Subsumed by C1(a). If C1 is deferred, an
interim mitigation is `n_predict: k` micro-batches (k=4..8) with exact-id
recovery per position — but that changes mid-stream sampling semantics; do
not bother, do C1(a).

**H2 — Cancellation is deadline-only.** `decode_stream` checks only the
deadline (`llamacpp.rs:525-531`); `cancel()` is a best-effort tolerated-404
`POST /cancel` (`llamacpp.rs:580-622`). An SSE client that disconnects does
not stop the local decode: `stream_response`'s spawned task keeps driving the
executor to `max_tokens`/deadline (gateway default clamp 120 s,
`modelswarm-gateway/src/lib.rs:56`). Abandoned requests burn a serving slot.
Fix (~0.5-1 day): cancellation token threaded into the decode loop + abort the
spawned task when the SSE body stream drops.

**H3 — No download resume** (`artifact.rs:14-18` documents it): a dropped
connection restarts the transfer from byte 0. The Qwen3.8-27B artifact on this
machine is 18.97 GB — one flake costs ~20 minutes of restart. M5 exit gate
("interrupted … never become active; failed upgrades recover automatically")
is only half-met: safety yes (`.part` + hash + atomic rename), recovery no.
Fix (~1 day): `Range: bytes=<written>-` continuation against the `.part` file
with content-length validation before appending; hash still verified over the
final assembled file (reference: hf-hub Rust crate).

**H4 — Blocking file I/O in async engine verification.**
`verify_engine_variant` does synchronous `std::fs::read` of every bundle DLL
(`engine.rs:216-231`, tens of MB) inside async `start_engine`, stalling the
node's tokio worker during startup. Fix (~2 h): `spawn_blocking` (the pattern
already exists in `artifact.rs:108-113`). Same class, other owner:
`--list-devices` synchronous spawn in `crates/modelswarm-desktop/src/app.rs:527`
(flagged to Windows Product Engineer).

### MEDIUM

**M1 — Value-equality greedy mapping.** `sampling_body`
(`llamacpp.rs:318-332`) maps any `SamplingParams` *equal to the default*
(temp 1.0, top_p 1.0, top_k 40, seed None) to `temperature: 0`. A client that
explicitly sends those values (OpenAI-style default temperature 1.0) silently
receives greedy decoding. The determinism contract says "default sampling";
this implementation is value-based, not intent-based. The desktop chat sends
`temperature: 0.0` explicitly (correct). Needs a protocol decision (explicit
greedy flag or sentinel) — Architect's call, ADR touch.

**M2 — Serving metrics mislabeled.** The executor never calls `prefill()`, so
`RuntimeMetrics::prefill_tokens_per_ms` stays 0 in production and
`ExecutorEvent::Usage { prefill_ms: 0.0, decode_ms: <whole elapsed> }`
(`executor.rs:183-188`) folds prompt evaluation into decode. Downstream,
cost-model v2 consumes prefill/decode as independent quantities (my role:
measure them independently). Fix rides along C1(a)/(b): report prompt-eval
time from the streaming response `timings`/usage fields.

**M3 — No disk-space precheck** (`artifact.rs:127-172`): the transfer starts
unconditionally and fails mid-write on a full disk after minutes (19 GB
class). Fix (~2 h): HEAD/content-length vs available bytes on the target
volume before creating the `.part`.

**M4 — Context enforcement is engine-side only.** Engine `-c` default 4096
(`engine.rs:148`); gateway clamps prompt to 32 KB *bytes* and output to 2048
tokens, but nothing checks `prompt_tokens + max_tokens <= ctx` at admission —
an over-long prompt surfaces as an engine `Api` error after the lease was
granted. Fix (~2 h): check after tokenize in the executor, map to a stable
code (`prompt_too_long`).

**M5-artifact lifecycle — cleanup/deprecation/rollback absent.** Artifacts
accumulate per profile dir with no reclamation policy, no deprecation
handling, no previous-profile rollback concept at the runtime layer (three
profiles = 23.6 GB coexist on this machine). Catalog has `licenseId` /
`licenseEvidenceUrl` (attribution preserved per ADR-005), but nothing deletes
superseded weights. Product-level decision; enforcement belongs in the node.

**M6 — `ArtifactManager::new` panics on client build failure**
(`artifact.rs:64-68` `.expect("reqwest client")`). Constructor on the node
startup path should return `Result`. ~30 min.

### LOW

- **L1** — the ignored real-engine smoke test passes `Some(999)` `-ngl` for
  GPU variants (`engine.rs:575`), contradicting ADR-024's amendment ("No
  `-ngl` is passed"). The production caller is correct (`lib.rs:298`
  `gpu_layers: None`). Fix the test to `None`. ~15 min.
- **L2** — `propose()` inherits the one-POST-per-token loop (fine while
  cooperative work is gated; revisit with C1 since greedy windows are the
  cheapest streaming consumer).
- **L3** — gateway SSE `mpsc::unbounded_channel` is intentional (comment
  cites msp-v1 §6.4, client-side backpressure); each line is small. Revisit
  with multi-user admission (M10).
- **L4** — no unbounded buffers found on the engine HTTP path
  (`decode_stream` pre-bounds capacity to `min(max_tokens, 1024)`;
  completions responses are single-token).

---

## 2. Streaming analysis summary (the numbers, environment-labeled)

All probe numbers: **dev machine (Windows, CPU), SmolLM2-135M Q4_K_M, pinned
b11407 engine, loopback, warm slot cache unless noted.** Published numbers
re-cited with their original labels (70.3 tok/s = phase-H dev-machine adapter
path; 126.6/517 tok/s = ADR-024 llama-bench on RTX-5080-laptop-class CPU/GPU —
llama-bench, not the adapter).

- Per-token loop cost today: 9.11 ms/tok (16-tok prefix) → 24.73 ms/tok
  (1215-tok prefix), scaling with prefix — evidence for H1.
- Native SSE on the same pinned build: TTFT 4.0 ms / 4.1 ms (warm cache;
  cold-cache TTFT ≈ the 512.8 ms measured first-request prefill), ITL p50
  3.94 ms, total 108-253 ms.
- Exact-id recovery in streaming mode: **verified feasible** — chunks carry
  `logprobs.content[]` with `id` + `bytes` per token.
- What the existing numbers do and do not account for: 70.3 tok/s includes
  per-token RTT + prefix re-send (honest throughput); 517 tok/s does not flow
  through the adapter at all (llama-bench raw engine); no TTFT number exists
  anywhere in `docs/verification/` — the metric fake streaming breaks was
  never recorded.

---

## 3. M5 verified-download lifecycle table

| Item | Status | Evidence |
|---|---|---|
| Resumable downloads | **ABSENT** (documented) | `artifact.rs:14-18`; phase-h "Honest limits" |
| Staging | PARTIAL (`.part` sibling, always) | `artifact.rs:130-156` |
| Hash checks | PRESENT (streamed sha256; re-verified on every start) | `artifact.rs:157-186`, `107-125`; tests `downloads_verifies_and_reuses` |
| Signature checks | PRESENT at catalog layer (Ed25519 envelope, `protocol/keys/hub-public.hex`); artifact pinned transitively via signed manifest sha256 | phase-h H2; `catalog/schema-v2.json` |
| Atomic activation | PRESENT (rename after hash; cleanup on every failure path) | `artifact.rs:173-190`; test `hash_mismatch_fails_closed_and_cleans_up` |
| Corruption repair | PRESENT (mismatch on disk → delete + redownload) | `artifact.rs:119-124`; test `corrupt_existing_file_triggers_redownload` |
| Disk-space precheck | **ABSENT** | `artifact.rs:127-172` (M3) |
| Previous-profile rollback | **ABSENT** (no profile-switch lifecycle; dirs coexist) | §1 M5-artifact |
| Update migration | PARTIAL — node-state migration exists (`lib.rs:170-199` data-dir split); artifact/engine updates are installer-scope, not runtime-scope | `crates/modelswarm-node/src/lib.rs` |
| Cleanup | **ABSENT** | §1 M5-artifact |
| Deprecation behavior | **ABSENT** at runtime layer (catalog `status` exists upstream) | `catalog/schema.json:13` |
| ADR-022 identity re-derivation from GGUF | PRESENT (tokenizer/chat-template/architecture hashes re-derived and compared; store row) | `artifact.rs:196-246` |

## 4. M6 real-local-inference table (my domain)

| Item | Status | Evidence |
|---|---|---|
| Loading | PRESENT | `engine.rs:254-328` (spawn → /health gate) |
| Tokenizer parity | PRESENT (golden vectors + env-gated real-artifact job `real-model-e2e.yml`; resolver≡Rust parity locks incl. amendments 1+2, `modelswarm-types/src/gguf.rs:649,688`) | CI + `target/audit-logs/test-default.log` |
| ChatML application point (ADR-025) | PRESENT (executor renders ChatML exactly once; regression-pinned) | `executor.rs:93-104` + test |
| Greedy determinism (temp 0) | PRESENT, with M1 value-equality caveat | `llamacpp.rs:318-332` + tests |
| Sampling | PRESENT (verbatim forwarding incl. seed) | `llamacpp.rs:322-331` + test |
| Streaming | **FAKE (C1)** | §2 |
| Cancellation | PARTIAL — deadline-only, no client-abort propagation (H2) | §1 H2 |
| KV-cache lifecycle | PARTIAL — documented approximation (`KvCommitment` sha over span+ids; llama.cpp exposes no exact KV); serving never calls prefill (M2) | `runtime/src/lib.rs:125-151` |
| Context enforcement | PARTIAL — engine-side error only (M4) | §1 M4 |
| Memory accounting | ABSENT in runtime (winjob total-RAM only; `RuntimeMetrics` has no memory fields) | master prompt §0 concurs |
| Runtime restart | PRESENT (bounded restarts=3, fail-fast `dead` flag, dies with node, winjob kill-on-close) | `engine.rs:254-382, 433-448` |
| Local API compatibility | PRESENT for `/v1/chat/completions` + `/v1/models`; no `/v1/completions` passthrough, no cancel endpoint | `gateway/src/http.rs:115-116` |

## 5. Component classifications (salvage matrix rows I own)

| Component | Decision | Reason |
|---|---|---|
| `runtime/src/lib.rs` (trait, KvCommitment, errors) | KEEP WITH TESTS | Shape frozen per ADR-019; sound |
| `llamacpp.rs` exact-id recovery (`id`+`bytes`, EOS fix, fail-closed disagreement) | KEEP WITH TESTS | Qwen2.5-7B fix proven; 27 crate tests green; streaming-compatible (probe) |
| `llamacpp.rs` `decode_stream` | REFACTOR | C1(a) true SSE |
| `runtime/src/mock.rs` | KEEP | TEST-ONLY, feature-off in production |
| `node/src/artifact.rs` ArtifactManager | KEEP WITH TESTS → REFACTOR for resume (H3) + disk precheck (M3) + Result ctor (M6) | Core possession-verification chain is solid |
| `node/src/engine.rs` supervisor seam | KEEP WITH TESTS (fix H4 blocking verify, L1 test) | Loopback+bearer+`--no-webui`+bounded restarts+fail-fast+winjob all verified in code; `gpu_layers: None` honored in production (ADR-024) |
| `runtime-pins.json` + verification chain | KEEP | canonical_build_hash anchors identity on every OS incl. Vulkan variant; per-file sha256; live engine matched its pin during the probe |
| `scripts/resolve-candidate.mjs` | KEEP | ADR-022 + amendments 1+2 exactly implemented (`optionalBool/optionalU32`, `eos` via `need()` = required) |
| `scripts/promote-candidates.mjs` | KEEP | Admin-gated, fail-closed immutability, dry-run |
| Serving-stream seam (`executor.rs` event materialization) | REFACTOR | C1(b) — file owned by node; interface contract is mine |

## 6. Test coverage (fresh logs vs missing)

Green in `target/audit-logs/test-default.log`: modelswarm-runtime 27/27;
node crate includes artifact (3), engine pins/variant/args/tamper tests;
workspace totals 301 (default) / 313 (libp2p) / 7 (tauri-shell), 0 failed.
Missing for my domain: streaming-parity test (SSE vs one-token loop produce
identical token sequences); client-abort cancellation test; TTFT/ITL
measurement in a verification doc (only throughput exists); resume test once
H3 lands; long-prompt per-token-cost regression bound.

## 7. Assumptions

- Probe numbers are single-machine, single-run, small-model, CPU; they size
  the mechanism, not the product's absolute latency (labels above).
- Warm-cache TTFT stated where measured; cold-cache prefill separately
  measured at 512.8 ms for 1215 tokens.
- `engine.rs`/`artifact.rs` sit in `modelswarm-node` (Integrator/Windows
  ownership by map) but are the runtime's Phase H2/H3 deliverables; I audited
  them as my seam and propose fixes without editing the files in this audit.
- The 70.3 tok/s figure is taken at its recorded label (phase-H dev machine).

## 8. Unresolved risks

- M1 (value-equality greedy) needs a protocol decision before any change.
- llama-server slot-cache behavior under concurrent remote+local load may
  evict prefixes, making the per-token loop's worst case the 512.8 ms
  re-prefill per token — unmeasured under contention; true streaming also
  removes this risk.
- `cancel()`'s helper-thread + fresh reqwest client per call is correct but
  unmeasured against a build that actually implements `POST /cancel`.

## 9. Suggested next task for the integrator

Approve, as the first post-gate runtime change, the bounded streaming refactor
C1(a)+(b) (+H2 cancellation token, M2 metrics) behind the unchanged
`InferenceRuntime`/`InferenceExecutor` interfaces: canned-SSE mock tests,
streaming-parity golden test (SSE ≡ loop token-for-token, temperature 0), and
a `docs/verification/` doc recording TTFT/ITL P50/P95 in the established
labeled format. Sequence it **before** any M9 baseline freeze — every TTFT
comparison made against the fake-streaming path would have to be redone.
Route H4+L1 (engine file, ~2.5 h) to me immediately; H3 resume next;
M1 to the Protocol Architect.

---

# P1 — true streaming end to end (implemented 2026-10-09, post-gate)

Owner-approved architecture-gate §C.4 critical-path item #1. Commits:
`3b64562` (implementation) and `7c7d78e` (real-engine validation + L1).
Branch: `main`. Status: **complete and green**; C1(a)+(b)+(c)+(d) of my fix
plan landed plus M2 metrics and audit-L1.

## What changed (files, why)

- `crates/modelswarm-runtime/src/lib.rs` — new frozen-trait-compatible
  incremental API: `DecodeEvent {Token(u32), Timings(DecodeTimings)}`,
  `DecodeEventStream<'a>`, and `InferenceRuntime::decode_stream_events` with
  a collect-then-emit default (wraps the unchanged `decode_stream`, so
  MockRuntime and every non-overriding runtime stays conformant; pinned by
  `default_decode_stream_events_matches_decode_stream`). Existing
  `decode_stream` (Vec) is untouched in signature — bench/session/sim/
  selftest all keep compiling unchanged.
- `crates/modelswarm-runtime/src/llamacpp.rs` — `decode_stream_events`
  override: ONE `POST /v1/completions {stream:true, logprobs:true}` parsed
  incrementally (probe-verified b11407 wire format, §"Measured results" for
  the chunk shapes). Exact-id recovery is the SAME fail-closed contract as
  `decode_step`, now factored into `exact_token_id` (server `id`
  authoritative — the Qwen2.5-7B `<|im_end|>` fix; pinned-vocab `bytes`
  cross-check; disagreement ⇒ `MalformedResponse`; unknown bytes ⇒ error).
  EOS terminates the stream without being emitted (vocab `eos_id`; the EOS
  chunk arrives with `id` + empty `bytes`). The caller's deadline is the
  reqwest per-request timeout across the whole body ⇒ same
  `RuntimeError::Timeout` surface. Mid-stream engine error events map to
  `RuntimeError::Api`; unparsable chunks fail closed. `decode_stream` is now
  a fold over the same stream — one decode code path. `Shared` metrics moved
  behind `Arc` so the stream records prefill/decode rates from the final
  chunk's server-reported `timings` (M2: prompt-eval split out of decode;
  client-observed fallback when the server reports none).
- `crates/modelswarm-node/src/executor.rs` — `SingleLocalExecutor::execute`
  now spawns a driver over `decode_stream_events` + an mpsc channel:
  `Accepted` immediately, one `TokenDelta` per 16 committed tokens
  (batched detokenize — the protected event sequence is byte-identical in
  shape and order, only timely). Mid-stream runtime failures become
  `ExecutorEvent::Error` with the SAME stable code the pre-stream
  `Err(Fatal)` carried (`runtime_error_code` mapping, redacted telemetry
  unchanged); the gateway's ADR-007 machine (failover only before the first
  token, `interrupted` after) consumes it as designed. Dropping the
  consumer drops the decode stream ⇒ SSE connection abort ⇒ the pinned
  server cancels the request (partial H2 relief, connection-scoped).
  `Usage` now carries server-reported `prefill_ms`/`decode_ms` when the
  engine provided timings, else the previous client-elapsed fallback.
- `crates/modelswarm-gateway/src/lib.rs` — no behavior change was needed:
  `stream_response` already forwarded per-delta once the executor streams
  (the audit's burst was upstream materialization). Added the pin:
  `sse_content_chunks_arrive_before_the_executor_completes` (slow executor
  double; first content chunk readable from the body before completion).
- `crates/modelswarm-desktop/src/app.rs` (+ `Cargo.toml` reqwest `stream`
  feature, `ui/index.html`) — `send_chat` emits a `chat-delta` Tauri event
  per delta on EVERY producing path: swarm remote, local fallback
  (`drain_chat` now takes an `on_delta` callback) and the local gateway
  request, which switched `stream:false` → `stream:true` with incremental
  SSE parsing. Error surface preserved in shape: HTTP-level gateway errors
  and mid-stream SSE error events both render the same ERROR-turn JSON;
  empty-reply and mode-label behavior untouched. The UI renders a live
  assistant bubble per delta (generation-guarded), replaced by the
  authoritative final message with its meta line.

## What did NOT change (protected seams)

`InferenceExecutor::execute` signature, `ExecutorEvent` vocabulary,
`InferenceRuntime::decode_stream` signature, `decode_step`, `propose`
(still the greedy per-token loop per fix-plan (d)), `prefill`, `cancel`,
serving.rs (untouched — consumes the same executor interface), protocol/,
ADR files, `.github/workflows`, tracker.

## Measured results (environment-labeled)

**Environment: dev machine (Windows 11, CPU-only), pinned llama.cpp b11407
(hash-verified against `runtime-pins.json` windows-x64), SmolLM2-135M-
Instruct Q4_K_M, loopback, warm slot cache, greedy (temperature 0).**

Fresh same-instance A/B (both paths back-to-back against one engine
instance, 32 tokens each; env-gated test `real_engine_streaming_parity_
and_timings`, two consecutive runs):

| Prompt | Loop (before-path, per-token POST + growing prefix) | SSE (after) | Parity |
|---|---|---|---|
| 4 tokens | 5.0–5.2 ms/tok; total 159–166 ms | TTFT 6.2–6.6 ms; ITL p50 4.7–4.9 ms; total 137–153 ms (4.3–4.8 ms/tok) | identical |
| 1217 tokens | 5.7–6.0 ms/tok; total 182–193 ms | TTFT 5.3–6.5 ms; ITL p50 5.1–5.3 ms; total 154–158 ms (4.8–4.9 ms/tok) | identical |

The client-visible TTFT change is structural, not incremental: pre-P1 the
executor materialized the whole decode before the first `TokenDelta`, so
client TTFT ≈ total decode time (159–193 ms here; the audit's longer-run
measurements: 437 ms @ 48 toks / 791 ms @ 32 toks incl. 512.8 ms cold
re-prefill) — now the first delta surfaces at TTFT ≈ 6 ms + one 16-token
batch. Cross-checking the audit's probe (same machine, busier load that
day): loop 9.11–24.73 ms/tok and native SSE TTFT 4.0 ms / ITL p50 3.94 ms —
the machine's absolute numbers move with load; the fresh table above is the
honest apples-to-apples record, and the parity assertion is load-independent.
The O(prefix) per-token cost (H1) is gone by construction: one request per
decode. `decode_tokens_per_ms` semantics note: when the server reports
timings it is now the engine's decode-phase rate (smoke test printed
384.6 tok/s engine-side) rather than client-observed ms/token incl. HTTP
RTT — label accordingly in future verification docs.

## Test evidence

`cargo test -p modelswarm-runtime` 23/23 (9 new/rewritten SSE tests: exact
id recovery incl. bytes-only + EOS-empty-bytes + id/bytes disagreement +
SSE error events + deadline→Timeout + no-vocab server-id path + ONE-POST
assertion + max_tokens cap + incremental arrival); `cargo test
-p modelswarm-node` executor 6/6 (2 new: deltas-before-completion,
mid-stream-failure→Error-event) + engine suite incl. 2 env-gated real-engine
tests (both executed against the staged pinned engine, then verified no
llama-server process remains); gateway 20/20 (1 new timeliness pin);
workspace totals 310 (default) / 322 (libp2p) / 7 (tauri-shell), 0 failed.
clippy `-D warnings` clean on all three gate configurations; fmt clean.

## Assumptions

- b11407's SSE chunk shapes as probed (content chunks carry
  `logprobs.content[].{id,bytes}`; final chunk carries `timings`
  `{prompt_n,prompt_ms,predicted_n,predicted_ms}`; `[DONE]` terminator;
  generation errors as `data: {"error":…}` events). The parser tolerates
  comment/keep-alive lines and `\r\n` framing but fails closed on
  unparsable `data:` payloads.
- The no-vocab streaming path requires server-reported `id`s (production
  always attaches the GGUF vocab; dev/test adapters without one get an
  explicit MalformedResponse instead of the loop path's lossy text
  round-trip).
- Desktop `tauri-shell` tests don't exercise the new SSE client loop (it
  is UI-path code); its correctness is pinned by the runtime-level SSE
  tests + gateway tests on the same wire format.

## Unresolved risks / notes for the integrator

- H2 is only partially addressed (connection-drop cancellation). The
  gateway still spawns an unbounded-channel task that drives the executor
  to `max_tokens`/deadline if the SSE client disconnects — an abandoned
  request can still burn a serving slot. Next runtime task: cancellation
  token threaded gateway→executor→adapter.
- Detokenization remains one `POST /detokenize` per 16-token batch (event-
  shape parity); local vocab-side detokenization would drop that RTT but
  changes delta boundaries at UTF-8 splits — needs a decision before M9
  baseline freeze.
- No standalone `docs/verification/p1-streaming.md` yet (measurement table
  lives here); create it at the M9 baseline freeze with P50/P95 across
  both staged models.
- Remote P2P serving (`serving.rs`) now streams timely events over QUIC for
  free (same executor interface) but the wire burst behavior was never
  measured — scheduler's F15 measurement plumbing should capture it.
- Desktop live bubble is a placeholder-per-turn (generation-guarded); if
  concurrent chat turns ever become possible the UI needs a turn id.
- M1 (value-equality greedy mapping) still open — Architect's call.
