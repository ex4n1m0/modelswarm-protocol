//! Node-local persistence: SQLite via SQLx, migrations, and typed access for
//! installation state, license acceptances, EWMA observations, and job
//! accounting. Never stores prompts or completions (`docs/privacy.md`).
//!
//! Phase A: interface freeze only. Implemented in Phase C.
