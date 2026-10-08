# Handoff — agent-auditor — 2026-10-09

Master-prompt §6/§7 audit of every agent definition (nine `msp-*` files,
`AGENTS.md` map, `.zcode/plans/`, repo-referenced agent configuration).
Read-only audit: no code, config, doc, or agent definition was modified; no
git commit/push; no deploy; no cargo/npm (validation suite holds the
target lock).

## 1. Changed files (paths) and why

Created, and only these (all new files under the repo-convention
`docs/reviews/`):

1. `docs/reviews/audit-agent-inventory.md` — per-agent record for all nine
   definitions + AGENTS.md + `.zcode/plans/` + other agent config: purpose,
   model/reasoning, tools, ownership, inputs/outputs, overlap, security
   risks, currency vs ADR-020..026/F2/relay/vectors, verdict each.
2. `docs/reviews/audit-agent-gap-analysis.md` — overlap matrix; §7
   draft-role mapping check; genuine-gap verdicts; minimal new-agent set.
3. `docs/reviews/audit-agent-ownership-map.md` — full path-coverage audit
   (orphans) and proposed updated ownership map, every change marked and
   justified.

## 2. Exact commands run and outcomes

- `ls -la C:\Users\igorc\.zcode\agents\` — nine `msp-*.md` present
  (expected set confirmed); mtimes show **2 of 9 (prior-art-auditor,
  scheduler-scientist) dated Oct 6, not the claimed Oct 8 revision**;
  unrelated agents (image-analyst, n38worth-*, rd-*, security-reviewer)
  noted as context.
- Full reads: `AGENTS.md`, `zcode-master-prompt.md`, all nine `msp-*.md`,
  `.zcode/plans/plan-sess_c3803811-...md`,
  `docs/reviews/gap-engineering-study-2026-10-09.md`,
  `docs/reviews/handoff-integrator-2026-10-08.md`,
  `docs/reviews/audit-freeze-2026-10-09.md`,
  `docs/reviews/expanded-mission-roadmap-review-2026-10-08.md` (§15 +
  outline), `docs/verification/f2a-relay-2026-10-08.md`, `phase-i.md`
  (head), ADR-014, ADR-026, `docs/architecture.md` (crate table),
  `docs/research/prior-art-matrix.md` (+notes head).
- `git grep`/`grep` sweeps: agent/`msp-`/HANDOFF references across
  `docs/`, `scripts/`, `installer/`, `.github/`, README (workflows and
  scripts contain none — `x-msp-admin` strings are protocol headers, not
  agents); `modelswarm-relay` references repo-wide (map absent);
  `ModelSwarmProtocol..png` references (none — orphaned asset);
  `ADR-016` (absent by design — closed not-needed, `phase-a.md:59`).
- `ls` enumerations: repo root, `crates/` (17), `apps/` (2), `docs/adr/`
  (25), `docs/verification/` (13), `docs/reviews/`, `docs/research/`,
  `catalog/`, `scripts/`, `tests/`, `.github/workflows/`, `.zcode/`.
- `Cargo.toml` read: workspace members confirm the 17-crate set incl.
  `modelswarm-relay` and `modelswarm-winjob`; version 0.2.20 single-source.
- No cargo/npm/git-mutating commands were run (constraint honored).

## 3. Test evidence (command + summary)

Not applicable by design: this is a static audit with a no-cargo/no-npm
constraint; its "tests" are the evidence citations (file:line) throughout
the three output files. Every finding carries at least one path +
line/section reference (e.g. relay orphan: `AGENTS.md:101` vs
`docs/verification/f2a-relay-2026-10-08.md:43`).

## 4. Assumptions made

- File mtimes on `~/.zcode/agents/` are reliable evidence of the 2026-10-08
  revision pass (agent defs live outside the repo; no git history exists
  for them). Content staleness corroborates the two Oct 6 files.
- `AGENTS.md` ownership-map rows are the governing assignment where a
  definition's OWNED PATHS disagree (map wins; defs must be revised).
- The `deepseek/deepseek-v4-pro` pin on msp-security-engineer may be an
  intentional independence choice — flagged for decision, not assumed wrong.
- The three proposed new agents are recommendations; nothing is created
  until the Integrator records the roster change and the §9 gate approves
  the corresponding M-phases.
- Master prompt §0's verified-state snapshot was re-checked against the
  repo (HEAD 730cebf, v0.2.20, relay crate + F2A doc, lease-hubkey-1.json
  all present) and accepted as accurate.

## 5. Unresolved risks

- `crates/modelswarm-relay` remains ownerless in the live map until the
  Integrator applies the proposed row — any relay change today formally
  violates the cross-boundary rule.
- `crates/modelswarm-speculation` dual claim (Runtime def vs map/Scheduler)
  is live: two agents could edit it concurrently believing they own it.
- `crates/modelswarm-identity` has a map owner (Security) whose definition
  forbids production writes — the next identity change has no authorized
  editor under the definitions as written.
- The generic `security-reviewer.md` in the same agents directory can be
  mistaken for the MSP security role (it carries none of the MSP floor
  rules); naming discipline required until reconciled.
- `docs/architecture.md`'s crate table is missing `modelswarm-winjob` and
  `modelswarm-relay` rows — it will mislead the next reader even after the
  map is fixed (separate doc edit, outside this audit's write scope).
- AGENTS.md's cooperative-track section still names pre-ADR-010 crates
  (`ms-*`) — will misdirect P0 when the gate opens.

## 6. Suggested next task for the integrator

Apply the ownership-map proposal (one batch, one ADR log entry):
1. Add `crates/modelswarm-relay` to the Network Engineer row (critical).
2. Resolve the three definition-vs-map conflicts (speculation stays with
   Scheduler; identity stays with Security + scoped-write def update;
   winjob per the resource-engineer decision).
3. Record the three new agents (`msp-resource-engineer`,
   `msp-architecture-reviewer`, `msp-gateway-engineer`) — or defer each to
   its gate with the assignment pre-recorded.
4. Assign the orphaned docs/root files per the proposed table.
5. Update the stale crate names in AGENTS.md's cooperative section and the
   architecture.md crate table.
Then revise the seven agent definitions per ownership-map §4 (the two
`keep` verdicts — tracker, test-release — need nothing).
