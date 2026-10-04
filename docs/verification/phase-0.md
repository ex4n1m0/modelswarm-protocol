# Phase 0 Verification Report

Date: 2026-10-04 · Environment: Windows 11 x64 (10.0.26300), git 2.50.1,
Rust 1.98.1 (stable toolchain via `rust-toolchain.toml`), Node 22.18.0,
npm 10.9.3.

## Scope delivered (Phase 0 only — no tracker behavior, no inference)

- `AGENTS.md` — ownership map, coordination rules, handoff format, phase gate.
- `docs/architecture.md`, `docs/threat-model.md`, `docs/privacy.md`.
- Eight ADRs (`docs/adr/ADR-001…008`) covering: Vercel boundary, llama.cpp
  sidecar, tracker-first discovery, Ed25519 identity, immutable HF revisions,
  capability tokens, retry semantics, Windows packaging.
- `protocol/msp-v1.md` (frozen contract: envelopes, hub API, catalog,
  capability tokens, P2P protocol, receipts) + `protocol/messages.proto`.
- `catalog/schema.json` + empty-by-design candidate directory.
- Cargo workspace: 9 library crates + 3 app stubs; only `ms-core` contains
  logic (protocol constant + validated typed IDs + 4 unit tests); all other
  crates are documented interface freezes with zero dependencies.
- Hub skeleton: Next.js app with exactly one real route
  (`GET /api/v1/health`, static) + status page; migrations/tests are READMEs.
- CI: `.github/workflows/rust.yml` (fmt + clippy -D warnings + test on
  windows-latest), `hub.yml` (typecheck + build), `windows-package-dry-run.yml`
  (release build + smoke run + artifact).
- `docs/risk-register.md` (15 risks), `docs/acceptance/phase-1.md` (exact,
  executable Phase 1 acceptance tests A1–H2), `docs/acceptance/README.md`
  (per-phase acceptance index).
- Independent design reviews + reconciliation:
  `docs/reviews/phase-0-{architect,security}-review.md`,
  `docs/reviews/phase-0-reconciliation.md`. Both verdicts:
  APPROVE-WITH-CHANGES; all 12 blocking findings resolved pre-freeze (most
  significant: capability-token TTL now capped at lease expiry + 60 s).

## Commands run and results

| Command | Result |
|---|---|
| `cargo fmt --all --check` | pass (after `cargo fmt --all` normalization; no diffs remain) |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 (after fixing 3 `needless_borrows_for_generic_args` in ms-core tests) |
| `cargo test --workspace` | 4 passed, 0 failed (ms-core), 0 elsewhere (stubs) |
| `cargo check --workspace` | pass |
| `cd hub && npm install` | 29 packages, no audit failures reported |
| `cd hub && npm run typecheck` (`tsc --noEmit`) | pass |
| `cd hub && npm run build` (`next build`) | ✓ Compiled successfully; routes: `/` (static), `/api/v1/health` (dynamic) |
| `npx next start -p 3111` + `curl /api/v1/health` | `200 {"status":"ok","service":"modelswarm-hub","protocol":"1"}` |

## Known limitations / honest notes

1. CI workflows are authored but not yet executed on GitHub (no remote is
   configured; repo is local-only at commit time).
2. `messages.proto` is the schema of record but no protoc compilation step
   exists yet — wired in Phase 3 with the wire-encoding ADR.
3. One contract question deliberately deferred: handshake signature
   canonicalization over protobuf fields (Phase 3 opening ADR).
4. `hub/package-lock.json` is committed for reproducible CI (`npm ci`).
5. No real model profile exists anywhere in the repo — the catalog is empty
   by design; the Phase 2 resolver tool will resolve the Qwen3 4B Q4_K_M
   values from Hugging Face. No placeholder hashes were ever committed.

## Phase gate

Phase 0 is complete when this report, the reconciliation record, and a clean
initial commit exist. **Phase 1 (tracker implementation) may begin only after
the human owner reviews this report.**
