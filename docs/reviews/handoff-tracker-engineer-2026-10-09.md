# Tracker Engineer Handoff — Master-Prompt Domain Audit (2026-10-09)

- Branch: `audit/master-prompt-2026-10-09` (baseline `730cebf`, v0.2.20)
- Scope: `apps/tracker/` + `crates/modelswarm-tracker-api/` — static read-only
  audit (Read/Grep/git only; no npm/cargo/vercel/DB). This file is the only
  artifact created. Fresh suite evidence reused from
  `target/audit-logs/tracker-*.log` (typecheck, vitest 110 passed / 2 skipped,
  next build, golden vectors — ALL PASS, recorded in
  `docs/reviews/audit-freeze-2026-10-09.md`).
- Overlap note: `docs/reviews/audit-security-findings.md` owns the threat
  ranking. This handoff is the API/storage/protocol-shape view; where findings
  coincide (challenge self-attestation, `/catalog/requests` body cap) I cite
  their IDs and add the control-plane design work.

## 1. Changed files

| File | Why |
|---|---|
| `docs/reviews/handoff-tracker-engineer-2026-10-09.md` | This audit (only artifact; nothing else modified, no commits) |

## 2. Findings (severity-ranked, file:line, time-boxed fixes)

### T1 — HIGH — Hosting challenge is possession-blind at the control plane (earn chain passes on two self-attested integers)

