# ACCURACY-DIVERGENCE-REALENGINE (2026-10-11) — ANALYSIS

Real-engine validation of the divergence-detection instrument + the
FIRST cross-machine CPU determinism data point on this stack (the E0
open sub-question). Part of the accuracy-benefits study
(`docs/reviews/accuracy-benefits-study-2026-10-11.md`).

## Setup

- Engines: the pinned llama.cpp b11407 bundles
  (`runtime-pins.json`, canonical build hash `353c4aab…`); CPU variant
  `%LOCALAPPDATA%\ModelSwarm\engine\llama-server.exe` and Vulkan variant
  `%LOCALAPPDATA%\ModelSwarm\engine-vulkan\llama-server.exe`. The
  `llama-server.exe` binaries on machines A and B are byte-identical
  (sha256 `2fd854dfcc0e39da30410bbefbc28d6816f733183f1234cd6c660e078aa8789e`
  on both).
- Model: `SmolLM2-135M-Instruct-Q4_K_M.gguf`
  (`msp1_948d898a6ace88592f19b8e02ccd776f3e09be1049e0809454ebe4ee2f1acc2`,
  105,454,432 bytes), already staged on A; scp'd to B for the
  cross-machine leg (direct machine-to-machine copy for the experiment —
  no ModelSwarm redistribution path involved).
- Machines: A = Intel Raptor Lake-HX (Family 6 Model 198) laptop; B =
  Intel i9-9900K (Coffee Lake). Different CPU microarchitectures —
  exactly the case E0 flagged as untested.
- Sampling: `/v1/completions`, `temperature 0, seed 7, max_tokens 48`
  (the E0 discipline). Prompts: the frozen `synthetic-pass1-v1` corpus
  (4 synthetic texts, no user content).
- All engines bind `127.0.0.1` only (AGENTS.md rule 4); B's requests
  were issued ON B by a PowerShell script over ssh.

Commands (exact):

```text
# 3-instance matrix + repeat checks (machine A)
python realengine_divergence.py            # -> realengine-checks.jsonl
# p1-long repeat characterization (machine A, 2 fresh CPU instances x 10)
#   inline driver, same module imports -> realengine-p1long-reps.jsonl
# fresh-pair probe (machine A, 1 fresh instance, same prompt x4)
#   inline driver -> realengine-p1long-fresh-pair.jsonl
# A-side cache-cold rows (fresh process per prompt, 2 rounds)
#   inline driver -> realengine-crossmachine-acold.jsonl
# B-side cache-cold rows (fresh process per prompt, 2 rounds; ON B)
powershell -ExecutionPolicy Bypass -File C:\Users\Admin\Desktop\ModelSwarm-ACC-v7\bside.ps1
#   (script copied to B for the run; its RESULT lines are the b1/b2 rows
#    consumed by the aggregator; B-side script deleted at cleanup)
```

## Results (script-computed; `experiments/processed/ACCURACY-2026-10-11-summary.json`)

### 1. Cross-machine CPU, cache-cold (the E0 open sub-question)

Fresh process per prompt, first request only (cache-cold), 2 instances
per machine, 4 prompts: **8/8 A-vs-B pairs byte-identical** (and within
each machine a1==a2, b1==b2 — 16 decodes, one digest per prompt across
all four instances). Raptor Lake-HX and Coffee Lake agree on every
prompt of this corpus at temperature 0.

| Prompt | sha256-16 (all 4 instances) |
|---|---|
| p1-short | `b8d0d0b4867c6407` |
| p1-medium | `dbbce3bd4c9ba6ba` |
| p1-long | `868701fe9a3958d9` |
| p1-xl | `dbbce3bd4c9ba6ba` |

### 2. Within-process repeat (cache-hit): a REAL caught divergence

On `p1-long`, the FIRST decode of a fresh process and every REPEAT
decode land in two different greedy attractors that split at character
3 (shared prefix `100`, then `100% of the …` [139 chars] vs
`1000 words; …` [83 chars]):

- 3-instance matrix: the instrument's within-process repeat probe
  (`cpu-a#2` vs `cpu-a#1`) flagged `diverge` at first-diff char 3 —
  caught live.
- Characterization (2 fresh CPU instances x 10 sequential requests):
  BOTH instances 9x majority digest + 1x minority digest, minority
  ALWAYS at request 0 (`realengine-p1long-reps.jsonl`).
- Fresh-pair probe (fresh instance, p1-long x4, nothing else):
  digest sequence `[868701fe, dbbce3bd, dbbce3bd, dbbce3bd]`
  (`realengine-p1long-fresh-pair.jsonl`).
- The other three prompts are repeat-stable (12/12 repeat comparisons
  agree).

Interpretation: the pinned llama.cpp HTTP server's request-state/prompt
cache changes the greedy continuation for repeat requests on
near-tie prompts — real, state-dependent nondeterminism on the
production HTTP adapter path. E0's single-shot design (one run per
config) could not see this class; repeated cross-checking (what a swarm
can afford and a single deployment cannot) exposed it. Consequence for
detection design (and ADR-032 §3's audit path): divergence checks and
audit re-executions MUST run cache-cold (fresh process/context or a
cache bypass) or the check itself can flip the answer.

### 3. Vulkan control

`vulkan-a` (RTX 5080, `-ngl 99`) vs CPU reference: 3/4 prompts
byte-identical at temperature 0 on this corpus; `p1-long` produced the
cache-warm attractor (`dbbce3bd…`, first-diff char 3) — flagged
`diverge` by the instrument. On E0's different prompt set CPU != Vulkan
from the first token; on this corpus the two backends mostly agree and
the one observed split is consistent with request-state differences.
Honest statement: backend-family pairing (E0/ADR-032 §3) remains the
sound policy; this control shows the instrument fires on real
backend/state differences and the agreement rate is corpus-dependent.

## Findings

1. **Cross-machine CPU greedy determinism holds** on this tuple (build,
   model, quant, corpus, OS pair) — first data point toward unblocking
   the E0 open sub-question that gates real-draft pass 3 / ADR-032 B
   CPU-family auditing.
2. **The instrument caught a real fault class single-shot QA misses**:
   state-dependent greedy output on repeat requests (prompt-cache path),
   reproducible 3/3 tries.
3. **Detection requires cache discipline** — measured, not hypothesized.

## NOT claimed

- NOT a general cross-machine determinism answer: 1 model, 1 quant, 4
  synthetic prompts, 1 OS pair, 2 machines. It de-risks this tuple only;
  the E0 open sub-question stays open for other tuples.
- No latency claims; the matrix/vulkan runs measured agreement, not
  speed (timings recorded but not claimed).
- Completions are from synthetic prompts; digests and 12-char prefixes
  are published, never full texts (rule-5 discipline).

## Reproduce

Scripts and jsonl artifacts in this directory; run at any commit
(the scripts are engine-side, not cargo): `python
realengine_divergence.py` on A; `bside.ps1` content is preserved in the
git history of this ANALYSIS's commit (B-side copy deleted at cleanup).
