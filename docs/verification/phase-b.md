# Phase B Verification Report

Date: 2026-10-05 · Environment: Windows 11 x64, Rust 1.98.1, Node 22.18.0 ·
Commits: `e61b652` (Rust foundation), this commit (tracker + docs).

## Delivered

**Rust protocol foundation** (5 crates, 72 tests — independently re-run by
the integrator):

| Crate | Contents | Tests |
|---|---|---|
| `modelswarm-types` | `ModelProfileManifest`, canonical float-free JSON, `msp1:` derivation; **triple parity** with both golden vectors and the Node validator | 16 |
| `modelswarm-identity` | Ed25519 installation identity (`bs58(sha256(pub))`), signed envelopes, ±120 s window, nonces | 15 |
| `modelswarm-eligibility` | `EligibilityLease` (lease+60 s cap enforced at verify), `CapacityClass`, deterministic suspension table | 20 |
| `modelswarm-session` | Commit state machine — the phase gate: duplicates never advance, stale/reordered/cross-profile/cross-params rejected, sha256 prefix chain; 5 proptests + 10 000-iteration fuzz | 15 |
| `modelswarm-store` | rusqlite (ADR-017), embedded migration, privacy introspection | 6 |

**Tracker** (`apps/tracker`, 78 vitest tests — independently re-run):

- All frozen §3 endpoints + Phase B additions (`/peers/lease`,
  `/session-authorize`, `/receipt`, `/audit`), envelope pipeline with the
  frozen precedence, rate limits, 64 KiB cap, replay/nonce window, signed
  catalog, ADR-12 lease issuance with the cap, audit epochs, admin
  immutability, rendezvous mailbox.
- phase-1.md A1–H2 fully covered (A2/A3 via DDL introspection; live-DB half
  env-gated on `DATABASE_URL` — none available locally, see boundary).
- `deriveProfileId` parity against both vectors (TS = Rust = fixture).

## Commands and results (integrator-run)

| Command | Result |
|---|---|
| `cargo fmt --all --check` / `clippy --workspace --all-targets -- -D warnings` | PASS / exit 0 |
| `cargo test -p modelswarm-types -p …-identity -p …-eligibility -p …-session -p …-store` | 72 passed, 0 failed |
| `cd apps/tracker && npm run typecheck` | exit 0 |
| `npm run build` | exit 0 (21 dynamic API routes) |
| `npm run validate:vectors` | 2/2 PASS |
| `npm test` | 78 passed / 78 (10 files) |
| `npm run db:migrate` | "skipped (no DATABASE_URL)" exit 0 (expected) |

## Integrator decisions on agent-flagged ambiguities

1. **Endpoint paths**: kept as implemented (`/api/v1/peers/lease`,
   `/session-authorize`, `/receipt`, `/audit`); DESIGN.md table updated to
   match. Rationale: all are peer-envelope-authenticated actions under the
   frozen `/api/v1` prefix; no external consumer exists yet.
2. **peerId placeholder equality** → resolved by **ADR-020** (dual
   derivations; multihash binding + Phase F migration task).
3. Suspension-policy server side + Phase B lease test vectors: policy table
   is implemented + tested Rust-side; tracker-side enforcement scheduled
   with the Phase D/F integration (recorded, not silently dropped).
4. `verified_capacity` timings table and fixed-window limiter semantics:
   accepted as documented placeholders in code comments.

## Honest boundary

- **PgStore has never executed against a live Postgres** (no Docker/psql on
  this machine; SQL mirrors the introspected DDL 1:1). The live-DB suite is
  env-gated and MUST run in CI with a service container before any
  deployment claim.
- In-memory rate limiter/notices are single-instance; serverless
  per-instance dilution is a known Vercel deployment risk (recorded R22).
- No transport on the wire yet (ADR-018) — envelope semantics are tested at
  the HTTP layer.
