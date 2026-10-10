# Handoff — Scheduler Scientist (2026-10-10)

**Branch:** `main` (commits `db88abc` → `4faa6c0` → `3dbc161`, all pushed;
`origin/main` verified equal after each push).

Scope: the two tasked items — (1) the 9.6 shadow decision-join as a LEFT
join on realized rows, and (2) 9.6 pass 2: making speculative arms
measurably ENGAGE, honestly, with committed evidence on loopback AND the
real two-machine LAN.

---

## Task 1 — shadow divergence join LEFT-joins realized-only rows (`db88abc`)

`f4472f3` added `sched.shadow.realized served_by=failed` rows for every
total-failure path in `try_swarm_chat`; pre-plan failures (tracker/
lookup/identity) mint fresh traces with **no** `sched.shadow.decision`
row. No join over the two telemetry streams existed yet — an inner join
would drop exactly those rows. Built it as a LEFT join:

- `crates/modelswarm-bench/src/shadow_join.rs` (NEW, default features):
  `join_shadow_telemetry(lines)` parses telemetry JSONL, keeps EVERY
  realized row (`has_plan=false`, plan fields `None` when no decision
  row shares the trace), counts and visibly reports decision-only rows
  dropped (`decision_only_dropped`), maps the emitters' sentinels
  (`none`/`unmeasured`/`unknown`) to `None`, counts (never fatals)
  traceless/unparsable lines, and attaches one plan to every duplicate
  realized trace. 4 unit tests; the pin is
  `realized_rows_survive_without_a_plan_row`.
- `crates/modelswarm-bench/examples/shadow_join.rs` (NEW): CLI over a
  real node telemetry file (stats to stderr, joined JSONL out).
- `crates/modelswarm-bench/src/lib.rs`: module registration.

## Task 2 — 9.6 pass 2: engagement (`4faa6c0` + evidence `3dbc161`)

### Option chosen and why

Option (a) matching-seed synthetic mode, extended beyond seed parity to
a **dialable acceptance profile**. Reasons recorded against (b)/(c):
(b) C-API real-verifier cells measure real verify cost — but P16 already
measured that (33–36 ms one batched decode; 1.45× on 2-round k=8 with a
real model), the runtime crate is not mine to touch, and with synthetic
proposers drafts cannot match so it cannot produce engagement — defer.
(c) real engines on both machines needs real-serve support (Runtime's
path), a ~100 MB model on B, and leaves the E0 cross-machine determinism
question open — stretch, deferred with a recommendation (below).

