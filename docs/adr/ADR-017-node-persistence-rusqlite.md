# ADR-017: Node-local persistence via rusqlite (bundled SQLite)

Status: Accepted (Phase B, 2026-10-05) · Amends the original plan's stack
table entry "SQLite via SQLx" for the **node** side only.

## Context

The original plan chose SQLx for local storage. The node is a single
process with a single local database; SQLx's async pooling and
compile-time-checked queries add heavy dependencies and build time for no
benefit at our scale, and its macro path wants DATABASE_URL at build time —
wrong for a desktop app. The tracker (server, Postgres/Neon) is unaffected.

## Decision

Node-local storage uses **rusqlite with the bundled SQLite** feature:
synchronous calls inside the node's Tokio tasks are acceptable at our write
rates (accounting + EWMA updates, not hot-path inference); zero external
build prerequisites on Windows; migrations are embedded SQL strings.
Long-blocking calls get `spawn_blocking` if profiling ever shows jank.

Privacy invariants carry over: the schema is introspected in tests to prove
no prompt-like or secret-like column exists (phase-1.md A2 rule, applied to
the node too).

## Consequences

+ Fast cold builds, no DB service on the machine, trivially embedded.
− Divergence from the plan's stack line (this ADR is the record).
− If the node ever needs concurrent multi-process access, revisit.
