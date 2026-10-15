# Handoff — Tracker Engineer, 2026-10-15: ADR-038 relay admission tickets (tracker half of the D12 close)

Seat: Tracker Engineer (`apps/tracker/`, `crates/modelswarm-tracker-api/`).
Base: `c9fcc9f` on `main`. ADR: `docs/adr/ADR-038-relay-admission-tickets.md`
(Proposed) — this lands the §6.4 rows 4–5 (tracker minting + heartbeat field +
TS tests; the Rust `HeartbeatResponse` field).

## 1. Changed files and why

| Path | Change |
|---|---|
| `crates/modelswarm-tracker-api/src/lib.rs` | `HeartbeatResponse` gains `relay_ticket: String` (`#[serde(default, rename = "relayTicket")]`) — opaque to this client; `#[serde(default)]` implements the additive deploy-order rule (tracker first): an OLD tracker response (field absent) still parses for a NEW client, empty string = "no ticket issued". New unit test `heartbeat_response_relay_ticket_deploy_skew_tolerance` pins both directions (absence tolerance + wire name round-trip), per §6.2(7)'s "serde default tolerance pinned by test". |
| `apps/tracker/lib/relay-ticket.ts` (new) | `issueRelayAdmissionTicket(installationId, peerId, now, secretKey)`: canonical JSON of exactly `{installation_id, peer_id, issued_at, expires_at, nonce}` (snake_case, RFC-3339 millisecond timestamps via `toISOString()`), Ed25519 signature over the canonical form with the signature omitted, serialized `base64url(json) "." base64url(sig)` — the capabilityToken/§5 lease discipline, same code path as `lib/eligibility.ts` (`signBase64` → base64url re-encode). Fresh `randomId128()` (32 lowercase hex) nonce per issuance. Plus a structural `splitRelayAdmissionTicket` parse helper. |
| `apps/tracker/lib/constants.ts` | `RELAY_TICKET_TTL_MS = 600_000` — the §3.1 frozen constant (TTL exactly 600 s), with the reasoning comment. |
| `apps/tracker/app/api/v1/peers/heartbeat/route.ts` | On every SUCCESSFUL heartbeat (existing auth path unchanged: signed envelope + live session via `guardPeerRequest`, current lease row via `leaseBindingError`), mint one ticket SERVER-SIDE from `installationId` (the AUTHENTICATED identity — `guarded.installationId`, envelope-verified) and `lease.peerId` (the tracker-side registered row; `peerId` is only ever set at `/peers/register` where it is validated to derive from the registered `pubKey`). Signed with `ctx.keys.secretKey` — the SAME hub key surface as catalog envelopes and eligibility leases (ADR-038 §3.4; deliberately NOT a second key). Response becomes `{leaseExpiresAt, relayTicket, notices}` per the §4.1 amendment. All failure paths (401/410) return BEFORE minting — no ticket for unauthenticated/stale-lease callers. The heartbeat request body contains no peerId/installationId field at all, so client input cannot reach the ticket. |
| `apps/tracker/tests/relay-ticket.test.ts` (new) | 11 tests; see §3. |
| this file | Handoff record. |

Nothing else touched. `protocol/`, `crates/modelswarm-eligibility`,
`modelswarm-relay`, `modelswarm-transport`, `modelswarm-node`, and
`.github/workflows/` are other seats' §6.4 rows and were NOT modified.

## 2. Exact commands and outcomes

| Command | Outcome |
|---|---|
| `npx vitest run tests/relay-ticket.test.ts` (in `apps/tracker/`) | 11/11 passed |
| `npm run test` (in `apps/tracker/`) | 147 passed, 2 skipped (pg suites; no DATABASE_URL), 0 failed |
| `npm run typecheck` (in `apps/tracker/`) | clean |
| `npm run build` (in `apps/tracker/`) | Next.js production build succeeded |
| `cargo fmt --check` | clean |
| `cargo clippy -p modelswarm-tracker-api -- -D warnings` | 0 warnings |
| `cargo test -p modelswarm-tracker-api` | 6 lib + 5 integration, all passed |
| `cargo clippy --workspace -- -D warnings` | 0 warnings |
| `cargo test --workspace` | ALL suites green (78 `test result: ok` lines, 0 failures), run twice consecutively |

Flake disclosure: one intermediate full-workspace `cargo test` run showed a
single FAILED suite ("3 passed; 1 failed", 1.70 s — a timing-sensitive suite
outside my owned paths; most plausibly one of the loopback/quic/governor
network suites). It did not reproduce in two subsequent complete
workspace runs, and no crate I touched has timing-dependent tests. Recorded
here for honesty; if it recurs, Test and Release should own the triage.

