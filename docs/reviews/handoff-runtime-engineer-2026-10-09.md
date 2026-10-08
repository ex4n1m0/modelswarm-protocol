# Handoff — Runtime Engineer audit (2026-10-09)

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
