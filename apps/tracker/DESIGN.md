# Tracker Design (apps/tracker) — Phase B implementation map

Owner: Tracker Engineer · Boundary: ADR-001 (content-blind control plane) ·
Contract: `protocol/msp-v1.md` §2–§5 + revision endpoint set.

## Endpoint map (existing vs Phase B additions)

| Endpoint | msp-v1 §3 | Phase B notes |
|---|---|---|
| `GET /api/v1/health` | ✅ shipped (skeleton) | returns `service:"modelswarm-tracker"` |
| `GET/POST /api/v1/catalog…` | ✅ frozen | serves v2 ProfileRecords once schema v2 lands (ADR-011); signed envelope unchanged |
| `auth/device/*` | ✅ frozen | unchanged |
| `peers/{register,heartbeat,drain,challenge/*}` | ✅ frozen | D16 (gate 2026-10-09 / ADR-028 §7): `challengePrompt` is **nonce-bound** per challenge instance (`"ModelSwarm readiness challenge <challengeId>"`) and embeds the profile's **pinned greedy-canary digest** (`"; greedy-canary sha256:<64hex>"`) when `MSP_CANARY_PINS` is configured — riding the existing wire field, no schema change (construction byte-pinned by `protocol/vectors/challenge-prompt-1.json`; see lib/challenge.ts). Strict mode: a configured pin map with no entry for the profile refuses the challenge (`403 no_capability`) — unpinned profiles cannot earn leases |
| `GET /peers?profile_id=` | ✅ frozen | response adds `capacity_class` (ADR-012) — **self-reported label** (D7): derived from the challenger's own timings, ranking telemetry only, never a hub-verified measurement |
| `rendezvous/{offer,answer,pending}` | ✅ frozen | unchanged |
| `events/job-result` | ✅ frozen | unchanged |
| `POST /api/v1/session-authorize` | — | **New (Phase B, shipped)**: cooperative session authorization (micro-swarm roster; metadata only) |
| `POST /api/v1/peers/lease` | — | **New (Phase B, shipped)**: eligibility-lease issuance/refresh (ADR-012); challenge/complete records the passed challenge, this endpoint issues |
| `POST /api/v1/receipt` | — | **New (Phase B, shipped)**: canonical receipt intake (co-receipt style settlement evidence) |
| `POST /api/v1/audit` | — | **New (Phase B, shipped)**: audit-epoch sweep (per-peer counter; stale-epoch lease refresh rejected) |
| `model-catalog` admin | ✅ §3.5 | gains manifest-v2 candidate promotion |

Every addition is metadata-only. **Structural invariant (test A2/F4 carry
over): no endpoint schema contains prompt-like or token-like fields.**

## Rate-limiting semantics (documented posture, audit T4)

- The limiter (`lib/ratelimit.ts`) is a fixed-window, **in-memory** counter
  map: on Vercel its buckets are **per warm lambda instance**. Effective
  ceilings under burst traffic are the configured per-minute values times
  the number of warm instances sharing a key; cold starts begin empty. It
  shapes abuse cost; it is not exact quota accounting.
- The durable `rate_counters` table + `Store.bumpRateCounter` are
  **intentionally unwired** observability seams: a DB write per request on
  the serverless Pg connection would cost more than the bound it buys.
  Wiring the limiter to the table is a future option behind the same
  interface.
- Bucket classes: `inst:<installationId>` (signed peer calls, last in the
  frozen precedence), `unsigned:<ip>` (peer-endpoint calls that can never
  reach a verifiable envelope — unparseable headers AND well-formed
  envelopes naming unknown installations), `enroll:<ip>` / `modelreq:<ip>`
  (public writes), `public:<ip>` (health/catalog), `download:<ip>`
  (counted installer downloads — T4: otherwise unthrottled DB writes and
  inflatable public counters).

## Retention and write-only tables (T5, write-time pruning)

No TTL jobs exist by design (ADR-001 row-aging). High-churn and write-only
tables prune themselves at write time using the rendezvous-mailbox pattern
(latest in `lib/store.ts`, constants in `lib/constants.ts`):

| Table | Policy |
|---|---|
| `peer_observations` | newest 240 rows per peer + 7-day TTL (read by nothing; highest churn — one row per heartbeat) |
| `sessions` | **census-safe**: expired-beyond-24h rows deleted EXCEPT the newest per installation — `countEnrolledInstallations()` (DISTINCT, the `DEVICE_APPROVAL_CAP` basis) stays cumulative; the cap's monotonicity is intentional and documented |
| `device_codes`, `capability_tokens` | rows swept 24h past expiry (grace-window debugging only) |
| `session_authorizations` | expired rows dropped at write (10-min TTL table, write-only) |
| `peer_notices` | `{all:true}` fan-out first drops queues of leases dead >24h |

Migration-0004 (ADR-012 suspension DDL: `max_slots = 0` vs
`suspended_until`) is a separate gated change with snapshot + rollback
conditions (gate D6) and is NOT part of this posture.

## Honest-label posture (D7, gate 2026-10-09)

`capacityClass` (challenge/complete result, GET /peers) and the lease field
`verified_capacity` are **self-reported labels**: the tracker cannot observe
a challenger's engine, so the class always derives from the challenger's own
timings via the fixed table in `lib/eligibility.ts`. Field names are frozen
wire shapes (byte-pinned by `protocol/vectors/lease-hubkey-1.json`); the
honesty lives in docs, code labels, and the D16 nonce-binding that kills
precomputed/replayed timing pairs. Timing plausibility is enforced
(nonzero total, `firstTokenMs <= totalMs`); possession spot-verification is
deferred to the reputation chassis with its sampling rate sized from
measured detection math (Sarmenta `1-(1-s)^n`; TBD-numeric until the E-A
detection curves exist — never invented).

## Statelessness and storage

- Functions stay request-scoped; all durable state in managed Postgres
  (Neon via Vercel Marketplace). No in-process caches that can diverge.
- Lease expiry stays row-aging (no background workers) — the v0.1 property
  that passed review carries forward.
- Audit epochs (ADR-012) are a counter column per peer, incremented by sweep
  runs triggered on-request or by scheduled Vercel cron — never a resident
  process.

## Deployment

Project `modelswarm-tracker` on Vercel; domain `modelswarm.deepflux.space`
(lowercase canonical); CNAME target read from project settings, never
guessed. Migrations forward-only under `migrations/`; A2 privacy assertion
runs in the integration suite.

## Sequencing note

Phase B implements the frozen §3 surface + lease/session-authorize/receipt/
audit; cooperative *sessions* themselves are Phase D and never touch the
tracker beyond these metadata endpoints.
