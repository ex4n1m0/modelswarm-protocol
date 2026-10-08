# Handoff — code-auditor (full code inventory audit) — 2026-10-09

Per `AGENTS.md` handoff format. Companion deliverable:
`docs/reviews/audit-code-inventory.md` (full tables + evidence).

## 1. Scope reviewed

All code at HEAD `730cebf` (branch `audit/master-prompt-2026-10-09`):
17 crates under `crates/`, `apps/tracker` (Next.js control plane),
`apps/modelswarm-sim`, `protocol/` (msp-v1.md, vectors, keys,
messages.proto), `catalog/` (schema.json, schema-v2.json,
candidate-profiles), `installer/`, `scripts/`, `experiments/`, `tests/`,
`.github/workflows/` (5 workflows), `runtime-pins.json`, root manifests.
Static analysis only — no cargo/npm execution (background validation suite
holds the target lock); no code, config, or doc modified; no git writes.
Untracked context files `zcode-master-prompt.md` and
`docs/reviews/gap-engineering-study-2026-10-09.md` were read, not touched.

## 2. Files inspected (summary counts)

310 tracked files total. Read in full or in targeted depth: all 18 workspace
`Cargo.toml`s, ~30 Rust source files (~18k lines of the ~23k-line Rust
codebase, incl. every production-path file), `apps/tracker` structure +
store/test surface (15 test files, 120 tests), the Tauri UI HTML, 5 CI
workflows, installer staging script, both catalog scripts, msp-v1.md,
6 golden vectors (headers), 4 verification docs (phase-f-lan, f2a-relay,
phase-i, phase-h excerpt), gap-engineering study, ADR list. Grep sweeps:
placeholders, unsafe, unwrap, mock vocabulary, crate-import graph, ADR-016,
MSP_LISTENER, 11435, circuit references.

## 3. Findings with severity and evidence

Severity-ordered; full detail and file:line evidence in the inventory doc §1.

- HIGH H1 — Fake streaming end to end: `SingleLocalExecutor` awaits the full
  decode before emitting any event (`crates/modelswarm-node/src/executor.rs:136-195`)
  and `LlamaCppAdapter::decode_stream` costs one HTTP POST per token with the
  growing prefix re-sent (`crates/modelswarm-runtime/src/llamacpp.rs:411-535`).
  Local-API TTFT = full completion time; SSE bursts after completion
  (production, local + LAN). Dominant perf/UX defect.
- HIGH H2 — Shipped peer selection is first-found, not fastest; scheduler
  crate unwired (bench-only import: `crates/modelswarm-bench/src/lib.rs:39`;
  desktop picks first peer: `crates/modelswarm-desktop/src/app.rs:1435-1446`).
- HIGH H3 — Remote serving off by default in shipped builds (`MSP_LISTENER=1`
  env gate, `app.rs:252`, `libp2p_backend.rs:220-225`; no UI toggle) — LAN
  proof required manual opt-in + firewall rule on both machines.
- MEDIUM M1 — Plaintext `identity.seed` (DPAPI deferred,
  `node/src/lib.rs:608-622`), `session.token`, `lease.id`.
- MEDIUM M2 — Blocking calls in async: sync `--list-devices` spawn
  (`app.rs:521-539`), sync rusqlite/fs in async commands, whole-file engine
  hashing in `Node::start` (`engine.rs:184-241`).
- MEDIUM M3 — `license_accepted()` opens a second FileSink on `node.jsonl`
  (`app.rs:147-159`), violating the one-sink rule stated at `app.rs:46-49`.
- MEDIUM M4 — Protocol drift: msp-v1 §6 protocol id `/msp/infer/1` +
  handshake not implemented on the QUIC path (raw streams, no
  multistream-select) — also blocks F2b relay use.
- MEDIUM M5 — Artifact download has no resume and no disk-space precheck
  (`crates/modelswarm-node/src/artifact.rs:15-17`).
