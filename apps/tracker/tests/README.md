# Tracker tests

Phase 0: no tests yet; `npm run typecheck` + `npm run build` are the gate.

Phase 1 adds: an integration suite running against disposable Postgres
(docker or Neon branch) covering every endpoint in
`docs/acceptance/phase-1.md`, including replay protection, rate limits,
payload limits, lease expiry by row aging, and the no-prompt-fields invariant.
Phase 1 also adds a version-parity test asserting `lib/version.ts`
`MSP_PROTOCOL_VERSION` equals the version in `protocol/msp-v1.md` and
`modelswarm_types::MSP_PROTOCOL_VERSION` (the Rust side asserts the same from its
tests), so the three copies cannot drift silently.
