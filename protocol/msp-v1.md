# MSP v1 — ModelSwarm Protocol (Draft for Freeze)

Status: Phase 0 draft. **Once Phase 1 starts this document is frozen**; changes
require an ADR. No implementation may invent fields not defined here.

Protocol version constant: `"1"` (`ms_core::MSP_PROTOCOL_VERSION`).

## 1. Actors and trust

| Actor | Trusted for | Not trusted for |
|---|---|---|
| Tracker (hub) | Catalog contents, eligibility, lease validity, capability issuance | Prompt content (never sees it) |
| Serving peer | Faithful execution of the agreed profile | Metrics claims, answers' correctness |
| Requesting peer | Nothing about itself | Everything it asserts about itself |
| Hub signing keys | Root of admission trust | — |

All timestamps are RFC 3339 UTC. All ids are strings. All digests are
lowercase hex. JSON where unsaid; field order is not significant except for
canonicalization (§2.2).

## 2. Identity, signatures, canonical encoding

### 2.1 Keys

Ed25519 keypair per installation (ADR-004). `installationId` = base58 SHA-256
of the public key. libp2p `PeerId` derives from the same key.

### 2.2 Canonical JSON (for anything signed)

1. UTF-8, no insignificant whitespace, object keys sorted lexicographically,
   numbers in shortest round-trip form, no trailing zeros on fractions.
2. The signature payload is the canonical JSON of the declared object; the
   signature is detached base64 over it.

### 2.3 Signed request envelope (hub-mutating calls)

The envelope travels in the `Authorization` header:

```text
Authorization: MSP1 <base64url(canonical JSON of envelope below)>
X-MSP-Session: <session token from /auth/device/complete>
```

```json
{
  "installationId": "b58…",
  "method": "POST",
  "path": "/api/v1/peers/heartbeat",
  "ts": "2026-10-04T12:00:00Z",
  "nonce": "16+ random hex chars, unique per installation within replay window",
  "bodyDigest": "sha256:<hex of canonical body JSON>",
  "signature": "base64 ed25519 over canonical {installationId, method, path, ts, nonce, bodyDigest}"
}
```

Rules:

- `method` and `path` are inside the signed payload, so a captured envelope
  cannot be replayed against a different endpoint.
- For bodyless calls (GET), `bodyDigest` is `sha256:` + the SHA-256 of the
  empty string.
- Replay window: ±120 s; nonces are single-use per installation inside the
  window (the window subsumes nonce-cache eviction: an evicted-but-replayed
  request still fails the timestamp check).
- `{session}` from `/auth/device/complete` is `{token, expiresAt}`; `token` is
  an opaque unguessable string with a 24 h lifetime, carried in
  `X-MSP-Session`, revocable server-side.
- Unsigned/unauthenticated variants are rejected with `401 unauthorized` or
  `400 unsigned_request`.

## 3. Hub REST API (base `https://modelswarm.deepflux.space/api/v1`)

JSON bodies; every request/response validated by strict schemas at the edge;
payload limit 64 KiB (inference-shaped endpoints do not exist here — the hub
must structurally reject any body containing prompt-like fields on these
routes).

### 3.1 Public

| Endpoint | Purpose | Notes |
|---|---|---|
| `GET /health` | Liveness + protocol version | `{status:"ok", service:"modelswarm-hub", protocol:"1"}` |
| `GET /catalog` | Signed catalog envelope (§4) | |
| `GET /catalog/{profileId}` | Signed single-profile envelope | `200` body is a CatalogEnvelope containing exactly that profile; `404 unknown_profile` |

### 3.2 Device enrollment

| Endpoint | Body | Result |
|---|---|---|
| `POST /auth/device/start` | `{installationId, pubKey}` | `{deviceCode, userCode, verifyUrl, expiresAt}`; `pubKey` is base58; 10 req/min per IP |
| `POST /auth/device/complete` | signed envelope `{deviceCode}` | `{session}` or `403 pending` |

### 3.3 Peer lifecycle (all require signed envelope + session)

`leaseId` values are unguessable random ids **bound to the signing
installation**; referencing another installation's lease is `401 unauthorized`.

| Endpoint | Body | Result |
|---|---|---|
| `POST /peers/challenge/start` | `{leaseId, profileId}` | `{challengeId, challengePrompt, deadlineAt}`; one open challenge per lease+profile |
| `POST /peers/register` | `{peerId, addresses:[multiaddr], profiles:[profileId…], maxSlots, runtime:{name,build}}` | `{leaseId, leaseExpiresAt}` |
| `POST /peers/heartbeat` | `{leaseId, activeProfiles:[profileId…], freeSlots, queueMs, draining:bool}` | `{leaseExpiresAt, notices:[Notice]}` (below) |
| `POST /peers/drain` | `{leaseId}` | `{draining:true}` |
| `POST /peers/challenge/complete` | `{leaseId, profileId, challengeId, timings:{firstTokenMs, totalMs}}` | capability token (§5) |
| `GET /peers?profile_id=…&limit=…` | — | `{peers:[{peerId, addresses, queueMs, freeSlots, leaseExpiresAt, lastSeenAt}]}`, limit ≤ 50 |
| `GET /rendezvous/pending` | — | `{items:[{fromPeerId, offer, receivedAt}]}`; items are deleted on read |
| `POST /rendezvous/offer` | `{toPeerId, offer}` | `{accepted:true}`; `offer`/`answer` are opaque strings ≤ 4 KiB |
| `POST /rendezvous/answer` | `{toPeerId, answer}` | `{accepted:true}` |
| `POST /events/job-result` | signed `{receiptDigest, outcome}` | `{recorded:true}` |

