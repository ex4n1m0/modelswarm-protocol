# Handoff — Scheduler Scientist (master-prompt audit, 2026-10-09)

**Branch:** `audit/master-prompt-2026-10-09` (baseline `730cebf`, v0.2.20).
**Scope:** deep audit of owned paths — `crates/modelswarm-scheduler`,
`crates/modelswarm-speculation`, `crates/modelswarm-bench`, `experiments/`
— plus the science view of `apps/modelswarm-sim` (CI ownership stays with
Test and Release). Read-only: this file is the only artifact created; no
code, no commits, no cargo/npm runs (fresh suite logs in
`target/audit-logs/` were used as test evidence).

Mandatory inputs read: `AGENTS.md`; `zcode-master-prompt.md` §3/§10/§11;
`docs/reviews/gap-engineering-study-2026-10-09.md` (§1/§2/§3);
`docs/reviews/expanded-mission-roadmap-review-2026-10-08.md` (§4/§5/§10);
ADR-013, ADR-012; `docs/reviews/audit-freeze-2026-10-09.md`; the code and
test-release auditors' handoffs (context only).

---

## 1. Verdict summary

**The scheduler crate is bench-only and unwired in production — CONFIRMED.**
The shipped desktop chat path takes the **first** tracker-roster peer with a
free slot and a real QUIC multiaddr (`crates/modelswarm-desktop/src/app.rs:1434-1441`):

```rust
// First peer with a real QUIC multiaddr that is not us.
let candidate = peers.into_iter().find(|p| {
    p.peer_id != own_peer_id
        && p.free_slots > 0
        && p.addresses.iter().any(|a| a.ends_with("quic-v1") && !a.contains("0.0.0.0"))
})?;
```

`tracker.lookup(profile, 10)` returns roster order (the expanded-mission
review §2 documents lookup as alphabetical ≤50 — a fairness bias), so
production selection is *alphabetical-first-with-capacity*, not
fastest-eligible. The node's `FailoverExecutor`
(`crates/modelswarm-node/src/remote.rs:345-387`) exists but the desktop
drives `RemoteExecutor` directly, and its own comment says v1 is
deliberately "one remote peer, no EWMA, no hedging"
(`remote.rs:352-354`). The scheduler crate's only consumer is
`crates/modelswarm-bench/src/lib.rs:39-42` (plus a doc-comment mention in
`crates/modelswarm-session/tests/adversarial.rs:38`). The code auditor
independently reached the same finding (their H2).

Cost model v2's **formula core is correct, frozen, and well-tested**, but it
is a scoring formula plus an engage predicate — not a planner. Several §11 /
ADR-013 inputs have no representation at all (detail in S2). The
expanded-mission §5 planner (`plan_request`, marginal-utility τ, demand
regimes, shadow mode) does not exist anywhere yet.

Speculation is a **property-tested pure algorithm library** exercised only
against `MockRuntime` in-process (bench) and over loopback TCP (sim/session).
Real two-peer speculative decode is blocked on the runtime engine adapter:
the pinned llama.cpp HTTP adapter re-posts the whole prefix per token, so
per-window cost is k+1 sequential HTTP completions and speculative speedup
is structurally ≤ 0 until logits/batch-verify exist (S3).

Honest-results culture is **genuinely implemented** in bench/sim
(negative-result records, TEST-ONLY labels, schema-closed manifests), with
one comparator-hygiene nit to fix before real-runtime use (S4).

---

## 2. Findings (severity-ranked, with evidence)

### HIGH

**S1 — Production selection is first-found; fastest-peer selection does not
exist in the shipped path.**
Evidence: `crates/modelswarm-desktop/src/app.rs:1421-1450` (lookup →
`.find(...)` → single `RemotePeer`); `crates/modelswarm-node/src/remote.rs:14-16`
("Selection … is deliberately NOT here"); scheduler crate consumers =
bench only. Impact: every M9/M10 claim about "fastest eligible single host"
currently describes bench behavior, not the product. This is the single
smallest high-leverage wiring task: `select_microswarm` +
`predicted_single_ms` + `Ewma` are already tested frozen code.