No deployment was performed (Vercel deploy is owner/Integrator-gated per the
standing deployment note).

## 3. Test evidence (what the 11 new TS tests pin)

- **Issuance on success**: successful heartbeat carries `relayTicket`; exact
  five-field set (T2 content-blind: nothing else); `installation_id` ==
  authenticated enrollment identity and `peer_id` == the registered
  ADR-020 peer id (server-side rows, never client input); RFC-3339
  millisecond timestamps; canonical (sorted-keys) JSON; signature verifies
  against the hub key (`verifyBase64Url`); serialized length ≤ 1024 B
  (measured 425 B for the fixture shape).
- **TTL**: `expires_at − issued_at == 600_000 ms` exactly; `issued_at` equals
  the injectable heartbeat clock; `RELAY_TICKET_TTL_MS == 600_000` pinned.
- **Fresh nonce per issuance**: consecutive heartbeats yield distinct 32-hex
  nonces and distinct signatures (new canonical form each time).
- **No ticket on failure paths**: unauthenticated (no envelope) → 401 with no
  `relayTicket` anywhere in the body; another installation's lease → 401; stale
  (row-aged) lease → 410 `lease_expired`; unknown lease → 401.
- **Golden fixture (inline, PENDING CROSS-TIE)**: since
  `protocol/vectors/relay-ticket-1.json` does not exist yet (Protocol seat's
  §6.4 row 2), the fixture lives inline in the test file recorded exactly per
  the `lease-hubkey-1.json` discipline — fixture-only signing seed
  `0707…0707` (same fixture key as the lease vector, NOT the production hub
  key), `signing_public_key_hex`, `fields`, byte-pinned `token` +
  `canonical_json_sha256`. Pins: payload is the byte-exact canonical JSON of
  the recorded fields; signature verifies against the fixture key (flipped
  first byte fails; the rig's different hub key fails); frozen shape (field
  set, 600 s delta, 32-hex nonce, ≤ 1024 B, relay admission deadline =
  `expires_at + 120 s` skew grace); issuer reproduces the deterministic
  fields (nonce excepted — the lease-vector comparison discipline); and the
  full wire form reconstructs byte-exactly from `canonicalJson` + the
  base64url Ed25519 discipline. The fixture's `installation_id`
  (`base58(sha256(pubkey-of-[9u8;32]))`) is coherent with its `peer_id` (the
  ADR-020 multihash of the same key), so the relay's §3.4 check-8
  re-derivation will hold for it too.

## 4. Assumptions

1. Response field name is `relayTicket` (camelCase wire, §4.1 amendment text);
   the signed object inside is snake_case (§3.1), mirroring the lease
   discipline exactly as the ADR specifies.
2. "Opaque string" per the task/ADR appendix row 5: Rust type is `String`
   (empty = absent under `#[serde(default)]`), not `Option<String>` — matches
   the ADR's own field description and keeps the client-side diff minimal for
   the Windows Product seat's node cache work.
3. `RFC-3339 millisecond timestamps` = `new Date(ms).toISOString()` output
   (`…T…:…:….mmmZ`) — the same form the eligibility lease uses and the
   lease-hubkey vector records.
4. Nonce source is `randomId128()` (crypto-random 16 bytes, 32 lowercase hex)
   — the tracker's existing per-issuance nonce discipline (lease/challenge).
5. No new store writes, no new rate limit, no new route (§3.3: issuance rides
   the existing heartbeat surface; bounded by the existing 60 req/min cap).
6. The ticket is NOT recorded server-side (the relay's admission table is
   memory-only per §3.4; the tracker needs no revocation channel per T8).

## 5. Unresolved risks / open items

1. **PENDING CROSS-TIE (the important one)**: ~~resolved by the §6 addendum —
   the vectors landed at `af1dc01` and the TS suite now consumes them
   directly~~. (Original text: the inline test fixture must be reconciled
   with `protocol/vectors/relay-ticket-1.json` when the Protocol seat lands
   it; the Rust consumer did not exist yet, so TS↔Rust ticket parity was
   unpinned.)
2. `HeartbeatResponse.relay_ticket` is `String` — if the Windows Product seat
   prefers `Option<String>` for the node cache, that is a one-line serde
   change on their side of the wiring (no wire change; both forms tolerate
   absence).
