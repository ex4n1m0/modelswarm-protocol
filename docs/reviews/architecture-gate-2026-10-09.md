# Architecture Decision Gate — 2026-10-09 (owner approval requested)

Consolidated decision per master prompt §9, from the eleven audits
(freeze + wave-1 six + wave-2 five), the salvage matrix, the production
risk register, and the rewrite boundaries. **Nothing destructive starts
until the owner approves this gate.** Sign-off seats: Integrator
(assembled) · Security Engineer · Test and Release Engineer ·
Architecture Reviewer (seat vacant — see D19; Protocol Architect served
pro-tem via its audit).

## A. The decision in one paragraph

**Preserve and harden — no rewrites.** The frozen core is byte-consistent
and CI-parity-pinned; the test estate is the repo's strongest asset. The
audit's CRITICAL/HIGH items are narrow, scoped fixes (streaming refactor,
earn-chain verification, suspension wiring, measurement plumbing, failover
mapping, wire reconciliation) that all fit behind existing interfaces.
The product's genuine gaps (M1 profiler, M2 governor, M4 auto-selection)
are new, additive subsystems — exactly the roadmap the master prompt
already defines.

## B. Subsystem decisions (§9 enumeration)

| Subsystem | Decision | Key evidence / condition |
|---|---|---|
| Protocol & schemas | **Preserve + harden** | one reconciliation ADR covering the FULL audit list: the 5 unlabeled endpoints, `/peers/lease` + `challenge/complete` §3.3 table fold-ins, the `capacityClass` frozen-response-shape change, the §6.1 libp2p handshake-skip record, the §6.3 `accepted`/queue-vocabulary deferral (M10 fair queueing depends on it), §6.4 serving-side sampling clamps, the §6.5 error-code registry fixes, and a new msp-v1 changelog section |
| Identity & cryptography | **Preserve + harden** | seed storage (DPAPI), fuzz targets; no new crypto (owner calibration) |
| Tracker | **Preserve + harden** | hygiene batch (T4/T5), migration-0004 (suspension), challenge nonce+digest |
| P2P transport | **Preserve; replace libp2p_backend incrementally at F2b** | libp2p Swarm migration, same public surface, 4–6 d + ADR |
| Eligibility | **Preserve + harden** | wire suspension (needs migration-0004) — M10 prerequisite |
| Runtime abstraction | **Preserve; refactor decode path behind executor contract** | P1 streaming (measured 7.3× long-prompt win, TTFT 4 ms) |
| Scheduler | **Preserve + wire** | measurement plumbing first (F15), shadow mode, then select_microswarm |
| Sessions | **Preserve with tests + ADR** | msp-cooperative-v1.md must exist before SpecMessage grows |
| Speculation | **Preserve algorithms; defer real execution** | gated on 9.6 + engine adapter (P16) |
| Storage | **Preserve + harden** | prune policy, write-only tables, blocking-in-async fixes |
| Gateway | **Preserve + harden** | Origin/Host checks (P11); SSE rides P1 |
| Telemetry | **Preserve + harden** | structural redaction |
| Node lifecycle | **Preserve + harden** | drain on Stop, cancel-over-pool, admission-after-lease-gate ordering |
| Desktop client | **Preserve; refactor app.rs + ui** | module extraction behind IPC surface; copy button; tauri-shell clippy gate FIRST — and that clippy step must carry the same TAURI_CONFIG resources overlay as the test step (clippy runs build scripts; Test/Release condition) |
| Simulator | **Preserve with tests** | add rtt-spike case (S5) with SYNTHETIC delay injection, not runner wall-clock (avoids the 730cebf flake class; Test/Release note) |
| Installer | **Preserve** | exemplary hash gates; F4 uninstall VM drill pending |
| Catalog | **Preserve + harden** | M3: 5 actionable rows (license metadata, runtime const, candidate validation, per-request signing) |
| Deployment pipeline | **Preserve; harden CI pins** | commit-pin actions, fix @stable toolchains, fold tauri-cli version |
| Existing agents | **Revise 7, keep 2; create 3** | D19 |

## C. The four §9 mandatory resolutions

**1. Expanded-mission §15 folded in (owner recommendations):**
- **Successor plan: APPROVE** a single plan of record
  (`docs/implementation/master-roadmap.md`, ADR-027 supersession map in
  ADR-015 style; ADR-027 restates that the msp-v1 freeze and
  ADR-gated-schema rules carry forward unchanged). Cooperative plan
  stays the gated lossless kernel; A–I remain historical evidence.
