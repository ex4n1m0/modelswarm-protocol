# Phase 0 — Independent Architect Review

Reviewer: Architect subagent (independent pass, read-only)
Date: 2026-10-04 · Verdict: **APPROVE-WITH-CHANGES**
Dispositions of each item are recorded in `phase-0-reconciliation.md`.

> Scope note: this is the verbatim reviewer report; headings below are the
> reviewer's own.

## Verdict rationale

The architecture is internally coherent and phase discipline is clean, but
the frozen hub contract (msp-v1.md §3–§5) has spec gaps that would force a
Phase 1 Tracker implementer to guess on required deliverables. These are
documentation fixes, best made now while §3–§5 are still pre-freeze.

## BLOCKING

1. **Admin catalog endpoints are missing from the frozen API.**
   `protocol/msp-v1.md` §3 defines no candidate/promote endpoint, yet Phase 1
   must build "an admin-only catalog candidate/promote flow"
   (`docs/build-plan.md` Phase 1) and `docs/acceptance/phase-1.md` G1/G2 test
   promotion (and imply an endpoint that returns 403 to non-admins).
   `AGENTS.md` forbids inventing fields not in msp-v1.md, so the Tracker is
   stuck. Add the admin endpoints (paths, bodies, admin-auth model, error
   codes) to §3 or a new ADR before freeze.
2. **Signed-envelope and session transport is undefined.** §2.3 defines the
   envelope object but not how it attaches to an HTTP request (header vs body
   wrapper), how GET calls are signed (`GET /peers` sits in the "all require
   signed envelope" table — bodyDigest of what?), and neither the `{session}`
   shape returned by `/auth/device/complete`, its carrier (header/cookie), nor
   its lifetime are specified. Tests C2–C5, D5 cannot be implemented without
   guessing. `protocol/msp-v1.md` §2.3, §3.2–§3.3.
3. **Capability-token serialization is undefined.** §5 lists the fields and
   says "detached signature over canonical token JSON" but never defines the
   wire/serialized form (how the signature is attached, string encoding).
   Phase 1 E1 must emit a concretely verifiable token, and
   `protocol/messages.proto` carries it as a plain `string`.
4. **Hosting-challenge lifecycle is undefined.** There is no
   challenge-issuance endpoint (nothing says where `challengeId` comes from),
   `timings:{…}` was a literal placeholder, and challenge
   deadline/verification semantics are absent — yet Phase 1 must design the
   `hosting_challenges` migration and E2 depends on a row with outcome
   `passed`. Define issuance + timings fields, or explicitly scope the Phase 1
   table as a stub with the already-frozen columns.
5. **`GET /catalog/{profileId}` 200 body is undefined** ("One signed profile
   entry" — envelope-with-one-profile? profile + detached signature?). Phase 1
   must ship signed catalog endpoints; only the 404 case is specified.
6. **Phase 0 gate artifact missing.** `README.md` and `AGENTS.md` ("Phase
   gate") reference `docs/verification/phase-0.md`; `docs/verification/` was
   empty and the repo had zero git commits at review time. (Expected — this is
   the integrator's closing step.)

## NONBLOCKING

1. Error-code registry gaps: `pending` used but unlisted; HTTP status mapping
   unspecified for several codes; admin-403 had no code.
2. `GET /rendezvous/pending` appears only in a note, not in the §3.3 endpoint
   table; `offer`/`answer` payload formats and response shapes undefined.
3. `ms-core` `ModelProfileId` validation laxer than `catalog/schema.json`
   (charset not enforced; `msp:Qwen3:…` would pass Rust, fail schema).
4. `JobReceipt` in `protocol/messages.proto` omits the `peerIds` field that
   `protocol/msp-v1.md` §7 includes in the signed receipt object.
5. Heartbeat response `notices` item shape undefined; register-body validation
   rules unspecified (peerId↔installationId binding, multiaddr validation,
   unknown-profile error code, `pubKey` encoding).
6. `docs/acceptance/phase-1.md` D4 ("exclude **or flag**") nondeterministic as
   an acceptance test.
7. Scaffold health route returns `{status, service, protocol}` while the draft
   contract said `{status, protocol}` only — spec/route mismatch (risk R10 in
   miniature).
8. `catalog/schema.json` `status` uses a bare `{"enum":[…]}` without
   `"type":"string"`.

## NOTES

- Internal consistency is strong across build-plan, architecture.md,
  ADR-001–008, msp-v1.md, risk-register, and phase-1.md (timings, replay
  window, rate limits, catalog lifecycle, retry semantics all agree; ADR
  cross-references resolve).
- Phase discipline is clean: all nine crates are dependency-free interface
  freezes; hub contains only a commented static health route; CI matches the
  plan; nothing anticipates later phases in a way requiring destructive
  rework.
- Layout deviations from the master plan are documented and acceptable
  (threat-model in `docs/`, `docs/operations.md` and `tests/*` deferred to
  their phases). Stray empty `protocol/catalog-temp/` flagged for removal.
- `hub/tsconfig.tsbuildinfo` was not covered by `.gitignore`.
- Handshake signature canonicalization over protobuf fields is undefined; fold
  into the Phase 3 opening ADR alongside wire encoding.
