# Phase 0 — Independent QA/Security Review

Reviewer: QA/Security subagent (independent pass, read-only)
Date: 2026-10-04 · Verdict: **APPROVE-WITH-CHANGES**
Dispositions of each item are recorded in `phase-0-reconciliation.md`.

> Scope note: this is the verbatim reviewer report; headings below are the
> reviewer's own.

## Verdict rationale

All file references relative to the repository root.

## BLOCKING

1. **Token TTL vs lease TTL defeats the core rule against the exact casual
   adversary it targets.** Lease TTL is 60–90 s (`protocol/msp-v1.md` §3.3)
   but capability token TTL was ≤ 30 min (§5; ADR-006). A free-rider can
   register, pass one challenge, stop heartbeating (hosting gone within 90 s),
   and keep consuming for up to 30 minutes — no binary patching required,
   below the bar the plan sets for itself ("enforceable against casual
   misuse"; Phase 5 expects a "short grace window"). Fix before freeze: cap
   `expiresAt` at `leaseExpiresAt + grace` (grace ~60–90 s), matching the
   planned "1 challenge per lease renewal" cadence (ADR-006).
2. **Capability-token nonce semantics contradictory; token↔request binding
   undefined.** §5 said "serving peers verify nonce uniqueness per peer"
   (implying single-use tokens → max 1 request per lease renewal), while §6.2
   has every request carry a token and ADR-006 bounds issuance to one per
   lease renewal. Either tokens are single-use (throughput broken) or
   multi-use (then what does nonce uniqueness mean, and what stops intra-peer
   replay of a captured token string?). Also nothing binds a token to a
   `requestId` or handshake nonce. Decide single-use vs multi-use, define the
   nonce scope, and state the binding — frozen at Phase 1 start.
3. **Revocation propagation has no mechanism.** ADR-004/ADR-006 and
   `docs/threat-model.md` §4.1 require serving peers to "consult revocation
   state at heartbeat cadence", but `protocol/msp-v1.md` §3 defined no
   endpoint or payload returning revocation/blocked state: heartbeat
   `notices:[…]` content undefined, `GET /peers` response had no revocation
   flag. The `revoked`/`revoked_token` error codes existed with nothing that
   can produce them. Define the endpoint (or the exact `notices` payload)
   before freeze.
4. **P2P replay window, nonce scope, and clock-skew policy unspecified.** The
   hub envelope gets ±120 s, but the P2P handshake had `ts` + `nonce` with no
   acceptance window, no statement of whether `requestId`/nonce single-use
   state persists across streams/reconnections, and no clock-skew tolerance
   for token expiry checks at serving peers. `replayed_request` existed with
   no defined detection rule — untestable deterministically as drafted.
5. **Phase 1 acceptance missing the unauthenticated abuse tests.** F1 covered
   only installation-keyed limits. Not covered: 120 req/min per-IP on
   `/health`+`/catalog`, 10 req/min enrollment (keyed on what?
   `/auth/device/start` is unauthenticated and creates rows: per-IP keying
   plus a device-start flood test required), and unsigned-garbage spam to peer
   endpoints rejected cheaply and per-IP rate-limited (Vercel function/cost
   DoS by a casual adversary). The hub is never again the focus — these must
   exist at Phase 1.
6. **Lease ownership binding untested and unspecified.** Nothing stated
   `leaseId` is bound to the signing installation or that it is unguessable.
   A sequential `leaseId` would let any enrolled adversary heartbeat, drain,
   or challenge-complete against another installation's lease. Add one spec
   sentence (lease is installation-bound; ids unguessable) and one D-test
   (installation B referencing A's lease → 401/403).

## NONBLOCKING

1. Signed envelope did not bind HTTP method+path → cross-endpoint envelope
   replay possible; cheap to add pre-freeze.
2. `Handshake.peer_id` should be required to equal the Noise-authenticated
   remote PeerId, with the key chain spelled out; also no defined source for
   hub-keyed peer verification from `GET /peers`.
3. Job-receipt flow inconsistent: server emitted a receipt already containing
   `requester_signature`, but no message ever carried the requester's
   signature to the server.
4. A2 (prompt-privacy structural test) was name-based only — a column named
   `notes` of unbounded `text` could hold prompts; also extend forbidden names
   with token-like names (`hf_token`, `access_token`) to structurally back the
   "HF token never reaches the hub" promise (`docs/privacy.md` §3 was
   prose-only).
5. Validation/error precedence and GET-envelope edge (bodyDigest of empty
   body) undefined → C2–C5 potentially flaky; requester-supplied
   `deadlineMs`/sampling ranges need clamp rules (§6.2/§6.4).
6. Node-log prompt redaction had no gate before the Phase 7 audit; add
   redaction assertions to the Phase 3/4 acceptance sets when prompts first
   flow through Rust code.
7. Drift already visible: health route returned a `service` field not in the
   §3.1 draft contract — tighten or amend the spec (risk R10 in miniature).

## NOTES

- Threat-model adversary set (A–G) covers the build plan's key risks well,
  and every acceptance-matrix row had a mitigation path except revocation
  propagation (blocking 3).
- Hub-side prompt privacy is the design's strongest part: enforced
  structurally (A2 + size caps + schema absence of prompt fields), not prose.
- `docs/threat-model.md` §6 claimed its abuse tests "map 1:1" to phase-1.md —
  overstated (P2P/token/metric abuses live in phases 3/5/7); correct the
  claim or stub those acceptance homes.
- The ts-window-subsumes-nonce-cache replay design is sound (no cache-lifetime
  race).
- `ms-core` validation matched ADR-005 and the catalog schema; the version
  constant triplicated across hub/Rust/protocol needs a parity check to stay
  honest.
