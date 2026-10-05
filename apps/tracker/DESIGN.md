# Tracker Design (apps/tracker) — Phase B implementation map

Owner: Tracker Engineer · Boundary: ADR-001 (content-blind control plane) ·
Contract: `protocol/msp-v1.md` §2–§5 + revision endpoint set.

## Endpoint map (existing vs Phase B additions)

| Endpoint | msp-v1 §3 | Phase B notes |
|---|---|---|
| `GET /api/v1/health` | ✅ shipped (skeleton) | returns `service:"modelswarm-tracker"` |
| `GET/POST /api/v1/catalog…` | ✅ frozen | serves v2 ProfileRecords once schema v2 lands (ADR-011); signed envelope unchanged |
| `auth/device/*` | ✅ frozen | unchanged |
| `peers/{register,heartbeat,drain,challenge/*}` | ✅ frozen | challenge flow extended with randomized **chunk challenges** (ADR-011 possession) |
| `GET /peers?profile_id=` | ✅ frozen | response adds `capacity_class` (ADR-012) |
| `rendezvous/{offer,answer,pending}` | ✅ frozen | unchanged |
| `events/job-result` | ✅ frozen | unchanged |
| `POST /api/v1/session-authorize` | — | **New (Phase B, shipped)**: cooperative session authorization (micro-swarm roster; metadata only) |
| `POST /api/v1/peers/lease` | — | **New (Phase B, shipped)**: eligibility-lease issuance/refresh (ADR-012); challenge/complete records the passed challenge, this endpoint issues |
| `POST /api/v1/receipt` | — | **New (Phase B, shipped)**: canonical receipt intake (co-receipt style settlement evidence) |
| `POST /api/v1/audit` | — | **New (Phase B, shipped)**: audit-epoch sweep (per-peer counter; stale-epoch lease refresh rejected) |
| `model-catalog` admin | ✅ §3.5 | gains manifest-v2 candidate promotion |

Every addition is metadata-only. **Structural invariant (test A2/F4 carry
over): no endpoint schema contains prompt-like or token-like fields.**

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
