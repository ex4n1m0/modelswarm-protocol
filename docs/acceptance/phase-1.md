# Phase 1 Acceptance Tests — Tracker (now Phase B scope)

> Phase A note (ADR-010/015): `hub/` → `apps/tracker/`; service name
> `modelswarm-hub` → `modelswarm-tracker`; original "Phase 1" is absorbed by
> **Phase B** of the A–G program. Content below is otherwise unchanged and
> remains the tracker's acceptance source, extended by the Phase B lease
> vectors (ADR-012).

Exact, executable criteria. The tracker phase is not done until every test
below passes with recorded evidence in `docs/verification/phase-b.md`.
Contract: `protocol/msp-v1.md` §3–§5. Environment: disposable Postgres
(docker or Neon branch), `npm run test:integration` inside `apps/tracker/`.

## A. Schema & migrations

- **A1** `npm run db:migrate` applies all migrations to an empty database and
  exits 0; a second run is a no-op.
- **A2** Structural prompt-privacy assertion: query `information_schema.columns`
  for all tables; no unbounded text columns exist (any `text` without a
  `character_maximum_length`, other than enumerated small metadata fields
  whitelisted in the test); specifically forbidden column names anywhere:
  `prompt`, `completion`, `messages`, `conversation`, `content`, `history`,
  `hf_token`, `access_token`, `api_key`, `secret`. Test fails if any appears.
- **A3** All timestamps stored `timestamptz` UTC.

## B. Public endpoints

- **B1** `GET /api/v1/health` → `200 {"status":"ok","service":"modelswarm-tracker","protocol":"1"}`.
- **B2** `GET /api/v1/catalog` → `200` with a CatalogEnvelope
  (`catalog/schema.json`) whose `signature` verifies against the hub public
  key and fails verification after flipping any byte of `profiles`.
- **B3** `GET /api/v1/catalog/<unknown>` → `404 {"error":{"code":"unknown_profile"}}`.

## C. Enrollment & signed requests

- **C1** `POST /api/v1/auth/device/start` with `{installationId, pubKey}` →
  `200` with device code pair; completing without user approval → `403`.
- **C2** Every mutating peer call without a valid signed envelope →
  `400 unsigned_request` or `401 unauthorized`.
- **C3** Envelope with `ts` skewed > 120 s → `400 stale_timestamp`.
- **C4** Replaying a captured valid request → `400 replayed_nonce`.
- **C5** Tampering with the body after signing (digest mismatch) → `401`.

## D. Peer lifecycle

- **D1** Valid `POST /api/v1/peers/register` → `{leaseId, leaseExpiresAt}`
  with TTL between 60–90 s; installation + peer key recorded.
- **D2** `POST /api/v1/peers/heartbeat` extends `leaseExpiresAt`; heartbeat
  referencing an expired lease → `410 lease_expired` (must re-register).
- **D3** Lease expiry is row-aging: after TTL passes with no heartbeat,
  `GET /api/v1/peers?profile_id=…` no longer returns the peer. No background
  job may exist for this (assert: no cron/worker process in the repo).
- **D4** `POST /api/v1/peers/drain` → subsequent `GET /api/v1/peers` lookups
  **exclude** the draining peer entirely (deterministic); new capability
  issuance for it returns `403 ineligible`.
- **D5** `leaseId`s are unguessable (random, ≥ 128 bits) and bound to the
  signing installation: an envelope signed by installation B referencing
  installation A's lease (heartbeat, drain, or challenge) → `401 unauthorized`.
- **D6** `GET /api/v1/peers?profile_id=X` returns only peers whose active
  profiles include X; `limit` > 50 is clamped to 50.

## E. Capability tokens (schema-level in Phase 1; full flow in Phase 5)

- **E1** Token records conform to §5: peer-bound, profile-bound, serialized as
  `base64url(json).base64url(sig)`, signature verifiable with the hub public
  key, and **`expiresAt` ≤ issuing lease's `leaseExpiresAt` + 60 s**.
- **E2** Issuance requires a `hosting_challenges` row with outcome `passed`
  for the same lease + profile; without it → `403 no_capability`.
- **E3** A token issued against lease L is rejected (as consumption
  authority) once L has expired past the 60 s grace — verified at the record
  level in Phase 1; the live serving-peer check is Phase 5.

## F. Abuse controls

- **F1** > 60 peer-endpoint requests/min for one installation → `429
  rate_limited` with `retryable:true`.
- **F2** Any request body > 64 KiB → `413 payload_too_large`.
- **F3** Malformed JSON / schema-violating body → `400 invalid_body`, and the
  hub never logs the offending body verbatim (redacted structured logs only).
- **F4** Sending inference-shaped payloads (`messages:[{role,content}]`,
  `prompt`, `input`) to any hub endpoint → `400 invalid_body` **by schema
  design** (these fields are absent from every hub request schema).
- **F5** Unauthenticated flooding: > 120 req/min from one IP on
  `/health`+`/catalog` → `429 rate_limited`.
- **F6** `POST /auth/device/start` flooding: > 10 req/min from one IP →
  `429 rate_limited` and no database rows created for rejected calls.
- **F7** Garbage (unsigned, malformed) requests to peer endpoints are
  rejected before body parsing beyond the size cap and are counted against
  the per-IP unsigned limit (60/min); assertion covers "rejected cheaply"
  (no 5xx, median latency < 50 ms against a warm instance).

## G. Catalog administration

- **G1** Promoting a candidate profile is admin-only (non-admin → `403`);
  promotion is append-only: publishing a changed artifact under an existing
  `profileId` → `400 invalid_body`.
- **G2** No public endpoint can insert or mutate `model_profiles`.

## H. Non-goals enforced

- **H1** No hub route accepts or stores prompts/completions (A2 + F4 prove
  this structurally and behaviorally).
- **H2** The hub repository contains no inference runtime and no model files
  (CI grep: `*.gguf` absent, no llama.cpp dependency).

## Evidence format

For each test: command run, HTTP status/body observed, pass/fail. Recorded in
`docs/verification/phase-1.md` by the Tracker agent; QA/Security re-runs the
suite independently before Phase 1 is accepted.
