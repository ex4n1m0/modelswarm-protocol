# ADR-031 §5 productionization verification (2026-10-11)

Gate record for the in-process C-API adapter (`modelswarm-runtime/capi-adapter`,
OFF by default — productionization means **safe-to-turn-on**, the flag STAYS OFF
until the ADR-032 family lights up). Owner: Runtime Engineer. Commit: `48536d2`
(+ `5f8280e` pre-amend). Everything below ran owner-local on the dev machine.

## Environment (honest label)

Single machine, Windows 11 x64 (24 logical CPUs; RTX 5080 Laptop GPU for the
Vulkan run), under dev-session load; pinned llama.cpp **b11407**, bundle
sha256-verified against `runtime-pins.json` at every test start (windows-x64
CPU bundle + windows-x64 `variants.vulkan`; both bundles staged under
`%LOCALAPPDATA%\ModelSwarm\engine` and `\engine-vulkan`); SmolLM2-135M-Instruct
Q4_K_M (100.6 MiB), ctx 4096, greedy; loopback; NOT LAN, NOT a product claim.
Timings are single-run medians over fixed call sequences (n = 10–20 per cell);
exactness pins are pass/fail assertions and load-independent.

## Gate items and evidence

### Item 1 — shape-stable batching (measured before/after)

Instrument: `tests/capi_batching.rs` (env-gated `#[ignore]`d), a FIXED call
sequence with per-shape steady phases (A/B/C), an interleave phase (D), and
discrimination phases (E no-op floor, F verify-width alternation, G
step↔delta without verify, H same-shape fresh-tail vs freed-cells, I
rollback-only, K/L 1-token→k-token transitions with/without rollback, M the
real propose→verify→next-round-delta loop).

Commands (CPU bundle):

```
MSP_CAPI_ENGINE_DIR=%LOCALAPPDATA%\ModelSwarm\engine \
MSP_REAL_GGUF=<SmolLM2-135M path> \
cargo test -p modelswarm-runtime --features capi-adapter --test capi_batching -- --ignored --nocapture
```

BEFORE = pre-change adapter (run 2026-10-11, log `target/…` console capture
`/tmp/batching-before-4`); AFTER = productionized adapter (three runs; final
run below). Per-call wall-time medians:

| Phase (call class) | BEFORE median | AFTER median | Notes |
|---|---|---|---|
| A steady step (1 tok / 1 out) | 5.43–5.82 ms | 5.74–5.93 ms | unchanged (compute) |
| B steady delta (+3 tok / 0 out) | 9.26–9.77 ms | 10.63–11.04 ms | +1.2–1.5 ms: the uniform-output row (one lm_head pass) — the price of slot stability |
| C steady verify (9 rows / 9 out) | 22.8–24.0 ms | 23.5–24.5 ms | unchanged |
| D interleave verify (9/9) | +0.06–0.70 ms vs C | −0.4–+0.3 ms vs C | verify after other shapes: free |
| F verify widths 5/9/13/17 alternating | −0.9…+1.2 ms vs steady | −0.8…+1.2 ms | **differing verify widths are FREE at baseline AND after** |
| D/G interleave delta (3/0) | +24.1–26.9 ms vs B | +23.7–26.1 ms vs B | **residual anti-pattern, see negative #1** |
| E delta-0 sync floor | 0.128 ms | 0.14–0.28 ms | bookkeeping only |
| K step with live tail logits | 5.4–5.8 ms (always re-eval) | **0.32–0.39 ms** | uniform outputs: tail-logits re-eval SKIPPED (decode-free step) |
| M propose (8 one-token steps) | — | 47.2 ms median | the real round shape |
| M verify (one 9-row pass) | — | 23.93 ms == C steady | **no churn in the production loop** |
| M next-round delta sync | — | 9.5–9.7 ms | cheaper per token than B (9 tokens) |

Mechanism (corrected against b11407 sources, fetched read-only — spike
precedent): `llama_context::decode` reuses a cached graph only when
`gf_res_prev[n_outputs > 0]` is the same slot AND `can_reuse` sees equal
`n_tokens`/`n_outputs` (`llama-context.cpp: process_ubatch`,
`llama-graph.h: allow_reuse`). Width changes therefore rebuild the graph —
but rebuilds are CHEAP (phase F). The expensive transition is narrow and
reproducible: a **small multi-token decode (3 tokens) immediately after
[longest-common-prefix KV rollback + 1-token tail re-eval]** (phases D/G,
+24 ms). Ruled out empirically: KV hole-filling (H2−H1 = +0.0–1.1 ms),
rollback itself (phase I, 0.4–0.5 ms), 1→k transitions without rollback
(phases K/L, free), verify shapes after anything (D/M/F, free).

