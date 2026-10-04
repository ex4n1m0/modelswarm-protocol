# Phase 0 — Review Reconciliation

Integrator's disposition of the two independent reviews
(`phase-0-architect-review.md`, `phase-0-security-review.md`; both returned
APPROVE-WITH-CHANGES). Applied 2026-10-04, before the protocol freeze and the
initial commit.

## Blocking items — all resolved

| # | Finding (source) | Resolution |
|---|---|---|
| A1 (Arch) | Admin catalog endpoints missing | Added §3.5 to `protocol/msp-v1.md`: `POST /admin/catalog/candidates` + `POST /admin/catalog/promote`, admin role + `X-MSP-Admin` carrier, `forbidden` code |
| A2 (Arch) | Envelope/session transport undefined | §2.3 now defines `Authorization: MSP1 <base64url(...)>` + `X-MSP-Session`, signs `method`+`path` (also fixes Security-N1), defines GET bodyDigest (empty-string hash), `{session}` shape and 24 h lifetime |
| A3 (Arch) | Token serialization undefined | §5 defines `base64url(json).base64url(sig)` wire form |
| A4 (Arch) | Challenge lifecycle undefined | Added `POST /peers/challenge/start` (issues `challengeId`, prompt, `deadlineAt`); `timings` fields frozen as `{firstTokenMs, totalMs}` |
| A5 (Arch) | `/catalog/{id}` 200 body undefined | Defined as a single-profile CatalogEnvelope |
| A6 (Arch) | Verification report + commit missing | This file set + `docs/verification/phase-0.md` + initial commit (closing step of Phase 0) |
| S1 (Sec) | Token TTL 30 min ≫ lease TTL | **Accepted as the most important finding of the gate.** Token expiry now capped at lease expiry + 60 s grace in §5 and ADR-006; worst-case post-hosting-stops consumption ≤ 150 s |
| S2 (Sec) | Token nonce semantics contradictory | Tokens are multi-use within lifetime; `nonce` = issuance-dedup/revocation handle; per-request replay protection is single-use `requestId` (§5, §6.6, ADR-006) |
| S3 (Sec) | Revocation had no propagation mechanism | Heartbeat response `notices` payload frozen: `revoked_peers`, `revoked_tokens`, `catalog_update` (§3.3) |
| S4 (Sec) | P2P replay/skew policy unspecified | New §6.6: handshake ts window ±120 s, `requestId` single-use memory window, token-expiry skew ±120 s, per-peer nonce state surviving reconnects |
| S5 (Sec) | No unauthenticated abuse tests | Phase-1 tests F5 (per-IP health/catalog limit), F6 (device/start flood, no rows on reject), F7 (cheap garbage rejection) added; per-IP unsigned limits added to §3.4 |
| S6 (Sec) | Lease ownership unbound | §3.3: `leaseId` unguessable, installation-bound; cross-lease use → `401`; test D5 added |

## Nonblocking items — dispositions

| # | Finding | Disposition |
|---|---|---|
| A-N1 / A-N5 | Error codes, notices shape, register validation, HTTP mapping | Fixed: codes `pending`/`forbidden` added, status mapping + validation precedence chain added (§3.4), register validation rules added (§3.3) |
| A-N2 | Rendezvous gaps | Fixed: `GET /rendezvous/pending` in table, delete-on-read semantics, opaque ≤ 4 KiB payloads (§3.3) |
| A-N3 / S-N7 | ms-core charset laxer than schema; health-route drift | Fixed: `is_slug` validation + new tests in `crates/ms-core/src/lib.rs`; §3.1 health body amended to include `service` (matches route) |
| A-N4 | JobReceipt missing `peerIds` | Fixed: `peer_ids` field added to `messages.proto` |
| A-N6 | D4 nondeterministic | Fixed: draining peers are excluded from lookup, full stop |
| A-N7 / S-N4 | Schema + A2 hardening | Fixed: `"type":"string"` on `status`; A2 bans unbounded text and token-like column names |
| S-N1 | Envelope method+path binding | Fixed (folded into §2.3, see A2) |
| S-N2 | PeerId/Noise key chain | Fixed in §6.6: handshake `peerId` MUST equal Noise-authenticated PeerId; hub attestation enters only via token binding (no extra key directory needed) |
| S-N3 | Receipt two-phase inconsistency | Fixed: §7 + `ReceiptAck` message in `messages.proto` — server signs in `completed`, requester countersigns via `receipt_ack` |
| S-N5 | Clamps + precedence | Fixed: §3.4 precedence chain; §6.4 clamp table for `deadlineMs`, `maxTokens`, sampling |
| S-N6 | Redaction gates too late | Fixed: `docs/acceptance/README.md` mandates redaction assertions in phases 3 and 4 (when prompts first flow), not just phase 7 |
| Notes | threat-model §6 overclaim; version parity; tsbuildinfo; stray dir | All fixed: §6 rewritten with per-phase homes; parity test required in `hub/tests/README.md` + Phase 1; `*.tsbuildinfo` ignored; `protocol/catalog-temp/` removed |

Deferred deliberately: handshake canonicalization over protobuf fields —
bundled into the Phase 3 opening ADR (wire encoding decision), per the
Architect note; recorded here as the single open contract question entering
Phase 1 planning.

## Outcome

Both reviewers' blocking lists are fully resolved in-repo; no finding was
rejected. Protocol §1–§7 is now considered **frozen** for Phase 1.
