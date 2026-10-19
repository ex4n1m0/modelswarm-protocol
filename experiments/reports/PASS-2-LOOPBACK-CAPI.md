# PASS-2-LOOPBACK-CAPI — 9.6 pass 2 over the capi-composed path (2026-10-19)

**ENVIRONMENT LABEL: single-machine loopback, SmolLM2-135M, greedy,
dev-session load. NOT LAN. NOT a product performance claim.** The LAN variant
(wire authenticity, 2 machines) remains a separate owner-gated pass.

## What ran

- Harness: `crates/modelswarm-runtime/tests/capi_pass2.rs` (owner-local,
  env-gated `#[ignore]`, `--features capi-adapter`). Three arms over ONE
  pinned engine + model + prompt class, per-run greedy bit-parity asserted
  against the truth generator for BOTH speculative arms:
  - **B1 `single-http`** — the production fastest-single path shape:
    sequential per-token completions over llama-server HTTP, each step
    re-posting the full prefix (`no_prompt_cache: true`, see F2 — the
    determinism requirement, and the honest structural cost both HTTP arms
    pay by design).
  - **B2 `wire-spec-http`** — the pass-1 wire protocol shape (propose k +
    per-position verify decodes, all over HTTP).
  - **A `spec-capi`** — ADR-031 in-process composition: k one-token
    in-context propose decodes + ONE batched verify decode per round via
    the C-API adapter, fresh context per run.
- Protocol: windows k ∈ {4, 8, 16, 32}; 30 measured runs/cell + 1 discarded
  warmup; 32 target tokens/run; one llama-server PER CELL (F4); every
  completed run appended to a durable JSONL the moment it finishes (F4/F5).
- Model/engine: SmolLM2-135M-Instruct Q4_K_M (`msp1:948d898a…`), pinned
  engine `b11407` (canonical_build_hash `353c4aab…`), sha256-verified
  bundle, ctx 4096, 24 logical CPUs.

## Exact commands (Git Bash, Windows, repo root)

```bash
CARGO_TARGET_DIR="$PWD/crates/modelswarm-runtime/target" \
MSP_PASS2_RUNS=30 \
MSP_LLAMA_SERVER="$LOCALAPPDATA/ModelSwarm/engine/llama-server.exe" \
MSP_REAL_GGUF="$LOCALAPPDATA/ModelSwarm/Data/models/msp1_948d898a…/SmolLM2-135M-Instruct-Q4_K_M.gguf" \
MSP_CAPI_ENGINE_DIR="$LOCALAPPDATA/ModelSwarm/engine" \
cargo test -p modelswarm-runtime --features capi-adapter --test capi_pass2 -- --ignored --nocapture
# after a crash, resume completed runs instead of re-measuring:
MSP_PASS2_RESUME=target/pass2-capi/pass2-runs-<stamp>.jsonl  # + the same gate vars
```

Observed (2026-10-19 20:11–21:57, this machine): 120/120 measured runs,
all four per-cell servers alive at cell end, wall ~105 min.

## Headline numbers (loopback — NOT LAN, NOT product)

| window | B1 single-http P50 | B2 wire-spec P50 | A spec-capi P50 | P95(A) | accept | B1/A P50 | B2/A P50 |
|---|---|---|---|---|---|---|---|
| k=4  | 15044.4 ms | 29888.7 ms | **1339.3 ms** | 1542.5 | 100.0% | 11.23× | 22.32× |
| k=8  | 16297.4 ms | 34689.0 ms | **1373.0 ms** | 1427.9 | 100.0% | 11.87× | 25.26× |
| k=16 | 16861.0 ms | 34509.0 ms | **1333.7 ms** | 1438.9 | 100.0% | 12.64× | 25.88× |
| k=32 | 15735.2 ms | 32085.4 ms | **1344.6 ms** | 1458.1 | 100.0% | 11.70× | 23.86× |

Acceptance is 100% in every cell — the filler-continuation regime drafts from
the same greedy engine that verifies, so drafts self-agree perfectly. That is
the BEST case for speculation and is labeled as such; real-prompt acceptance
is lower and the planner (ADR-027) exists to pick windows by measured
acceptance.

## Verdict