What landed: `uniform_outputs` (default on; every decode requests ≥1 logits
row — llama-server's own pipeline discipline; graph slots never switch, and
steps after a Last-logits delta skip the tail re-eval entirely — K row), one
**pooled `llama_batch` per handle** at `n_ctx` capacity (no per-call
`llama_batch_init/free`), `n_batch` pinned to `n_ctx` (removes the engine's
`n_tokens ≤ n_batch` GGML_ASSERT abort for prefills > 2048 tokens — a real
pre-existing landmine), `n_outputs_max` pinned to `verify_window_max + 1`,
and a load-time warmup decode at max verify geometry that sizes the output
buffer once (KV fully cleared afterwards; the warmup does not count against
`decode_calls`). Verify windows longer than `verify_window_max` (default 32
= ADR-032 WINDOW_CAP) are refused with `413 payload_too_large` BEFORE any
engine work.

### Item 2 — `spawn_blocking` offload

Every engine call that can block now runs on the tokio blocking pool through
a cloneable `Engine` core (`Arc<EngineInner>`): context creation + warmup
(`load`), `tokenize`/`detokenize`, every decode/eval (sync deltas, stream
steps, propose, verify batches), KV rollback, sampler chains, and
`llama_state_get_size` walks (`kv_usage`). The async executor only sequences
steps and observes deadline/cancellation between them. Requires a tokio
runtime (all research consumers run tokio) — documented on the module.
tokio `rt` feature added to the crate. `LlamaCppCapi::new` (library + model
load) stays synchronous by design, like the HTTP adapter's constructor, and
is documented as blocking.

### Item 3 — per-handle KV RAM accounting

- `kv_token_cap` (default = `n_ctx`): enforced in `sync` BEFORE any engine
  work; honest `429 kv_budget_exceeded` (pinned by the loopback test: cap 64
  keeps KV at 32 tokens, refuses the 1217-token prefill, zero engine work).
- Adapter-wide `kv_byte_budget` (optional): `load()` refuses a handle whose
  capacity estimate would exceed the committed total (same honest error).
- Estimate: `2 × 2 B (f16) × n_layer × n_head_kv × head_dim × n_ctx` from the
  engine's public GQA dimension getters — lands on **90 MiB** for
  SmolLM2-135M @ ctx 4096, exactly ADR-031 §3's "~90 MiB" open-item number.
- `kv_usage()` reports per handle: tokens, cap, capacity estimate,
  engine-reported `llama_state_get_size` (26 MiB used at 1225 tokens —
  consistent with the formula), and `decode_calls`.
- `RuntimeMetrics` gained additive `kv_bytes_estimate` + `engine_decode_calls`
  (Default 0 for adapters without accounting; no exhaustive literals outside
  this crate — verified).

### Item 4 — Vulkan in-process (mandatory before any cross-peer claim)

`tests/capi_vulkan.rs` (env-gated `#[ignore]`d; `MSP_CAPI_VULKAN_DIR` +
`MSP_REAL_GGUF`; one bundle per process by construction — separate test
binary). Owner-local run (RTX 5080): vulkan-variant sha256s verified against
`runtime-pins.json`; backend registration picks up `ggml-vulkan.dll` through
the same `ggml_backend_load_all_from_path` path (log: 2 Vulkan devices,
NVIDIA + Intel); adapter constructs and runs in-process. Pins (all passed):
tokenize parity; 16-token greedy stream token-exact vs a **Vulkan
llama-server HTTP ground truth** (23–24 ms total, perf printed only);
verification outcomes == ground truth (8/8 + bonus); reject-path rollback +
single-mode continuation token-for-token; warm verify == exactly ONE engine
decode call (I4); KV usage reported. E0 discipline: CPU≠Vulkan is allowed —
exactness is asserted WITHIN the backend only.

Found + fixed while testing: the reject-path corruption in the tests was
prompt-dependent (a shift-by-one draft legitimately accepts when a greedy
continuation starts with a repeated token — the Vulkan backend hit exactly
that). Both loopback and vulkan tests now corrupt position 0 with an
explicitly-wrong token.

### Item 5 — CI coverage