Validation: `peerId` must derive from the registered `pubKey`; `addresses`
must parse as multiaddrs; every `profileId` must exist in the catalog
(else `400 unknown_profile`); `maxSlots` ∈ [1, 8].

Heartbeat cadence 15–30 s; lease TTL 60–90 s; expiry is row-aging, no timers
(ADR-001).

**Notice** (revocation propagation channel):

```json
{ "type": "revoked_peers", "peerIds": ["12D3Koo…"] }
{ "type": "revoked_tokens", "tokenNonces": ["…"] }
{ "type": "catalog_update", "catalogVersion": 18 }
```

Serving peers must stop serving/accepting anything named in a notice
immediately; requesters must drop matching candidates. This is the mechanism
that produces the `revoked` / `revoked_token` errors at peers.

### 3.4 Error model

```json
{ "error": { "code": "machine_readable", "message": "human text", "retryable": false } }
```

Codes (initial set — additions require ADR): `unauthorized`,
`unsigned_request`, `stale_timestamp`, `replayed_nonce`, `rate_limited`,
`payload_too_large`, `invalid_body`, `unknown_profile`, `unknown_installation`,
`pending`, `forbidden`, `revoked`, `lease_expired`, `no_capability`,
`ineligible`.

HTTP status mapping: `400` invalid_body / stale_timestamp / replayed_nonce /
unknown_profile, `401` unauthorized / unsigned_request / revoked /
unknown_installation, `403` pending / forbidden / ineligible / no_capability,
`410` lease_expired, `413` payload_too_large, `429` rate_limited.

Validation precedence (deterministic, testable): size cap → auth/session →
timestamp → nonce → schema/rate-limit. A test may rely on this order.

Rate limits (defaults, tunable via env): 60 req/min/installation on peer
endpoints **and** 60 req/min/IP on unsigned calls to peer endpoints;
10 req/min/IP on enrollment; 120 req/min/IP on `/health`+`/catalog`.

### 3.5 Admin (catalog management)

Admin = a hub user holding the `catalog_admin` role; authenticated by a
separate admin session header `X-MSP-Admin` issued out-of-band (ops runbook,
Phase 1). These are the only endpoints that mutate `model_profiles`:

| Endpoint | Body | Result |
|---|---|---|
| `POST /admin/catalog/candidates` | resolver output validated against `catalog/schema.json` | `201 {profileId}` (status `candidate`) |
| `POST /admin/catalog/promote` | `{profileId}` | `200 {profileId, status:"active"}` |

Publishing an artifact that differs from an existing `profileId` is
`400 invalid_body` (immutability, ADR-005). Non-admins get `403 forbidden`.

## 4. Catalog envelope

```json
{
  "catalogVersion": 17,
  "generatedAt": "2026-10-04T00:00:00Z",
  "profiles": [ { …ModelProfile per catalog/schema.json… } ],
  "signature": "base64 ed25519 over canonical {catalogVersion, generatedAt, profiles}"
}
```

Nodes verify the signature against the pinned hub public key embedded in the
build, then verify each downloaded artifact's SHA-256 against the profile.

## 5. Capability token (ADR-006)

```json
{
  "peerId": "12D3Koo…",
  "installationId": "b58…",
  "profileId": "msp:qwen3-4b:q4_k_m:v1",
  "canHost": true,
  "canConsume": true,
  "slots": 1,
  "challengeId": "opaque id of the passed hosting challenge",
  "issuedAt": "2026-10-04T14:30:00Z",
  "expiresAt": "2026-10-04T14:31:30Z",
  "nonce": "…"
}
```

**Serialized form** (this is what travels in requests):

```text
capabilityToken = base64url(canonical token JSON) "." base64url(ed25519 signature over that JSON)
```

Semantics (frozen):

- Issued only after: active lease + digest-verified artifact + runtime
  challenge passed within deadline + ≥1 slot + not draining.
- **Expiry is capped at the peer's current lease expiry plus a 60 s grace
  window.** A token can never outlive its lease by more than the grace
  period, so a host that stops heartbeating loses consumption rights within
  ≤ lease-TTL + 60 s (≤ 150 s worst case) — the "short grace window" of
  ADR-006. Tokens are re-issued at challenge/heartbeat cadence.
