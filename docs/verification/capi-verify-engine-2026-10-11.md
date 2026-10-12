# CapiVerifyEngine — the production BatchVerifyEngine over the capi adapter (2026-10-11 task, run 2026-10-12)

What: `crates/modelswarm-runtime`'s production batched-verify ENGINE
(`CapiVerifyEngine`, feature `capi-adapter`, still OFF by default) asserted
against the pinned engine + SmolLM2-135M-Instruct through the architect's
REAL `modelswarm_session::verifydrafts::BatchVerifyEngine` trait. Nothing
production-visible changed (the feature stays off; the HTTP adapter, the
frozen trait shape, and every foreign crate are untouched).

Run date 2026-10-12 (the task's 10-11 file name kept as instructed),
owner-local, single machine (Windows 11 x64), staged pinned sets
`%LOCALAPPDATA%\ModelSwarm\engine` (CPU, sha256-verified against
`runtime-pins.json` at test start: `llama-server.exe`, `llama.dll`,
`ggml.dll`, `ggml-base.dll`) and
`.../Data/models/msp1_948d898a.../SmolLM2-135M-Instruct-Q4_K_M.gguf`
(100.6 MiB). Environment label: single-machine loopback, dev-session load;
correctness pins only — no performance claims.

## Command

```
MSP_LLAMA_SERVER=%LOCALAPPDATA%\ModelSwarm\engine\llama-server.exe ^
MSP_REAL_GGUF=%LOCALAPPDATA%\ModelSwarm\Data\models\msp1_948d898a6ace88592f19b8e02ccd776f3e09be1049e0809454ebe4ee2f1acc2f\SmolLM2-135M-Instruct-Q4_K_M.gguf ^
MSP_CAPI_ENGINE_DIR=%LOCALAPPDATA%\ModelSwarm\engine ^
cargo test -p modelswarm-runtime --features capi-adapter --test capi_verify -- --ignored --nocapture
```

Outcome: **1 passed / 0 failed** (run twice — once pre-`cargo fmt`, once
post-format — identical results; greedy determinism). No `llama-server`
left running after the run (tasklist-checked).

## Asserted pins (all through the REAL trait call)

| Pin | Observed |
|---|---|
| `capabilities()` from the adapter's real geometry | backend `cpu` (from the configured CPU bundle), `batch_verify true`, `batch_window_max 32` == adapter verify geometry == session `WINDOW_CAP`; `is_valid_disclosure()` true |
| Full-window accept + bonus | `(8, Some(truth[8]))` vs the HTTP ground truth (77-token main prompt, 32 greedy HTTP tokens) |
| I4 one engine pass | exactly **1** engine decode call for the warm verify through the trait (`decode_calls 1 -> 2`) |
| Mid-window reject + correction | corrupt absolute position 13 → `(4, Some(truth[13]))` |
| Continuation parity vs HTTP | next round fully accepts `truth[14..22]` with bonus `truth[22]`; committed stream == `prompt ++ truth[..23]` token-for-token |
| Over-window-cap refusal BEFORE engine work | window 34 → `413 payload_too_large`, **0** engine decode calls, committed prefix unchanged |
| EOG (bonus path) | full acceptance of the 2-token pre-EOG window → `(2, None)`; prefix-only commit (no correction appended); post-EOG verify refuses fail-closed (`MalformedResponse`) |
| EOG (replacement path) | true window + 1 garbage token → reject at the EOG position → `(2, None)`; same prefix-only commit; post-EOG refuse |
| KV token cap honesty | cap 77 tokens: round 1 verifies, round 2 → `429 kv_budget_exceeded`, 0 engine calls, committed prefix unchanged |

Ground-truth construction note: the server never emits the EOS token, so
the EOG position comes from an HTTP `decode_stream` that stopped BEFORE
`max_tokens` (a self-terminated stream stops exactly at EOG) — both the
pre-EOG tokens and the position are HTTP-arm facts, never engine claims.
Prompt choice mattered and is recorded as an honest negative: the first
EOS-prompt candidate (`Say hi.`) produced **36** pre-EOG tokens — over the
`WINDOW_CAP`-bounded window, unusable for both EOG sub-cases; the shipped
prompt (`Reply with exactly one word and nothing else: hi`, 20 prompt
tokens) terminates at position **2**. The test asserts the 1..=31 band and
names the failure mode, so an engine/prompt change surfaces loudly. The
pinned vocab reports three EOG tokens (`<|endoftext|>` 0, `<|im_end|>` 2,
`<reponame>` 4); the engine nulls the correction for ANY `vocab_is_eog`
token and the server stops at any of them, so the two arms agree without
the test hard-coding which one fires.

## Regression re-runs (same staged sets, same session)

- `--test capi_loopback -- --ignored --nocapture` — **1 passed / 0
  failed** (tokenize/stream/detokenize parity, reject-at-0 + mid-window
  k=4 rollback, warm verify = 1 decode call, 413 + 429 caps, KV report:
  1225 tok / cap 4096, estimate 90 MiB, engine state 26 MiB).
- `--test capi_vulkan -- --ignored --nocapture` (engine-vulkan bundle,
  RTX 5080) — **1 passed / 0 failed** (within-backend exactness only; E0
  CPU≠Vulkan respected).
- `--test capi_batching` NOT re-run: it measures adapter call-shape costs
  and no measured path changed (this change is additive: one module, one
  re-export, one dev-dep).

## Workspace gates (owner-local)

- `cargo fmt --all --check` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- `cargo clippy --workspace --all-targets --features
  modelswarm-runtime/capi-adapter -- -D warnings` — PASS.
- `cargo test --workspace` — **459 passed / 0 failed / 6 ignored**
  (matches the stated baseline exactly).
- `cargo test --workspace --features modelswarm-runtime/capi-adapter` —
  **474 passed / 0 failed / 10 ignored**. Crate-level pre/post
  verification (`cargo test -p modelswarm-runtime --features capi-adapter`
  with this change stashed): 32/0/3 → 38/0/4 — the delta is exactly this
  change's additions (+4 unit tests, +1 doctest, +1 disclosure-mapping
  test, +1 env-gated ignored real-engine test); the workspace feature
  number moves by the same +6/+1.

## Trait-gap finding (recorded for the integrator)

The architect's trait cannot be implemented for a runtime-crate type in
the runtime LIB: `modelswarm-session` already depends on
`modelswarm-runtime` (`RuntimeError`/`SamplingParams`), and a reverse lib
dependency is a cyclic package dependency, which Cargo rejects. A
DEV-dependency cycle IS legal, so `tests/capi_verify.rs` implements the
REAL trait for a newtype (`CapiVerifyEngineAsTrait`) of the engine and
runs every pin above through it — that file's newtype + `to_disclosure`
mapping (`VerifyEngineCaps` → `negotiation::EngineDisclosure`) is the
reference pattern for the serve loop. The lib ships a clearly labeled
minimal local mirror (`BatchVerifyEngineMirror`, identical method shapes)
as the compile-proof stopgap. No session-crate file was touched.
