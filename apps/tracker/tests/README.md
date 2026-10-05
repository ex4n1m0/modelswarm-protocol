# Tracker tests

`npm test` (vitest, node environment) covers docs/acceptance/phase-1.md A–H
against MemoryStore, plus the Phase B endpoints (DESIGN.md):

- `crypto-vectors.test.ts` — ADR-011 golden-vector parity with
  protocol/vectors (TS/Rust/validator triple parity), canonical JSON, base58,
  eligibility-lease serialization round-trip.
- `schema-ddl.test.ts` — A1 (no-DB migrate path), A2 (forbidden column names,
  no unbounded text, enumerated jsonb set), A3 (timestamptz everywhere).
- `public.test.ts` — B1/B2/B3, F5.
- `enrollment.test.ts` — C1–C5 + register validation.
- `lifecycle.test.ts` — D1–D6 (row-aging via injectable clock; no timers).
- `tokens.test.ts` — E1–E3 (shape/binding/TTL cap/signature, no_capability,
  record-level expiry past the grace window).
- `abuse.test.ts` — F1–F7 (rate limits, 413, redacted logging, inference-
  shaped rejection, enrollment flood, cheap garbage rejection).
- `admin.test.ts` — G1/G2.
- `phaseb.test.ts` — /peers/lease, /session-authorize, /receipt, /audit,
  rendezvous, notices.
- `meta.test.ts` — H2 (no runtime/model references), no background timers,
  protocol-version parity, frozen-constant bands.

The Postgres suite (PgStore against disposable Postgres via DATABASE_URL) is
the Phase-B integration follow-up; `npm run db:migrate` applies
`migrations/0001_init.sql` when DATABASE_URL is set and no-ops otherwise.