1. **The in-process composed path (A) beats the B1 fastest-single comparator
   11.2–12.6× P50 in this loopback regime, at every window size.** The gap is
   dominated by the boundary, not the speculation: A does zero HTTP round
   trips and zero re-prefill per token (one batched verify per round),
   while B1/B2 re-post and re-prefill the whole ~1200-token prefix EVERY
   step (`no_prompt_cache` determinism — F2). Production keeps the prompt
   cache, so production B1 is faster than this B1; the clean claim is the
   structural one: in-process batched verify removes both the per-token
   round-trips and the re-prefill, and that is worth 11–13× P50 here.
2. **B2 wire-spec is ~2× SLOWER than plain B1 at every window** even with
   100% acceptance: per-position verify HTTP calls cost more than they
   save. Numbers now stand behind ADR-032 §4's wire-fallback rationale —
   speculative cooperation only pays in-process (or with a wire protocol
   that batches verification).
3. Window size barely moves A (1334–1373 ms P50 across k=4..32) at 100%
   acceptance — consistent with one batched verify per round absorbing the
   draft length; the k-choice matters for real acceptance regimes, not this
   one.
4. Exactness: every run, every window, both speculative arms
   token-for-token equal to the greedy truth.

M10 exit-rule context ("every cooperative mode reproducibly beats
FASTEST_SINGLE or falls back"): in this labeled loopback regime the
capi-composed path beats it at every window. LAN pass still owner-gated.

## Findings (getting a clean run was itself the experiment)

- **F1 — eos-terminal truth.** The truth generator must STOP at eos
  (propose and production streams do); stepping past it makes the
  speculative arms' eos-terminated commits unmatchable by construction.
- **F2 — llama-server cached-prefix reuse makes greedy output
  history-dependent.** Under a verify pattern that probes backward relative
  to a slot's furthest processed state, the server's prompt-cache reuse
  changes answers (observed: draft `[32,30,32,32]` vs truth `[32,32,42,32]`
  on identical prefixes). Fixed in-experiment with an additive
  `LlamaCppConfig.no_prompt_cache` (default false, production untouched;
  `decode_step` posts `cache_prompt:false`). Independent reinforcement of
  ADR-032 §4.
- **F3 — near-tie argmax instability constrains exactness-pinned
  methodology.** "Run number N:"-style prompts sit on near-ties (e.g.
  token 30 vs 32 vs 42 within noise) that flip between compute paths
  (full re-prefill vs incremental vs batched verify). Cross-engine
  bit-parity only holds in wide-margin prompt regimes — the harness
  decorrelates runs by varying trailing FILLER count (7 phases), staying
  inside the proven-stable filler-continuation class.
- **F4 — run-5 crash: one shared llama-server for all 124 runs went
  unreachable at ~75 min.** Its stderr was piped to `Stdio::null()` (no
  evidence), the B2 unwrap panicked, and the process died with
  STATUS_ACCESS_VIOLATION during panic teardown. All in-memory samples were
  lost. Harness since: one llama-server per cell, server stderr to
  `server-w<k>-<stamp>.log`, every completed run appended to
  `pass2-runs-<stamp>.jsonl` immediately, `MSP_PASS2_RESUME` folds a
  partial JSONL back in. Run 6 then completed all 120 runs with every
  per-cell server alive at cell end.
- **F5 — exit-time STATUS_ACCESS_VIOLATION after successful completion.**
  Run 6 delivered the full table, the aggregate JSON, and the 120-line
  JSONL, and only THEN died 0xC0000005 at process exit (same native
  teardown class as run 5's panic path; likely C-API backend state at
  exit). Harness-level only — flagged to the Runtime seat; no product path
  tears the engine down like this, and the artifacts land before the crash.

## Artifacts

- `experiments/raw/PASS-2-LOOPBACK-CAPI-2026-10-19/pass2-capi-20261019-201126.json` — aggregates (the table above).
- `experiments/raw/PASS-2-LOOPBACK-CAPI-2026-10-19/pass2-runs-20261019-201126.jsonl` — all 120 per-run records (window, run, want, per-arm ms, accepted/proposed).
- Per-cell server logs retained untracked at
  `crates/modelswarm-runtime/target/pass2-capi/server-w{4,8,16,32}-20261019-201126.log` (~2.9 MB each).
