# MSP v1 — ModelSwarm Protocol (Draft for Freeze)

Status: Phase 0 draft. **Once Phase 1 starts this document is frozen**; changes
require an ADR. No implementation may invent fields not defined here.

Protocol version constant: `"1"` (`modelswarm_types::MSP_PROTOCOL_VERSION`).

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
| `GET /health` | Liveness + protocol version | `{status:"ok", service:"modelswarm-tracker", protocol:"1"}` |
| `GET /catalog` | Signed catalog envelope (§4) | |
| `GET /catalog/{profileId}` | Signed single-profile envelope | `200` body is a CatalogEnvelope containing exactly that profile; `404 unknown_profile` |
| `POST /catalog/requests` | Community model request (ADR-023): `{hf_repo, hf_revision, artifact_path, artifact_sha256, artifact_bytes, quant_method, quant_bits, display_name, note?}` — GGUF pointers only, content-blind by schema | `201 {status:"queued"\|"already-requested"}`; 10 req/min per IP (enrollment grade); dedupe on `artifact_sha256` |

### 3.2 Device enrollment

| Endpoint | Body | Result |
|---|---|---|
| `POST /auth/device/start` | `{installationId, pubKey}` | `{deviceCode, userCode, verifyUrl, expiresAt}`; `pubKey` is base58; 10 req/min per IP |
| `POST /auth/device/complete` | signed envelope `{deviceCode}` | `{session}` or `403 pending` |
| `POST /admin/devices/approve` | admin token (`X-MSP-Admin`) `{userCode}` | `{approved:true}` or `404`; the `verifyUrl` approval page — knowing the user code alone is never enough |

**Automatic approval (deployment option, 2026-10-06).** A tracker MAY
auto-approve device codes at `/auth/device/start` (`DEVICE_AUTO_APPROVE=1`):
the code is created already approved, so enrollment completes on the
client's first `/complete` poll with no owner action. The wire format is
unchanged — `/complete` still reports `403 pending` when a code is not
approved. Exposure is bounded by the `/auth/device/start` rate limit
(10 req/min/IP) and an enrollment cap (`DEVICE_APPROVAL_CAP`, default 250
distinct installations); beyond the cap, codes stay pending and only the
admin-token flow above admits them. The census therefore counts
self-enrolled installations on such deployments — an owner-visible
trade-off, recorded in `docs/threat-model.md`.

### 3.3 Peer lifecycle (all require signed envelope + session)

`leaseId` values are unguessable random ids **bound to the signing
installation**; referencing another installation's lease is `401 unauthorized`.

| Endpoint | Body | Result |
|---|---|---|
| `POST /peers/challenge/start` | `{leaseId, profileId}` | `{challengeId, challengePrompt, deadlineAt}`; one open challenge per lease+profile. `challengePrompt` is **nonce-bound** (D16, gate 2026-10-09 / ADR-028 §7): `"ModelSwarm readiness challenge <challengeId>"`, plus `"; greedy-canary sha256:<64hex>"` when the profile has a pinned possession digest (hash-only, content-blind; construction byte-pinned by `protocol/vectors/challenge-prompt-1.json`) |
| `POST /peers/register` | `{peerId, addresses:[multiaddr], profiles:[profileId…], maxSlots, runtime:{name,build}}` | `{leaseId, leaseExpiresAt}` |
| `POST /peers/heartbeat` | `{leaseId, activeProfiles:[profileId…], freeSlots, queueMs, draining:bool}` | `{leaseExpiresAt, notices:[Notice]}` (below) |
| `POST /peers/drain` | `{leaseId}` | `{draining:true}` |
| `POST /peers/challenge/complete` | `{leaseId, profileId, challengeId, timings:{firstTokenMs, totalMs}}` | `{passed, challengeId, capacityClass}`; token issuance moved to `/peers/lease` (ADR-012) |
| `POST /peers/lease` | `{leaseId, profileId}` | `{token, lease}` — the hub-signed EligibilityLease (§5; ADR-012) |
| `GET /peers?profile_id=…&limit=…` | — | `{peers:[{peerId, addresses, queueMs, freeSlots, capacityClass, leaseExpiresAt, lastSeenAt}]}`, limit ≤ 50 (`capacityClass` folded by ADR-028, field per ADR-012) |
| `GET /rendezvous/pending` | — | `{items:[{fromPeerId, offer, receivedAt}]}`; items are deleted on read |
| `POST /rendezvous/offer` | `{toPeerId, offer}` | `{accepted:true}`; `offer`/`answer` are opaque strings ≤ 4 KiB |
| `POST /rendezvous/answer` | `{toPeerId, answer}` | `{accepted:true}` |
| `POST /events/job-result` | signed `{receiptDigest, outcome}` | `{recorded:true}` |
| `POST /session-authorize` | `{peerIds:[…], profileId, mode}` | `{authorized:true}` — metadata-only; `mode` MUST be a registered ADR-013 execution mode (registry-enum constraint per ADR-028); unconsumed by the node today |
| `POST /receipt` | signed `{receiptDigest, outcome}` | `{recorded:true}` — duplicate of `/events/job-result` intake; superseded by receipts-v2 (ADR-030) |

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
| `POST /admin/catalog/candidates` | resolver output validated against `catalog/schema-v2.json` (ADR-011/022) | `201 {profileId}` (status `candidate`) |
| `POST /admin/catalog/promote` | `{profileId}` | `200 {profileId, status:"active"}` |
| `GET /admin/catalog/requests` | — (ADR-023) | `200 {requests:[…]}` open first (oldest first), then resolved |
| `POST /admin/catalog/requests/resolve` | `{id, resolution:"promoted"\|"rejected"}` (ADR-023) | `200 {id, resolution}` or `404 unknown_request` |
| `POST /audit` | admin auth (no body) | bumps the suspension audit epoch (ADR-012); enforced at lease refresh (folded by ADR-028) |

