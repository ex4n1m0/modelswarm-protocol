# Handoff — Scheduler Scientist (2026-10-11)

**Branch:** `main` (commits `e85e869` → `bea1749` → `fa91ffb` → evidence
commit, all pushed; `origin/main` verified equal after each push).

Scope: the two tasked items — (1) the ADR-032 option-A wire-true
verification term (capability-aware, wire-true by default, engaging gate
BLOCKS on HTTP-only cohorts), and (2) the guard-calibration review's
recommendation-(iii) per-profile engaged-loss EWMA engage gate, plus the
gated pass-2 re-run (loopback + real two-machine LAN) with both changes
live and the honest (all-fallback) result published.

Note on sequencing: two other-seat commits (`5ad5215` Security review,
`10de665` T/R review, `59e1cea` ADR-032 revision 2) landed from a
concurrent session between my baseline and my first commit; my commits
stacked on top with no conflict, and `bea1749` exists specifically to
satisfy the rev-2 execution-venue matrix (item 4a lives per-push in
`modelswarm-scheduler` under default features).

---

## Task 1 — wire-true verification term (`e85e869` + `bea1749`)

- `crates/modelswarm-scheduler/src/lib.rs`: `Candidate::batch_verify`
  (NEW field) — the ADR-032 §4 adapter capability, ABSENT at every
  construction site (no production adapter can declare it; the pinned
  llama.cpp HTTP adapter cannot batch). `VerifyTerm{Batch,SequentialWire}`
  (NEW, default-feature crate surface per the rev-2 venue matrix):
  `of_verifier`/`of_cohort` (the VERIFIER — `cohort[0]` — decides; an
  HTTP peer may still propose, never batch-verify; empty cohort stays
  conservative), `decode_steps_per_round(window, batch_steps)` (window+1
  sequential vs the caller's batch coefficient — ADR-013's 1.5 stays
  with the bench cost-model coefficients), `label()` for artifacts.
- `crates/modelswarm-scheduler/src/shadow.rs`: the production mapping
  sets `batch_verify: false` (roster has no declaration surface; pinned
  by test).
- `crates/modelswarm-bench/src/runner.rs`: `swarm_inputs` is
  capability-aware — sequential cohorts are charged window+1 verifier
  tokens per round PLUS the per-round re-post of the GROWING committed
  prefix (average `output × (R−1)/(2R)` tokens/round on top of the
  prompt; exact arithmetic pinned); batch-declared cohorts keep the
  ADR-013 ENGINE model byte-identically. `verify_mode` delegates to the
  scheduler. Engage-gate fallback reasons name the term in force.
- `crates/modelswarm-bench/src/lib.rs`: `VERIFY_BATCH_STEPS` doc now
  states the capability gate; Phase C mock candidates declare the
  capability (the mock runtime IS the batch-verify ENGINE model —
  Phase C behavior and committed Phase C records unchanged).
- Pins: HTTP-only never engages (w8 and the w16 parity corner; the
  prediction loses OUTRIGHT, before any margin); batch-declared engages
  exactly when ×(1+margin) beats the fastest single and stops when the
  single wins; mixed cohorts fall back when the fastest single wins
  (HTTP verifier never batches; batch verifier + HTTP proposers still
  single-gated); exact `swarm_inputs` arithmetic incl. the
  (window+1)/1.5 = 6× (w8) verify gap. Two of the pins live in
  `modelswarm-scheduler` under DEFAULT features (item 4a venue);
  pool-level pins in the quic-runner-gated runner tests.
- Behavioral consequence, honestly handled: the loopback smoke test
  previously relied on the pass-1 pool engaging under the old batch
  term; it now pins the never-engage contract (all cooperative rows
  fall back with the wire-true reason, acceptance store empty). The
  engaged wire-round machinery is no longer CI-reachable by design; it
  remains pinned by runner unit tests and the committed pass-2 runs.

## Task 2 — engaged-loss EWMA engage gate + gated re-run (`fa91ffb` + evidence)

Gate specification (explicit, per the review's ask; module docs in
`crates/modelswarm-bench/src/engage_gate.rs`, UNGATED so its arithmetic
runs in default CI):

- **Key:** `realized_total_ms / fastest_single_prediction_ms` of
  COMPLETED engaged attempts (both already in every join row — no extra
  single execution).
- **α = 0.3** (the frozen `modelswarm_scheduler::EWMA_ALPHA` — one
  estimator family, no new tunable). **Warm-up N = 3.** **Block at
  ≥ 1.0** (strict beat-or-fall-back).
- **Scope: per profile** within a harness lifetime (the pass-2 drivers'
  per-cell harnesses scope it per cell — the recorded isolation
  choice; the production shape is one gate per profile per requester).
- **Cold start: margin-gated allow** — default-block would deadlock
  (blocked ⇒ no attempts ⇒ no samples ⇒ blocked forever = a kill
  switch, not a gate).
