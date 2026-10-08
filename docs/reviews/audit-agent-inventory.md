# Agent Definition Inventory — Master-Prompt §6 Audit

**Date:** 2026-10-09 · **Branch:** `audit/master-prompt-2026-10-09` (baseline tag
`audit-baseline-2026-10-09` = `730cebf`, v0.2.20) · **Auditor:** agent-auditor
(read-only; no definitions were modified)

**Method:** static reading only (no cargo/npm; a validation suite holds the
target lock). Sources: the nine definitions at `~/.zcode/agents/msp-*.md`
(Windows path `C:\Users\igorc\.zcode\agents\`), `AGENTS.md`, `.zcode/plans/`,
ADR log, verification docs, workflows, scripts, and the two untracked audit
inputs (`zcode-master-prompt.md`, `docs/reviews/gap-engineering-study-2026-10-09.md`).

---

## CRITICAL FINDINGS (top)

1. **CRITICAL — `crates/modelswarm-relay` is owned by no one.** The crate
   shipped 2026-10-08 (commit `a8d8a63`,
   `docs/verification/f2a-relay-2026-10-08.md:43`) but appears in neither the
   `AGENTS.md` ownership map (Network Engineer row lists
   `crates/modelswarm-transport` only, `AGENTS.md:101`) nor the Network
   Engineer definition's OWNED PATHS (`msp-network-engineer.md:25-28`). The
   integrator handoff claim "AGENTS.md ownership map covers every crate"
   (`docs/reviews/handoff-integrator-2026-10-08.md:57`) is FALSE for this
   crate. The freeze doc already flagged this
   (`docs/reviews/audit-freeze-2026-10-09.md:74`); this audit confirms it
   from the agent side and proposes the fix (ownership-map file, §Orphans).
2. **HIGH — dual claim on `crates/modelswarm-speculation`.** The `AGENTS.md`
   map assigns it to Scheduler Scientist (`AGENTS.md:102`); the Runtime
   Engineer definition also claims it as gated-owned
   (`msp-runtime-engineer.md:27`). The Scheduler definition (stale, see #3)
   omits it from its OWNED PATHS. Two agents believe they own one crate.
3. **HIGH — the "nine were revised 2026-10-08" claim did not fully hold.**
   File mtimes: seven definitions carry Oct 8 21:11–21:15; **
   `msp-prior-art-auditor.md` and `msp-scheduler-scientist.md` carry Oct 6
   00:06** — they were NOT touched by the ADR-020..026/F2 grounding pass.
   Content confirms it: the Scheduler definition cites only ADR-013 and knows
   nothing of the lease-gate/earn chain (ADR-026), the F2 relay path-type
   cost input (ADR-014 §4), or the fact that its crate is bench-only/unwired
   (`zcode-master-prompt.md:64-66`). The Prior-Art definition predates the
   2026-10-09 gap study's prior-art extension (BOINC/Gridcoin/Sarmenta/ACM
   CSUR live in `docs/reviews/gap-engineering-study-2026-10-09.md`, not yet
   in `docs/research/`).
4. **MEDIUM — `crates/modelswarm-identity` ownership contradicts its
   definition.** Map: Security Engineer owns it (`AGENTS.md:103`). Definition:
   Security is validator-only, "test and documentation writes only", and does
   not list the identity crate (`msp-security-engineer.md:25-28`). Nothing in
   the current definitions authorizes production edits to a live crate the
   map says Security owns (canonical-JSON work in it was done by the
   Integrator, `handoff-integrator-2026-10-08.md:54-56`).
5. **MEDIUM — `crates/modelswarm-winjob` missing from the Windows Product
   definition.** The map assigns it (`AGENTS.md:105`); the definition's OWNED
   PATHS list desktop/node/installer only (`msp-windows-product-engineer.md:23-27`)
   and never mention winjob — precisely the M2 resource-governor seed.
6. **MEDIUM — model configuration inconsistency.** Eight definitions pin
   `account:zai-individual-coding-plan/GLM-5.3`;
   `msp-security-engineer.md:5` pins `deepseek/deepseek-v4-pro`. If
   intentional (independent model for the independent release blocker), it
   should be recorded as such; otherwise it is drift.

---

## Summary table

| Agent (definition) | File mtime | Model / thoughtLevel | Tools | Owned paths per definition | Verdict |
|---|---|---|---|---|---|
| msp-prior-art-auditor | **Oct 6** (missed Oct 8 pass) | GLM-5.3 / max | Read, Grep, Glob, WebFetch, WebSearch (read-only) | docs/research/ (read-only; writes via primary agent) | **revise** (minor) |
| msp-protocol-architect | Oct 8 21:11 | GLM-5.3 / max | Read, Grep, Glob, WebFetch, WebSearch (read-only until ADR-granted write) | protocol/, catalog/, types/session/eligibility crates, docs/adr/, "protocol fixtures under tests/" | **revise** (minor) |
| msp-runtime-engineer | Oct 8 21:11 | GLM-5.3 / max | Read, Grep, Glob, Edit, Write, Bash | modelswarm-runtime, **modelswarm-speculation (gated — conflicts with map)**, scripts/, runtime-pins.json, runtime tests | **revise** |
| msp-network-engineer | Oct 8 21:11 | GLM-5.3 / high | Read, Grep, Glob, Edit, Write, Bash | modelswarm-transport, transport tests, network emulator configs — **modelswarm-relay absent** | **revise** |
| msp-scheduler-scientist | **Oct 6** (missed Oct 8 pass) | GLM-5.3 / max | Read, Grep, Glob, Edit, Write, Bash | modelswarm-scheduler, modelswarm-bench, experiments/ — **speculation absent vs map** | **revise** (highest priority) |
| msp-security-engineer | Oct 8 21:15 | **deepseek/deepseek-v4-pro** / max | Read, Grep, Glob, Bash, Edit, Write | threat-model.md, adversarial/fuzz tests under tests/, security ADR reviews — **modelswarm-identity absent vs map** | **revise** |
| msp-tracker-engineer | Oct 8 21:11 | GLM-5.3 / high | Read, Grep, Glob, Edit, Write, Bash | apps/tracker/, modelswarm-tracker-api, tracker schema/deploy tests | **keep** |
| msp-windows-product-engineer | Oct 8 21:11 | GLM-5.3 / high | Read, Grep, Glob, Edit, Write, Bash | modelswarm-desktop, modelswarm-node, installer/, product docs — **modelswarm-winjob absent vs map** | **revise** |
| msp-test-release-engineer | Oct 8 21:11 | GLM-5.3 / high | Read, Grep, Glob, Bash, Edit, Write | .github/workflows/, tests/, apps/modelswarm-sim, packaging/release metadata | **keep** |

**Verdict counts: keep 2 · revise 7 · merge 0 · replace 0 · remove 0.**
No definition is obsolete; none should be merged or removed. All seven
revisions are targeted text/ownership-list corrections, not rewrites.

---

## Per-agent detail

### 1. msp-prior-art-auditor — `C:\Users\igorc\.zcode\agents\msp-prior-art-auditor.md`

- **Purpose:** research comparable decentralized-inference systems, licenses,
  differentiation; read-only research role.
- **Model/reasoning:** GLM-5.3, `thoughtLevel: max` (frontmatter lines 5–6).
- **Tools/permissions:** Read, Grep, Glob, WebFetch, WebSearch — genuinely
  read-only against the repo; network access is appropriate for research.
  Writes to `docs/research/` are explicitly delegated to the primary agent
  (line 22). Security profile: minimal risk; no shell, no write.
- **File ownership:** `docs/research/` per map (`AGENTS.md:98`); the
  definition itself claims no write scope — the map's "Output required:
  verified comparison matrices" therefore depends on the primary agent
  applying the auditor's findings.
- **Inputs/outputs:** input = repo + live web sources; output = structured
  report with per-claim source link + retrieval date, completion report with
  handoff (lines 18–30).
- **Overlap:** adjacent to Scheduler Scientist (experimental validation
  claims) and Security (threat-relevant prior art); no path overlap.
- **Currency:** PARTIAL. Its mandatory comparables list (line 18:
  p2ptokens, LocalAI, Petals, Hyperspace, KwaaiNet, exo, Pooled, PARALLAX,
  distributed speculative decoding, FlowSpec) still matches
  `docs/research/prior-art-matrix.md` rows 12–21 — content is current even
  though the file missed the Oct 8 pass. Gaps: (a) no knowledge of the
  2026-10-09 gap-study research (BOINC/Sarmenta spot-checking, Gridcoin,
  EigenTrust, ACM CSUR reputation-attack survey — verified citations live in
  `gap-engineering-study-2026-10-09.md` §1/§3 and belong in
  `docs/research/`); (b) matrix header says "verified 2026-10-04" — star/
  commit counts aging; (c) no ADR grounding at all.
- **Security risks:** none material.
- **Verdict: revise (minor).** Add the reputation/economy comparables and a
  re-verification cadence; fold the gap-study citations into
  `docs/research/` via the normal write-through convention.

### 2. msp-protocol-architect — `msp-protocol-architect.md`

- **Purpose:** design msp-v1 contracts, ModelProfileId, capability
  negotiation, leases, state machines, golden vectors.
- **Model/reasoning:** GLM-5.3, max.
- **Tools/permissions:** read-only now; scoped write (Edit/Write/Bash over
  protocol/, catalog/, the three crates, docs/adr/, fixtures) is granted by
  editing the definition after ADR approval (line 24). This staged-write
  pattern is good security practice; the Bash grant is only active in that
  post-ADR state.
- **File ownership:** matches the map row (`AGENTS.md:99`) plus two
  additions the map does not list: `docs/adr/` (implied by "ADRs" output)
  and "protocol fixtures under tests/". **The latter is stale:** top-level
  `tests/` contains only `README.md` (observed); protocol fixtures actually
  live in `protocol/vectors/` (5 manifest vectors + `lease-hubkey-1.json`),
  which the map covers via `protocol/`.
- **Inputs/outputs:** input = msp-v1.md, schema, crates, ADR log; output =
  contracts, ADRs, state diagrams, golden vectors; handoff file once writing.
- **Overlap:** `catalog/` here vs Tracker Engineer's candidate flow
  (admin promote) and Runtime's `scripts/resolve-candidate.mjs` emitting
  schema manifests — coordinated by ADR-022/023; acceptable, documented.
- **Currency:** GOOD — cites ADR-005/007/011/012/013/020/022/023/024/025/026
  (line 18), i.e. the full ADR-020..026 range the audit was told to verify.
  Minor: says `catalog/schema.json` while `catalog/` also holds
  `schema-v2.json` + `candidate-profiles/` (observed; M3 calls these out in
  the master prompt). Does not mention `protocol/messages.proto` (exists)
  or the cross-language wire-compat job as a constraint on schema edits.
- **Security risks:** low; write scope is ADR-gated.
- **Verdict: revise (minor).** Reference schema-v2 and candidate-profiles,
  fix the fixtures location to `protocol/vectors/`, mention messages.proto
  and the wire-compat CI job as freeze constraints.

### 3. msp-runtime-engineer — `msp-runtime-engineer.md`

- **Purpose:** runtime abstraction — tokenization, prefill/decode, llama.cpp
  adapter, pinned HF acquisition, rollback, metrics.
- **Model/reasoning:** GLM-5.3, max.
- **Tools/permissions:** full Edit/Write/Bash. Justified for an
  implementation agent; risk is mitigated only by ownership discipline and
  AGENTS.md coordination rules. Note the definition's own scope limits
  ("Do not edit tracker, networking, scheduler, or product code", line 23).
- **File ownership:** `crates/modelswarm-runtime/`,
  `crates/modelswarm-speculation/ (gated)`, `scripts/`, `runtime-pins.json`,
  runtime tests/benches (lines 25–30). Two deltas vs map (`AGENTS.md:100` —
  runtime + scripts only): `runtime-pins.json` (root file; reasonable,
  should be added to the map) and **speculation, which the map gives to
  Scheduler Scientist** (`AGENTS.md:102`) — the conflict in Critical #2.
- **Inputs/outputs:** input = ADR-019/021/022/024/002/005 state; output =
  runtime crate work + handoff with reproducible measurements.
- **Overlap:** speculation crate (conflict, above); `scripts/` includes
  `resolve-candidate.mjs`/`promote-candidate.mjs`, which talk to tracker
  admin routes — cross-boundary with Tracker Engineer but read-only API
  clients; fine.
- **Currency:** MOSTLY GOOD — ADR-019 adapters, ADR-021 engine decision with
  the logprobs.bytes token-recovery detail, ADR-024 Vulkan engine,
  runtime-pins platforms{}, ADR-022 GGUF-anchored identity all match the
  repo. **One stale statement:** line 19 calls the Qwen2.5-7B gateway decode
  failure "open" and the bytes-path "the prime suspect" — that failure was
  root-caused and FIXED the same day (empty-bytes special tokens; fix
  prefers server-reported `logprobs.content[].id`, commit `5642224`,
  `docs/verification/f2a-relay-2026-10-08.md:1-39`, 17/17 tests + live
  evidence). The definition must state the fix and the fail-closed
  disagreement tripwire as current reality.
- **Security risks:** medium-generic (write+Bash), contained by scope text.
- **Verdict: revise.** Fix the stale Qwen statement; drop the speculation
  claim in favor of trait-integration rights (algorithms stay with
  Scheduler per map), or get an ADR moving the crate — pick one, not both.

### 4. msp-network-engineer — `msp-network-engineer.md`

- **Purpose:** authenticated direct P2P transport; framing, NAT traversal
  (ADR-014), deadlines, backpressure, connection limits, telemetry.
- **Model/reasoning:** GLM-5.3, `thoughtLevel: high` (the only transport-
  adjacent agent not at max; defensible for an implementation role, noted
  for completeness).
- **Tools/permissions:** Edit/Write/Bash; same medium-generic profile.
- **File ownership:** `crates/modelswarm-transport/`, transport integration
  tests, network emulator configurations (lines 25–28). **Missing
  `crates/modelswarm-relay/`** even though the definition's own task text
  (line 19) says "Your current open task class is F2: a rust-libp2p
  circuit-relay-v2 relay binary" — the agent is told to build a crate its
  OWNED PATHS list and the AGENTS.md map both fail to grant.
- **Inputs/outputs:** input = ADR-018 staging, ADR-014 roadmap; output =
  transport/relay work + handoff.
- **Overlap:** ADR-026 assigns this agent "transport PeerId binding"
  (`ADR-026-lease-gate-at-session-open.md:6`); serving-side lease gating
  lives in node/serving.rs (Windows Product) — documented cross-review, fine.
- **Currency:** MOSTLY GOOD — libp2p 0.57 QUIC, ADR-020-derived keys,
  pooled per-peer connections, raised relay limits (4096 reservations,
  uncapped circuits) all match `f2a-relay-2026-10-08.md:41-64`. Stale in one
  way: F2A is DONE (crate shipped, relay.exe staged on the owner's desktop,
  `f2a-relay-2026-10-08.md:43,74`), while the definition frames F2 as
  current. Next task per integrator handoff is the F2B cross-machine relay
  proof (`handoff-integrator-2026-10-08.md:112-114`).
- **Security risks:** the relay is a public service to run and secure
  (ADR-014 consequences); the definition correctly forbids Vercel hosting.
  Definition-level permissions are unremarkable.
- **Verdict: revise.** Add `crates/modelswarm-relay/` to OWNED PATHS (and
  mirror in the map); mark F2A complete; set F2B (real-network relay proof,
  DCUtR) as the open task class.

### 5. msp-scheduler-scientist — `msp-scheduler-scientist.md` (stalest)

- **Purpose:** fastest-single baselines, latency-aware selection, cost model
  v2, simulators, honest benchmarks incl. negatives.
- **Model/reasoning:** GLM-5.3, max.
- **Tools/permissions:** Edit/Write/Bash; medium-generic.
- **File ownership:** `crates/modelswarm-scheduler/`, `crates/modelswarm-bench/`,
  `experiments/` (lines 23–26). Map row (`AGENTS.md:102`) additionally lists
  **`crates/modelswarm-speculation`** — absent here (Critical #2's other
  half).
- **Inputs/outputs:** input = ADR-013 + measurements; output = cost model,
  harness, experiments/ raw+processed data, handoff.
- **Overlap:** speculation (vs Runtime def); `apps/modelswarm-sim` is
  Test-and-Release-owned but consumes scheduler policy — coordination only.
- **Currency:** STALE (Oct 6 — missed the Oct 8 grounding pass):
  - cites only ADR-013 (line 19); no ADR-020..026;
  - unaware the scheduler crate is **bench-only, unwired**
    (`zcode-master-prompt.md:64-66` ground truth: "latency-aware selection
    (the scheduler crate is bench-only, unwired)");
  - unaware of ADR-014 §4 (NAT path type direct/hole-punched/relayed is a
    scheduler input with a relay cost penalty);
  - unaware of the earn-chain/lease gate (ADR-026) as a scheduling
    precondition, the presets layer (gap study §2 / master prompt §3), the
    fair-queueing work (gap study §4b), and the expanded-mission §5 planner
    shape (measurement plumbing, shadow mode, measured τ).
- **Security risks:** none beyond generic write+Bash.
- **Verdict: revise — highest priority of the nine.** Add current-reality
  grounding (bench-only status, ADR-014/026 inputs, planner proposal,
  presets, negative-results gates) and reconcile the speculation row.

### 6. msp-security-engineer — `msp-security-engineer.md`

- **Purpose:** threat-model hostile peers; validate signatures, replay,
  artifact proofs, parsers, tracker content-blindness; independent release
  blocker.
- **Model/reasoning:** **`deepseek/deepseek-v4-pro`** (line 5 — the only
  definition off the shared GLM-5.3 pin), max.
- **Tools/permissions:** Read/Grep/Glob/Bash/Edit/Write, explicitly
  validator-scoped: "you may write and edit tests, fuzz targets, and
  security documentation, but never production behavior" (line 23). Bash for
  running adversarial suites is appropriate. Sound privilege design.
- **File ownership:** `docs/threat-model.md`, adversarial/fuzz tests under
  `tests/` (coordinate with T&R), security ADR reviews (lines 25–28). **Map
  row (`AGENTS.md:103`) also lists `crates/modelswarm-identity`** — absent
  here (Critical #4). Also: `docs/research/fuzz-targets.md` is this agent's
  working input (line 21) but lives in Prior-Art Auditor's owned path.
- **Inputs/outputs:** input = threat model, AGENTS.md constraints, ADR-026
  four enforcement points; output = abuse cases, property/fuzz/replay tests,
  severity-classified findings, handoff.
- **Overlap:** tests/ shared with Test and Release (explicitly coordinated,
  line 21 — acceptable); generic `security-reviewer.md` in the same
  directory (see §Other agent configs below) partially duplicates the
  mission at a project-agnostic level.
- **Currency:** GOOD — the four live enforcement points including the
  ADR-026 lease gate and the earn-chain ordering (line 19) match ADR-026 and
  the tracker definition. Missing: `crates/modelswarm-identity` scope; no
  mention of the reputation-attack surface now formalized in the gap study
  §3 / master prompt §12 (sybil rings, whitewashing, slandering,
  orchestrated combos; Sarmenta q-sizing) — that work will need this agent.
- **Security risks:** none in the permission set; the definition IS the
  control. Independence rule ("Do not approve your own security fixes",
  line 23) is present and good.
- **Verdict: revise.** Resolve identity-crate ownership (grant scoped write
  or reassign in map), add the reputation-attack mandate, decide and record
  the model pin.

### 7. msp-tracker-engineer — `msp-tracker-engineer.md`

- **Purpose:** content-blind control plane — registration, heartbeats,
  candidates, signaling, leases, receipts, audits, community queue.
- **Model/reasoning:** GLM-5.3, high.
- **Tools/permissions:** Edit/Write/Bash with an explicit production-deploy
  prohibition: "never deploy to production from a subagent; propose the
  deploy and its checks instead" (line 23). Exactly right.
- **File ownership:** `apps/tracker/` (incl. the public website/download
  counting), `crates/modelswarm-tracker-api/`, tracker schema/deploy tests
  — matches map (`AGENTS.md:104`).
- **Inputs/outputs:** input = ADR-012/023, hub-key discipline; output =
  API/migrations/enforcement + npm typecheck/build handoff evidence.
- **Overlap:** `.github/workflows/tracker.yml` (incl. the wire-compat job
  added by the Integrator) is T&R-owned — the definition correctly claims
  only "tracker schema and deployment tests"; `scripts/resolve-candidate.mjs`
  (Runtime) calls tracker admin routes — coordinated via ADR-023.
- **Currency:** EXCELLENT — earn-lease 403-no_capability flow,
  DEVICE_AUTO_APPROVE=1 under DEVICE_APPROVAL_CAP=250, manual /verify
  fallback, Vercel-CLI deploy path, gitignored installers under
  public/downloads (lines 17–23) all match the freeze record and
  integrator handoff.
- **Security risks:** generic write+Bash, offset by the deploy prohibition
  and content-blindness mandate.
- **Verdict: keep.** No changes required.

### 8. msp-windows-product-engineer — `msp-windows-product-engineer.md`

- **Purpose:** Windows product — node daemon composition, Tauri UI, NSIS
  installer, hosting controls, diagnostics, honest mode reporting.
- **Model/reasoning:** GLM-5.3, high.
- **Tools/permissions:** Edit/Write/Bash; medium-generic.
- **File ownership:** `crates/modelswarm-desktop/`, `crates/modelswarm-node/`,
  `installer/`, product docs (lines 23–27). **Map row (`AGENTS.md:105`) also
  lists `crates/modelswarm-winjob/`** — absent here (Critical #5). Note the
  definition DOES cover node's serving stack (serving.rs/remote.rs,
  MSP_LISTENER=1 disclosure, line 17) which is current and correct.
- **Inputs/outputs:** input = ADR-008/017/022/024/025/026 state; output =
  UI/daemon/installer + handoff with e2e-windows evidence.
- **Overlap:** serving.rs lease gating co-implemented with Security/Network
  per ADR-026's owner list (`ADR-026:5-7`) — documented, fine. The gateway
  port 11435 the desktop chat depends on is Integrator-held (gap-analysis
  candidate).
- **Currency:** GOOD — all-OS release rule, dual CPU/Vulkan staging via
  `installer/stage-engine-windows.mjs` (file exists), ChatML rendered once
  (ADR-025), lease-state surfacing per ADR-026, privacy plaintext warning.
  Missing only winjob.
- **Security risks:** generic write+Bash; the data-dir split (uninstall
  cannot delete identity/models) is already handled in code per integrator
  handoff.
- **Verdict: revise (small).** Add `crates/modelswarm-winjob/` to OWNED
  PATHS (or, if the proposed resource-engineer agent is adopted, transfer it
  there explicitly — see gap analysis).

### 9. msp-test-release-engineer — `msp-test-release-engineer.md`

- **Purpose:** CI, fixtures, integration environments, packaging, release
  gates; independent validator that blocks releases.
- **Model/reasoning:** GLM-5.3, high.
- **Tools/permissions:** Read/Grep/Glob/Bash/Edit/Write, validator-scoped
  ("may edit tests, CI, packaging, and release metadata, but not production
  behavior", line 23). Appropriate.
- **File ownership:** `.github/workflows/`, `tests/` (integration,
  e2e-windows, fixtures), `apps/modelswarm-sim/`, packaging/release metadata
  (lines 27–31) — matches map row (`AGENTS.md:106`) AND the map footnote
  (`AGENTS.md:109`, "apps/modelswarm-sim is owned by Test and Release").
- **Inputs/outputs:** input = workflows, sim, verification pattern; output =
  re-run machine-readable results, release checklists, handoff.
- **Overlap:** tests/ shared with Security (coordinated); tracker.yml is in
  its workflows tree though the wire-compat job exercises tracker code —
  covered by "Owner of the component under test" reviewer rule.
- **Currency:** EXCELLENT — wire harness `live_tracker.rs` with the
  three-shipped-wire-bugs rationale (matches
  `crates/modelswarm-node/tests/live_tracker.rs`, exists), all-OS rule,
  gitignored binaries + tracked SHA256SUMS, manual-dispatch real-model-e2e,
  docs/verification/ convention (lines 25–31) all match repo reality.
- **Security risks:** low; guardrails explicit ("never disable a failing
  test to pass CI").
- **Verdict: keep.** No changes required.

---

## Non-definition agent configuration

### AGENTS.md (ownership map + rules) — verdict: revise

The map's nine-row roster correctly mirrors ADR-010/015. Defects, all
evidence-cited above: (a) relay crate absent from Network row; (b)
speculation/identity/winjob rows disagree with the respective definitions in
both directions; (c) `runtime-pins.json`, `docs/adr/`, `docs/architecture.md`,
`docs/verification/`, `docs/reviews/`, `docs/privacy.md`,
`docs/risk-register.md`, `docs/deployment.md`, root `README.md`, root
`Cargo.toml`/`Cargo.lock`/`rust-toolchain.toml`, `.gitignore` family, and
`ModelSwarmProtocol..png` have no owner (full orphan table in the
ownership-map file); (d) **the cooperative-track section still names the
pre-ADR-010 crate set** (`crates/ms-coop-protocol`, `ms-decode`, `ms-planner`,
`ms-verification`, `ms-network-model`, `apps/ms-bench`, `apps/ms-coop-sim` —
`AGENTS.md:82-87`); those names were superseded by the restructure
(ADR-010; actual crates are `modelswarm-speculation` etc.) and will mislead
whoever opens P0; (e) `docs/architecture.md`'s crate table itself is missing
`modelswarm-winjob` and `modelswarm-relay` rows (`docs/architecture.md:70-85`).

### `.zcode/plans/plan-sess_c3803811-...md` — verdict: keep (historical)

The only plan file. It is the Phase H plan (real-model Windows client),
fully executed and evidenced (`docs/verification/phase-h.md` exists; phase-i
follow-ups too). No agent-conflict content; it references `HANDOFF`-free
flow and correct ADRs (008/011/021/022). Keep as history; do not execute
again. Nothing else exists under `.zcode/`.

### Other files in `~/.zcode/agents/` — verdict: keep, with one caution

`image-analyst.md`, four `n38worth-*.md`, six `rd-*.md` (all Sep 30–Oct 8)
belong to other projects and carry MSP-irrelevant scopes; `security-reviewer.md`
(Sep 30, project-agnostic, broader tools incl. WebFetch/WebSearch/TodoWrite)
overlaps `msp-security-engineer` in mission. **Caution:** an orchestrator
invoking "the security agent" could hit the generic one, which lacks every
MSP floor rule (content-blindness, loopback binding, no-secrets). The MSP
roster's `msp-` prefix is the discriminator; no action required beyond
naming discipline, recorded here so the risk is known.

### Referenced-in-repo agent configuration

Repo-wide grep for agents/`msp-`/`HANDOFF` references: `zcode-master-prompt.md`
§6–§7 (the driver of this audit); ADR-026 names per-aspect owners
(Security/Windows-Product/Network + Architect/T&R reviewers — the ADR-log
mechanism working as designed); `docs/reviews/handoff-integrator-2026-10-08.md`
documented the four review agents; workflows/scripts contain no agent
references (only `x-msp-admin` headers — protocol strings, not agents).
`.github/workflows/` five files all fall under T&R ownership. No further
agent configuration exists in the repo.

---

## Was the 2026-10-08 revision maintained? (explicit check)

Partially — 7 of 9. Held: protocol-architect, runtime-engineer,
network-engineer, tracker-engineer, windows-product-engineer,
security-engineer, test-release-engineer (all reference ADR-020..026 and/or
the F2 relay where relevant to their domain). Did NOT hold:
**msp-prior-art-auditor** and **msp-scheduler-scientist** (Oct 6 mtimes;
content confirms no ADR-020..026, no relay, no lease-gate knowledge).
Pre-Phase-A crate names: none found in any definition (good — no `ms-*`
names survive in the nine; they survive only in AGENTS.md's cooperative
section, q.e.d. above).