Publishing an artifact that differs from an existing `profileId` is
`400 invalid_body` (immutability, ADR-005). Non-admins get `403 forbidden`.

## 4. Catalog envelope

```json
{
  "catalogVersion": 17,
  "generatedAt": "2026-10-04T00:00:00Z",
  "profiles": [ { …ModelProfile per catalog/schema-v2.json… } ],
  "signature": "base64 ed25519 over canonical {catalogVersion, generatedAt, profiles}"
}
```

Nodes verify the signature against the pinned hub public key embedded in the
build, then verify each downloaded artifact's SHA-256 against the profile.

## 5. Capability token (ADR-006; renamed EligibilityLease by ADR-012)

The signed fields (snake_case, exactly as serialized inside the token —
byte-pinned by the cross-language golden vector
`protocol/vectors/lease-hubkey-1.json`):

```json
{
  "peer_id": "12D3Koo…",
  "installation_id": "b58…",
  "model_profile_id": "msp1:<64 hex>",
  "issued_at": "2026-10-04T14:30:00.000Z",
  "expires_at": "2026-10-04T14:31:30.000Z",
  "lease_expires_at": "2026-10-04T14:30:30.000Z",
  "can_host": true,
  "can_consume": true,
  "slots": 1,
  "verified_capacity": "gpu_mid",
  "audit_epoch": 4,
  "challenge_id": "opaque id of the passed hosting challenge",
  "nonce": "…"
}
```

`lease_expires_at` is the expiry of the peer's underlying tracker lease;
`expires_at` may exceed it by at most the 60 s grace. The issuance response
carries the wire token in its `token` field:
`POST /peers/lease` → `{ "token": "<capabilityToken>", "lease": { …fields } }`.

**Honest-label note (D7, owner gate 2026-10-09 / ADR-028 §7):**
`verified_capacity` (this object) and `capacityClass` (§3.3 responses) are
**self-reported labels** — they derive deterministically from the
challenger's own attested timings, never from a hub-side measurement. The
field names are frozen wire shapes; the label honesty is normative.

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

> ADR-028 records that the production libp2p path SKIPS the application
> Handshake frames below (QUIC+Noise PeerId authentication replaces them;
> the dialer sends `inference_request` first) — see the §9 changelog.

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

> Serving-peer sampling clamps remain BACKLOG (ADR-028); `maxTokens` uses
> the global cap — see the §9 changelog.

