# Scripts

Phase 0: empty by design.

Phase 2 adds the catalog resolver/admin tool (`resolve-candidate`): given an
HF repo + filename, resolves the full commit hash, byte size, SHA-256, and
license evidence, and emits a candidate manifest validated against
`catalog/schema.json` for human review. It must never guess or hardcode any
of those values.
