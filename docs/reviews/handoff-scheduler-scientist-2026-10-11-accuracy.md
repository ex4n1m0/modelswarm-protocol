# Handoff — Scheduler Scientist, accuracy-benefits study (2026-10-11)

**Branch:** `main`, commits `bde7eca` (code) + evidence/docs commit, both
pushed; `origin/main` == local verified after each push. Separate from
`docs/reviews/handoff-scheduler-scientist-2026-10-11.md` (the ADR-032
option-A/gate handoff) as tasked.

Task: turn the owner directive recorded at ADR-032 acceptance into a
grounded, ranked accuracy-benefits research plan with at least one
MEASURED proof-of-concept on reachable hardware.

Deliverables:

1. `docs/reviews/accuracy-benefits-study-2026-10-11.md` — the study:
   7 output-preserving + 4 output-changing candidates enumerated and
   classified, ranked (value x feasibility / contract risk), top-3
   developed into experiment proposals with comparators and
   honest-negative criteria; B-list (output-changing) named as
   ADR-gated future work only.
2. Measured proof-of-concept — cross-peer greedy divergence detection,
   on THREE venues: loopback (real QUIC), the real two-machine LAN
   (machine B), and the real pinned engines (b11407 SmolLM2-135M).
3. The instrument wired into my owned crate with tests (per-push CI
   pin) — it fit cleanly; nothing was forced.

## Headline measurements

- Structural (synthetic executors, inject/detect/report/de-rank):
  clean cells **0/24 false positives**; injected cells **8/8 detection
  at divergence position 0** (verifier-reference AND reference-free
  quorum), offender-only suspension, cohort de-rank
  (V,D,offender)->(V,D,M2). Identical shape on loopback and LAN.
- Real engines: **cross-machine CPU greedy determinism HOLDS on this
  tuple** (Raptor Lake-HX vs Coffee Lake, byte-identical b11407
  binaries, cache-cold, 8/8 prompt pairs byte-identical) — first data
  point on the E0 open sub-question that gates real-draft pass 3 and
  ADR-032 §3 CPU-family auditing.
- Real fault caught live: the pinned llama.cpp HTTP server flips the
  greedy continuation on REPEAT requests (prompt-cache state) on a
  near-tie prompt — two attractors splitting at character 3,
  reproduced 3/3, minority digest always at request 0. E0's
  single-shot design could not see this class. Extracted design rule:
  divergence checks / audit re-executions must run cache-cold.
- Racing envelope (same completions, no extra requests): median tail
  saved 69–97 ms across cells — structural shape only, and
  output-preserving only modulo the nondeterminism above.

## Changed files and why

- `crates/modelswarm-bench/src/divergence.rs` (NEW) — the instrument:
  `first_divergence`, `stream_digest` (sha256-16, digests never token
  text), `AgreementLedger` (EWMA at the frozen `EWMA_ALPHA`,
  suspend-after-first-divergence), `quorum` (strict-majority
  reference-free read; ties inconclusive), self-describing record
  types (`divergence-check/v1`, `accuracy-manifest/v1`,
  `cohort-derank/v1`) — the frozen `experiments/schemas/` set is
  untouched.
- `crates/modelswarm-bench/src/lib.rs` — module registration.
- `crates/modelswarm-bench/src/runner.rs` — `accuracy_pool()` (pinned
  4-bridge pool, fixed seeds), `bridge_peer_id()` (identity
  cross-check), environment labels.
- `crates/modelswarm-bench/examples/pass1_serve.rs` — `--pool accuracy`
  + `--divergent <indexes>` (serve-side fault injection; refuses
  index 0, the reference).
- `crates/modelswarm-bench/tests/accuracy.rs` (NEW) — shared cell
  driver + per-push CI pin `divergence_detection_over_real_quic` +
  owner-gated loopback and LAN drivers (LAN self-checks detection ==
  serve-side ground truth; cross-checks every dialed peer id).
- `crates/modelswarm-bench/Cargo.toml` — `[[test]] accuracy`
  (required-features quic-runner).
- `experiments/raw/ACCURACY-DIVERGENCE-LOOPBACK-2026-10-11/**` (NEW) —
  clean+injected artifacts + ANALYSIS.
- `experiments/raw/ACCURACY-DIVERGENCE-LAN-2026-10-11/**` (NEW) —
  same, with the full B-side ops record.
- `experiments/raw/ACCURACY-DIVERGENCE-REALENGINE-2026-10-11/**`
  (NEW) — driver scripts (python + preserved B-side PowerShell), jsonl
  results, ANALYSIS.
