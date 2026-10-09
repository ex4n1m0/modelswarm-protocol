# ADR-028: Tracker-surface and wire reconciliation

Status: Accepted (2026-10-09) · Owner-approved gate §B protocol row + §C.1
(architecture-seat amendment 1: the FULL audit list). Documentation-grade
except the explicitly marked R0.5 code items. Evidence base:
`docs/reviews/handoff-protocol-architect-2026-10-09.md` §2a/§2b/§2c (file:line
citations below refer to that handoff unless another document is named).

## Context

The frozen core is byte-consistent, but the edges the spec never re-adsorbed
drifted: five live tracker endpoints and one response field exist on the wire
with no msp-v1 §3 presence and no ADR (`session-authorize`, `/receipt`,
`/audit`, `/stats`, `/download`, `capacityClass`); §6.1's libp2p
handshake-skip was never recorded; §6.3's `accepted` event is
specified-but-unimplemented with no deferral; serving-peer P2P error codes
diverge from the §6.5 registry; msp-v1 has no changelog, so post-freeze
amendments are discoverable only by inline labels — several weren't labeled.
These are live production routes and wire behaviors a spec implementer cannot
discover.

## Decision

### 1. msp-v1 §3 folds (retroactive; each attributed to its originating ADR)

| Surface | Decision | Attribution |
|---|---|---|
| `POST /peers/lease` | Added to the §3.3 table: signed body `{leaseId, profileId}` → `{token, lease}` (§5 wire form; issuance checks per the live route) | ADR-012 (route `apps/tracker/app/api/v1/peers/lease/route.ts:17-77`; Rust `request_lease`, `modelswarm-tracker-api/src/lib.rs:290-302`) |
| `POST /peers/challenge/complete` result | §3.3 row corrected: returns `{passed, challengeId, capacityClass}`; token issuance moved to `/peers/lease` | ADR-012 (`challenge/complete/route.ts:7-9,49`) |
| `capacityClass` in `GET /peers` response | Folded into the §3.3 response shape (7th field). Frozen §3.3 shape change, recorded here | Field defined by ADR-012 (`CapacityClass`); served `peers/route.ts:43`, parsed `tracker-api/lib.rs:70-71` |
| `POST /api/v1/session-authorize` | Folded into §3.3 as protocol surface: signed body `{peer_ids, profile_id, mode}`. `mode` SHALL be constrained to the ADR-013 five-mode registry enum (`single`, `hedged`, `speculative_exact`, `search_verified`, `map_reduce`) — the free regex string is a registry-discipline divergence and ends under this ADR. The preset enum (ADR-029) does NOT ride this route; it lands only with `SessionOffer` | This ADR (route `session-authorize/route.ts:16-58`, schema `schemas.ts:126-132`; no Rust caller exists — grep zero hits) |
| `POST /api/v1/receipt` | Folded into §3.3 as a duplicate intake of `{receiptDigest, outcome}`. The canonical protocol intake remains `POST /events/job-result` (§3.3); the duplicate route SHALL be consolidated into it when receipts v2 (ADR-030) lands | This ADR + ADR-030 (`app/api/v1/receipt/route.ts:12-24`; Rust posts only to `/events/job-result`, `tracker-api/lib.rs:320-329`) |
| `POST /api/v1/audit` | Folded into §3.5 (admin): admin epoch-bump | This ADR; semantics ADR-012 (`audit/route.ts:14-28`) |
| `GET /api/v1/stats`, `GET /api/v1/download/[file]` | Declared **non-protocol website surface** (content-blind counters/downloads); recorded in a §3 appendix note, not as protocol endpoints | This ADR (`stats/route.ts`, `download/[file]/route.ts`) |

### 2. §6.1 libp2p handshake-skip — recorded as an ADR-018 scope extension

On the libp2p backend, the application-layer `Handshake`/`handshake_ack`
frames of §6.1 are **not exchanged**: authentication is QUIC+Noise PeerId,
and the serving peer reads `InferenceRequest` first (`serving.rs` first-frame
expectation; `remote.rs` sends it immediately after dial). §6.1's frame
sequence as written applies to the staged signed-frame backend; the libp2p
path skips to `inference_request` with peer identity carried by the
connection itself plus the ADR-026 lease gate. This exceeds ADR-018's
recorded scope and is hereby recorded as its extension. An implementer
following §6.1 verbatim on libp2p gets `expected InferenceRequest, got
Handshake` — that divergence is now documented, not accidental.

