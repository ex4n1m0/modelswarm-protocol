# Hub migrations

Phase 0: empty by design. Phase 1 creates the schema for: `users`,
`installations`, `peer_keys`, `model_profiles`, `license_acceptances`,
`peer_leases`, `peer_observations`, `hosting_challenges`, `capability_tokens`,
`job_receipts`, `blocked_peers`, `release_channels` (per
`docs/build-plan.md`, contract in `protocol/msp-v1.md`).

Rules:

- Migrations are forward-only SQL files, sequentially numbered.
- No column anywhere may store prompt or completion text; the Phase 1 test
  suite asserts this structurally over `information_schema`.
- Lease expiry must work by row aging (timestamp comparison), never by a
  background process.
