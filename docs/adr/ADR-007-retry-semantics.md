# ADR-007: Retry only before the first output token; honest interruption after

Status: Accepted (Phase 0)

## Context

When the selected host dies mid-stream, the gateway can silently retry
elsewhere (risking duplicated/divergent output the user already saw) or report
interruption. With autoregressive decoding, a second peer regenerates from its
own KV state and may produce different tokens.

## Decision

The gateway may fail over to another eligible peer **only until the first
`token_delta` has been emitted** to the local client. After that, host loss or
stall past the deadline ends the stream with an explicit interrupted-stream
error (mapped to an OpenAI-style error payload / terminated SSE with error
event). The client sees what it saw — no fabricated continuation, no silent
regeneration. Cancellation flows client→gateway→serving peer at every stage.

## Consequences

+ Output a user has seen is never silently contradicted or duplicated.
+ Simple mental model for clients; deterministic E2E tests.
− Mid-stream host loss costs the user the remainder of one answer (accepted
  for v0.1; seamless migration is an explicit non-goal).

## Alternatives rejected

- **Always retry and splice**: produces incoherent merged text.
- **Retry with prompt-so-far as context**: doubles cost, changes answers,
  still visible to the user.