### 3. §6.3 `accepted`/queue vocabulary — formally DEFERRED

The `accepted` stream event (queue position, etaMs) and the proto's
`Accepted{queue_position, eta_ms}` are specified but never emitted on the
libp2p path (`WireMessage` has no `Accepted`; serving drops the executor
event; `queue_position`/`eta_ms` live only in `HandshakeAck`). **Decision:
DEFER emission to M10 fair queueing** (bounded wait queue + DRR), which
depends on this vocabulary. Until then admission remains admit-or-reject and
peers SHALL NOT emit `accepted`. The deferral is recorded here so the gap is
a decision, not an omission.

### 4. §6.4 serving-side sampling clamps — BACKLOG

The serving peer's `clamp_wire_request` clamps only `deadline`/`maxTokens`/
prompt-bytes; sampling parameters pass through (the gateway clamps fully at
local HTTP entry). Bringing `temperature`/`top_p`/`top_k` clamps to the
serving side per §6.4 is **BACKLOG**, non-blocking. Related spec defect
fixed by this ADR: §6.4's `maxTokens ∈ [1, profile.maxOutputTokens]`
references a dead v1 field — catalog schema v2 (ADR-011/022) dropped
`maxOutputTokens`; the implemented clamp is the node-global
`DEFAULT_MAX_OUTPUT_TOKENS = 2048` (`modelswarm-gateway/src/lib.rs:73`).
msp-v1 §6.4 now states the global cap; a per-profile cap may return only
additively via a future ADR (schema v2 field + clamp change together).

### 5. §6.5 error-code registry — renames + additions; R0.5 code change

Effective §6.5 registry under this ADR:

- **Rename** (wire-visible; lands as a follow-up **R0.5 code change** under
  this ADR, client and server updated together — code-matching clients break
  today): serving emits `duplicate_request` → registry `replayed_request`;
  `over_limit` → registry `overloaded`.
- **Add**: `invalid_lease` (ADR-026 lease-gate refusal, live on the wire
  since 2026-10-08 but absent from §6.5), `bad_frame`, `executor_error`
  (both emitted by the serving path today).

The rename is the only client-visible wire change in the R0 batch and is
explicitly NOT documentation-grade (architecture-seat amendment 2).

### 6. msp-v1 changelog section — required

msp-v1.md SHALL carry a changelog section at its end listing every
post-freeze amendment with its ADR. Added by this batch; it records at
minimum: ADR-012 (§5 lease rename + `/peers/lease`, challenge/complete
result, `capacityClass`), ADR-023 (§3.1 `/catalog/requests`, §3.5 admin
rows), the 2026-10-06 deployment-option auto-approval note (§3.2; no ADR
number — threat-model pointer), ADR-026 (`invalid_lease` refusal, §6.5
effective registry), ADR-011/022 (schema-v2 references in §3.5/§4), ADR-018
(§6 JSON framing amendment + the libp2p handshake-skip extension recorded
here), and this ADR's folds. Future spec edits append to it — no more
undiscoverable amendments.

### 7. D16 rider — challenge nonce+digest (recorded, not designed here)

The CHALLENGE_PROMPT replacement (nonce + digest) rides the EXISTING
`challengePrompt` wire field — no new fields, ever, under this ADR; anything
beyond that requires its own ADR. Security's binding conditions carry:
Rust↔TS golden-vector update, and a spot-verification sampling rate sized
from measured detection math (master prompt §12), never invented. Lands as
its own R0.5-class code change; the honest `verified_capacity` relabel (D7)
precedes it.

### 8. What this ADR does not change

- No proto field numbers, no §5 lease signed fields, no §6.2 normalized
  request fields, no §3.4 hub error-code set (TS matches exactly today).
- `session-authorize` mode-enum tightening and the `/receipt` route
  consolidation are tracker code changes recorded here; they land with the
  R0.5 batch (wire-visible only to senders of unregistered mode strings —
  none exist in any crate today).
- Wire-compat CI extension to cover the folded routes (rendezvous,
  session-authorize, receipt/job-result, notices, epoch suspension) remains
  the Tracker Engineer's F14 follow-up.

## Consequences

+ Every live route and wire behavior is discoverable from msp-v1 + the ADR
  log; the spec implementer's blind spots close.
+ The §6.3/§6.4 gaps become recorded deferrals/backlog instead of silent
  drift.
− Two small R0.5 code changes (error-code rename; mode-enum tightening +
  `/receipt` consolidation) must ship client-and-server together before the
  registry text is true on the wire.
