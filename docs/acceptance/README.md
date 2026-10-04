# Acceptance tests, per phase

Index of exact acceptance criteria. Each phase's file is written at the start
of that phase (or before, when the contract is frozen) and must pass with
recorded evidence in `docs/verification/<phase>.md` before the next phase
starts.

| File | Scope | Status |
|---|---|---|
| `phase-1.md` | Tracker hub: schemas, signed envelopes, leases, tokens, abuse limits | Frozen in Phase 0 |
| `phase-2.md` | Node: identity, store, HF download/verify, llama.cpp supervision — **must include a log-redaction assertion once any code path touches license/challenge prompts** | To write in Phase 2 |
| `phase-3.md` | P2P: malformed frames, replay, wrong signatures, wrong profile, expired leases, oversized prompts, cancellation, disconnects, slow consumers — **must include a prompt-redaction gate (prompt text first flows here)** | To write in Phase 3 |
| `phase-4.md` | Gateway/scheduler: routing determinism, retry semantics, circuit breaker — **second prompt-redaction gate** | To write in Phase 4 |
| `phase-5.md` | Capability enforcement: token copy/forgery/expiry, clock skew, stale leases, patched metrics | To write in Phase 5 |
| `phase-6.md` | Installer, UX states, privacy disclosures | To write in Phase 6 |
| `phase-7.md` | Full release matrix from `docs/build-plan.md` | To write in Phase 7 |

Rule: a phase's acceptance file may only be authored from the frozen contract
documents (`protocol/`, `catalog/`, ADRs) — never from an implementation.