`rust.yml` already runs, on every push: `cargo clippy --workspace
--all-targets --features modelswarm-runtime/capi-adapter -- -D warnings` and
`cargo test --workspace --features modelswarm-runtime/capi-adapter`. The new
code compiles under exactly that configuration and its unit tests (config
clamps, KV estimate formula, bundle guard, layout pins) run in it —
**verified, no gap, no workflow change made** (workflows are T/R-owned
anyway). The real-engine tests stay `#[ignore]`d env-gated per the ADR-032
§7 venue matrix (item 2's exactness pins are manual-dispatch/owner-local —
this document is that record for the ADR-031 gate; the ADR-032 family's own
pin reruns follow the matrix's `real-model-e2e.yml` extension when that
family lights up).

### Item 6 — exactness pins re-run (+ KV-rollback extension)

`tests/capi_loopback.rs` vs the CPU HTTP ground truth, all asserted and
PASSED on the productionized adapter: tokenize parity (1217 + 8 tokens);
32-token greedy stream token-for-token; detokenize parity; eos agreement;
2-round speculative A/B 1.45–1.57× (177.7–183.6 ms vs 261.2–287.8 ms across
runs; single-run numbers, load-sensitive); verify outcomes == ground truth;
reject-at-0 rollback exact; **NEW** mid-window reject at k=4 (corrupt only
position 4; accepted == truth[..4], replacement == truth[4], then 12
single-mode `decode_step`s token-for-token == HTTP ground truth — the
KV-rollback path under the new shape-stable batching); **NEW** warm verify
== exactly ONE engine decode call (`decode_calls` 118 → 119; the ADR-032
I4 one-pass assertion); **NEW** oversized window → `413 payload_too_large`
with decode_calls unmoved; **NEW** KV cap honored with zero engine work.

## Honest negatives (first class)

1. **The residual +24–26 ms anti-pattern is NOT eliminated.** A 3-token
   delta decode immediately after [KV rollback + 1-token tail re-eval]
   costs ~35 ms vs ~11 ms steady (phases D/G, before AND after; every
   public-API lever tried — uniform outputs, pooled batch, pinned output
   reservation, warmup — leaves it intact). It does NOT appear in the
   verify loop's own call order (phase M: propose/verify/next-round delta
   all at steady-state cost; verify after a rollback == steady verify),
   and 9-row batch decodes after the same rollback are free (D/M). The
   mechanism inside b11407's CPU backend is not identified; any fix beyond
   usage-order discipline (avoid 1-token re-evals between a rollback and a
   small delta decode) would need engine-side investigation or a future
   ADR. Recorded as residual risk.
2. **The task's named churn — "repeated window verifies of differing
   widths" — does not exist** (phase F, −0.9…+1.2 ms at baseline and
   after). The P16 spike's "graph_reserve shape churn" row compared
   3-token real compute against a 0.15 ms delta-0 no-op; the production
   measurement corrects the record: width alternation was never the cost.
3. **Uniform outputs is not free**: steady 3/0 deltas pay one extra lm_head
   pass (+1.2–1.5 ms per call on this model/vocab; scales with vocab).
   Worth it at the adapter level (slot stability + decode-free steps);
   callers chasing raw prefill throughput can disable it via
   `CapiConfig::with_uniform_outputs(false)` and reproduce the churn.
4. **`llama_state_get_size` on Vulkan measures host-side state only** (0 MiB
   with GPU-offloaded KV). The capacity estimate remains the honest VRAM
   number for offloaded handles; the doc and `kv_usage` field names say so.
5. **Single-machine, single-session, load-sensitive timings** — relative
   comparisons only; the repeat/P50 protocol belongs to the ADR-032 pass-2
   family per ADR-031 §5.

## Gate commands and outcomes (owner-local, 2026-10-11)

```
cargo fmt --all --check                                                   PASS
cargo clippy --workspace --all-targets -- -D warnings                      PASS
cargo clippy --workspace --all-targets --features modelswarm-runtime/capi-adapter -- -D warnings   PASS
cargo test --workspace                                                    374 passed / 0 failed
cargo test --workspace --features modelswarm-runtime/capi-adapter         383 passed / 0 failed
cargo test -p modelswarm-runtime --features capi-adapter --test capi_loopback -- --ignored --nocapture   1/1 PASS (all pins)
cargo test -p modelswarm-runtime --features capi-adapter --test capi_vulkan -- --ignored --nocapture    1/1 PASS (all pins)
cargo test -p modelswarm-runtime --features capi-adapter --test capi_batching -- --ignored --nocapture  1/1 PASS (measurement record)
```

No `llama-server` process remained after any run (ServerGuard + tasklist
verified each time).
