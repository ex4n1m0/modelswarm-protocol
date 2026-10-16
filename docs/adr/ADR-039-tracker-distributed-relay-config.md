# ADR-039: Tracker-distributed relay configuration

- Status: Proposed 2026-10-16 — **Accepted** (owner directive "build it",
  2026-10-16, after the machine-C cross-NAT session proved the manual
  `relay.addr` seeding is adoption-hostile)
- Affects: `protocol/msp-v1.md` §3.3, `apps/tracker` (heartbeat route),
  `crates/modelswarm-tracker-api`, `crates/modelswarm-node`,
  `crates/modelswarm-desktop`
- Supersedes: nothing; extends ADR-014 (relay) and ADR-038 (admission
  tickets) — the F2B deployment record's "hand the owner the one line for
  C" interim becomes the fallback, not the path.

## Context

The F2B relay works end-to-end (first cross-NAT relayed swarm-served
completion 2026-10-16, machine C → A), but a fresh install ships with an
empty relay configuration by deliberate design (ADR-014: "no relay address
is invented or hard-coded"). Reaching the relay required a hand-placed
`relay.addr` file — correct for a private test fleet, fatal for adoption.

The original reason for default-EMPTY was safety: an unauthenticated open
relay is a free proxy. ADR-038 removed that reason — the relay now admits
only tracker-ticketed installations, fail-closed, content-blind, capped.

## Decision

1. **The tracker distributes the relay address.** The heartbeat response
   gains an additive `relayAddr` field: the operator-configured community
   relay multiaddr, or the empty string. Source: the tracker deploy's
   `MSP_RELAY_ADDR` environment variable — absent/unset = empty string =
   the feature is off (byte-identical behavior to today). Changing or
   moving the relay is a tracker env change + redeploy, never a client
   ship.
2. **Deploy-order tolerant both directions** (the `relayTicket`
   precedent): a new client parsing an old tracker's response (field
   absent) reads empty; an old client ignores the extra field. Tracker
   deploys first.
3. **Client precedence: the local `relay.addr` file wins.** Effective
   relay = file value if the file exists, else the tracker-supplied
   value. The file remains the tester/operator override and the only way
   to pin a different relay. The value must parse as a multiaddr before
   use; garbage is ignored with a log line, never written to config.
4. **Lazy reservation.** The reservation supervisor starts when an
   effective address exists: immediately (file present) at hosting start,
   or when the first heartbeat delivers a non-empty tracker value. One
   supervisor per hosting session; later tracker changes apply at the
   next hosting start (documented limitation — the tracker value is
   deployment-stable by nature).
5. **Honest disclosure, no new switch** (the owner's reduce-switches
   directive; swarm pairing remains the master control for all swarm
   traffic). The Diagnostics panel's Hosting detail gains a `Relay` row
   showing the effective address and its source (file/tracker/none), and
   the beta-boundary note states the fallback honestly: direct first,
   relay only on direct failure, content-blind, admission-ticketed,
   slower by design.
6. **Telemetry gap fix (same batch).** A cleanly-served swarm request
   left no trace on the serving peer (only errored sessions logged —
   F2B record note). The serving loop now emits `serving.completed`
   on the `Completed` executor event with COUNTS ONLY (peer id,
   completion_tokens, prefill_ms, decode_ms) — never content, matching
   the redaction discipline. The R-1 two-sided inventory
   (`protocol/advisory-egress-v1.json` + the fixture) gains the row in
   the same change.

## Security considerations

- The relay remains ADR-038's gate: ticketed, fail-closed, content-blind,
  capped. Distribution changes who KNOWS the address, not who may use it.
- The `relayAddr` value is operator-controlled (tracker env), carried on
  the authenticated heartbeat channel; the client validates it parses as
  a multiaddr before use. It is a network address, not secrets-bearing
  content; the hub stays content-blind (the value is infrastructure
  config, not user data).
- Failure modes degrade honestly: relay down → direct-only behavior with
  existing labels; tracker unset → exactly today's manual-file behavior.

## Review flags (Integrator-landed, per the house precedent)

- Protocol seat: §3.3 row amendment (additive, changelogged).
- Tracker seat: heartbeat route + env sourcing + TS deploy-skew tests.
- Network seat: node/desktop consumption seam (precedence + lazy
  supervisor).
- Security/T&R: telemetry row + the disclosure wording.
