# Tracker migrations

Forward-only SQL files, applied in order by `npm run db:migrate` (records
applied files in `schema_migrations`; skips cleanly with exit 0 when
DATABASE_URL is unset). Applied inside one transaction per file.

- `0001_init.sql` — full Phase B schema: users, installations, peer_keys,
  model_profiles (+ catalog_state), license_acceptances, peer_leases,
  peer_observations, hosting_challenges, capability_tokens, job_receipts,
  blocked_peers, release_channels, nonces, rate_counters, rendezvous,
  device_codes, sessions, peer_notices, peer_audit_state,
  session_authorizations.

Rules (asserted by tests/schema-ddl.test.ts):

- No column anywhere stores prompt or completion text; no column name contains
  a forbidden token and no unbounded `text` columns exist (varchar(n) +
  enumerated jsonb only).
- Every timestamp is `timestamptz` UTC and named `*_at`.
- Lease expiry works by row aging (timestamp comparison against the injected
  clock), never by a background process.
