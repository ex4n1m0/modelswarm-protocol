# Repository Dependency Graph (Phase A snapshot)

Generated from workspace `Cargo.toml`s and `apps/tracker/package.json`;
refresh on every dependency change (AGENTS.md rule).

## Rust workspace (all external dependency counts: **zero** in Phase A)

```text
modelswarm-types ──────────────┐ (no deps; foundational IDs + validation)
modelswarm-identity            │ (no deps yet)
modelswarm-transport           │ (no deps yet)
modelswarm-tracker-api         │ (no deps yet)
modelswarm-eligibility         │ (no deps yet)
modelswarm-runtime             │ (no deps yet)
modelswarm-scheduler           │ (no deps yet)
modelswarm-session             │ (no deps yet)
modelswarm-speculation         │ (no deps yet)
modelswarm-store               │ (no deps yet)
modelswarm-gateway             │ (no deps yet)
modelswarm-telemetry           │ (no deps yet)
modelswarm-bench               │ (no deps yet)
modelswarm-node ──► modelswarm-types   (only intra-workspace edge today)
modelswarm-desktop             │ (no deps yet; Tauri arrives Phase G)
```

Planned dependency direction rules (enforced by review, linted when deps
land): transport never depends on scheduler/gateway; runtime never depends
on tracker-api (revision constraint); speculation depends only on types +
runtime trait; nothing depends on node/desktop.

## Tracker (apps/tracker)

prod: next ^15.3, react/react-dom ^19, zod ^3.24 · dev: typescript ^5.6,
@types/{node,react,react-dom} ^22/^19, **ajv ^8** (vector validator).
Zero runtime services yet; Postgres client arrives in Phase B.

## External processes

None bundled yet: llama.cpp sidecar (pinned, checksum-verified) arrives
Phase C; the research runtime (vLLM adapter or fork decision) is a Phase D
entry gate.