Max prompt 32 KiB UTF-8; max frames 256 KiB; max concurrent requests =
advertised slots; per-request deadline 120 s; queue admission deadline 10 s;
slow-consumer backpressure via bounded libp2p buffers. The serving peer clamps
requester-supplied values: `deadlineMs` ∈ [1 000, 120 000], `maxTokens` ∈
[1, 2048] (node-global default; the v1 per-profile `maxOutputTokens` field
was dropped with schema v2 — a per-profile cap may return only additively
via ADR, ADR-028 §4), `temperature` ∈ [0, 2], `top_p` ∈ (0, 1],
`top_k` ∈ [1, 200]; out-of-range values are clamped, not rejected.

### 6.5 Error codes (P2P, initial set)

> Effective registry amended by ADR-028 §5 — `replayed_request`/
> `overloaded` renames land as R0.5 code (client+server together);
> `invalid_lease`/`bad_frame`/`executor_error` added — see §9.

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

## 9. Changelog (post-freeze amendments; maintained by ADR-028 §6)

Every amendment to this frozen document is listed here with its ADR. No
undocumented amendments.

| Date | ADR / note | Sections touched |
|---|---|---|
| Phase A–B | ADR-012 (EligibilityLease) | §5 renamed/reworked: capability token → EligibilityLease, wire form, `POST /peers/lease` issuance; §3.3 `challenge/complete` result became `{passed, challengeId, capacityClass}`; `capacityClass` added to the `GET /peers` response (folded into §3.3 by ADR-028) |
| Phase A–B | ADR-011 / ADR-022 | §3.5 and §4 catalog validation now references `catalog/schema-v2.json` (stale v1 references corrected by ADR-028) |
| 2026-10-05 | ADR-018 (transport staging, Phase C amendment) | §6 wire encoding: JSON external-tag framing is the staged encoding (the §6 preamble's "fixed in the Phase 3 opening ADR" wording refers to this); the libp2p path's application-handshake skip is a recorded ADR-018 scope extension (ADR-028 §2) |
| 2026-10-06 | ADR-023 | §3.1 `POST /catalog/requests`; §3.5 admin request-queue rows |
| 2026-10-06 | deployment option (no ADR number; threat-model pointer) | §3.2 automatic-approval note (`DEVICE_AUTO_APPROVE=1`) |
| 2026-10-08 | ADR-026 (lease gate at session open) | Effective §6.5 registry gains `invalid_lease` (recorded by ADR-028 §5); serving gate before the executor |
| 2026-10-09 | ADR-028 (tracker-surface and wire reconciliation) | §3.3 folds: `/peers/lease` row, `challenge/complete` result, `capacityClass`; §3.3 `session-authorize` + `/receipt` routes folded (mode → ADR-013 registry enum); §3.5 `/audit`; §3 appendix: `GET /stats`, `GET /download/[file]` declared non-protocol website surface; §6.3 `accepted` emission DEFERRED to M10 fair queueing; §6.4 `maxTokens` global cap 2048 (dead `profile.maxOutputTokens` reference removed); effective §6.5 registry: `duplicate_request`→`replayed_request`, `over_limit`→`overloaded` renames land as R0.5 code (client+server together); `invalid_lease`/`bad_frame`/`executor_error` added |
| 2026-10-09 | ADR-029 (execution presets) | Creates `protocol/msp-cooperative-v1.md` (SessionOffer/preset namespace); this document stays byte-identical to its pre-ADR-029 frozen text |
| 2026-10-09 | ADR-030 (receipts v2) | §7 additive successor: ReceiptV2 is a new signed object (v1 §7 shape unchanged in place); `/events/job-result` `receiptDigest` preimage defined as SHA-256 over the canonical counter-signed ReceiptV2 |
| 2026-10-09 | Post-gate R0.5 code batch (owner gate D7/D16; ADR-028 §7) | §3.3 `challenge/start` row: `challengePrompt` is nonce-bound (`"ModelSwarm readiness challenge <challengeId>"` + optional `"; greedy-canary sha256:<64hex>"` pinned possession digest, hash-only) — value-level change riding the existing field, NO schema edit, golden vector `protocol/vectors/challenge-prompt-1.json`; §5 honest-label note: `verified_capacity`/`capacityClass` are self-reported labels, not hub-verified measurements |

### §3 appendix — non-protocol website surface

`GET /api/v1/stats` and `GET /api/v1/download/[file]` are website features
(content-blind counters and installer downloads). They are NOT protocol
endpoints, carry no peer state, and may change with the website without an
ADR (ADR-028 §1).