What engagement actually required (the pass-1 diagnosis was wrong — see
record correction): the pass-1 LAN arms fell back at the **engage gate**
(every fallback row's reason string says so), because engagement needs a
cohort whose drafter is much faster than the fastest single, and the
pass-1 pools were symmetric/slow-drafter shaped. Two pieces landed:

1. **Prefix-match seed family** (`runner.rs`): a proposer request seed
   encodes a match length m; a non-divergent synthetic executor emits
   the reference (verifier) continuation for its first m tokens, then
   its own stream. Acceptance stays protocol-level and MEASURED: real
   propose request → real verifier request over the real transport →
   client-side leading match → acceptance-EWMA per proposer. Divergent
   peers never match (E0 model preserved).
2. **Engagement pool** (`pass1_serve --pool pass2`, mirrored by
   `pass2_pool()` in tests): V = fastest single (prefill 3.0, decode
   0.10 tok/ms, free), D = drafter (decode 4.0 = 40× V, queue 300 so it
   is drafted from, never the comparator), M/M2 busy mids — the bench
   "asymmetric hardware" shape that lets the frozen engage rule
   legitimately pass.
3. **Geometric acceptance profiles** (`AcceptanceRegime::Geometric`):
   per-token match draws keyed (round, rep+prompt) — the classic
   speculative model. Correct semantics documented and pinned: implied
   per-round RATE = p(1−p^w)/((1−p)w) ≈ 0.64/0.42/0.27 at p=0.9/0.8/0.7
   (w8); my first "rate ≈ p" expectation failed its own distribution
   test and was corrected in-code before commit.
4. **Recorded guard relaxation** (`MSP_BENCH_LOSS_MULTIPLIER`, default
   production 2.0 unchanged; `off` for the engaged-curve instrument):
   every engaged row carries a `LossGuardRecord` with the multiplier,
   round-1 wall, predicted per-round, `fired`, and
   `would_fire_at_default` — a relaxed run can never masquerade as
   guard-passing. Pass-1 behavior is bit-reproducible (defaults).
5. Per-cell acceptance isolation in the pass-2 drivers (fresh harness +
   store per cell): cross-profile EWMA carryover would flip gate-blocked
   profiles into engaged ones — a confound. Window-tagged cell names
   keep pass-1 names unchanged.

### Honest numbers (details + tables in the two ANALYSIS.md files)

- **Loopback** (`experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10/`,
  label `loopback+injected-delay`, 12 cells, 1,920 rows, 0 failures):
  946 engaged completions. Best engaged median vs the prompt-best
  independently-executed single: **1.047×** (the parity corner:
  high/w16, one round, 17 vs 16 verifier tokens); worst **3.579×**.
  geo800/geo700 gate-blocked 27–31/32 after EWMA warm-up; planner
  refused geo800 32/32; production guard would have aborted 911/946
  engaged rows at round 1; engaged realized/predicted 2.9–7.4×.
- **LAN** (`experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/`, label
  `lan-2machine-quic`, 8 cells, 30 reps/arm, 4,800 rows, 0 failures,
  1,435 s): 1,982 engaged completions over real QUIC + Noise + ADR-026
  leases. Measured acceptance tracks the dial to three digits
  (geo700-w8 0.275 vs classic-model 0.275, n=2; geo900-w8 0.741, n=372).
  Engaged cells lose 1.38–1.94× (best engaged 1.071× at the parity
  corner; worst 4.257×); mostly-fallback cells 0.95–1.02× — never-worse
  held on a real two-machine run; guard would have aborted 1,857/1,982;
  the deterministic planner was more conservative than fixed-k
  throughout (geo900-w16: 30/120 vs 4/120 fallback).
- **The pass-2 conclusion (negative, published):** NO engaged arm ever
  beat the fastest eligible single on the msp-v1 whole-request wire.
  The wire charges window+1 sequential verifier tokens per round plus a
  full prefix re-post; the frozen model charges 1.5 decode steps —
  realized/predicted 2.8–8.3× across engaged cells. With a wire-true
  verification term, every engagement in both runs would have been
  gate-blocked. Adaptive cohort sizing must not engage speculation on
  this wire; speculative wins require a batch-verify engine (P16's
  measured 1.45× loopback win with a real model is the engine-side
  complement to this wire-side evidence).
- **Record correction (pass-1):** the pass-1 ANALYSIS files' mechanism
  claim ("per-bridge synthetic seeds never match") is wrong — the
  executor folds the REQUEST seed only, so drafts match under seed
  parity (the loopback dry run's ~1.0 acceptance and the 2 engaged
  4-bridge LAN runs both showed it). Pass-1 fell back at the engage
  gate. Pinned by `non_divergent_peers_agree_per_request_seed`; both
  new ANALYSIS files carry the correction.

### Ops (machine B)

Fresh B-side exe required (serve-side behavior changed; wire format
unchanged): built `pass1_serve` release from `4faa6c0`+
doc-fixes (sha256 `d26d487f…`, scp-verified equal), new folder
`ModelSwarm-9.6-LAN-v5` + `serve-headless.bat` (`--pool pass2 --bridges
4 --bind-ip 192.168.100.43`) + scheduled task `MSPPass1V5` (the
documented scheduled-task detach pattern; processes started directly
over ssh die with the session job). Fresh serve per sweep attempt
(deterministic request ids ⇒ `replayed_request` dedup; leases minted
fresh, 1 h). One early sweep attempt failed transiently in warm-up
against a just-started serve — discarded (artifacts deleted), clean
re-run per the ops rule; the ANALYSIS records it. **B left as found:**
serve stopped (`Get-Process pass1_serve` count 0), `MSPPass1V5` task
deleted, v5 folder removed, v4 setup and `MSPPass1` untouched.

## Changed files and why

- `crates/modelswarm-bench/src/shadow_join.rs` (NEW) + `examples/shadow_join.rs`
  (NEW) + `src/lib.rs` — Task 1.
- `crates/modelswarm-bench/src/runner.rs` — prefix-match family,
  Geometric profile + `parse_regime`, `cell_tag`, guard/`DEFAULT_LOSS_MULTIPLIER`
  plumbing on the executor side, tests incl. the record-correction pin.
- `crates/modelswarm-bench/src/harness.rs` — `proposer_seed(round,
  window, draw_key)` call site with rep+prompt draw key;
  `LossGuardRecord` on `JoinRow`; multiplier-aware rule-6 guard with the
  default-production counterfactual; `window_tag` on cell specs;
  regime `cell_tag` in summaries.
- `crates/modelswarm-bench/src/params.rs` — `loss_multiplier` param +
  `MSP_BENCH_LOSS_MULTIPLIER` (≥2.0 or `off`; below-default refused).
- `crates/modelswarm-bench/examples/pass1_serve.rs` — `--pool
  pass1|pass2`.
- `crates/modelswarm-bench/tests/pass1_loopback.rs` — `pass2_pool()`,
  `pass2_env()`, `pass2_engagement_loopback` + `pass2_lan_run`
  (ignored, owner-invoked), regime discriminant, window tags; pass-1
  tests unchanged.
- `experiments/processed/aggregate-pass2.py` (NEW) + the two processed
  summaries (NEW) — script-computed aggregates; engaged discriminator =
  `loss_guard` presence (a `rounds>1` test misses single-round engaged
  completions at window ≥ target).
- `experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10/**` (NEW, sqlite
  gitignored) + `experiments/raw/PASS-2-ENGAGE-LAN-2026-10-10/**`
  (NEW) — artifacts + ANALYSIS.md each.

## Exact commands and outcomes

- `cargo fmt --all --check` — PASS (each commit).
- `cargo clippy -p modelswarm-bench --all-targets -- -D warnings`
  (default and `--features quic-runner`) — PASS (each commit).
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
  (before `4faa6c0`).
- `cargo test --workspace` — **354 passed / 0 failed** (at `4faa6c0`).
- `cargo test -p modelswarm-bench --features quic-runner` — **54
  passed / 0 failed** (37 lib + 16 harness + 1 loopback smoke; 4
  ignored owner-gated: dry run, pass-1 LAN, pass-2 loopback, pass-2
  LAN). Default-features bench: 25 lib + 16 harness.
- Runs: loopback pass-2 (2 invocations, ~11 min total); LAN pass-2
  (1,435 s, 4,800 records) after a 40-record probe; serve exe build +
  scp + hash verify; B cleanup verified (0 processes, task deleted,
  v4 intact).
- `git push origin main` after each commit; `origin/main` ==
  local `main` verified by `git rev-parse`.

## Assumptions

1. The coordinator's tasking sanctions the pass-2 instrument choices
   (dialable acceptance via the seed family; guard relaxation RECORDED
   per row; asymmetric engagement pool) — all inside owned paths.
2. Loopback 8 reps/arm (32 samples/arm/cell) is enough for the medians
   reported there; the LAN headline run carries the 9.6-design 30 reps.
3. The engagement pool's synthetic speeds are instrument parameters,
   recorded in every `candidate-set.json`/manifest; no product claim
   rides on them.
4. msp-v1 wire format untouched; the B-side exe change is behavior of
   the TEST-ONLY serve example, not a protocol change.

## Unresolved risks

- 125 LAN engaged runs passed even the default guard (tight-prediction
  cells) — the guard threshold's calibration against the wire deserves
  its own small study from the committed rows.
- The pass-2 conclusion is executor-synthetic: it constrains the
  PLANNER and the WIRE, not model-level speculation. The engine-side
  win claim rests on P16's separate real-model evidence.
- `VERIFY_BATCH_STEPS=1.5` remains the frozen model's engine
   assumption; whether the scheduler crate should grow a wire-true
   verification-cost mode (for the acting planner) is a review question
   for Protocol/Runtime — changing the frozen derivation needs sign-off.
- Cross-machine CPU determinism (E0) still gates any real-draft pass-3.

## Suggested next task for the integrator

Two candidates, in order: (1) route the P16 C-API adapter decision —
pass 2 proves the wire can never win without batched verify, so the
adapter is the single gating item for any real speculative mode (the
P16 ADR-031 productionization gate should cite this evidence);
(2) schedule Test/Release review of the guard-relaxation semantics
(`would_fire_at_default` per row) before any non-bench consumer copies
the pattern. Inside my paths, the natural follow-up is a pass-3 spike
with the C-API verifier in the harness (real verify cost in the loop
against dialed synthetic proposers) once the runtime seat approves the
adapter's bench-side use.