- **No recovery yet (honest limitation):** blocked stays blocked; a
  reviewed probe/decay policy or (profile, verify-term) keying is the
  documented follow-up.

Layering (all three live): wire-true margin rule (structural, blocks
first, stays cold downstream) → this EWMA (for predictions that pass)
→ rule-6 round-1 backstop unchanged at 2.0.

Harness wiring: both arms consult the gate next to the frozen margin
rule; only Completed engaged rows feed it; every consultable
cooperative row carries a `loss_gate` `GateDecision` (`#[serde(default)]`
— committed join rows still parse); fallback reasons distinguish
"engaged-loss EWMA … per-profile engage gate" from the cost-model
reason. REPLAY PIN (`tests/harness.rs`, quic-runner-gated) drives the
REAL gate over the committed pass-2 LAN rows: 7/7 warmed cells block at
engaged attempt 4 (high-w16 parity corner at 5, EWMA 1.0069); geo700-w8
never warmed (2 attempts; the margin rule had already blocked the
rest); whole-run one-profile replay in actual sweep order: exactly the
3 warm-up attempts run, **1,979/1,982 refused pre-round** — the
review's "blocks 8/8 engaged cells" re-derived with the shipped
implementation. `aggregate-pass2.py` now publishes zero-engaged runs
(best/worst `null`) instead of crashing; re-aggregation of BOTH
committed pass-2 dirs verified **bit-identical**.

### The gated re-run (both changes live)

- **Loopback** (`experiments/raw/PASS-2-GATED-LOOPBACK-2026-10-11/`,
  12 cells mirroring the committed pass-2 matrix, 1,920 rows, 0
  failures): **0 engaged rounds; all 1,536 cooperative rows wire-true
  gate-blocked; EWMA cold (0 samples).** Fallback band 0.977–1.023
  across all 48 arm-cell medians. No cell regresses vs the committed
  pass-2 numbers except the honest exception pair recorded in the
  ANALYSIS (inj20ms-geo800-w8 +0.001 noise; the high-w16 parity corner
  0.920–0.999 → 0.991–1.011, which the strict comparator already
  showed as parity-not-win 1.047 — the gate trades that corner for
  eliminating losses up to 2.755×).
- **LAN** (`experiments/raw/PASS-2-GATED-LAN-2026-10-11/`, 8 cells, 30
  reps/arm, 4,800 rows, 0 failures, 1,071 s): **0 engaged rounds; all
  3,840 cooperative rows wire-true gate-blocked** — including pass-2's
  two leak cells and the w16 parity corner, on the same pool, seeds,
  and machines that produced 1,982 engaged losses. Fallback band
  0.943–1.058 (both edge groups explained in the ANALYSIS: sub-1.0 is
  fallback singles beating the single arm's median — same peer,
  independent executions; the 1.05 highs sit in the last cell's
  latest-running arms, late-sweep drift, visible not hidden). Per-cell
  worst medians improve or hold everywhere (geo900-w16 1.938 → 1.004).
- Both ANALYSIS.md files are in the house style (medians tables, honest
  findings incl. the negative headline, NOT-claims, limitations, ops
  records with exact commands). Processed summaries committed.

### Ops (machine B)

v4 + `MSPPass1` cannot host the pass-2 pool (v4 predates `--pool
pass2`/the prefix-match family), so a fresh exe was built from
`fa91ffb` (sha256 `c44b9df2…eb18557c`, scp + `Get-FileHash` verified),
deployed as a SEPARATE folder `ModelSwarm-9.6-LAN-v6` + task
`MSPPass1V6` (the documented scheduled-task detach pattern). One serve
process served the whole sweep (unique per-cell request roots, no
`replayed_request`). **B left as found and verified:** serve stopped
(0 `pass1_serve` processes), `MSPPass1V6` deleted, v6 folder removed,
`MSPPass1` Ready + v4 intact.

## Changed files and why

- `crates/modelswarm-scheduler/src/lib.rs` — `batch_verify` field,
  `VerifyTerm` + selection/step/label API, default-feature item-4a
  pins.
- `crates/modelswarm-scheduler/src/shadow.rs` — capability absent in
  the production mapping (+ pin).
- `crates/modelswarm-bench/src/runner.rs` — capability-aware
  `swarm_inputs`, `verify_mode` delegate, pool-level engage pins,
  exact-arithmetic pin.
- `crates/modelswarm-bench/src/lib.rs` — module registration,
  `VERIFY_BATCH_STEPS` doc, mock candidates declare the engine
  capability.
- `crates/modelswarm-bench/src/harness.rs` — gate field/consult/feed,
  `loss_gate` on `CoopSpec`/`RunJoined`/`JoinRow`, verify-term fallback
  reasons, EWMA-blocked reason.
- `crates/modelswarm-bench/src/engage_gate.rs` (NEW) — the gate +
  specification + unit tests.