**S2 — Cost model v2 lacks most §11 inputs; queue depth is advertised, not
measured; no planner function.**
Evidence per input (`crates/modelswarm-scheduler/src/lib.rs`):
- Modeled: measured RTT EWMA (`Candidate::measured_rtt_ms`), measured
  prefill/decode rates, failure penalty, staleness penalty, NAT-path penalty
  (ADR-014), margin floor enforcement (`should_engage_cooperative`,
  lib.rs:218-227), deterministic tie-break, cap 8.
- **Absent:** jitter, loss, bandwidth (no fields anywhere — they exist only
  in the bench's synthetic `NetworkCell`, `modelswarm-bench/src/lib.rs:100-127`);
  speculative acceptance history (bench uses the labeled synthetic constant
  `ESTIMATED_ACCEPTANCE = 0.8`, `modelswarm-bench/src/lib.rs:62-64`);
  marginal endpoint utility τ (grep for "marginal" finds only a test
  comment); deadline (gateway `NormalizedRequest.deadline_ms` exists — used
  at `app.rs:1478` — but the scheduler has no deadline concept); demand
  regimes; per-mode cost derivation — `SwarmInputs`
  (lib.rs:166-197) is six caller-computed aggregates with **no function that
  derives them from candidates/measurements**.
- **Queue depth:** `advertised_queue_ms` is peer-advertised and added
  directly into `predicted_single_ms` (lib.rs:134-141). ADR-013 says
  requester-measured inputs dominate (true — the token-rate terms dominate by
  magnitude, proven by lib.rs:400-473), but expanded-mission §5 is explicit:
  "measured queue depth — advertised values are untrusted inputs, not
  measurements." Today there is nothing to measure with: QUIC
  `measure_rtt` does not exist (only the TCP `Session::measure_rtt`,
  `crates/modelswarm-transport/src/transport.rs:368`; zero hits in
  `libp2p_backend.rs`).
- Readiness as planner base: **correct core, incomplete surface.** Keep the
  frozen formulas; add candidate measurement fields (jitter/loss/bandwidth
  EWMAs, measured queue), an acceptance-history EWMA source, and the §5
  planner as `planner.rs` in shadow mode first.

**S3 — Speculative decode over the pinned HTTP runtime adapter is
structurally non-competitive; llama.cpp HTTP cannot tree-verify.**
Evidence: `crates/modelswarm-runtime/src/llamacpp.rs:411-508` —
`decode_step` = `POST /v1/completions` with the **full prefix re-posted** and
`max_tokens: 1` (plus a `/tokenize` round trip when no vocab is attached);
`llamacpp.rs:545-574` — `propose` = window **sequential** `decode_step`
calls. Verifying a k-token window therefore costs k+1 sequential HTTP
completions, each re-sending the prompt; the runtime auditor's suspicion is
confirmed (their H1 covers the production decode path; the speculative
implication is the same). Expanded-mission §2/§3 already recorded this
("speculative speedup over this adapter is structurally zero"; "tree verify
impossible over llama.cpp HTTP").
What a REAL two-peer speculative decode needs:
1. Runtime: logits/batch-verify/KV-rollback — in-process `llama.dll` C API
   binding (ADR-021 Option A, no fork) or a research pin; this is the
   gating dependency for both SPECULATIVE and TOKEN_TREE.
2. Transport: cohort messaging — N-peer dialer, per-request deadlines,
   cohort cancel (a straggler kills the whole QUIC session today,
   expanded-mission §2); QUIC `measure_rtt`/JSON-passthrough parity.
3. Determinism: greedy cross-machine exactness is proven for Vulkan-class
   GPU and per-CPU-class (E0); cross-microarch CPU kernel determinism
   remains open (expanded-mission §12.7) and blocks CPU-CPU exact
   speculation.
**Honest fallback for tree verification:** keep the coordinator-side trie
(dedup/prune/bounds are runtime-agnostic and correct —
`crates/modelswarm-speculation/src/trie.rs`) but charge verification as
sequential forwards and expect/publish negative results at k>1 (prior art:
co-located SD dominates distributed; multi-proposer gains sublinear). Do not
claim tree speedup until the engine adapter passes batch-verify evidence.

### MEDIUM

**S4 — Hedged comparator hygiene: `fastest_single_actual_ms` is
model-derived, not an independent run.**
`crates/modelswarm-bench/src/lib.rs:632-637`: for `run_hedged`, prediction =
`predicted_single_ms(selected[0])` and actual = `simulated[0].0` — the
first-selected (fastest-**predicted**) peer's own noise draw. With per-run
noise, the actually-fastest peer in the raced set may be another one, so the
recorded single baseline can overstate the hedged win margin. Acceptable for
closed-schema mock structural evidence; must become "independently executed
single run in the same cell (or min over actual single completions)" before
any real-runtime records, or the fastest-single rule is technically preserved
but statistically weakened.

**S5 — The sim disables the RTT-spike fallback detector; observed-loss
fallback is asserted nowhere against realistic variance.**
`apps/modelswarm-sim/src/lib.rs:200-205` — `sim_fallback_policy()` sets
`rtt_multiplier: 1_000_000.0` with an honest comment (CI wall-clock
stretch). The production default is 2.0
(`crates/modelswarm-session/src/spec.rs:627-635`). Consequence: the sim's
contracts cover acceptance-collapse, receipts, greedy-equality, straggler
drop, determinism — but **rtt_spike detection has no test that injects
realistic latency spikes**; it fired only in the wild CI incident that
prompted `730cebf`. Add a sim case with an injected round-delay distribution
against the default multiplier (deterministic injection at the runtime
boundary, like `DelayedRuntime` at lib.rs:667-721).

**S6 — Bench cannot sweep the proposal window; the acceptance estimate is a
constant.**
`PROPOSAL_WINDOW` is a crate const consumed directly by both run paths
(`modelswarm-bench/src/lib.rs:59,700,921,1243`); the 9.6 experiment requires
window {2,4,8,16} as a variable. `ESTIMATED_ACCEPTANCE = 0.8` is honestly
labeled TEST-ONLY but must become a measured EWMA input for the planner arm.
Also `OUTPUT_TOKENS_TARGET = 64` and the synthetic 4-peer pool are fixed.

**S7 — Production writes no accounting rows and no receipts.**
`Store::record_job` (`crates/modelswarm-store/src/lib.rs:179-205`) has zero
callers outside the store crate; receipt sign/verify exists only in the
speculative session layer (`crates/modelswarm-session/src/spec.rs:531-589`)
used by sim/loopback; the desktop chat path produces neither. M10's
accounting/reputation/fair-queueing substrate therefore starts from a
correct two-phase receipt *design* and an empty production pipeline.

### LOW

**S8 — NAT-path penalty constants are heuristics awaiting measurement.**
`HOLEPUNCHED_PATH_PENALTY_MS = 10`, `RELAYED_PATH_PENALTY_MS = 40`
(`modelswarm-scheduler/src/lib.rs:38-43`). F2A relay verification
(2026-10-08) produced real relayed-path runs; calibrate the constants from
that evidence when the scheduler is wired.

**S9 — `Ewma` has no time-decay variant.**
`modelswarm-scheduler/src/lib.rs:263-305` is per-observation alpha only.
Gap-study §1 accounting decay and reputation availability EWMAs need
timestamp-weighted (half-life) decay; add a second constructor rather than
overloading this one.

**S10 — Desktop chat is hard-coded greedy (`temperature: 0.0` at
`app.rs:1329,1472,1761`).** Consistent with E0 determinism and correct for
the lossless contract; noted because the sampled speculative path
(`verify_sampled` partial-q documented deviation,
`modelswarm-speculation/src/acceptance.rs:104-121`; RNG alignment open
question §12.8) will need a product decision on sampling exposure.

---

## 3. Component classification (salvage matrix, owned paths)

| Component | Current role | Evidence | Decision | Reason | Dependencies |
|---|---|---|---|---|---|
| `modelswarm-scheduler` formulas (`predicted_single_ms`, `predicted_swarm_ms`, `should_engage_cooperative`, `Ewma`, `select_microswarm`) | frozen ADR-013 core, bench-consumed | lib.rs:118-305; 15 unit tests green | **KEEP** | correct, frozen, deterministic, measured-dominance proven (lib.rs:424-473) | none |
| `modelswarm-scheduler` as production selector | not wired | app.rs:1434-1441 | **KEEP + WIRE** (new work, S1) | smallest M9 step; code exists | QUIC `measure_rtt` for real inputs |
| `modelswarm-scheduler` planner (§5 `plan_request`, τ, demand regimes, shadow mode) | does not exist | grep; expanded-mission §5 | **BUILD** after measurement plumbing | no learned scheduling; deterministic first | S2 inputs, receipts v2 for demand/fairness |
| `modelswarm-speculation` acceptance/trie/metrics/rng | pure exact algorithms, property-tested | acceptance.rs, trie.rs; 14 integration tests green | **KEEP WITH TESTS** | token-exact contracts hold for any draft policy incl. adversarial | runtime adapter for real logits (S3) |
| `speculation` real two-peer execution | mock/loopback only | sim lib.rs:17-26 | **NEEDS EXPERIMENT** (9.6) + runtime engine work (S3) | prior art predicts WAN-negative; LAN gate | engine adapter ADR-021, cohort transport, E0-CPU question |
| TOKEN_TREE verification over llama.cpp HTTP | impossible | llamacpp.rs:411-574; expanded-mission §3 | **NEEDS EXPERIMENT** (expect negative; publish) | k sequential forwards erase tree gain | engine adapter; else linear-verify fallback |
| `modelswarm-bench` engine/records/stats | schema-honest mock harness | lib.rs:1-51, records.rs; 27+16 tests green | **KEEP WITH TESTS** (fix S4/S6) | negative results first-class; closed schemas enforced | runtime for real backend variant |
| `apps/modelswarm-sim` (science view) | loopback contract runner (acceptance/receipt/greedy/straggler/determinism/relay) | lib.rs:1145-1322 | **KEEP WITH TESTS** (add S5 case; CI stays Test-Release) | 78.5 s suite; contracts are the right ones | none |
| `experiments/` | empty raw/processed; frozen schemas + rules | README.md; schemas/ | **KEEP** | first real entries land with 9.6 | 9.6 harness |

What the sim asserts today (science contracts): D1 greedy-equality
unconditional **including the fallback path** (180 cases);
acceptance-collapse at zero accuracy; receipts verified per case; exactness
across proposers {2,4,7}; straggler-drop round completion; same-seed token
determinism; relay-path end-to-end exactness with >0 forwarded frames;
hang-free injected peer death. What the sim **fakes that production must
measure**: wall-clock round-vs-estimate ratio (rtt_spike disabled, S5);
acceptance from the `draft_accuracy` knob (production: real proposal-match
rates); per-peer RTT/jitter/loss EWMAs (none — TCP `measure_rtt` only);
queue depth (admit-or-reject, `crates/modelswarm-node/src/serving.rs`
`Admission`, 8 sessions / 2 per peer, hard errors `session_limit` /
`per_peer_session_limit`); the wire's `queue_position`/`eta_ms` are constant
(`crates/modelswarm-transport/src/message.rs:80-81,341-342`) while msp-v1 §6
already fixes a 10 s queue-admission deadline (`protocol/msp-v1.md:296`).

---

## 4. Honest-results culture check (scope item 5)

Verdict: **pass, with one nit (S4).**
- `experiments/README.md`: "negative results mandatory; no performance
  claim without a reproducible run-manifest chain; no simulated acceptance
  in any user-facing path."
- ADR-013: "Negative results are first-class" — implemented as
  `RunStatus::FellBackToSingle` + mandatory reason; `cell_report` prints
  "negative result: N/M runs fell back to single (visible, not hidden)"
  (`modelswarm-bench/src/lib.rs:394-399`); zero-acceptance runs are recorded
  failures, never wins (lib.rs:803-815, 1064-1076; asserted in
  `tests/harness.rs:134-201`).
- Mock honesty labeling is structural: `TEST_ONLY_MOCK_LABEL` on every
  human line, `runtime.name = "mock"` in every manifest (ADR-019), closed
  schemas reject extra fields (mini-validator proven non-rubber-stamp,
  `tests/harness.rs:316-359`).
- Average-baseline scan: none found. The comparator is
  `select_microswarm(...)[0]` (fastest predicted) and the engage rule
  compares against `predicted_fastest_single`; the crate doc restates the
  invariant (`lib.rs:13-15`). The only weakness is S4's model-derived
  "actual" for hedged — not an average, but not an independent measurement
  either.

---

## 5. M10 build order (scope item 4) — dependency-ordered, effort-sized

Scale: **S** ≤ 2 days · **M** ≈ 1–2 weeks · **L** ≥ 2 weeks or needs
hardware/spend. Nothing below starts before the §9 architecture gate; all
protocol/wire items are ADR-gated; no approximate mode default-on, ever.

1. **R0 governance batch (S, parallel):** mode-registry ADR extension with
   the five-preset table (d-ADR skeleton below) + receipts-v2 ADR (fields
   already specified expanded-mission §4.3: modeId, cohort digest, per-peer
   roles, accepted/verified token counts, endpoint-seconds). No code.
2. **Measurement plumbing (M):** QUIC `measure_rtt` + JSON passthrough
   parity in `libp2p_backend`; RTT/jitter/completion-distribution EWMAs
   plumbed into the node; candidate fields extended (S2 list). Advertised
   queue becomes a bounded, discounted term until measured.
3. **Shadow planner (M):** `crates/modelswarm-scheduler/src/planner.rs`
   implementing expanded-mission §5 pseudocode over the preset envelope;
   logs the plan it would choose against real traffic; never acts. τ is a
   recorded placeholder until 9.6 measures it.
4. **9.6 experiment, pass 1 (harness M, run L) — see §6.** Runs on the
   existing 2-machine LAN with the current HTTP adapter; expected honest
   outcome outside loopback is largely negative, which is itself a
   first-class result and calibrates τ. **This precedes any acting planner
   code.**
5. **Scheduler wiring (S):** replace first-found desktop selection with
   `select_microswarm` + EWMA RTT over the roster (code-auditor action 2
   agrees; uses only tested frozen code).
6. **Engine adapter spike (M, de-risk early — expanded-mission risk 4):**
   in-process `llama.dll` C API binding for logits/batch-verify; gates 9.6
   pass 2 and all lossless modes.
7. **Receipts v2 implementation (M–L):** counter-signed two-phase receipts
   in the production serving/chat path (reuse the spec-layer sign/verify
   machinery); `Store::record_job` gains real callers; granted-accounting
   grant rules — counters increment only from counter-signed receipts
   (granted, never claimed), self-work and sybil-ring work zero
   (installation + subnet/ASN diversity), new identities zero, EWMA decay
   (S9 variant). Precondition for 8–9 and for any approximate mode
   default-on.
8. **Fair queueing (M):** bounded wait queue at serving admission (stop
   hard-rejecting at `session_limit`; honor msp-v1 §6's 10 s queue
   deadline; wire the already-existing `queue_position`/`eta_ms`), then
   deficit-round-robin keyed on the accounting from 7.
9. **Reputation chassis (M):** per `(peerId, profileId)` machine-derived
   counters (granted receipts, AUDITED outcomes, heartbeat availability)
   riding ADR-012 suspension; enforcement ladder
   scheduler-EWMA-weight → planner cohort exclusion → slots-to-zero bound
   to `audit_epoch`; NO peer ratings, NO trust propagation. Hard
   enforcement only after the hostile-peer gauntlet (seeded-wrongness
   detection curves ≥0.9/<1%, ring detection, whitewash test); audit rate
   q sized from Sarmenta math, measured not invented.
10. **HEDGED honesty patch + cohort transport (M each):** cohort messaging
    (N-dial, per-request deadlines, cohort cancel) then HEDGED with
    spot-verify/background agreement check, Sybil diversity, fan-out
    charged to the requester's lease → unlocks **BALANCED**.
11. **Presets light-up (S per tier, gated):** FAST live now; BALANCED at
    10; VERIFIED with AUDITED (detection curves published); DEEP with
    best_of_n + deliberation (own ADRs, M11); MAXIMUM last (HYBRID =
    composition of individually-gated modes only). Each preset declares
    budget + correctness label + recipient-set disclosure on the wire.

Preset needs from the planner (scope item 4d): the planner receives
`{preset, budget}` at session open (rides the proposed `SessionOffer` in
the not-yet-written `msp-cooperative-v1.md`), filters candidate modes to
the preset's allowed set, still chooses the smallest useful cohort inside
the envelope, and the receipt records preset + resolved modes + budgets.
Gates already existing: FAST (ships); BALANCED partial (hedged mode
registered + bench-tested, honesty patch missing); SPECULATIVE registered
+ property-exact but runtime-blocked (S3); best_of_n/consensus/critique/
audited unregistered (own ADRs); HYBRID has no composition rules — the
preset layer IS that rule.

---

## 6. The 9.6 invalidating experiment — plan skeleton and readiness

**Question.** Does planner-chosen cohort size beat the best fixed k and
FASTEST_SINGLE on real hardware at LAN-class RTT — or is adaptive sizing
dead below ~8 peers?

**Design.** Draft window {2,4,8,16} × proposer count {1,2,4} × injected RTT
{0,5,10,20 ms} with real weights on the 2-machine LAN testbed. Arms:
(a) fixed k=2; (b) planner-chosen k from measured acceptance EWMA +
marginal-benefit test (deterministic heuristic, expanded-mission §5 shape);
(c) FASTEST_SINGLE. Metrics: vs-fastest-single completion ratio;
endpoint-seconds per accepted token; TTFT; acceptance rate. ≥30 reps/cell;
bootstrap 95% CI — `modelswarm-bench/src/stats.rs:48,83,99` already provide
percentile/aggregate/bootstrap_ci (verified deterministic).

**Invalidation criterion** (from expanded-mission 9.6): if planner-k does
not beat the best fixed k by more than its CI width in ≥2/3 of cells,
adaptive cohort sizing is dead on real hardware below ~8 peers and the
roadmap must say so.

**Environment pinning.** One run-manifest per cell (frozen schema, live);
hardware classes, OS, runtime build hash (pinned llama.cpp), profile id,
prompt corpus id, seeds, confidence margin, `measured_rtt_ms_p50` +
`calibration_valid` (10% tolerance — the manifest fields already exist,
`modelswarm-bench/src/records.rs:232-237`).

**Readiness: NOT READY.** Gaps, in dependency order:
1. No real-peer bench runner: `BenchEngine` executes in-process runtimes
   only; the LAN cross-machine test
   (`crates/modelswarm-node/src/remote.rs:548-620`) does single-stream chat.
   Need a transport-backed variant (extend `modelswarm-bench` or a small
   `apps/` runner) that drives `speculate`/`speculate_multi` over the
   network against the peer's real runtime.
2. QUIC `measure_rtt` missing (S2) — needed both for planner inputs and for
   netem calibration checks.
3. Window/acceptance not parameterizable (S6) — the sweep dimensions are
   compile-time constants today.
4. No acceptance-EWMA store (the 0.8 constant is the stand-in).
5. RTT injection: needs tc/netem-class shaping on the path (or a
   delay-injecting transport wrapper at the same boundary as sim's
   `DelayedRuntime`); must be verified by the manifest's measured-p50
   calibration.
6. No frozen real-prompt corpus (only `synthetic-mock-v1`); a small fixed
   corpus must be pinned with a corpus id.
7. The planner arm itself does not exist — it is the thing under test and
   must be implemented as the shadow planner (build item 3) restricted to
   deterministic heuristics.

**Run strategy.** Pass 1 with the current HTTP adapter (cheap, honest,
expected negative outside loopback — calibrates τ and publishes the
baseline); pass 2 after the engine adapter spike (batched verify) — that
pass is the one that can legitimately engage. No spend required for pass 1;
pass 2 rides the R3 engine decision.

---

## 7. Changed files

- `docs/reviews/handoff-scheduler-scientist-2026-10-09.md` — this file
  (only artifact created; audit is read-only).

## 8. Exact commands run and outcomes

Read-only inspection (Git Bash, Windows), all against
`audit/master-prompt-2026-10-09` at `730cebf`:

- `grep -rn "modelswarm_scheduler|select_microswarm|should_engage_cooperative|..." --include="*.rs"` →
  consumers = bench only (plus a session-test doc mention).
- Read paths: `crates/modelswarm-scheduler/src/lib.rs` (full);
  `crates/modelswarm-speculation/src/{lib,acceptance,trie,metrics,rng}.rs`;
  `crates/modelswarm-bench/src/{lib,records,stats}.rs` +
  `tests/{harness.rs,common/mod.rs}`; `apps/modelswarm-sim/src/lib.rs` +
  `tests/scenarios.rs`; `crates/modelswarm-node/src/{remote,serving}.rs`
  (selection/admission); `crates/modelswarm-desktop/src/app.rs:1390-1530`
  (chat path); `crates/modelswarm-runtime/src/llamacpp.rs:384-600`
  (per-token HTTP); `crates/modelswarm-store/src/lib.rs:178-206`;
  `crates/modelswarm-transport/src/message.rs` + grep for `measure_rtt`.
- `grep target/audit-logs/test-default.log` → per-crate results (below).
- No cargo, npm, git write, deployment, or network-mutating commands run.

## 9. Test evidence (from `target/audit-logs/test-default.log`, freeze suite at `730cebf`)

| Suite | Result |
|---|---|
| `modelswarm-scheduler` unit (lib) | ok — 15 passed / 0 failed |
| `modelswarm-bench` unit (lib) | ok — 27 passed / 0 failed |
| `modelswarm-bench` integration (`tests/harness.rs`) | ok — 16 passed / 0 failed |
| `modelswarm-speculation` unit (lib) | ok — 5 passed / 0 failed |
| `modelswarm-speculation` `acceptance_properties` | ok — 9 passed / 0 failed |
| `modelswarm-speculation` `trie_properties` | ok — 5 passed / 0 failed |
| `modelswarm-sim` unit (lib, incl. 180-case D-matrix + 90-case E-matrix) | ok — 4 passed / 0 failed (78.5 s) |
| `modelswarm-sim` main + scenarios | ok — 3 + 3 passed / 0 failed |
| `modelswarm-session` `adversarial.rs` | ok — 10 passed / 0 failed |
| Full workspace (freeze doc) | fmt/clippy/tests green; 301 default + 313 libp2p-feature tests |

Not run by me (constraint): `cargo fmt/clippy/test` directly — the freeze
suite logs above are the evidence of record.

## 10. Assumptions

1. The gap study §1/§2/§3 verdicts are correct as written (spot-verified
   against code: receipts two-phase in spec.rs; serving admit-or-reject;
   `queue_position`/`eta_ms` constant; store rows unsigned; `record_job`
   uncalled).
2. "Desktop chat" = `try_swarm_chat` in `modelswarm-desktop/src/app.rs`
   (the shipped v0.2.20 swarm path); the local gateway path is the
   fallback, correctly labeled.
3. Effort sizes assume one engineer per item, no protocol-review latency;
   ADR review time is extra.
4. The 9.6 planner arm is deterministic-heuristics-only (master prompt §11
   bans learned scheduling before production evidence).
5. msp-v1/messages.proto remain byte-frozen; every wire addition rides new
   ADRs (receipts v2, msp-cooperative-v1, SessionOffer).

## 11. Unresolved risks

1. **Engine adapter slip** (expanded-mission risk 4) blocks 9.6 pass 2 and
   every lossless cooperative mode; the spike must land early.
2. **9.6 pass 1 may be negative almost everywhere** outside loopback —
   planned for, but the roadmap must then say adaptive sizing is dead below
   ~8 peers (invalidation criterion) instead of quietly continuing.
3. Cross-machine CPU determinism (E0 sub-question) could confine exact
   speculation to same-backend-class pairs; the experiment cells must
   record backend classes per peer.
4. Preset wire surface depends on `SessionOffer`/msp-cooperative-v1, which
   do not exist; the ADR batch (build item 1) is on the critical path of
   items 3, 10, 11.
5. S4 (hedged comparator) and S5 (rtt_spike untested against realistic
   variance) are small but sit exactly on the honesty surface the release
   gate audits.
6. `advertised_queue_ms` discounting changes selection behavior once
   wired; needs the F1.5-style LAN re-prove afterwards.

## 12. Suggested next task for the integrator

Approve, inside the §9 gate, the R0 batch as specified in §5 items 1–3
(mode-registry preset ADR + receipts-v2 ADR + QUIC measurement plumbing +
shadow planner), with the 9.6 pass-1 harness (§6) as the Scheduler
Scientist's next implementation assignment. Immediately mergeable
independently of the gate: the S4 comparator fix and the S5 sim rtt-spike
case (both inside owned paths, small, evidence-only).
