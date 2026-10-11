# Handoff — Runtime Engineer, ADR-031 productionization (2026-10-11)

Scope: ADR-031 §5 gate items 1–5 for the in-process C-API adapter
(`crates/modelswarm-runtime`, feature `capi-adapter`, **still OFF by
default** — productionization = safe-to-turn-on). ADR-032 (accepted
2026-10-11) names this gate as the hard dependency for anything runnable in
its family; this handoff clears it. Verification record with all numbers:
`docs/verification/adr-031-productionization-2026-10-11.md`.

Commits on `main`: `5f8280e` → amended to `48536d2` (implementation +
tests), plus this documentation commit. All gates green at each commit.

## 1. Changed files (paths) and why

- `crates/modelswarm-runtime/src/capi/mod.rs` — the productionization core:
  `Engine`/`EngineInner` cloneable core so every blocking engine call can
  ride `tokio::task::spawn_blocking` (item 2); `uniform_outputs` + pooled
  per-handle batch + `n_batch = n_ctx` + pinned `n_outputs_max` + load-time
  warmup (item 1); `kv_token_cap` / `kv_byte_budget` / `kv_usage()` /
  `estimate_kv_bytes` (item 3); `verify_window_max` (ADR-032 WINDOW_CAP)
  with pre-engine-work `413 payload_too_large`; one-bundle-per-process
  fail-closed guard (ADR-031 §2); per-handle `decode_calls` (the I4
  one-pass counter); new unit tests.
- `crates/modelswarm-runtime/src/capi/ffi.rs` — `PooledBatch` (one
  `llama_batch` per handle, freed on drop), `decode_pooled`,
  `Logits::promote` (uniform-output promotion), `memory_seq_rm_all`,
  `llama_state_get_size` + GQA dimension getters
  (`llama_model_n_layer/n_embd/n_head/n_head_kv`) required-symbol pins,
  `unsafe impl Send/Sync` moved to `EngineInner` (same confinement).
- `crates/modelswarm-runtime/src/lib.rs` — `RuntimeMetrics` gains additive
  `kv_bytes_estimate` + `engine_decode_calls` (Default-0; no exhaustive
  literals outside this crate — checked).
- `crates/modelswarm-runtime/src/mock.rs` — the one exhaustive
  `RuntimeMetrics` literal updated with the documented zeros.
- `crates/modelswarm-runtime/Cargo.toml` — tokio `rt` feature (spawn_blocking).
- `crates/modelswarm-runtime/tests/capi_batching.rs` (NEW) — the fixed-call-
  sequence shape-stability measurement (phases A–M) with before/after
  comparability; env-gated `#[ignore]`d like the other real-engine tests.
- `crates/modelswarm-runtime/tests/capi_loopback.rs` — extended pins:
  mid-window reject at k=4 + single-mode continuation token-for-token, warm
  verify == ONE engine decode call (I4), oversized-window 413 with zero
  engine work, KV-cap 429 with zero engine work, KV usage/estimate
  cross-check, metrics print extended; reject-path corruption made
  prompt-independent (see §Assumptions).
- `crates/modelswarm-runtime/tests/capi_vulkan.rs` (NEW, item 4) — real
  in-process Vulkan test vs a Vulkan llama-server ground truth (within-
  backend exactness only; E0 CPU≠Vulkan respected).
- `docs/verification/adr-031-productionization-2026-10-11.md` (NEW) — the
  gate record with every measured row and the honest negatives.

NOT changed: `runtime-pins.json`, the production HTTP adapter, the frozen
trait shape (`verify_drafts` stays an inherent method), node/bench/gateway/
desktop, `.github/workflows` (verified no gap — see §5), any foreign path.

## 2. Exact commands run and outcomes

Gates (owner-local, Windows 11 dev machine):

- `cargo fmt --all --check` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- `cargo clippy --workspace --all-targets --features
  modelswarm-runtime/capi-adapter -- -D warnings` — PASS.
- `cargo test --workspace` — **374 passed / 0 failed**.
- `cargo test --workspace --features modelswarm-runtime/capi-adapter` —
  **383 passed / 0 failed**.

Real-engine (env-gated `#[ignore]`d; staged pinned bundles +
SmolLM2-135M-Instruct Q4_K_M from the local store; sha256s verified against
`runtime-pins.json` at test start; no llama-server left running after any
run — tasklist-checked):

- `cargo test -p modelswarm-runtime --features capi-adapter --test
  capi_loopback -- --ignored --nocapture` — 1/1 PASS, every exactness pin
  (tokenize/stream/detokenize/eos/verify/reject-at-0/NEW mid-window
  k=4 + 12 single-mode tokens token-for-token/NEW one-pass/NEW 413+429).
- `cargo test -p modelswarm-runtime --features capi-adapter --test
  capi_vulkan -- --ignored --nocapture` — 1/1 PASS on the RTX 5080 (backend
  registration via the engine-vulkan bundle; within-backend exactness).
- `cargo test -p modelswarm-runtime --features capi-adapter --test
  capi_batching -- --ignored --nocapture` — 1/1 PASS; run BEFORE the change
  (pre-change adapter) and AFTER (three runs); full tables in the
  verification doc.

## 3. The measured batching answer (what improved, what did not)