- `crates/modelswarm-bench/tests/harness.rs` — EWMA replay pin over
  committed LAN artifacts (guard-calibration pin untouched).
- `crates/modelswarm-bench/tests/pass1_loopback.rs` — smoke test pins
  the never-engage contract; driver docs updated to the gated
  expectation; pool doc records why it stays pinned.
- `experiments/processed/aggregate-pass2.py` — zero-engaged support
  (bit-identity re-verified on both committed summaries).
- `experiments/raw/PASS-2-GATED-LOOPBACK-2026-10-11/**` (NEW, sqlite
  gitignored) + `experiments/processed/
  PASS-2-GATED-LOOPBACK-2026-10-11-summary.json` (NEW).
- `experiments/raw/PASS-2-GATED-LAN-2026-10-11/**` (NEW) +
  `experiments/processed/PASS-2-GATED-LAN-2026-10-11-summary.json`
  (NEW).
- `docs/reviews/handoff-scheduler-scientist-2026-10-11.md` (this file).

## Exact commands and outcomes

- `cargo fmt --all --check` — PASS (each commit).
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS (each
  commit; one intermediate `cloned_ref_to_slice` lint fixed before any
  commit).
- `cargo clippy -p modelswarm-bench --features quic-runner
  --all-targets -- -D warnings` — PASS (each commit).
- `cargo test --workspace` — **363 passed / 0 failed** (at `fa91ffb`;
  354 baseline + 2 scheduler item-4a + 7 engage_gate).
- `cargo test -p modelswarm-bench --features quic-runner` — **48 lib +
  18 harness + 1 smoke passed / 0 failed** (4 owner-gated ignored);
  the replay pin and the guard pin both run in the harness target.
- Loopback gated runs: 342.6 s (w8) + 165.3 s (w16); LAN gated sweep:
  1,070.9 s (4,800 records, 0 failures). `aggregate-pass2.py` runs on
  all four pass-2 raw dirs (2 committed re-verified bit-identical, 2
  new).
- `git push origin main` after each commit; `origin/main` == local
  verified by `git rev-parse` every time.

## Assumptions

1. The tasked companion correction is sanctioned by ADR-032 §4 ("This
   scheduler-crate change needs Scheduler + Protocol sign-off and
   precedes any light-up") — the tasking is that sign-off for the
   Scheduler seat; Protocol's formal ack remains theirs to record.
2. `Candidate` is constructed only inside my owned crates (verified by
   workspace grep: desktop/node use shadow types only) — the new field
   crosses no foreign path. A future production capability declaration
   surface (ADR-032 §3 `SessionAccept.engine`) would plumb through
   `shadow::candidate_from_observations` — recorded here, not built.
3. Batch-declared cohorts keep the ADR-013 engine model unchanged
   (byte-identical predictions); a VerifyDrafts-class wire would drop
   the per-round re-post, but that wire is Proposed, not accepted —
   modeling it now would be inventing a wire.
4. Committed pass-1/pass-2 artifacts remain reproducible at their
   pinned commits (`4faa6c0`-era); re-running them at HEAD now yields
   the gated (all-fallback) behavior, which is the point of the change
   and is recorded in the driver docs.
5. The engaged-loss EWMA has no off switch (by design); any future
   engaged-curve instrument must be designed around cold-start warm-up
   or a reviewed knob.

## Unresolved risks

- The EWMA's no-recovery property: a blocked profile stays blocked
  until policy changes; if a batch-verify cohort ever lights up under
  the SAME profile id, its fresh loss history is conflated with the
  sequential wire's — keying by (profile, verify term) is the
  recommended follow-up before ADR-032 B implementation.
- Cold-start warm-up (≤ 3 engaged attempts) is unprotected by the EWMA
  on cohorts whose predictions pass the margin rule; the wire-true term
  closes the known structural case, not every modeling error.
- The gated LAN fallback band's late-sweep drift (1.05× in the final
  cell's later arms) suggests serve-side drift over ~18-minute sweeps;
  harmless here (all within noise of parity) but worth a B-side
  telemetry look if longer sweeps are planned.
- The loopback smoke test no longer exercises engaged wire rounds in
  CI (they are unreachable on HTTP-only cohorts by design); the
  engaged machinery's next live exercise is a batch-capable cohort,
  which cannot exist until ADR-032 B + the ADR-031 gate.

## Suggested next task for the integrator

Route the ADR-032 owner decision with this evidence attached: option A
is now ENFORCED and measured on both transports (all-fallback, parity
band, zero engaged rounds — the honest negative the ADR predicted); if
B is accepted, the scheduler-side prerequisites are done (capability
flag, term selection, both engage gates) and the remaining gating item
is the ADR-031 productionization gate for the C-API adapter plus the
item-4b mixed-swarm venue in `apps/modelswarm-sim`. Inside my paths,
the natural follow-up is the (profile, verify-term) EWMA keying +
recovery policy proposal before any B-track work starts.
