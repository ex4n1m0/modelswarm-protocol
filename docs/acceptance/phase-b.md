# Phase B Acceptance Tests — Protocol Foundation & Tracker

Gate (revision): "Property tests prove that stale, duplicated, reordered,
and cross-profile messages cannot advance session state." Plus the frozen
phase-1 (now B) tracker tests. Evidence → `docs/verification/phase-b.md`.

## Rust protocol foundation

- **R1** Manifest parity: `modelswarm-types` derives `msp1:` ids identical
  to both `protocol/vectors` fixtures (triple parity: Rust = Node validator
  = fixture).
- **R2** Canonical round-trip: parse→canonicalize→re-serialize is
  byte-identical; no floats anywhere in a manifest (enforced).
- **R3** Identity: envelope sign/verify, tamper + wrong-key rejection,
  ±120 s window logic, nonce uniqueness (100 draws).
- **R4** Eligibility lease: issue/verify; expiry capped at
  lease+60 s (`ExceedsLeaseCap`); peer/profile binding in signed payload;
  suspension policy table encoded in tests.
- **R5** Session machine (the phase gate): valid chains advance exactly
  once; duplicates never advance twice; stale/future/reordered rounds
  rejected; cross-profile/cross-params rejected; token mutation →
  `HashMismatch`; 10k-iteration randomized mutation fuzz, no panics.
- **R6** Store: round-trips; privacy introspection (no forbidden column
  names) passes on the node schema too.

## Tracker (apps/tracker)

- **T1** All tests A1–H2 from `docs/acceptance/phase-1.md` green against
  the in-memory store (DDL-file introspection stands in for
  `information_schema` where no live Postgres is present; with
  `DATABASE_URL` set, the PgStore suite runs additionally).
- **T2** Triple parity repeated in TS: `deriveProfileId` matches both
  fixtures.
- **T3** Lease endpoint issues `base64url(json).base64url(sig)` tokens with
  the ADR-12 cap; no challenge → `403 no_capability`.
- **T4** New metadata endpoints (session-authorize, receipt, audit) accept
  no prompt-shaped fields (schema-level assertion).
- **T5** `npm run typecheck`, `npm run build`, `npm run validate:vectors`
  stay green; CI paths unchanged.

## Honest boundary

- Postgres-backed integration runs execute only where a database is
  actually available (locally: in-memory suite is authoritative; CI later
  adds a service container when a remote exists).
- libp2p is intentionally absent this phase (ADR-018).
