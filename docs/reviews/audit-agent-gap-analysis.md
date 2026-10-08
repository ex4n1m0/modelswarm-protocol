# Agent Gap Analysis — Master-Prompt §7 (extend, do not duplicate)

**Date:** 2026-10-09 · **Branch:** `audit/master-prompt-2026-10-09` ·
Companion to `audit-agent-inventory.md` (per-agent detail) and
`audit-agent-ownership-map.md` (proposed map). Read-only audit; no agent
was created, modified, or removed.

Ground truth used: the nine installed `msp-*` definitions, the `AGENTS.md`
map, `zcode-master-prompt.md` §0/§6/§7 (verified-state snapshot), the
expanded-mission review (`docs/reviews/expanded-mission-roadmap-review-2026-10-08.md`),
and the gap engineering study
(`docs/reviews/gap-engineering-study-2026-10-09.md`).

---

## 1. Overlap matrix (definitions and map combined)

Legend: **OWN-CONFLICT** = two agents claim write rights to the same path;
**COORD** = deliberate cross-review coordination (documented, healthy);
**READ** = one consumes the other's output; blank = no interaction.

| Path / concern | PriorArt | Architect | Runtime | Network | Scheduler | Security | Tracker | WinProduct | TestRel | Integrator |
|---|---|---|---|---|---|---|---|---|---|---|
| `crates/modelswarm-speculation` | | | **claims (gated)** | | map-owner; def omits | | | | | — |
| `crates/modelswarm-identity` | | | | | | map-owner; def refuses writes | | | | de-facto editor (canonical-JSON shim, integrator handoff #7) |
| `crates/modelswarm-winjob` | | | | | | | | map-owner; def omits | | |
| `crates/modelswarm-relay` | | | | does the work; **no claim, no map row** | | | | | | |
| `runtime-pins.json` | | | claims | | | | | | | not in map |
| `tests/` | | fixtures claim (stale) | runtime tests | transport tests | adversarial/fuzz (COORD) | tracker tests | e2e-windows | map-owner | |
| `docs/adr/` | | def grants write | | | | reviews | | | implicit (ADR-log keeper) |
| `docs/research/fuzz-targets.md` | map-owner | | | | | uses + edits? (undefined) | | | | |
| `catalog/` candidates flow | | map-owner | `scripts/resolve+promote` call tracker admin | | | | promote routes (COORD) | | | |
| `.github/workflows/tracker.yml` (wire-compat job) | | schema freeze | | | | | tests it | | map-owner | authored it (93bccb5) |
| lease gate (ADR-026) | | msp-v1 §5 | | PeerId binding | unaware (stale def) | gate semantics | issuer | serving.rs attach | gate tests | fixed H1 interop |
| gateway port 11435 / serving admission | | | | | fair-queueing lands here (unaware) | | | consumes (node chat) | | map-owner of `modelswarm-gateway`, "held pending ADR reassignment" |

Three real ownership conflicts (speculation, identity, winjob — all
inventory Critical #2/#4/#5), one orphan doing real work (relay), and one
"held" crate with active upcoming work and no implementing agent
(gateway). Everything else is documented coordination.

---

## 2. Draft-role mapping check (master prompt §7 table)

All thirteen mapped rows in `zcode-master-prompt.md:345-359` verified
correct against the current map and definitions — no corrections needed.
Two notes: "Cooperative-Inference Engineer" correctly maps to Scheduler +
Runtime with the AGENTS.md prerequisite gate still closed (P0 conditions
not yet demonstrably met repo-wide), and "Release Engineer" split across
WinProduct (installers) + TestRel (gates) matches the actual handoff
practice in `handoff-integrator-2026-10-08.md`.

## 3. Candidate new agents — genuine gap or extend?

### 3.1 hardware-profiling + resource-governor engineer (M1 + M2) — **GENUINE GAP, CREATE**

- M1 (platform-neutral hardware profiler) and M2 (resource governor) are
  brand-new subsystems with **no owner in the map or any definition**.
  Master prompt §0: "Does not exist: automatic hardware analysis … a
  cross-platform resource governor (Windows `winjob` crate only handles
  kill-on-close job objects + total RAM today)". §2 names the governor a
  new platform-neutral crate; M1 names the profiler crate.
- Why not extend Runtime Engineer: runtime owns engine interaction and is
  already the largest write-scoped owner; hardware detection, OS resource
  enforcement, thermal/battery/foreground policy is a distinct competency
  and a distinct review surface (needs Windows Product + TestRel review on
  every platform path).
- Why not extend Windows Product: the governor must be platform-neutral
  (§2: "platform-specific enforcement lives behind it"); hanging it off the
  Windows agent bakes Windows assumptions into a cross-platform crate and
  overloads the product agent.
- **Recommended definition: `msp-resource-engineer`.**
  - Owned paths: `crates/modelswarm-profiler/` (new, M1),
    `crates/modelswarm-governor/` (new, M2), `crates/modelswarm-winjob/`
    (transferred from Windows Product — it is the M2 Windows seed the
    master prompt names; transfer prevents dual ownership the moment the
    governor generalizes it), hardware-matrix docs under `docs/research/`.
  - Output required: validated hardware-profile matrix, governor policy
    implementation, memory-exhaustion/responsiveness tests.
  - Required reviewers: Runtime, Windows Product, Test and Release.
  - Creation is ADR-gated like any roster change (Integrator records it);
    nothing is implemented before the §9 architecture gate approves M1/M2.

### 3.2 architecture reviewer (read-only, reports to Integrator) — **GENUINE GAP, CREATE (minimal)**

- Master prompt §9 requires an **Architecture Reviewer** (and an
  "Integration Reviewer") among the approvers of the consolidated decision
  (`zcode-master-prompt.md:453-456`); no such role exists today. The
  Protocol Architect proposes and cannot approve its own designs; the
  Integrator merges but is not staffed as an independent reviewer.
- The four-agent review batch of 2026-10-08 (`handoff-integrator-2026-10-08.md:4-6`)
  shows the pattern works when improvised; a standing read-only reviewer
  makes it repeatable and satisfies "independent review" in §14 change
  control.
- **Recommended definition: `msp-architecture-reviewer`.** Read-only tools
  (Read, Grep, Glob); outputs findings/approvals under `docs/reviews/`;
  reports to the Integrator; reviews cross-agent ADRs, supersession maps,
  and §9 gate decisions. No owned production paths (it is a reviewer, not
  an implementer — deliberately no map row beyond a reviews output).

### 3.3 local-API engineer (gateway, Integrator-held today) — **GENUINE GAP, CREATE (small)**

- The map parks `crates/modelswarm-gateway` with the Integrator "held
  pending ADR reassignment" (`AGENTS.md:107`). The Integrator is a merge
  role, yet real gateway work is imminent and some already happened ad hoc:
  the Qwen2.5-7B gateway decode fix (`f2a-relay-2026-10-08.md:1-39`) and
  the gateway-binds-before-engine fix
  (`handoff-integrator-2026-10-08.md:34-37`). Upcoming: bounded wait queue +
  DRR fair queueing keyed on receipts-v2 accounting (gap study §1/§4b —
  "gateway fair queueing (DRR + per-installation token bucket)") and the
  §1 port-11435 product audit. None of this has an implementing owner.
- Alternative considered — extend Windows Product (node/serving.rs already
  holds admission control): rejected as the primary home because admission
  fairness is protocol policy (quota classes ride the lease), not product
  UI, and it needs Protocol + Security review rather than product review.
- **Recommended definition: `msp-gateway-engineer`.**
  - Owned paths: `crates/modelswarm-gateway/`; serving-admission policy in
    `crates/modelswarm-node/src/serving.rs` jointly with Windows Product
    (COORD, mirroring the ADR-026 owner-list pattern).
  - Output required: OpenAI-compatible local API, queue semantics per
    msp-v1 §6, fair-queueing implementation from receipts-v2 (once that
    ADR lands), loopback-default enforcement.
  - Required reviewers: Protocol Architect, Security, Test and Release.
  - Mechanism: an ADR closes the map's own "pending ADR reassignment" note.

### 3.4 repository auditor — **NOT a gap (transient specialist only)**

This audit (freeze record, inventory, gap analysis, salvage matrix) is the
repository auditor's job and is nearly done; a standing agent would
duplicate Test-and-Release validation (guards job, wire-compat job —
`handoff-integrator-2026-10-08.md:23-29`) and the Integrator's merge
review. Master prompt §7 allows "temporary specialists where justified" —
that is the right shape for future re-audits. Do not create.

### 3.5 model-catalog engineer — **NOT a gap (extend Protocol Architect)**

`catalog/` (schema.json, schema-v2.json, candidate-profiles/) is
Architect-owned; the resolver/promote scripts are Runtime-owned; the
promote routes are Tracker-owned. M3's remaining work (signature/approval
enforcement) is protocol-contract work. Every file already has exactly one
owner; the gap is ADR-gated enforcement, not staffing. Extend the Architect
definition's currency (schema-v2) per the inventory; create nothing.

### 3.6 deliberation engineer (M11) — **DEFERRED by the project's own gate**

AGENTS.md's prerequisite gate (`AGENTS.md:75-81`) bans cooperative agents
until the original prototype demonstrably passes its checklist; the
cooperative plan already prescribes appending planner/decode agents at P0
under its own mechanism. Creating a deliberation agent now would violate
the repo's governing rules. Defer to P-series; when created, follow the
cooperative-plan append mechanism (and fix its stale crate names — see
ownership-map file).

### 3.7 multi-model-orchestration engineer (M12) — **DEFERRED behind the §9 gate**

Federation reverses a recorded non-goal (expanded-mission §11) and the
master prompt §4 requires the reversal to be recorded with preconditions
at the architecture gate first. No crate exists; no agent until an ADR
does. Defer.

### 3.8 independent red-team reviewer — **NOT a gap (extend Security Engineer)**

`msp-security-engineer` is already the "independent release blocker", is
prohibited from approving its own fixes, and writes no production code.
The master prompt's §9 approval quorum (Security + Test + Architecture
Reviewer + Integrator) supplies the independence. What IS missing is the
reputation-attack red-team mandate (gap study §3, master prompt §12) — add
it to the Security definition's grounding (inventory already recommends
this). Creating a second security agent would produce exactly the parallel
bureaucracy §7 forbids, and the generic `security-reviewer.md` already
poses the name-collision caution (inventory, §Other agent configs).

---

## 4. Recommended minimal set (summary)

**Create exactly three, all ADR/Integrator-recorded, none before the §9
architecture gate approves the corresponding phase:**

| New agent | Owned paths | Required reviewers | Unblocks |
|---|---|---|---|
| `msp-resource-engineer` | `crates/modelswarm-profiler/` (new), `crates/modelswarm-governor/` (new), `crates/modelswarm-winjob/` (transfer) | Runtime, Windows Product, Test and Release | M1, M2 |
| `msp-architecture-reviewer` | none (read-only; outputs under `docs/reviews/`) | — (is a reviewer; reports to Integrator) | §9 gate independence |
| `msp-gateway-engineer` | `crates/modelswarm-gateway/`; serving admission COORD with Windows Product | Protocol, Security, Test and Release | M10 fair queueing, §1 local-API audit |

**Extend, do not create:** model-catalog → Protocol Architect; red-team →
Security Engineer; repository audits → temporary specialists; deliberation
and federation → deferred by the prerequisite gate and the §9 gate
respectively; SRE/observability → Test and Release (per §7, unchanged).

Net roster: 9 installed + 3 proposed = 12 standing agents against the
owner's 25-role draft — still "extend, do not duplicate".