- **The thing the gate named — verify calls of differing widths — was
  already free** (widths 5/9/13/17 alternating vs steady: −0.9…+1.2 ms,
  before AND after). The P16 "graph_reserve shape churn" row compared
  3-token compute against a 0.15 ms no-op sync; corrected here.
- **What improved:** (a) steps after a Last-logits delta are now
  decode-free (0.32–0.39 ms vs 5.4–5.8 ms — the tail-logits re-materialization
  is skipped); (b) steady-state calls perform zero batch allocation (pooled
  batch) and never grow the output buffer (warmup pre-sizing); (c) the
  >2048-token prefill abort landmine (`n_tokens ≤ n_batch` GGML_ASSERT) is
  gone (`n_batch = n_ctx`); (d) the production verify loop (propose →
  verify → next-round delta) measured end-to-end at steady-state cost with
  zero churn (phase M).
- **What did NOT improve (honest negative):** a 3-token delta decode
  immediately after [KV rollback + 1-token tail re-eval] still costs
  ~+24–26 ms (D/G phases). No public-API lever removed it; ruled out
  empirically: hole-filling (H2−H1 ≈ 0), rollback itself (0.4 ms), 1→k
  transitions without rollback (free), any verify/batch shape (free). It is
  absent from the verify loop's call order; usage discipline (avoid 1-token
  re-evals between a rollback and a small delta) is the documented
  mitigation; engine-side investigation is a follow-up if any caller needs
  that exact pattern fast.
- **Uniform outputs costs** one lm_head pass per prefill delta (+1.2–1.5 ms
  on this model) — the deliberate price of slot stability; disable via
  `with_uniform_outputs(false)` to reproduce the churn in measurements.

## 4. Assumptions

- tokio runtime required for the adapter (all research consumers run
  tokio); `LlamaCppCapi::new` stays synchronous (library + model load) like
  the HTTP adapter's constructor.
- The KV byte estimate is capacity-based f16 (the engine's default K/V
  types) from public GQA dims; `kv_byte_budget` enforcement uses it as an
  upper bound (engine-reported `state_bytes` cross-checks usage). On Vulkan
  with offloaded KV, `state_bytes` measures host-side state only (recorded).
- One model per adapter, handles never evicted (trait has no unload) — the
  byte budget is therefore monotonic within an adapter lifetime; the cap
  still bounds per-handle KV (the session-growth concern the gate named).
- The reject-path corruption in tests was made prompt-independent after the
  Vulkan run showed a shift-by-one draft legitimately accepting (repeated
  first token in that greedy continuation).
- b11407 source files (`llama-context.cpp`, `llama-graph.cpp/h`,
  `llama-kv-cache.cpp`) were fetched READ-ONLY from the pinned tag for
  mechanism analysis — reference material only, no remote code executed
  (same precedent as the spike's header derivation).

## 5. CI coverage verification (gate item 5)

`rust.yml` runs on every push (paths include `crates/**`): workspace fmt,
both clippy configurations (default + `capi-adapter`), and both test
configurations. The new unit tests run in the capi job; the three
real-engine tests are `#[ignore]`d env-gated BY DESIGN (ADR-032 §7 venue
matrix — GitHub runners never stage engines; owner-local runs +
`docs/verification/` records are the venue, which this handoff provides).
**No gap found; no workflow change made** (also T/R-owned).

## 6. Unresolved risks

- The residual 3-token-after-[rollback+1/1] penalty (negative #1) — unknown
  engine-internal cause; flagged for any future engine-pin bump
  investigation.
- Cross-machine CPU determinism remains E0's open sub-question and still
  gates real-draft pass 3 (scheduler handoff 2026-10-10) — nothing here
  changes that.
- `spawn_blocking` serializes per handle via its mutex; cross-handle
  parallel decode is now possible (separate contexts share the read-only
  model) but was NOT measured under concurrency — the KV byte budget is the
  RAM guard, not a throughput claim.
- Exactness pins are build-sensitive: every `runtime-pins.json` engine bump
  re-runs them (ADR-032 §7 standing rule).

## 7. Concurrent-workspace note (for the integrator)

Another agent worked in this tree during my run (Scheduler Scientist:
`modelswarm-bench` WIP landed mid-session as `bde7eca`, and untracked
`experiments/…ACCURACY…` files remain). Nothing of mine touched their
paths; my commits contain only the files in §1. The transient
`modelswarm-bench` breakage I observed mid-session (missing
`tests/accuracy.rs` + a `//!` typo in an untracked file) was their
in-flight state, resolved by their own commit — main was never left broken
by me.

## 8. Suggested next task for the integrator

ADR-031's gate is cleared with evidence. Before the ADR-032 family lights
up, the remaining (non-runtime) items are the Protocol Architect's:
`VerifyDrafts`/`VerifyDraftsResult` in msp-cooperative-v1 + the §7 golden
vectors + `SessionAccept.engine` disclosure fields, then the Scheduler
Scientist's pass-2-style consumption of `verify_drafts` behind the still-OFF
feature flag. Runtime's readiness statement: the adapter is safe to enable
for that work (spawn_blocking-clean, KV-bounded, shape-stable in the verify
loop, exact on CPU and Vulkan within-backend, one-pass verify pinned); the
flag itself must stay off in production until that family's own gates pass.