- Tokens are **multi-use within their lifetime**; `nonce` identifies the
  token for issuance-dedup and revocation (`revoked_tokens` notices), it is
  not per-request. Per-request replay protection is `requestId` single-use
  (§6.6), not the token.
- Peer- and profile-bound: `peerId` MUST equal the authenticated libp2p
  PeerId of the requester; `profileId` MUST equal the request's target.

## 6. P2P stream protocol (libp2p, protocol id `/msp/infer/1`)

Transport: QUIC + Noise; streams are request-scoped. Message schema of record:
`protocol/messages.proto` (wire encoding decision — protobuf bytes vs.
length-prefixed protobuf in varint frames — is fixed in the Phase 3 opening
ADR; fields below are frozen either way).

### 6.1 Flow

```text
Requester                                 Serving peer
   │ handshake(protocol, peerId, profileId,        │
   │           runtime, ts, nonce, sig)            │
   │ ─────────────────────────────────────────────►│ verify sig, token, profile,
   │                                               │ admission control
   │ ◄────────────────────────── handshake_ack ────│
   │ inference_request(requestId, capability,      │
   │          normalized prompt, maxTokens, deadlineMs) │
   │ ─────────────────────────────────────────────►│ proxy to loopback llama.cpp
   │ ◄─ stream: accepted ──────────────────────────│
   │ ◄─ stream: token_delta* ──────────────────────│
   │ ◄─ stream: usage ─────────────────────────────│
   │ ◄─ stream: completed + job_receipt ───────────│
   │ cancel(requestId) (either side, anytime)      │
```

### 6.2 Normalized request (frozen fields)

`requestId` (uuid v4, single-use), `profileId`, `capabilityToken`,
`messages: [{role, content}]` (chat-template applied by the serving peer's
runtime), `sampling: {temperature, top_p, top_k, seed?}`,
`maxTokens`, `deadlineMs` (total wall budget), `stream: true|false`.

### 6.3 Stream events

| Event | Payload | Notes |
|---|---|---|
| `accepted` | queue position, etaMs | admission done |
| `token_delta` | requestId, delta text, index | first one flips the gateway into no-retry mode (ADR-007) |
| `usage` | promptTokens, completionTokens, prefillMs, decodeMs | emitted once before completed |
| `error` | code, message, retryablePeerHint | stream terminates |
| `cancelled` | reason | either side |
| `completed` | finishReason (`stop`\|`length`\|`cancelled`\|`error`), jobReceipt | receipt carries the server's signature; the requester finalizes it with `receipt_ack` (§7) |

### 6.4 Limits (defaults; env-tunable on each side)

Max prompt 32 KiB UTF-8; max frames 256 KiB; max concurrent requests =
advertised slots; per-request deadline 120 s; queue admission deadline 10 s;
slow-consumer backpressure via bounded libp2p buffers. The serving peer clamps
requester-supplied values: `deadlineMs` ∈ [1 000, 120 000], `maxTokens` ∈
[1, profile.maxOutputTokens], `temperature` ∈ [0, 2], `top_p` ∈ (0, 1],
`top_k` ∈ [1, 200]; out-of-range values are clamped, not rejected.

### 6.5 Error codes (P2P, initial set)

`handshake_failed`, `incompatible_protocol`, `profile_mismatch`,
`invalid_token`, `expired_token`, `revoked_token`, `replayed_request`,
`overloaded`, `deadline_exceeded`, `payload_too_large`, `runtime_error`,
`cancelled_by_peer`, `interrupted`.

### 6.6 Replay, identity, and clock-skew rules (frozen)

- The `Handshake.peerId` MUST equal the Noise-authenticated remote PeerId of
  the libp2p connection; the serving peer rejects mismatches with
  `handshake_failed`. Hub-signed identity enters only via the capability
  token's `peerId` binding — peers need no extra key directory.
- Handshake `ts` acceptance window: ±120 s, same as the hub envelope.
- `requestId` is single-use per serving peer: the server remembers ids for
  max(token TTL, 10 min) and answers duplicates with `replayed_request`.
- Token `expiresAt` checks at serving peers allow ±120 s clock skew.
- Nonce state lives per serving peer and survives reconnects within the same
  process lifetime.

## 7. Job receipts

The signed object is `{requestId, profileId, peerIds, startedAt, endedAt,
outcome, usageDigest}` — timings and outcome only, never prompt text.
Two-phase exchange: the server's `completed` event carries the receipt with
`serverSignature`; the requester verifies it, adds `requesterSignature`, and
returns a final `receipt_ack` message on the same stream. The requester posts
the digest to `/events/job-result`; each peer's copy feeds its local
accounting. Receipts are the evidence base for reputation and later
contribution policy.

## 8. Sequences worth re-reading before implementing

- Cold start → catalog verify → register → heartbeat → challenge → token.
- Gateway request → lookup → filter → probe → connect → stream → receipt.
- Pause hosting → drain → lease expiry → token grace window → consumption
  blocked.
- Tracker outage → streams continue → lookup fails with degraded status.