- **R0 start: APPROVE immediately after this gate.** R0 = governance
  ADRs: presets (5-tier table per master prompt §3, incl. MAXIMUM/HYBRID
  composition rules and per-tier budget/correctness-label disclosures;
  the wire enum lands only with SessionOffer), receipts-v2, the full
  tracker-surface + wire reconciliation (scope per §B protocol row),
  msp-v1 changelog, AND creation of `protocol/msp-cooperative-v1.md` as
  the home for SessionOffer + the preset enum (before SpecMessage
  grows). R0 is documentation-grade **except one item**: the P2P
  error-code rename (`duplicate_request`→`replayed_request`,
  `over_limit`→`overloaded`) is wire-visible and lands as a small R0.5
  code change under its ADR.
- **Relay host: RECORDED** (owner decided 2026-10-08: own desktop B, no
  rented VM). F2b Swarm migration sanctioned; internet exposure only
  after auth + per-circuit caps (P10).
- **k=4 harness spend: RECOMMEND DEFERRAL** until 9.6 pass 1 (existing
  2-machine LAN) publishes its result; the pass-1 outcome sizes the
  harness need. Owner decides.

**2. Federation non-goal reversal: RECORDED as gated future work (M12)**
with preconditions: own ADR + correctness contracts; no mixed-ModelProfileId
cohort until its contract exists; mixed-model outputs never labeled exact
lossless unless the target verifier enforces it; every output records
which profile performed each role (M12 exit criterion); M12 begins only
after M10 (cooperative) and M11 (deliberation) gates. Nothing silent.

**3. Phase mapping (audit-corrected M0–M13 status):**
M0 DONE (validated; `63b98e1` verified green run 37858753428; `86450aa`
run 37860513036 in flight at gate time — "green pending one run" until
it completes, per the Test/Release condition) · M1 PARTIAL-seed
(hw detection exists; profiler crate new) · M2 NEW (governor ADR incl.
70%-vs-90% reconciliation) · M3 PARTIAL→harden · M4 NEW (top-6 seed;
automatic selection new) · M5 PARTIAL (resume/disk/rollback absent) ·
M6 DONE + P1 streaming attached · M7 DONE with gaps (copy button, Stop
drain) · M8 DONE on LAN (F2b outstanding) · M9 PARTIAL (WAN outstanding;
F1+F15 prerequisites) · M10 NOT STARTED (11-step build order delivered;
prerequisites P3/P16/F15) · M11, M12 NOT STARTED (gated) · M13 PARTIAL
(beta channel, signed updates, incident process, uninstall drill open).

**4. Critical path (dependency-ordered):**
R0 governance ADRs → **P1 streaming** → F15 measurement plumbing →
scheduler shadow-mode wiring → receipts-v2 + granted accounting →
suspension wiring (migration-0004) → fair queueing → reputation chassis →
HEDGED honesty patch → BALANCED light-up → presets per-tier light-up.
Parallel tracks: F2b Swarm migration (Network) · M1/M2/M4/M7-gap product
track (Windows) · M3/M5 hardening (Runtime/Tracker) · P4 fuzzing +
P7 test-gate closures (Security/Test) · 9.6 pass 1 after F15 (Scheduler).
WAN + k=4 after 9.6; M11/M12 after M10.

## D. Owner decisions requested