- MEDIUM M6 — Relay unusable by nodes (F2b client missing; F2A doc "Honest
  boundary").
- MEDIUM M7 — ChatML hardcoded in the serving executor (ADR-025) — catalog
  limited to ChatML-family profiles.
- LOW L1-L7 — eprintln on serving path (remote.rs), unused gateway audit
  ring, dead `FailoverExecutor`, duplicated cfg-attrs/hub-key includes,
  stale v1-schema README reference, pointer-only `tests/`, cosmetic 11435
  display while stopped.

Ground-truth claims 1-8 of master-prompt §0: all CONFIRMED (claim 1 and 4
with precision caveats — listener default-off; relay separate and unusable
by nodes). ADR-016 absence is documented in-repo as "closed, not needed";
no code references it. See inventory §2.

## 4. Proposed actions (suggested, none executed)

1. H1 refactor: use llama-server streaming (`stream:true`) or batched
   `max_tokens:k` windows with incremental emit through the executor
   trait — the single highest-leverage production change; measure TTFT/ITL
   before/after on the LAN pair (production labels).
2. Wire a minimal planner (scheduler crate) into the desktop swarm path:
   EWMA RTT + predicted_ms over the roster, replacing first-found — small,
   uses already-tested frozen code (M9 direction).
3. Product decision for H3: promote `MSP_LISTENER` to a UI/diagnostic toggle
   with its firewall story, or keep dev-only and document.
4. Fix M3 (one telemetry sink), L1 (eprintln→telemetry), L4/L5 (dedupe,
   README) — trivial hygiene batch.
5. M5 resumable download + disk-space gate before M5 phase of the roadmap.
6. M4: fold the protocol-id/multistream decision into the F2b relay ADR
   rather than fixing the doc in isolation.
7. Add an ADR-index tombstone line for ADR-016.

## 5. Files expected to change (if actions accepted)

`crates/modelswarm-runtime/src/llamacpp.rs`,
`crates/modelswarm-node/src/executor.rs`, `crates/modelswarm-desktop/src/app.rs`,
`crates/modelswarm-node/src/artifact.rs`, `crates/modelswarm-node/src/remote.rs`,
`catalog/candidate-profiles/README.md`, `docs/adr/` (index/tombstone + any
F2b ADR). Shared-schema/protocol changes (M4) go through an ADR per
AGENTS.md — nothing here invents wire fields.

## 6. Tests required

- H1: streaming-parity tests (same token sequence; incremental emission
  assertions at the gateway SSE layer), plus a labeled LAN TTFT/ITL
  measurement before/after (docs/verification pattern).
- Planner wiring (action 2): unit tests over roster fixtures + keep
  scheduler's frozen-rule suite green.
- Artifact resume: interrupted-download property test (kill at byte k,
  restart, verify final hash + GGUF identity).
- Hygiene batch: existing suites stay green; add a test pinning
  single-sink telemetry if the rule is kept.
- All Rust work must pass the standard gate: `cargo fmt --check`,
  `cargo clippy --workspace -- -D warnings` (both CI feature
  configurations), `cargo test --workspace`; tracker work (none proposed)
  would add typecheck/test/build.

## 7. Compatibility and security impact

No wire/protocol changes proposed outside an ADR (M4 explicitly gated).
H1 changes client-visible timing only, not token content — greedy output
must remain byte-identical (the E0 determinism suite is the guard).
Security: no weakening proposed; M1 (DPAPI) tightens; H3's toggle must
keep the lease gate and loopback-honesty guards untouched. No secrets,
prompts, or completions appear in any file inspected; redaction and
content-blind invariants verified structurally in code and tests.

## 8. Uncertainties

- Runtime-behavior findings (H1 latency shape, engine prompt caching
  effectiveness) are from code reading, not measurement — the background
  validation suite and any bench run should confirm with numbers.
- The tracker was audited structurally (routes, store invariants, tests);
  individual route handlers were sampled, not exhaustively read.
- `docs/reviews/audit-freeze-2026-10-09.md` (untracked) was not in my brief;
  I did not cross-check it.
- Phase-verification docs were trusted as historical evidence where they
  record live runs (LAN proof, F2A); the code matches their claims.

## 9. Suggested next task for the integrator

Fold this inventory into `audit-salvage-matrix.md` and the §9 architecture
gate: the three REFACTOR rows (runtime streaming, artifact resume, desktop
hygiene) are the only code-level remediations; everything else is KEEP
(with wiring decisions H2/H3 and the M4 ADR). Sequence H1 first — it
changes the performance baseline every later claim builds on.
