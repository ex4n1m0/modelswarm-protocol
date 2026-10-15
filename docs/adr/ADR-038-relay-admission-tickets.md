# ADR-038: Relay admission tickets — the D12 close (open relay for enrolled installations)

Status: **Proposed (2026-10-15) — committed verbatim by the Integrator from the
read-only Protocol seat's draft; acceptance per the ADR log process.
Owner-directed; the directive is quoted verbatim in §1. Authored against HEAD
`7cc1274` on `main`.** Amends `protocol/msp-v1.md` §3.3 (heartbeat response) and
adds §5a (exact texts in §4); amends the relay's frozen F2A resource posture
(finite per-circuit caps replace "UNCAPPED"); `protocol/messages.proto`,
`catalog/schema.json`, and the msp-cooperative-v1 namespace remain
byte-identical. Closes architecture gate **D12** ("no relay authn/authz and no
per-circuit caps — reservations are first-come",
`docs/reviews/handoff-network-engineer-2026-10-15.md` §6) when the §6 gates
pass. Owner: Protocol Architect (contract, ticket type, vocabularies, vectors,
document amendments — `protocol/`, `crates/modelswarm-eligibility`). Required
reviewers per the ownership map: Security Engineer and Test and Release
Engineer — the implementation gate; no `--public` internet deployment before
both sign off and the §6.3 verification entry exists.

## 1. The owner directive, and the constraints that survive it

Verbatim, 2026-10-15: **"can't we open the relay to everyone... if we want one
million models swarm."**

The owner wants the relay open by default, not IP-gated. The interim measure —
trusted-LAN-only deployment guidance plus whatever host-level IP filtering the
operator improvised around the F2B relay — was scaffolding, never the design,
and is superseded by this ADR without being codified. Three constraints survive
the directive and bound everything below:

1. **The relay stays content-blind.** It forwards opaque streams and never
   parses MSP frames (`crates/modelswarm-relay/src/main.rs` module contract).
   Admission adds a *pre-stream* check of a signed blob; it does not add a
   parser for relayed traffic.
2. **The tracker stays metadata-only and one-way.** The ticket is minted from
   installation identity and a clock — nothing else. The relay NEVER contacts
   the tracker: no polling, no revocation channel, no relay-originated traffic
   of any kind (T8).
3. **Honesty discipline.** The relay's refusal vocabulary stays the upstream
   one (`permission_denied(202)`, `resource_limit_exceeded(201)` — surfaced
   verbatim by the client per the F2B `status_name` discipline); every refusal
   is labeled, never silent; the caps that replace the uncapped posture are
   stated with their ADR-007 interplay, not hidden.

## 2. Context — the D12 gap, and what already exists

The F2A relay shipped deliberately uncapped and unauthenticated: reservations
first-come, duration/bytes at `u32::MAX`/`u64::MAX` because the stock defaults
(2 min / 128 KiB) cut token streams mid-flight, which ADR-007 forbids
retrying. That posture is safe only on a trusted LAN; gate D12 holds the
public internet off. What already exists and is reused, unchanged in kind:

- The relay binary (`crates/modelswarm-relay/src/main.rs`): rust-libp2p
  `relay::Behaviour` at libp2p 0.57 / **libp2p-relay 0.22.0** (Cargo.lock),
  loopback-default bind with the `--public` opt-in (ADR-018 discipline), limit
  machinery (`relay_config()`), honest log lines, and in-process enforcement
  proofs (cap denial `ResourceLimitExceeded`, unreserved-destination refusal
  `NoReservation`).
- The hand-rolled relay client
  (`crates/modelswarm-transport/src/relay_client.rs`): multistream-select
  1.0.0 (`mss_dial`/`mss_listen`), the libp2p-relay 0.22 protobuf codec
  (uvarint-length-prefixed, 4096-byte cap validated BEFORE body allocation),
  `reserve` with 3/4-lifetime auto-renewal, `dial_circuit`, and verbatim
  status surfacing.
- The tracker's signed-object machinery: the hub Ed25519 key pins at
  `protocol/keys/hub-public.hex` (embedded via `include_str!` in
  `crates/modelswarm-node/src/catalog.rs`, `HUB_PUBLIC_KEY_HEX`), the
  EligibilityLease signed shape (msp-v1 §5), the heartbeat surface
  (`POST /peers/heartbeat`, signed envelope + session, msp-v1 §3.3).
- **The enforcement seam (decisive, verified against the pinned 0.22.0 source
  in the local registry):** `libp2p_relay::rate_limiter::RateLimiter` is a
  PUBLIC trait (re-exported at `lib.rs` of libp2p-relay 0.22.0) with a blanket
  impl for any `FnMut(PeerId, &Multiaddr, Instant) -> bool + Send`, and
  `Config`'s `reservation_rate_limiters` / `circuit_src_rate_limiters` fields
  are public. The behaviour consults every limiter at BOTH accept sites —
  reservation requests (renewals included) and circuit-source requests — and
  denies with `ResourceLimitExceeded` when any returns `false`. A closure over
  an admission table is therefore a complete, race-free, fail-closed gate at
  exactly the two resource-grant sites. No vendored behaviour, no upstream
  hook request.

## 3. Decision

### 3.1 Ticket form (content-blind, tracker-signed, 600 s TTL)