| # | Decision | Recommendation |
|---|---|---|
| D1 | Approve subsystem table (§B) | Yes — audit-unanimous |
| D2 | Successor plan of record (ADR-027) | Approve |
| D3 | R0 governance ADRs start | Approve |
| D4 | k=4 harness VM spend | Defer until 9.6 pass 1 result |
| D5 | Code-signing certificate (~$100–400/yr; kills SmartScreen) | Approve when convenient — blocks only M13 "signed" items |
| D6 | Migration-0004 (suspension DDL) on production DB | Approve with nonproduction-first deploy |
| D7 | Earn-chain honest relabel NOW (labels `verified_capacity` as self-reported) | Approve — trivial, honest |
| D8 | Federation reversal preconditions (§C.2) | Acknowledge |
| D9 | Remote-serving default-off (`MSP_LISTENER=1`) stays until F2b + P5 + drain land | Acknowledge (then flip default with a release) |
| D10 | P1 streaming before any M9 baseline freeze | Acknowledge (ordering constraint) |
| D11 | 70% ceiling reconciled at M2 (recommendation engine may advise up to its own bar; runtime governor enforces 70%) | Approve shape |
| D12 | Relay internet exposure blocked until auth + caps | Acknowledge |
| D13 | CI pins hardening (commit-pin actions, toolchain pins, tauri-cli fold) | Approve |
| D14 | UI batch (copy button, chat_log clear, patch-in-place cards, geometry gate) | Approve |
| D15 | `download_model` IPC deletion | Approve |
| D16 | CHALLENGE_PROMPT replacement (nonce + digest) — rides the EXISTING `challengePrompt` wire field (no new fields); anything beyond rides the reconciliation ADR; plus Security's conditions (golden vectors, measured sampling rate) | Approve |
| D17 | ADR-016 index note ("reserved, not issued") | Approve |
| D18 | Presets registered in R0 per master prompt §3 table | Approve |
| D19 | Three new agents: msp-resource-engineer (M1/M2), msp-architecture-reviewer (this gate's seat), msp-gateway-engineer; revise 7 existing defs; assign relay crate to Network | Approve |

## E. Evidence index

Freeze + validation + CI story: `audit-freeze-2026-10-09.md` · system
consolidation: `audit-current-system.md` (F1–F17) · salvage:
`audit-salvage-matrix.md` · risks: `audit-production-risk-register.md`
(P1–P16 + R-item updates) · boundaries: `audit-rewrite-boundaries.md` ·
domain detail: the five wave-2 handoffs + architect handoff + security/
test/dependency/agent docs — all under `docs/reviews/`, 2026-10-09.

## F. What proceeds without waiting (already-sanctioned maintenance)

CI green upkeep (63b98e1 green — run 37858753428; 86450aa pushed, its
run 37860513036 in flight at gate time), gated-proof upkeep, doc-truth
fixes that change no behavior. Everything else in §C waits for this
gate's approval.

## G. Review-seat sign-offs (2026-10-09)

- **Security Engineer: APPROVE with binding conditions** —
  (1) D16 amended: the challenge nonce+digest change rides the R0
  reconciliation ADR + msp-v1 changelog + Rust↔TS golden-vector update —
  never a silent schema edit of the frozen `ChallengeCompleteSchema`;
  spot-verification sampling rate sized from measured detection math
  (master prompt §12), not invented. (2) D6 amended: migration-0004
  deploys nonproduction-first AND with a pre-migration DB snapshot,
  rollback SQL, and the real-Pg suite run against the new DDL before
  production. (3) F2b condition: the Swarm migration preserves the
  loopback honesty guard (`MSP_LISTENER=1`), dial-side PeerId
  verification, and size-before-allocation frame semantics, with parity
  tests on the unchanged public surface. D7 and D12 approved as stated;
  floor table and P-register confirmed carried as-is; no objection to
  §C ordering; P1/P4/P11 confirmed as post-gate first moves.
- **Test and Release Engineer: APPROVE with two binding conditions** —
  (1) EVIDENCE TRUTH: `63b98e1` is independently verified green (run
  37858753428, 17m32s, success incl. the tauri-shell step);
  `86450aa`'s run (37860513036) was IN FLIGHT at sign-off time — the
  gate and M0 may record "CI green restored" only once that run
  completes green; until then M0 records "green pending one run"
  (applied in §C.3 and §F). (2) The future tauri-shell CLIPPY gate must
  set the same `TAURI_CONFIG='{"bundle":{"resources":[]}}'` overlay —
  clippy executes build scripts and would otherwise fail on clean
  runners exactly like the just-fixed test step. Notes carried:
  simulator S5 case must inject synthetic spike delays, not runner
  wall-clock (avoid re-importing the 730cebf flake class); D14 items
  land with per-item verification steps and an automated geometry
  assertion where feasible (desktop currently has zero UI tests);
  critical-path ordering endorsed (P1-before-M9 — a frozen baseline
  would be invalidated by the 7.3× streaming change, and baselines must
  label their runtime build; 9.6-after-F15 per §11 untrusted-advertised
  values; k=4 deferral sized by pass-1 outcome).
- **Architecture seat (Protocol Architect, pro-tem): APPROVE —
  conditions folded verbatim above.** Required amendments, all applied:
  (1) §B protocol row + §C.1 reconciliation scope now enumerate the full
  audit list (not just "5 endpoints, error codes, §6.1"); (2) R0 split —
  the error-code rename is wire-visible and lands as R0.5 code change
  under the ADR, not inside "documentation-grade"; (3) R0 now includes
  creating `protocol/msp-cooperative-v1.md` (SessionOffer/preset home);
  (4) D16 conditional on the existing `challengePrompt` field. Approved
  as-is: §C.2 (with per-role profile attribution added as the M12 exit
  criterion), §C.3, D2 (ADR-027 restates the msp-v1 freeze carries
  forward), D17, D18 (wire enum only with SessionOffer; composition
  rules + per-tier disclosures in the ADR).
- **Integrator: APPROVE** (assembled from audit evidence; no dissent
  recorded in any of the eleven audits against §B/C/D).