- Mirrors security HIGH-1. Tracker-side evidence:
  - `apps/tracker/app/api/v1/peers/challenge/complete/route.ts:37-47` —
    `passChallenge` persists client-supplied `firstTokenMs`/`totalMs`; no
    sanity check at all (not even `firstTokenMs <= totalMs`).
  - `apps/tracker/lib/eligibility.ts:41-46` — `capacityFromTimings(totalMs)`
    maps the self-reported integer to a `verified_capacity` class; a claimed
    `totalMs <= 2000` labels the peer `gpu_high` ("verified" is a misnomer).
  - `apps/tracker/lib/constants.ts:26` — one fixed public prompt for every
    profile, so a single precomputed pair of timings (or any machine's) works
    forever, across installations.
  - `apps/tracker/DESIGN.md` already promised "randomized **chunk challenges**
    (ADR-011 possession)" for this flow — designed, never implemented.
  - The wire harness itself says it: `crates/modelswarm-node/tests/live_tracker.rs:117-119`
    "Timings are synthesized here: the tracker cannot observe the requesting
    peer's engine."
- Design options (control-plane view) in §4; recommendation: A+B+C.
- Time-box: (a) 30 min honest relabeling; (b) 2 h nonce-bound prompt;
  (c) 1–2 d possession digest + ADR. See §4.

### T2 — HIGH — ADR-012 suspension machinery is ~1/3 implemented; the reputation chassis cannot ride it yet

Precise inventory (this is what the gap study needs):

| ADR-012 element | Status | Evidence |
|---|---|---|
| `audit_epoch` counter | EXISTS, admin-only | table `peer_audit_state` (`migrations/0001_init.sql:213-217`); bump via `POST /api/v1/audit` (`app/api/v1/audit/route.ts:24`, X-MSP-Admin); enforced only at lease *refresh*: `app/api/v1/peers/lease/route.ts:46-49` (stale epoch → 403 ineligible) |
| Already-issued token invalidation | MISSING | no route calls `store.revokeToken` (grep: zero callers outside store/tests); `revoked_tokens` notices can therefore never be produced by any API action; the Rust serving gate (`LeasePolicy`) has no epoch oracle. Effective worst-case enforcement latency = natural token TTL ≤ lease 75 s + 60 s grace ≈ 135 s from last heartbeat — that TTL bound is the *de facto* suspension |
| Blocked peers | HALF | `blocked_peers` table + store methods (`lib/store.ts:779-790, 1600-1617`), checked at register (`app/api/v1/peers/register/route.ts:35`) and lease issuance (`lease/route.ts:34`) — but **no route can set it** (direct DB write only) |
| Slots-to-zero suspension | STRUCTURALLY IMPOSSIBLE | DDL `CHECK (max_slots BETWEEN 1 AND 8)` (`0001_init.sql:73`); `updateLease`'s patch type excludes `maxSlots` (`lib/store.ts:220-236`); no challenge-refusal/timeout counter, no deterministic policy table, no Phase B test vectors (all promised by ADR-012) |
| "Invalid output" degradation | MISSING | `job_receipts.outcome` recorded (`app/api/v1/receipt/route.ts`) but read by no policy; `recordObservation`/`peer_observations` also write-only |
| Suspension reasons | AD HOC | `blocked_peers.reason` free-text varchar(120), default `'admin'`; no enum, no admin visibility route for epochs/blocks |
| Automatic sweep trigger | MISSING | `DESIGN.md` says "scheduled Vercel cron — never a resident process"; none exists (manual POST only) |

- Gaps for the chassis: no per-(peer, profile) counters, no reason taxonomy,
  no read path for audit state, no admin block route, no token revocation
  propagation. Enforcement-latency budget (≤135 s) should be stated in the
  threat model as the current bound.
- Time-box: 4 h — admin `POST /admin/peers/block {peerId, reason}` +
  `POST /admin/tokens/revoke {nonce}` (both already have store methods and
  notice wiring); migration 0004 decision (allow `max_slots = 0` vs
  `suspended_until`) needs an ADR before the chassis lands.

### T3 — MEDIUM-HIGH — M3 signed-catalog enforcement gaps

See the gap table in §5. Headlines: machine-readable `licenseId` was dropped
by schema-v2 (license survives only as an optional *unsigned* provenance URL);
`runtime.name` const is enforced by CI vectors, not at publication; 14 of 15
on-disk candidate profiles are never machine-validated before the owner
promotes them.

### T4 — MEDIUM — API hygiene: two unthrottled/uncapped public paths

- `app/api/v1/catalog/requests/route.ts:21` reads the body with
  `await req.json()` — bypasses the shared 64 KiB cap
  (`lib/envelope.ts:217-230`). The 10/min/IP limit IS applied first (line 18),
  so this is parse-cost amplification bounded only by Vercel's ~4.5 MB
  platform cap. Confirms security M-5. Fix: 30 min — route through
  `guardPublicBody`/`readBody`.
- `app/api/download/[file]/route.ts:30` — `bumpDownloadCount` per GET with
  **no rate limit**: unthrottled DB writes at network speed and trivially
  inflatable public download counts rendered on the landing page.
  Fix: 15 min — `public:` limiter class.
- `lib/envelope.ts:122-135` — a *well-formed* MSP1 envelope naming an
  unenrolled `installationId` gets a `getInstallation()` DB read per request
  and never touches any limiter bucket (the per-IP "unsigned" bucket at
  line 125 only counts unparseable envelopes). msp-v1 §3.4's "60 req/min/IP on
  unsigned calls to peer endpoints" is implemented as "unparseable-envelope
  calls". Fix: 1 h — count `unknown_installation` failures against the same
  per-IP bucket.
- Rate limiting is per-warm-lambda-instance memory
  (`lib/ratelimit.ts:30-72`, `lib/context.ts:72`); the durable
  `rate_counters` table + `bumpRateCounter` (0001:165-170) are **dead code** —
  no route ever calls them (grep: zero callers). Document the per-instance
  semantics or back the limiter with the existing table.

### T5 — MEDIUM — Write-only tables and storage bloat

Ranked by ops risk (production = Neon Postgres; no TTL jobs exist anywhere by
design — ADR-001 row-aging only):

1. `peer_observations` — one row **per heartbeat** (~every 15–30 s per peer),
   read by nothing, pruned by nothing (`lib/store.ts:1104-1116`; MemoryStore
   caps itself at 10k, PgStore does not). Highest growth rate, zero value.
2. `sessions` — never deleted; and `countEnrolledInstallations()` counts
   DISTINCT installations **ever** (`lib/store.ts:964-969`). Consequence:
   `DEVICE_APPROVAL_CAP=250` gates *cumulative* enrollments, not live ones —
   once 250 distinct devices have ever completed enrollment, auto-approve is
   permanently off until the owner raises the cap or approves manually.
   This monotonicity is the surprising cap/flag interplay to document.
3. `device_codes` (one per `/auth/device/start`, incl. never-approved),
   `installations` (upserted at start, `start/route.ts:30-34`) — logical
   expiry only, rows live forever.
4. `capability_tokens` — one row per issuance; index on `expires_at` exists
   but nothing prunes.
5. `session_authorizations` — write-only (`lib/store.ts:1619-1632`), never
   queried, never pruned.
6. `peer_notices` — `{all:true}` pushes insert one row per lease including
   dead leases that will never drain.

Correctly bounded (keep): rendezvous mailboxes (24 h TTL + 64/mailbox with
write-time pruning, `lib/store.ts:25-26, 743-756, 1524-1548` — commit
`3ef0331`), nonces (reader-side eviction, durable in Pg), `model_requests`
(sha256 dedupe makes the queue un-spammable per artifact).

Fix: 2–4 h — adopt the rendezvous write-time-prune pattern for
observations/sessions/device_codes/tokens, or stop writing observations until
a reader exists.

### T6 — LOW — Spec/implementation shape drift (schema discipline)

Mismatch table (zod vs `protocol/msp-v1.md` vs Rust `modelswarm-tracker-api`):

| Item | zod/tracker | msp-v1 | Rust client | Verdict |
|---|---|---|---|---|
| Token issuance endpoint | `POST /peers/lease` issues; challenge/complete returns `{passed,…}` | §3.3 table says challenge/complete returns the token; §5 (line 225) says `/peers/lease` | `request_lease` → `{token, lease}` | msp-v1 §3.3 internally contradicts §5; impl follows §5 — fix the §3.3 row |
| `capacityClass` in `GET /peers` | returned (`app/api/v1/peers/route.ts:43`) | absent from the §3.3 table (line 119) | optional field with default | additive, undocumented in §3.3 |
| `POST /catalog/requests` | strict pointer-only schema (`lib/schemas.ts:209-222`) | §3.1 matches (ADR-023) | desktop posts it via raw reqwest (`crates/modelswarm-desktop/src/app.rs:2108`), not tracker-api | OK; desktop bypasses the typed client (acceptable — unsigned public route) |
| Challenge timings bounds | `0..3_600_000` (`schemas.ts:88-91`) | unspecified | u64 | impl stricter, fine |
| Rendezvous, session-authorize, receipt, audit, admin device approve | shipped + TS-tested (DESIGN.md) | absent from §3 tables (Phase B additions) | no client methods for rendezvous/session-authorize/receipt (`crates/modelswarm-tracker-api/src/lib.rs:10-11`) | doc drift; client surface deliberately subset |
| Error code set | adds `unknown_request` (admin resolve) | §3.4 list lacks it ("additions require ADR") | tolerant | ADR-023 introduced the route; list the code |
| Manifest validation reference | zod mirrors schema-v2 | §3.5 says candidates validate against `catalog/schema.json` | schema-v2 types | §3.5 points at the v1 schema — stale |
| DDL privacy test scope | `tests/schema-ddl.test.ts:13` parses **only** 0001 | — | — | the "no unbounded text" invariant is unasserted over 0002/0003, which DO use `TEXT` (`0002:5`, `0003:8-17`); bounds hold only at the zod layer |
| `package.json` version `0.1.0` | unused | — | — | confirmed nothing reads it (site versions hardcoded in `app/page.tsx:11-17`; package is `private`). Cosmetic |
| Lease `slots` field | `slots: lease.maxSlots` (`lib/eligibility.ts:73`) — a static cap, not current `freeSlots` | "slots" per §5 | gate checks `slots >= 1` | consistent with ADR-026 but worth one doc line: availability is heartbeat-side |

### T7 — LOW/INFO — Assorted

- CI coupling: `tracker.yml` runs `cargo test -- --ignored`, which also runs
  `production_catalog_verifies_over_https` (`live_tracker.rs:10-24`) — that
  test has **no `MSP_LIVE` gate** and hits live `modelswarm.deepflux.space`
  (read-only GETs). A production outage fails CI. Acceptable, but the gate
  should match the signed harness's `MSP_LIVE` pattern.
- Admin actions (promote, epoch bump, resolve) keep no audit trail — no
  `logEvent`, `provenance.reviewedBy` is caller-supplied. Single shared
  `ADMIN_TOKEN`, constant-time compared, no rotation story. Honest posture
  for an owner-only tool; record as accepted risk.
- Production enrollment posture (stated for the record): `DEVICE_AUTO_APPROVE=1`
  under cap 250 means the first 250 distinct installations are internet-open
  self-enrollment; combined with T1 that is the current free-rider envelope.
  The `/verify` page itself is sound — it requires the owner token, knowing
  the pairing code alone is never enough (`app/verify/page.tsx:34-37`,
  `msp-v1 §3.2`).
- `guardPeerRequest` precedence = size → envelope(auth/session/sig/ts/nonce/
  digest/per-install rate) → schema — matches the frozen §3.4 order
  (`size cap → auth/session → timestamp → nonce → schema/rate-limit`); rate
  before schema is within the spec's combined final stage.

## 3. Content-blindness verification (scope item 1) — PASS, with evidence

- Every request-body schema in `lib/schemas.ts` ends `.strict()` — unknown
  keys (incl. `messages`/`prompt`/`input`) are structurally rejected; abuse
  suite asserts it incl. session-authorize/receipt
  (`tests/abuse.test.ts:137-166`).
- H2 source ban enforced by test: no `llama|gguf` in
  lib/app/scripts/migrations (`tests/meta.test.ts:33-41`); no `.gguf/.bin`
  artifacts (`:43-46`); the deliberate `runtime.name` pattern deviation is
  documented at `lib/schemas.ts:6-11` and compensated by the CI
  golden-vector gate against `catalog/schema-v2.json` (`scripts/validate-vectors.mjs`).
- Error paths never log bodies or field values — issue counts only
  (`lib/envelope.ts:243-250`, `lib/errors.ts:80-86`).
- Migration 0003 (`migrations/0003_model_requests.sql:6-21`) persists exactly:
  artifact sha256/repo/revision/path/bytes, quant hints, bounded labels
  (`display_name` ≤ 80, `note` ≤ 280), requesting IP, timestamps, resolution.
  The only user free-text in the entire tracker is those two labels — the
  accepted ADR-023 trade-off.
- Challenge storage holds ids/deadline/outcome/two integers; the prompt is a
  code constant, not a column (`lib/constants.ts:26`, `lib/store.ts:1635-1637`).
- Receipts: digest + outcome only. Store interface comment restates the
  invariant (`lib/store.ts:15-17`).

## 4. Hosting-challenge honest-verification options (scope item 3)

All options keep the hub content-blind (hash/nonce arithmetic only; the hub
never sees prompts or completions):

- **A. Honest relabeling (0 h, do now).** Rename/demote `verified_capacity`
  semantics to `reported_capacity` in docs (field rename needs a vector bump —
  docs-only first), state in ADR-026/threat-model that timings are
  self-attested telemetry, and add `firstTokenMs <= totalMs` + nonzero
  plausibility checks (30 min).
- **B. Nonce-bound prompt (2 h).** Make the challenge prompt
  `"ModelSwarm readiness challenge <challengeId>"` (challengeId is already a
  fresh 128-bit id per lease+profile). Kills cross-install replay and
  precomputation of a single solved pair; timings remain self-attested.
  Zero new cryptography, zero client-protocol change beyond the constant.
- **C. Possession proof via a pinned greedy-canary digest (1–2 d + ADR —
  recommended).** The resolver pipeline (which already downloads and hashes
  the artifact with the pinned runtime) additionally computes, offline, the
  greedy completion of the fixed canary prompt and pins
  `sha256(canonical token stream)` per profile (manifest-adjacent field or
  signed provenance extension; needs an ADR because it extends the
  catalog identity surface). `challenge_complete` then carries only
  `output_digest = sha256(challengeId || tokens)`; the tracker compares.
  Content-blind (hash only). Enabling evidence already in-repo: greedy
  determinism across GPU vendors and thread counts (E0). Failure modes to
  document: chat-template application point (ADR-025) and sampler settings
  must match the resolver's; a revision bump creates a new profile and a new
  digest naturally (ADR-005). Timings stay as telemetry for capacity
  *labeling* only.
- **D. Peer-run spot-check audits (defer to the reputation chassis).** The
  tracker asks other currently-passing peers to reproduce a canary and
  compares; this is Sarmenta-style spot-checking and belongs with the
  granted-work accounting work, not v0.2.x.
- **E. Hub-side timing probes — REJECT.** The hub dialing a peer's inference
  endpoint violates ADR-001 (control plane never carries inference traffic).

Recommendation one-liner: A immediately, B+C as the v0.2.x earn-chain
hardening; D deferred to the chassis study.

## 5. M3 gap table (scope item 6)

| # | Gap | Evidence | Can an unapproved/tampered profile reach clients? | Fix (time-box) |
|---|---|---|---|---|
| 1 | `licenseId` dropped by schema-v2; tracker zod `.strict()` actively *rejects* it; license survives only as optional unsigned `provenance.licenseEvidenceUrl` | `catalog/schema.json:13,53` (v1 requires it) vs `catalog/schema-v2.json` (absent) vs `lib/schemas.ts:142-180,192` | No (unrelated to serving), but M3's "license metadata" exit criterion is unmet machine-readably | ADR: restore a signed license field in a manifest revision, or record the provenance-only decision (2–4 h) |
| 2 | `runtime.name` const `"llama.cpp"` not enforced at publication (H2 deviation) | `lib/schemas.ts:169-170` pattern; const lives in schema-v2 + CI vectors only | A wrong-named manifest would be signed and served (client types would reject) | Validate the const at promote time (resolver output already carries it) or document client-side enforcement (1 h) |
| 3 | 14 of 15 on-disk candidate profiles never machine-validated pre-promote | `scripts/validate-vectors.mjs` covers only `protocol/vectors/manifest-*.json`; wire-compat seeds exactly one (`tracker.yml` "Seed one active profile") | No — but a schema-drifting candidate fails only at owner promote time | Extend `validate:vectors` to `catalog/candidate-profiles/*.json` (1 h) |
| 4 | Catalog signed per-request with fresh `generatedAt` — no stable per-version artifact to pin/cache | `app/api/v1/catalog/route.ts:27-33` | No (every signature is valid over the served payload) | Acceptable; document, or sign+store at promote time (needs design) |
| 5 | Promote ↔ model-request resolution decoupled | `admin/catalog/promote` vs `admin/catalog/requests/resolve` — manual bookkeeping only | No | Auto-resolve matching requests at promote (1 h) |
| 6 | Approval gating, profile-id derivation, immutability | `GET /catalog` filters `status='active'` (`lib/catalog` route :21, `[profileId]` :23); register/challenge/lease/session-authorize all re-check active; `profileId` derived server-side at insert (`admin/catalog/candidates/route.ts:24`); re-publish conflict → 400 (ADR-005, `lib/store.ts:654-659,1353-1358`); Rust client re-derives + verifies signature (`modelswarm-tracker-api/src/lib.rs:486-534`) | **HOLDS**: unapproved cannot reach clients; tampered cannot, while the hub key and client re-derivation hold | none (recorded as verified) |
| 7 | Key hygiene: one `MSP_HUB_SEED` signs catalogs and leases; no key id / rotation window | `lib/context.ts:36-56` | Hub-key compromise = both surfaces | Note for M5 update-channel design |

## 6. Wire-compat coverage verdict (scope item 2)

The tracker.yml `wire-compat` job (real `next start` + real Postgres 16 +
deterministic seed whose public half is pinned in the vector) asserts, via
`live_tracker.rs::signed_flow_enroll_register_heartbeat_lookup`:
enroll(auto-approve) → device/complete → register → heartbeat (incl. typed
notice parse, Rust `HeartbeatResponse`) → signed GET `/peers` lookup (the
query-string-path + empty-body-digest bug class) → challenge_start →
challenge_complete (synthesized timings — honest comment at
`live_tracker.rs:117-119`) → `request_lease` parsing the `{token}` shape →
**Rust `LeasePolicy::check` verifying the TS-issued lease against the pinned
hub key** (signature, freshness, lease-cap, profile, rights, peer binding) →
drain. The H1 fix (tracker signs `lease_expires_at`; issuance shape
`{token, lease}`) is byte-pinned by `protocol/vectors/lease-hubkey-1.json`
and asserted on both sides (TS `tests/crypto-vectors.test.ts`, Rust
eligibility `lease.rs`). Postgres suite runs migrations for real
(`integration-postgres` job; idempotency asserted).

**Not covered cross-language** (TS-suite-only today): rendezvous
offer/answer/pending (no Rust client methods exist —
`crates/modelswarm-tracker-api/src/lib.rs:10-11`), session-authorize,
`/receipt`, `/events/job-result` (Rust client method exists but the harness
never calls it), `/catalog/requests` (desktop posts it via raw reqwest,
`modelswarm-desktop/src/app.rs:2108`), notices *content* beyond the parse
shape, the manual `/verify` approval path, and the epoch-suspension path.
Also note CI runs the ungated production-catalog test (T7).

## 7. Component classifications

| Component | Class | Note |
|---|---|---|
| `lib/schemas.ts` | KEEP | strict zod everywhere; H2 deviation documented + CI-compensated |
| `lib/envelope.ts` | KEEP WITH TESTS | replay/precedence solid; body-cap is post-read (see T4 for the one uncapped route) |
| `lib/guard.ts`, `lib/leases.ts`, `lib/errors.ts`, `lib/crypto.ts` | KEEP | |
| `lib/ratelimit.ts` | KEEP WITH TESTS | per-instance semantics must be documented (T4) |
| `lib/store.ts` | REFACTOR | dual impl drift risk; write-only tables; prune policy (T5); MemoryStore is test-only |
| `lib/eligibility.ts` | REFACTOR | `capacityFromTimings` from self-attested input (T1); hook for §4-C |
| `lib/catalog.ts` | KEEP WITH TESTS | per-request signing documented (§5 #4) |
| `lib/constants.ts` `CHALLENGE_PROMPT` | REPLACE | nonce-bound + digest-verified per §4 B/C |
| `app/api/v1/catalog/requests/route.ts` | REFACTOR | through `guardPublicBody` (T4) |
| `app/api/download/[file]/route.ts` | KEEP WITH TESTS | add rate limit (T4) |
| all other API routes | KEEP | shapes match §3 modulo T6 doc drift |
| `app/verify/page.tsx`, `app/page.tsx` | KEEP | |
| migrations 0001–0003 | KEEP WITH TESTS | forward-only, idempotent, DDL-privacy-tested; extend the test scope (T6) |
| `scripts/validate-vectors.mjs` | REFACTOR | also validate `catalog/candidate-profiles/*.json` (§5 #3) |
| `scripts/migrate.mjs` | KEEP | |
| `crates/modelswarm-tracker-api` | KEEP WITH TESTS | envelope/GET-signing/error-map/lease-parse all tested; add methods only when consumers exist |

## 8. Commands run (this audit)

Read-only static analysis only: `Read`/`Grep`/`ls`/`find`, plus
`git status`, `git log --oneline` (tracker history confirmed:
`3ef0331` seed fail-closed + bounded mailboxes, `dfc6b99` H1 lease interop,
`50de6f4` GET-envelope fix). No npm, no cargo, no vercel, no DB, no HTTP, no
commits. Suite evidence cited from `target/audit-logs/` (all PASS, see
`docs/reviews/audit-freeze-2026-10-09.md` §Results).

## 9. Assumptions

1. Production runs `DATABASE_URL`-backed PgStore (Neon) with
   `DEVICE_AUTO_APPROVE=1`, `DEVICE_APPROVAL_CAP=250`, `MSP_HUB_SEED` set —
   per the brief, freeze doc, and env plumbing; not re-verified against the
   live deployment (read-only static audit).
2. "15 active profiles" (brief) — on-disk evidence is 15
   `catalog/candidate-profiles/*.json`; production promotion state is
   owner-seeded and not re-queried.
3. The Rust serving gate (`modelswarm-node/src/serving.rs`) enforces the
   ADR-026 checks as security's audit describes; I did not re-audit that
   crate (outside my paths).
4. Vercel's platform body cap (~4.5 MB) is the practical bound on the
   uncapped `/catalog/requests` read.

## 10. Unresolved risks

- T1 free-rider envelope is live in production today (open enrollment ×
  fabricated challenge × signed lease).
- Cumulative-250 cap will silently end auto-approval someday; no
  observability exists to notice (no admin read of the count).
- Unbounded growth tables (T5) on Neon free-tier storage.
- Per-instance rate limits under-coordinate during burst traffic.

## 11. Suggested next task for the integrator

Approve the §4 package (A+B now, C as a 1–2 d follow-up with its ADR) and the
T4 hygiene batch (30 min + 15 min + 1 h fixes, tracker-only, all
TS-test-covered); route the T2 suspension-chassis work to the reputation
gap-study track with an ADR for migration 0004 semantics. Do not deploy from
this branch; deploy remains owner-run Vercel CLI from `apps/tracker/` after
review, with `npm run typecheck && npm test && npm run build &&
npm run validate:vectors` green locally.