3. Wire-compat CI job extension (§6.2(7)) is Test and Release's row — my
   Rust-side absence-tolerance unit test is the local stand-in.
4. Deploy-order note: production `modelswarm.deepflux.space` does NOT mint
   tickets until the owner deploys this build (tracker-first order is safe:
   old Rust clients ignore/absorb the additive field — pinned by the new
   lib.rs test; the node's ticket cache lands with the Windows Product seat).
5. The observed (non-reproducing) workspace test flake in §2.

## 6. Follow-up addendum (2026-10-15, later) — the pending cross-tie is CLOSED

The Protocol seat landed the vectors at `af1dc01`
(`protocol/vectors/relay-ticket-1.json`, `relay-ticket-2-expired.json`,
`relay-ticket-forge-1.json`, `relay-admission-frame-1.json`, plus
`crates/modelswarm-eligibility/src/relay_ticket.rs` + its vector-consumer
tests). The recorded follow-up is done: `apps/tracker/tests/relay-ticket.test.ts`
now consumes the FROZEN vector files directly (the same
`crypto-vectors.test.ts` vectorsDir discipline) — the inline fixture is
deleted. The suite grew 11 → 26 tests:

- **relay-ticket-1 (accept)**: fixture seed derives the recorded public key;
  payload is byte-exact against `canonical_json` AND `canonicalJson(fields)`
  AND `canonical_json_sha256`; signature verifies against the fixture key
  (flip + wrong-key fail); frozen shape (exact field set, exactly 600 s delta,
  32-hex nonce, ≤ 1024 B); admission-deadline arithmetic
  (`expires_at + 120 s == expected_admission_deadline`) and liveness at
  `verify_at`; the tracker issuer reproduces the vector's deterministic fields
  (nonce excepted) under the vector's own seed; and the full wire form
  reconstructs byte-exactly from `canonicalJson` + the base64url Ed25519
  primitives — **TS↔Rust ticket byte-parity is now pinned against the same
  files the eligibility crate consumes** (risk §5.1 resolved).
- **relay-ticket-2-expired**: same token, signature STILL verifies; the TS
  skew-rule computation agrees it is expired at its `verify_at` (expiry, not
  signature, is the failure).
- **relay-ticket-forge-1 (tracker-side mirror)**: the corpus's 12 rows carry
  the frozen status/reason vocabulary; the syntax/signature rows are
  re-derived in code exactly as the Rust forge test derives them (bad
  signature flip; wrong-key re-sign; unknown field; missing nonce;
  non-canonical key order; 601 s delta validly signed — signature ok, delta
  wrong; uppercase/31-char nonce; plain-date timestamp; single segment;
  > 1024 B wire). The two relay-context rows (peer_mismatch /
  installation_mismatch) are mirrored only as "the mutated tickets are
  internally valid" plus the signature-precedes-binding order note — the
  binding checks belong to the relay, and the tracker deliberately does not
  reimplement them. `relay-admission-frame-1.json` is the transport seat's
  frame vector and is intentionally NOT consumed here.
- The issuance/route/failure-path tests are unchanged (rig-keyed, not
  fixture-keyed). The strict `from_wire` parser remains the Rust consumer's;
  the test-local `ticketFormError` mirror is test-only, so no new production
  surface was invented in the tracker.

Commands/outcomes for the follow-up: `npx vitest run
tests/relay-ticket.test.ts` 26/26; `npm run test` 162 passed / 2 skipped / 0
failed; `npm run typecheck` clean; `npm run build` compiled successfully;
`cargo test -p modelswarm-tracker-api` + `cargo fmt --check` green (crate
untouched this round; sanity after pulling `af1dc01`).

Remaining §6.4 rows after this: Network 6–7 (relay admission gate +
pre-flight), Windows Product 8 (node ticket cache), Test and Release 10
(wire-compat job — should consume the same vector files), Security §6.1.

## 7. Suggested next task for the Integrator

Route to Protocol Architect: land `crates/modelswarm-eligibility/src/relay_ticket.rs`
+ `protocol/vectors/relay-ticket-*.json`, then have me re-point the inline TS
fixture at the vector file (one small follow-up commit) so the TS↔Rust
byte-parity gate closes before Network's relay admission half ships. After
that: Network Engineer's §6.4 rows 6–7 (admission behaviour + pre-flight),
then Windows Product's row 8 (node ticket cache), then Test and Release's
row 10 (wire-compat job) — the §6.2(7) lockstep test should consume both my
`relayTicket` heartbeat field and the `relay-ticket-1.json` vector.