A **RelayAdmissionTicket** is a tracker-signed JSON object with EXACTLY this
field set (snake_case inside the signed object, mirroring the §5 lease
discipline; `deny_unknown_fields` on parse):

```json
{
  "installation_id": "b58…",
  "peer_id": "12D3Koo…",
  "issued_at": "2026-10-15T12:00:00.000Z",
  "expires_at": "2026-10-15T12:10:00.000Z",
  "nonce": "<32 hex chars, fresh per issuance>"
}
```

`signature` is base64 Ed25519 over the canonical JSON (msp-v1 §2.2) of the
object with `signature` omitted entirely. The serialized form is the
`capabilityToken` form (msp-v1 §5):

```text
relayTicket = base64url(canonical ticket JSON) "." base64url(ed25519 signature over that JSON)
```

**Frozen constants.** TTL: `expires_at − issued_at == 600 s` exactly (pinned
by vector). Serialized ticket length ≤ 1024 bytes (enforced before
allocation). The ticket carries NO payload metadata: no profile ids, no
addresses, no relay address, no slots, no mode, no capacity class. It is
deliberately relay-agnostic — any relay honoring the tracker's pinned key
admits it, so multi-relay operations need no per-relay issuance.

**TTL reasoning.** 600 s = 20–40 heartbeat intervals (cadence 15–30 s), so a
hosting installation always holds a fresh ticket with ≥ 570 s of life; it
survives brief tracker outages for new relayed circuits without operator
action; and it bounds every compromise window (stolen ticket, tracker key
compromise per-ticket, revoked-installation tail) to TTL + the ±120 s skew
grace = ≤ 12 min. It is also the reservation-duration bound (§3.4): admission
can never outlive its ticket by more than the skew grace.

**Why 600 s and not shorter:** ticket freshness rides the EXISTING heartbeat
cadence — a shorter TTL would force a new fetch surface or make cross-NAT
sessions brittle during tracker blips. Why not longer: every abuse bound in
§3.5 scales with TTL.

### 3.2 How the ticket travels: a thin pre-flight frame (Option B), not
reservation-bytes squatting

Two candidate carriers were considered:

- **(A) Ride the hop RESERVE bytes** (occupy an unused field of the
  libp2p-relay `HopMessage`). REJECTED. The upstream proto2 schema has no free
  field we own — field 2 is `peer` (CONNECT's destination), and squatting it
  rides a field with different upstream semantics that upstream may begin
  validating (an interop regression we cannot test for in advance).
  Decisively: circuit-side admission has NO rideable bytes at all (CONNECT's
  field 2 is load-bearing), so option A can gate at most half the resource
  surface.
- **(B) A thin pre-flight frame** — a new MSP-owned multistream-select
  protocol negotiated on the SAME QUIC connection before any hop exchange.
  **CHOSEN.** Zero upstream reservation bytes are touched (stock-relay
  interop stays byte-identical); the negotiation itself is graceful (`na`
  from a relay that does not know the protocol = "no admission surface
  here", the standard libp2p optional-protocol behavior); the client already
  carries the exact machinery (`mss_dial`/`mss_listen`, uvarint framing,
  protobuf codec, size-before-allocation reads); and both roles (reserving
  listeners AND circuit dialers) present the same frame on the same
  connection.

**The admission protocol, frozen (this ADR is its schema of record — it is a
transport-layer protocol in the ADR-018 family, NOT an msp-v1 §6 message, and
it deliberately does NOT touch `protocol/messages.proto`):**

- Protocol name (mss-negotiated, trailing `\n` per mss rules):
  **`/msp/relay-ticket/1`**.
- After negotiation, exactly ONE request frame then ONE response frame, each
  a uvarint-length-prefixed protobuf body with the 4096-byte cap validated
  before allocation (the hop/stop codec discipline):

```protobuf
// request, client → relay
message AdmissionMessage {
  enum Type { PRESENT = 0; }        // only value in v1
  required Type type = 1;           // varint
  optional bytes ticket = 2;        // the serialized relayTicket string (UTF-8)
}
// response, relay → client
message AdmissionStatus {
  enum Type { STATUS = 1; }         // only value in v1
  required Type type = 1;           // varint
  optional uint32 code = 3;         // status value
}
```

- Status VALUES reuse the upstream relay code values, so one vocabulary names
  one fact across the relay's protocols: `ok(100)`,
  `resource_limit_exceeded(201)` (admission table full),
  `permission_denied(202)` (signature/expiry/binding refusals),
  `malformed_message(400)` (frame/schema/ticket-syntax failures). The client
  surfaces these verbatim through the existing `status_name` discipline.

**Sizing:** the serialized ticket is ≈ 430 bytes (44-char base58 installation
id, 52-char PeerId, two RFC-3339 timestamps, 32-hex nonce, JSON overhead,
base64url expansion, 86-byte signature) — one frame, comfortably inside the
4096-byte cap and the 16 KiB mss frame bound.

**Client presentation rule (frozen).** After dialing a relay leg
(PeerId-verified, unchanged) and BEFORE the first hop exchange on that
connection (RESERVE or CONNECT): offer `/msp/relay-ticket/1`. On echo, send
PRESENT with the freshest cached ticket; on `ok(100)` proceed to the hop
exchange; on any other status, do NOT open the hop stream on that connection
— fail with the verbatim label. On `na` (a relay without the admission
protocol, e.g. a third-party relay), proceed directly to the hop exchange
(that relay has no gate; the trust posture is the documented relay-vouched
one). Before a renewal re-RESERVE, re-present iff the last-presented ticket's
`expires_at` has passed or is within 120 s; presentations are idempotent at
the relay (a later PRESENT refreshes the deadline).

### 3.3 Issuance — the existing heartbeat endpoint, to heartbeat-current
installations only

**Endpoint (no new route):** `POST /peers/heartbeat` (msp-v1 §3.3). Every
SUCCESSFUL heartbeat response additionally carries one ticket. The response
becomes `{leaseExpiresAt, relayTicket, notices:[Notice]}` (exact amendment
text in §4.1).

**To whom:** the ticket is issued to the AUTHENTICATED installation whose
heartbeat just succeeded — which requires the signed envelope, a live
session, and a current lease row. This is exactly "enrolled,
heartbeat-current installations," with zero new auth machinery.
`installation_id` comes from the authenticated identity and `peer_id` from
the installation's tracker-side registered row (msp-v1 §3.3 register
validation: `peerId` derives from the registered `pubKey`) — never from
client input in the heartbeat. Every legitimate MSP installation hosts (the
project rule), hence heartbeats, hence holds tickets; both relay roles
(reserving server, dialing requester) are covered by the same surface.

**Deliberate separation (normative):** the ticket asserts installation
liveness ONLY. It grants relay CAPACITY access; it is not eligibility, not a
serving right, and not consumed by any msp-v1 §6 logic. Serving eligibility
remains the EligibilityLease + ADR-026 session-open gate, untouched.

**Rate limits:** none new. Issuance is bounded by the existing heartbeat
surface: 15–30 s cadence, and the frozen 60 req/min/installation
peer-endpoint cap is the hard bound (worst case 60 tiny tickets/min from a
misbehaving installation — each ≤ ~700 B of response, trivial and
content-blind).

**Revocation posture — expiry-only, stated honestly.** There is NO
relay-side revocation channel in v1 and none is authorized by this ADR: the
relay never contacts the tracker (T8), and `revoked_peers` notices ride
heartbeats to peers, not to relays. Consequences, stated without softening:
an installation revoked at the tracker keeps its LAST ticket valid until its
`expires_at` (+120 s skew) — a tail of ≤ 12 min, during which per-peer caps
(§3.4) bound the damage. Rotation of the pinned tracker key (new
`protocol/keys/hub-public.hex`, relay restart with the new key) is the one
INSTANT global revocation lever, and it is also the tracker-compromise
recovery path.

### 3.4 Verification at the relay, key distribution, and the fail-closed rule

**Key distribution (existing bootstrap surface):** the ticket is verified
against the SAME pinned hub Ed25519 public key the node build already embeds
— `protocol/keys/hub-public.hex`. The relay binary pins it the same way
(`include_str!`, as `crates/modelswarm-node/src/catalog.rs` does),
overridable for rotation testing by a new command-line flag `--tracker-key
<PATH>` (64 lowercase hex, trimmed; command surface, not protocol surface).
One key guards catalog, leases, and tickets; ticket compromise exposure is
therefore never worse than the existing trust root's.

**Boot discipline (fail-closed at startup):** a `--public` (non-loopback)
bind REQUIRES a parseable tracker key — embedded default or `--tracker-key`.
Refusal to start (exit 2, honest one-line error) if `--public` is set and no
valid key resolves. The gate itself is unconditional: the admission protocol
answers and the gate limiters are installed in every mode (loopback testing
uses fixture keys per the §6 vector discipline); no ungated mode exists.

**Verification order at PRESENT (first-failure-wins, frozen, testable — the
outcome is the pre-flight status plus the local log reason):**

| # | Check | Failure → status / log reason |
|---|---|---|
| 1 | Frame: announced length ∈ [1, 4096], validated before allocation | `malformed_message(400)` / `malformed` |
| 2 | Protobuf schema: `type == PRESENT`, ticket bytes present, no unknown or wrongly-typed fields (strict) | `malformed_message(400)` / `malformed` |
| 3 | Ticket string length ≤ 1024 bytes | `malformed_message(400)` / `malformed` |
| 4 | Ticket syntax: two base64url segments; canonical JSON strict parse (`deny_unknown_fields`); exact field set; RFC-3339 timestamps; 32-hex nonce; `expires_at − issued_at == 600 s` | `malformed_message(400)` / `malformed` |
| 5 | Ed25519 signature verifies against the pinned tracker key | `permission_denied(202)` / `bad_signature` |
| 6 | Expiry: `issued_at ≤ now + 120 s` and `now ≤ expires_at + 120 s` (the msp-v1 §6.6 skew window, same rule as lease checks) | `permission_denied(202)` / `expired` |
| 7 | Peer binding: `ticket.peer_id` == the connection's Noise-authenticated remote PeerId | `permission_denied(202)` / `peer_mismatch` |
| 8 | Identity consistency: extract the Ed25519 public key embedded in the authenticated PeerId (ADR-020 identity multihash), compute `base58(SHA-256(pubkey))`, MUST equal `ticket.installation_id` | `permission_denied(202)` / `installation_mismatch` |
| 9 | Admission table capacity (≤ 4096 live entries; expired entries evicted first) | `resource_limit_exceeded(201)` / `table_full` |
| 10 | Store/refresh admission: key = PeerId, deadline = `expires_at + 120 s` (idempotent) | `ok(100)` / `admission accepted <peer> until <rfc3339>` |

**Enforcement (the actual fail-closed rule — at the resource-grant sites,
not only at the pre-flight):** the relay prepends ONE closure limiter to BOTH
`reservation_rate_limiters` and `circuit_src_rate_limiters`:

```text
gate = |peer: PeerId, _addr, _now| admission_table.is_live(&peer)
```

The pinned libp2p-relay 0.22.0 behaviour consults it on EVERY reservation
request (renewals included) and EVERY circuit-source request, denying with
`ResourceLimitExceeded` when it returns false. Therefore: **no valid
admission ⇒ no reservation and no circuit, regardless of whether the client
ever ran the pre-flight** — a hostile client cannot skip, race, or spoof its
way past the gate, because the gate is not a handshake step but a predicate
on the accept path itself. The gate limiter is FIRST in both vectors so
unadmitted peers never consume the per-IP token buckets honest peers behind
shared NATs depend on (`.all()` short-circuits on the first `false`).

**Honest status-labeling note (frozen):** the hop-layer wire status for a
gate denial is upstream-frozen as `resource_limit_exceeded(201)` — the
limiter seam emits only that code. The pre-flight response is the honest,
reason-bearing admission verdict (`202` + the seven-value reason set below);
a client that presented, got `ok(100)`, and then receives `201` on the hop
exchange reports the verbatim upstream label (it cannot and must not
reinterpret it). The relay log vocabulary extends with exactly: `admission
accepted <peer> until <rfc3339>` and `admission denied <peer>: {malformed |
bad_signature | expired | peer_mismatch | installation_mismatch |
table_full}` — nothing else; no ticket bytes are ever logged.

**Admission state is MEMORY-ONLY** (never persisted): a relay restart empties
the table and every peer re-presents — one cheap frame each.

### 3.5 Abuse bounds — the ADR-frozen relay policy defaults

The F2A/F2B posture (duration/bytes UNCAPPED) is superseded: the same cap
machinery now runs with FINITE, ADR-frozen defaults. The full policy table
(the relay's `relay_config()`, frozen by this ADR; changes only by ADR-038
amendment):

| Knob | Frozen value | Was | Reasoning |
|---|---|---|---|
| `max_reservations` | 4096 | 4096 | unchanged |
| `max_reservations_per_peer` | **8** | 8 | unchanged (the existing per-peer bound) |
| `reservation_duration` | **600 s** | libp2p default 3600 s | ≤ ticket TTL: admission can never outlive its ticket beyond the skew grace; renewals re-hit the gate every ≤ 450 s (clients already renew at 3/4 of the ANNOUNCED expiry) |
| reservation rate limiters | upstream defaults (per-peer 30/2 min, per-IP 60/min) + **admission gate prepended** | defaults only | the gate closes D12; the defaults stay for accounting |
| `max_circuits` | 4096 | 4096 | unchanged |
| `max_circuits_per_peer` | 128 | 128 | unchanged |
| circuit src rate limiters | upstream defaults + **admission gate prepended** | defaults only | src-side admission (both roles present) |
| `max_circuit_duration` | **1800 s (30 min)** | `u32::MAX` (uncapped) | 15× the maximum legitimate request deadline (120 s, msp-v1 §6.4); supports ≥ 15 maximum-length family/cooperative rounds; bounds abuse residency |
| `max_circuit_bytes` | **16 MiB (16,777,216)** | `u64::MAX` (uncapped) | 64 max-size frames (256 KiB, §6.4) per direction; ≈ 128× the largest plain-serving exchange (32 KiB prompt + 2048-token completion ≈ ≤ 128 KiB total); bounds per-circuit abuse |
| admission table | 4096 PeerId entries, memory-only, deadline-evicted | — | new |
| ticket / frame bounds | ticket ≤ 1024 B; admission frame ≤ 4096 B | — | new |

**ADR-007 interplay (normative honesty).** A cap cut mid-stream after the
first output token is an unrecoverable interruption — this is exactly why
F2A shipped uncapped. The frozen numbers sit two orders of magnitude above
every legitimate maximum enumerated above, so a cap hit on legitimate
traffic is a defect report, not a policy event; when it happens it surfaces
as the existing honest `interrupted` outcome (§6.5) with NO retry (rule 6).
If any legitimate workload EVER measures against these caps, the caps move
by recorded ADR amendment with measurements — never by config tweak, never
silently. The relay MUST announce its finite circuit limits with every
reservation (upstream `Limit` message; the client already records
`CircuitLimits` "so callers can price relayed paths honestly" — ADR-013/014
pricing consumes this).

**Residual abuse arithmetic (deterrence-grade, stated honestly in the
AGENTS.md sense — prototype-grade deterrence, not resource guarantee).**
Worst sustained relay burn by ONE admitted rogue installation:
circuit-creation rate 30/2 min × 16 MiB ≈ 480 MiB / 2 min ≈ 32 Mbps. That
exceeds the ~10 Mbps deployment budget (F2B §6), so the operator's actual
pipe remains the global bound; the caps bound per-peer fairness and
residency, not total cost. An UNadmitted peer gets nothing (T1).

### 3.6 The open-relay posture — D12 closure

When §6 lands and passes, gate D12 CLOSES and the relay's posture becomes:
**open-by-default on `--public`, admission by ticket, bounded by caps.**
There is no IP allowlist, no pre-registration of relays with the tracker, no
relay-side tracker dependency. Scaling answer to the owner's directive:
admission scales with TRACKER ENROLLMENT (the cap the owner actually
controls — admin approval / `DEVICE_APPROVAL_CAP`, default 250 on
auto-approve deployments), not with IP curation. A "million-model swarm" is
admitted by enrolling installations, not by the relay operator knowing
peers.

**Honest residual risks (recorded, not softened):**

1. **Tracker key compromise = relay abuse until rotation.** The holder of
   the hub Ed25519 key can mint tickets for any PeerId it controls (the
   ticket is tracker-signed, not peer-signed) — unlimited admission while
   the compromise lasts, each ticket capped at TTL + skew. The same key
   already signs the catalog and every lease, so this is the existing
   root-of-trust exposure, not a new class; recovery = rotate
   `protocol/keys/hub-public.hex` + relay restart (instant global
   invalidation).
2. **Ticket farming via enrolled installs — bounded, not prevented.** A
   farmed ticket admits ONLY the PeerId it names (check 7) and only that
   peer's QUIC key can present it; farming therefore means running many
   enrolled installations (sybils), bounded by enrollment controls and by
   the per-peer caps (8 reservations, 128 circuits, 30 creations/2 min,
   16 MiB/30 min per circuit).
3. **The relay operator sees flow metadata.** Who reserves, who circuits to
   whom, timing, sizes — and, as already documented in F2B risk 2, a relay
   CAN read relayed frames (QUIC terminates at the relay; the payload is
   plaintext AT the relay — our relay is content-blind by construction, a
   hostile operator's fork need not be). The standing mitigation is the
   msp-v1 §6.1 end-to-end handshake over relayed circuits — the
   session-layer decision F2B §7.2 already queued; this ADR does not change
   it and does not claim confidentiality it cannot deliver.
4. **Expiry-only revocation tail** ≤ TTL + 120 s (§3.3).
5. **New relayed circuits during a tracker outage fail honestly** (no fresh
   tickets): existing circuits live under the caps, direct paths are
   unaffected — the msp-v1 §8 "tracker outage → streams continue"
   discipline holds for established streams, with this labeled degradation
   for new relayed ones.

### 3.7 State machines and invariants

**Relay admission state (per PeerId; memory-only):**

```text
                 PRESENT ok(100)                    deadline = expires_at + 120s
UNADMITTED ─────────────────────────────► ADMITTED ────────────► UNADMITTED (evicted)
    ▲                                        │
    │  PRESENT refused (202/201/400;         │ PRESENT (idempotent refresh;
    └── table unchanged; client may ◄────────┘ new nonce/deadline replaces old)
        retry with a fresh ticket)
Hop layer (per RESERVE/CONNECT arrival, upstream code path):
  gate(peer) live ──► upstream accept path (caps/accounting as today)
  gate(peer) dead  ──► upstream denial, status resource_limit_exceeded(201)
```

**Client (per relay leg; both roles):**

```text
DIAL relay leg (PeerId-verified, unchanged)
  → mss-offer /msp/relay-ticket/1
      ├─ echo  → PRESENT (freshest cached ticket)
      │           ├─ ok(100)        → hop RESERVE (listener) / CONNECT (dialer)
      │           └─ 201|202|400    → honest verbatim label; NO hop stream on this connection
      └─ na    → (relay has no admission surface) → hop exchange directly
Ticket cache (node): each heartbeat response replaces the cached ticket;
present iff no cache or last-presented ticket expired/near-expiry (120 s);
re-present before renewal re-RESERVE under the same rule.
```

**Invariants (test obligations attach in §6):**

- **(T1) FAIL-CLOSED AT THE GRANT SITES.** No live admission ⇒ no
  reservation, no circuit — enforced by the gate limiter on the upstream
  accept path, independent of the pre-flight.
- **(T2) CONTENT-BLIND TICKET.** Field set is exactly `{installation_id,
  peer_id, issued_at, expires_at, nonce}` + signature; vector-asserted; the
  tracker mints from identity + clock only.
- **(T3) PEER BINDING.** Admission is keyed to the Noise-authenticated
  PeerId; `ticket.peer_id` mismatch refuses; `installation_id` must
  re-derive from the authenticated PeerId's embedded key (ADR-020).
- **(T4) TTL BOUND + MEMORY-ONLY STATE.** `reservation_duration ≤ TTL`;
  admission never outlives its ticket beyond the ±120 s grace; state never
  persists.
- **(T5) CAPS ANNOUNCED.** Finite limits ride every reservation response;
  clients record and price them (ADR-013/014).
- **(T6) ONE VOCABULARY.** Upstream status names verbatim; admission
  reasons are the frozen seven-value set; pre-flight codes reuse upstream
  values.
- **(T7) ADR-007 INTERPLAY.** Cap cuts are honest `interrupted` outcomes,
  never retried after the first token; caps move only by amendment with
  measurements.
- **(T8) ONE-WAY, DECOUPLED TRACKER.** No relay→tracker traffic of any
  kind; the relay learns nothing from the tracker except what the pinned
  key lets it verify.

**Binding-set note (ADR-033 §4 precedent):** the admission frame is not an
msp-v1 §6 state-changing message and carries no session/seq/round/profile
binding — deliberately. Its binding set is: protocol version (the frozen
protocol name), sender identity (Noise-authenticated PeerId + ticket
`peer_id`/`installation_id`), nonce (ticket `nonce`, issuance-dedup),
deadline (`expires_at` + the 120 s skew rule), and signature (tracker
Ed25519). The only state it changes is the relay-local, memory-only
admission table.

## 4. Frozen-document amendments (exact texts; the Integrator lands them)

`protocol/messages.proto`, `catalog/schema.json`, `catalog/schema-v2.json`,
and `protocol/msp-cooperative-v1.md` stay byte-identical (asserted by
existing guards; the wire-compat job is extended, not widened, per §6.2(7)).

**4.1 `protocol/msp-v1.md` §3.3, heartbeat row** — replace the row's
response cell with:

> | `POST /peers/heartbeat` | `{leaseId, activeProfiles:[profileId…], freeSlots, queueMs, draining:bool}` | `{leaseExpiresAt, relayTicket, notices:[Notice]}` (below; `relayTicket` is the §5a admission ticket string, issued on every successful heartbeat — ADR-038) |

**4.2 `protocol/msp-v1.md` — insert as §5a (between §5 and §6):** the §5a
text as committed (relay admission ticket: signed field set, canonical-JSON
Ed25519 signature form, 600 s TTL, heartbeat issuance, pinned-key
verification, PeerId binding, multi-use within lifetime, expiry-only
revocation, capacity-access-only grant, `/msp/relay-ticket/1` transport,
golden vectors `protocol/vectors/relay-ticket-*.json` +
`relay-admission-frame-1.json`).

**4.3 `protocol/msp-v1.md` §9 changelog — append row:**

> | 2026-10-15 | ADR-038 (relay admission tickets) | §3.3 heartbeat response gains `relayTicket`; §5a added (relay admission ticket, 600 s TTL, `/msp/relay-ticket/1`); §6 untouched |

**4.4 `docs/adr/ADR-014` — append revision note:** Phase F's separately
hosted public relay gains admission control and frozen per-circuit caps
(tracker-signed ticket via the `/msp/relay-ticket/1` pre-flight; enforcement
at the reservation/circuit accept sites; 30 min / 16 MiB circuit caps).
Decision 2's hosting posture unchanged; decision 5's relayed-path labeling
unchanged; DCUtR remains separate.

## 5. What this ADR does NOT authorize

1. **No DCUtR / hole-punch upgrade** — relayed paths stay relayed; a
   separate ADR owns any DCUtR work (F2B risk 3 stands).
2. **No peer-run-relay discovery changes** — relays remain
   operator-configured multiaddrs riding existing config/`addresses`
   surfaces; no relay advertisement protocol, no DHT, no peer-run relay
   fleet.
3. **No payments or relay economics** — no relay pricing, no byte accounting
   in receipts, no ReceiptV2 changes.
4. **No relay→tracker channel of any kind** — no polling, no revocation
   fetch, no relay registration at the tracker (T8). Revocation is
   expiry-only plus key rotation.
5. **No change to the relay-vouched far-end trust model** — end-to-end peer
   authentication over relayed circuits (msp-v1 §6.1 at the session layer,
   F2B §7.2) is a separate decision this ADR explicitly does not make.
6. **No `protocol/messages.proto`, msp-cooperative-v1, catalog schema, or
   EligibilityLease change** — byte-identical surfaces; the admission
   protocol is transport-layer with this ADR as its frozen schema of
   record.
7. **No ticket semantics beyond admission** — the ticket is never consumed
   by §6 stream logic, eligibility, receipts, or scheduling; and no new
   tracker REST route exists (issuance rides the existing heartbeat).

## 6. Gate requirements (before any `--public` internet deployment)

**6.1 Security Engineer — the admission adversary matrix (enumerate
learn/inject).** LEAK surface: the relay's own logs (peer ids, admission
reasons, timing — already the F2B log discipline, counts/ids only); flow
metadata (§3.6 item 3 — restated in `docs/threat-model.md` with the
plaintext-at-relay caveat, uncopied from F2B risk 2). INJECT/ATTACK surface
to adjudicate: (S1) ticketless flood — T1 denies at the accept sites,
including clients that never run the pre-flight; (S2) forged signature —
Ed25519 against the pinned key; (S3) farmed/stolen ticket — peer-binding
(check 7) means it admits only the named PeerId whose QUIC key must present
it; sybils bounded by enrollment + per-peer caps; (S4) tracker key
compromise — §3.6 item 1; confirm the rotation runbook step; (S5)
admission-table exhaustion — 4096 entries keyed by PeerId, deadline-evicted,
201 on full, bounded by the enrolled population; (S6) clock-skew games —
the ±120 s windows pinned by test; (S7) pre-flight injection — strict
protobuf, single frame each way, size-before-allocation, mss-bounded, no
unbounded parse path, connection-bound presentation; (S8) rate-limiter
token theft via shared NAT — gate limiter prepended FIRST so unadmitted
peers never consume per-IP tokens; (S9) restart/state — memory-only
admission, no persistence, re-presentation cost. The Security review also
signs off the `docs/threat-model.md` D12 update (gate closed; residual rows
for key-compromise window, farming bound, operator
metadata/plaintext-at-relay, expiry-only tail).

**6.2 Test and Release Engineer — fixtures, properties, venue.**

Golden vectors (`protocol/vectors/`, the lease-hubkey fixture discipline —
fixture-only tracker signing key recorded in the file, `verify_at` time
anchor, wire form + `canonical_json_sha256` byte-pinned, consumer-neutral):

| Fixture | Pins |
|---|---|
| `relay-ticket-1.json` | Accept: canonical-JSON bytes + sha256; signature preimage (signature-omitted canonical form); the 600 s delta; serialized form; admission deadline computation (`expires_at + 120 s`); outcome `ok(100)` |
| `relay-ticket-2-expired.json` | Expired-at-presentation under the skew rule → `permission_denied(202)`, reason `expired` |
| `relay-ticket-forge-1.json` | Array with expected status+reason per row: bad signature; ticket for peer B presented on A's connection (`peer_mismatch`); `installation_id` not re-deriving from the PeerId's embedded key (`installation_mismatch`); unknown field; non-canonical segment; oversized ticket (>1024 B); wrong issued/expired delta; nonce wrong charset/length |
| `relay-admission-frame-1.json` | Byte-pinned protobuf frames: PRESENT encode (uvarint length prefix + body), STATUS encodes for 100/201/202/400; malformed-proto rejection corpus (field 0, length overrun, unknown type, truncation) — a second implementation must reproduce identical bytes and verdicts |

Per-push property/suite tests:

1. **Fail-closed proofs (relay crate, real in-process relay stack):** a
   standard libp2p client that never runs the pre-flight is REFUSED —
   reservation AND circuit both denied `ResourceLimitExceeded`;
   presentation with a fixture-key ticket then admits the same client
   (reserve → circuit → bidirectional ping, the F2B test extended);
   admission expiry (virtual clock past `expires_at + 120 s`) re-closes
   both gates.
2. **Forge/expiry/refuse paths:** every `relay-ticket-forge-1.json` row
   re-derived in-code and asserted against the frozen status+reason pair;
   first-failure-wins ORDER pinned (e.g., bad signature is never reported
   for a ticket that also fails peer binding — signature precedes binding).
3. **Caps enforcement with the frozen numbers:** duration cut at 1800 s and
   byte cut at 16 MiB against the existing announced-limit cut machinery;
   the reservation response ANNOUNCES the finite limits (T5);
   `reservation_duration` 600 s announced expiry and 450 s client renewal
   cadence.
4. **Boot discipline:** `--public` without a resolvable tracker key exits 2
   with the honest error (parser test); loopback default unchanged.
5. **Client paths (transport crate):** pre-flight before reserve and before
   connect; `ok` → hop proceeds; `202` → no hop stream on the connection,
   label surfaced verbatim; `na` from a relay without the protocol → hop
   proceeds (third-party interop preserved); renewal re-presentation rule;
   ticket cache refresh from heartbeat responses.
6. **Redaction/content-blindness:** ticket field set asserted EXACTLY (T2);
   no ticket bytes in relay logs; node logs/telemetry carry counts and
   codes only.
7. **Wire-compat lockstep:** the TS tracker and the Rust client heartbeat
   schema updated together; the TS-tracker-vs-Rust-client wire-compat CI
   job extended with the `relayTicket` field and the `relay-ticket-1.json`
   vector; old-client-tolerant deploy order (tracker first; old clients
   ignore the additive field — serde default tolerance pinned by test).
8. **Guards:** `messages.proto` and catalog schema byte-identity assertions
   unchanged and green; msp-v1 amendments are exactly §4's texts.

**6.3 Verification evidence (the D12 close record).** A
`docs/verification/relay-admission-<date>.md` entry records, against the
real deployed relay: (a) a ticketed cross-NAT reservation + circuit +
streamed completion (the F2B exit shape, now ticketed); (b) the
ticketless-refusal proof from a second machine; (c) the relay's log lines
for both; (d) the tracker heartbeat issuance capture (field present, ticket
verifies against `protocol/keys/hub-public.hex`). Owner-gated real-network
venue per the F2B §8 precedent; CI never touches production. **Gate D12
closes only when §6.1 and §6.2 approve AND this entry exists.**

**6.4 Implementation routing (ownership map).** Contract, ticket type +
verification (`crates/modelswarm-eligibility/src/relay_ticket.rs`, the
lease's sibling), vectors, and the §4 document amendments: **Protocol
Architect**. Tracker minting + heartbeat field + TS tests (`apps/tracker/`)
and the Rust `HeartbeatResponse` field
(`crates/modelswarm-tracker-api/src/lib.rs`): **Tracker Engineer**
(Protocol, Security, T/R review). Relay admission behaviour, gate limiters,
frozen policy config, `--tracker-key`, boot rule, log vocabulary, relay
tests (`crates/modelswarm-relay/`): **Network Engineer** (Protocol,
Security, T/R review). Pre-flight codec + presentation + renewal rule +
labels (`crates/modelswarm-transport/src/relay_client.rs`; a feature-gated
dependency edge modelswarm-transport → modelswarm-eligibility under
`libp2p-backend`): **Network Engineer**, Protocol review. Node ticket cache
+ handoff to the transport's relay paths (`crates/modelswarm-node/`):
**Windows Product Engineer** (Protocol, T/R review). Threat-model D12
update: **Security Engineer**. CI, guards, wire-compat job extension:
**Test and Release Engineer**. All hands run the standard `cargo fmt
--check`, `clippy -D warnings`, `test --workspace` gate (tracker work adds
`npm run typecheck` + `npm run build` in `apps/tracker/`) before handoff.

## 7. Consequences

+ The owner's open relay lands with an honest admission story: any enrolled,
  heartbeat-current installation is admitted, on any relay honoring the
  tracker's key; scaling is enrollment, not IP curation; D12's two gaps
  (authn/authz, per-circuit caps) both close.
+ The enforcement rides verified upstream machinery (the limiter seam)
  rather than a fork: no vendored behaviour, upstream wire compatibility
  preserved, and the gate cannot be skipped because it sits on the accept
  path.
+ The uncapped-stream hazard is bounded by numbers two orders above
  legitimate traffic, with the ADR-007 interplay stated and a
  measurement-driven amendment path.
− One more signed object, protocol name, and key-dependency to maintain
  forever; the tracker key becomes load-bearing for relay admission too
  (rotation now touches relay operations as well as node releases).
− Honest limits that will eventually bite: expiry-only revocation (≤ 12 min
  tail), new relayed circuits fail during tracker outages, and an admitted
  rogue can still burn up to ~32 Mbps sustained until revoked-and-expired —
  the operator's pipe, not the protocol, is the global bound.

## 8. Evidence base (all committed or pinned)

`docs/reviews/handoff-network-engineer-2026-10-15.md` (§5 risk 5 / §6 D12
gap / §7 cross-path requests) · ADR-014 (relay roadmap; revision note §4.4)
· ADR-018 (allow_off_loopback bind discipline the ticket boot rule extends)
· ADR-020 (the two identity derivations — the §3.4 check-8 preimage) ·
ADR-006/ADR-012 (the signed short-lived-token family and 60 s grace
precedent) · ADR-026 (the eligibility gate the ticket deliberately does not
touch) · ADR-007/`protocol/msp-v1.md` §6.4–6.6 (deadline ceilings, error
vocabulary, skew windows) · `crates/modelswarm-relay/src/main.rs` (limits
machinery, bind discipline, enforcement-proof tests) ·
`crates/modelswarm-transport/src/relay_client.rs` (mss + protobuf codec,
reserve/renewal/dial paths, `CircuitLimits` recording) ·
`crates/modelswarm-tracker-api/src/lib.rs:169-174` (`HeartbeatResponse`) ·
`crates/modelswarm-node/src/catalog.rs` (`HUB_PUBLIC_KEY_HEX`,
`protocol/keys/hub-public.hex`) · `Cargo.lock` pinning libp2p-relay 0.22.0,
whose locally cached source evidences: the public
`rate_limiter::RateLimiter` re-export with the closure blanket impl, and the
two limiter consultation sites mapping `false` to `ResourceLimitExceeded`
(reservations incl. renewals; circuit sources).

## Appendix A — code-path touch-points (briefing list for the implementation seats)

| # | Path | Change | Seat |
|---|---|---|---|
| 1 | `protocol/msp-v1.md` | §3.3 heartbeat row, §5a insert, §9 row — exactly §4's texts | Protocol |
| 2 | `protocol/vectors/relay-ticket-1.json`, `relay-ticket-2-expired.json`, `relay-ticket-forge-1.json`, `relay-admission-frame-1.json` | New golden vectors (§6.2 table) | Protocol |
| 3 | `crates/modelswarm-eligibility/src/relay_ticket.rs` | New: ticket type, strict parse, signature verify, constants (600 s, 1024 B), fixture-key test discipline | Protocol |
| 4 | `apps/tracker/` | Heartbeat handler mints + signs `relayTicket` from server-side rows; TS tests; typecheck/build | Tracker |
| 5 | `crates/modelswarm-tracker-api/src/lib.rs` | `HeartbeatResponse` gains `relay_ticket` (opaque string, `#[serde(default)]` for deploy skew) | Tracker |
| 6 | `crates/modelswarm-relay/src/main.rs` + new `admission` module | Admission behaviour (`/msp/relay-ticket/1`), gate limiters prepended to both limiter vecs, frozen policy table (§3.5), `--tracker-key`, `--public`-requires-key boot rule, admission log vocabulary, fail-closed + caps tests | Network |
| 7 | `crates/modelswarm-transport/src/relay_client.rs` | Pre-flight codec (AdmissionMessage/Status), presentation before reserve/connect, renewal re-presentation, `na` skip path, verbatim labels; feature-gated dep on `modelswarm-eligibility` | Network |
| 8 | `crates/modelswarm-node/src/` (heartbeat/relay wiring) | Ticket cache from heartbeat; hand-off into `reserve_relay` / `dial_with_fallback` paths | Windows Product |
| 9 | `docs/threat-model.md` | D12 row closed; residual rows (key-compromise window, farming bound, operator metadata/plaintext-at-relay, expiry-only tail) | Security |
| 10 | `.github/workflows/` | Vectors + suites in `rust.yml`; wire-compat job heartbeat-field extension; guards unchanged | Test and Release |
| 11 | `docs/adr/ADR-014-nat-traversal-roadmap.md` | Revision note §4.4 (with this ADR) | Integrator (with ADR-038) |