- `experiments/processed/aggregate-accuracy.py` (NEW) +
  `experiments/processed/ACCURACY-2026-10-11-summary.json` (NEW).
- `docs/reviews/accuracy-benefits-study-2026-10-11.md` (NEW) — the
  study.
- `docs/reviews/handoff-scheduler-scientist-2026-10-11-accuracy.md`
  (this file).

## Exact commands and outcomes

Gates (at `bde7eca`):

- `cargo fmt --all --check` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- `cargo clippy -p modelswarm-bench --features quic-runner
  --all-targets -- -D warnings` — PASS.
- `cargo test --workspace` — **374 passed / 0 failed** (363 baseline +
  11 divergence module tests).
- `cargo test -p modelswarm-bench --features quic-runner` — **79
  passed / 0 failed** (59 lib + 1 accuracy CI + 18 harness + 1 smoke;
  6 owner-gated ignored: 4 prior + 2 new accuracy drivers).

Experiment runs (exact commands in each ANALYSIS):

- Loopback committed run (clean + injected): PASS, artifacts committed.
- LAN: fresh release exe `c621fbbe…` (sha256 verified after scp) on B
  in a SEPARATE v7 folder + `MSPPassAcc` task; one fresh serve per
  cell (deterministic request ids); clean then injected
  (`--divergent 2`); both driver self-checks PASS.
- Real engine: matrix (12 comparisons), p1-long characterization
  (2x10), fresh-pair probe (4), A-side cache-cold (8), B-side
  cache-cold (8) — numbers above and in the ANALYSIS.
- `python experiments/processed/aggregate-accuracy.py experiments/raw
  experiments/processed/ACCURACY-2026-10-11-summary.json` — PASS.

Concurrent-session note: the Runtime seat's in-flight WIP
(`crates/modelswarm-runtime/src/capi/ffi.rs`, mid-edit several times)
transiently broke workspace compilation during my LAN run window; the
LAN driver was therefore built and run from a clean `git worktree` at
`bde7eca` (identical commit to the deployed serve exe). All four gates
were run against the main working tree once it compiled again, before
the code commit.

## Machine B, final state (verified)

0 `pass1_serve`/`llama-server` processes; task `MSPPassAcc` deleted;
folder `ModelSwarm-ACC-v7` (incl. engine+model copies) removed;
firewall rule `ModelSwarm ACC v7` removed (v2/v4 rules intact);
`MSPPass1` Ready; v4 folder intact; desktop back to v2/v4 + zips.

## Assumptions

1. The divergence instrument is harness/experiment surface (my owned
   crates) — wiring it into the production node (periodic canaries, a
   divergence store beside `AcceptanceStore`) is Proposal 1 work, not
   silently done here.
2. The self-describing accuracy record types deliberately do NOT touch
   the frozen `experiments/schemas/` (ADR-gated); they are documented
   in-module and in the ANALYSIS files.
3. The B-side PowerShell leg ran llama-server on `127.0.0.1` on B
   (rule 4 discipline); the model was scp'd machine-to-machine for the
   experiment — a direct copy for measurement, not a ModelSwarm
   redistribution path (constraint 3 untouched).
4. Cross-machine determinism is claimed ONLY for the measured tuple
   (build b11407, SmolLM2-135M Q4_K_M, 4 synthetic prompts, 2 machines,
   cache-cold); the study text says exactly this.

## Unresolved risks

- The real-engine cache-flip finding means ANY future audit/spot-check
  path (ADR-032 §3) must specify cache-cold re-execution or it will
  false-alarm on near-tie prompts; the VerifyDrafts implementation plan
  (a2b8a3f) should absorb this rule — routed to the Architect in the
  suggested next task.
- Suspend-after-first-divergence has no recovery policy (same posture
  as the engaged-loss gate); Proposal 1 adds the cache-cold re-check
  before suspension, but a reviewed decay policy is still open.
- The cross-machine determinism data point is ONE tuple; Proposal 2
  exists precisely because the general question is open.
- The p1-long attractor behavior may be model/quant-specific; whether
  near-tie prompts of this class exist for the production profiles is
  unknown until Proposal 2 sweeps them.

## Suggested next task for the integrator

Route two things to the Protocol Architect with this evidence
attached: (1) the cache-cold audit rule as a required-change note for
the ADR-032 VerifyDrafts implementation plan (audit-path re-execution
must be cache-cold; this study measured why); (2) whether determinism
certification (Proposal 2) should eventually surface as an eligibility
input (new ADR) or stay a local requester decision. Inside my paths,
the next task is Proposal 3's (profile, verify-term) EWMA keying with
the committed pass-2 replay pins.
