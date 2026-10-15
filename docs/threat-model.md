# ModelSwarm Threat Model

Status: Phase 0 draft, owned by the Architect with QA/Security review.
Scope: MSP v0.1 prototype (Windows nodes + Vercel tracker hub).
Updated 2026-10-15 (ADR-038 §6.1 Security-seat rows: the §4.6 relay
admission surface with the S1-S9 mitigation table and five residual
rows — gate D12 closes conditionally; the §5.4 relayed-path restatement;
the §6 ADR-038 abuse-test index. Previous: 2026-10-14 ADR-036 §6
Security-seat rows).

## 1. System under analysis

See `docs/architecture.md`. Surfaces an adversary can touch:

1. Hub REST API (`modelswarm.deepflux.space`) — unauthenticated + authenticated.
2. P2P streams between peers (QUIC/Noise, internet-reachable listeners).
3. Loopback llama.cpp sidecar (127.0.0.1, random port, bearer secret).
4. Local node files (SQLite, cached GGUF, logs) and Windows credential store.
5. The user's local OpenAI-compatible gateway on 127.0.0.1:11435.
6. Supply chain: GGUF artifacts, llama.cpp builds, npm/cargo dependencies.
7. The operator-hosted circuit relay (F2B deployment; `--public` QUIC
   listener, open-by-default with ADR-038 ticket admission — §4.6).

## 2. Assets

| Asset | Harm if compromised |
|---|---|
| Prompt/completion content of other users | Privacy breach (highest severity) |
| Hugging Face access token (if user supplies one for gated models) | Cloud account abuse |
| Ed25519 installation key | Impersonation, swarm admission of fake hosts |
| Hub signing keys (catalog + capability tokens) | Total protocol compromise |
| User machine resources | DoS, disk/bandwidth exhaustion |
| Swarm integrity | Wrong answers, poisoned metrics, split swarms |

## 3. Adversaries

A. **Casual free-rider** — wants inference without hosting. Not sophisticated;
   will pause hosting, fake UI state, or reuse tokens. Primary target of the
   core rule.
B. **Malicious serving peer** — joined legitimately, now returns wrong/garbage
   tokens, records prompts, exaggerates metrics, or stalls streams.
   ADR-036 restatement (owner directive 2026-10-13, default-on carriage):
   under the elected family-verified chat mode, a serving peer that
   admits a pin-carrier session receives the requesting user's WHOLE
   conversation as token ids and can read, log, or repurpose it — EVERY
   cohort member of the elected session, not one peer. The owner accepts
   this ("the idea of a swarm is that all agents work together, privacy
   is not a concern at this moment"). Remaining mitigations, stated
   without overstatement: transport encryption only (token ids are not
   encryption — a peer hosting the same exact profile holds the tokenizer
   and reads them trivially), no hub visibility (the carrier rides P2P
   QUIC only; the hub is content-blind unconditionally), invite-only
   population, teardown-bounded KV residency, honest always-visible
   disclosures on both ends, and NO confidential-computing claim. The
   consent-UX surface is downgraded per the same posture: no consent
   modal exists to phish; the residual is a user tricked into opting OUT,
   or a peer misrepresenting its carriage posture — the refusal label
   (`content_carriage_refused`) is honest either way, and no roster or
   wire field carries the posture, so a posture cannot be verified
   before dialing (ADR-036 §3.1's stated honest boundary).
C. **Malicious requesting peer** — floods a host, sends oversized payloads,
   replays/copies tokens, ignores limits.
D. **Network observer / on-path attacker** — reads or tampers P2P or hub
   traffic.
E. **Abusive account holder** — mass registration, scraping the peer
   directory, enumeration.
F. **Compromised supply chain** — poisoned GGUF file, tampered llama.cpp
   build, malicious dependency.
G. **Hub operator / database compromise** — out of scope for v0.1 defenses but
   in scope for damage limitation (the hub holds no prompts, so breach damage
   is bounded by design).

## 4. Analysis by surface

### 4.1 Hub REST

| Attack | Mitigation (v0.1) |
|---|---|
| Replayed signed requests | Timestamp window + nonce cache per installation |
| Forged signatures | Ed25519 verification; key registered at device enrollment |
| Enumeration / scraping | Auth on peer lookup, rate limits, bounded result sets, pagination caps |
| Oversized bodies | Strict Zod schemas, payload limits, reject before parse |
| Resource exhaustion | Per-account and per-endpoint rate limiting, function timeouts |
| Fake peer registration | Signed registration bound to enrolled installation key; heartbeat expiry; hosting challenge before capability issuance |
| Sybil enrollment under auto-approval (`DEVICE_AUTO_APPROVE=1`, owner decision 2026-10-06) | Enrollment rate limit (10 req/min/IP) bounds throughput; `DEVICE_APPROVAL_CAP` (default 250 distinct installations) bounds total exposure — beyond it the admin-token `/verify` gate resumes. Damage without a transport listener is census/roster pollution only (no remote serving in v0.1); census counters on such deployments count self-enrolled installations |
| Token theft/replay | Capability tokens are peer-bound, profile-bound, short-lived, nonce-carrying; revocation list checked by serving peers via hub lookups |
| Injected catalog entries | Admin-only promotion flow; catalog is signed and versioned; nodes verify signature |

Residual risk: the hub is a central trust anchor. Compromise of its signing key
lets an attacker admit arbitrary peers. Mitigation is operational (key hygiene
in Vercel-encrypted env vars, rotation procedure documented in Phase 1 ops
notes), not cryptographic, in v0.1.

### 4.2 P2P streams

| Attack | Mitigation (v0.1) |
|---|---|
| Eavesdropping / tampering | libp2p Noise-encrypted QUIC; messages carry signed digests |
| Wrong-answer serving | Out of scope to prevent cryptographically; job receipts + requesters' outcome sampling create evidence for reputation and blocking |
| Oversized frames | Maximum message size; reject before expensive work |
| Slowloris / slow consumer | Deadlines on every stream; bounded buffers; backpressure |
| Unbounded concurrent requests | Per-peer admission control + serving-slot cap |
| Replay of request frames | Request id + nonce; serving peer rejects duplicates |
| DoS via connection flooding | Rate-limited admission; circuit breakers per peer |
| ADR-036 carrier exposure: an elected family-verified chat sends the requesting user's conversation (token ids) to every cohort member, default-on at every serving peer | Owner-accepted by directive (2026-10-13); serving-side opt-out refuses honestly (`content_carriage_refused` — a refusal label, never a mislabeled `pin_mismatch`); always-visible §3.1 disclosures on both ends; token ids are readable by any peer hosting the same exact profile (tokenization is not encryption); the hub stays content-blind — the carrier rides P2P QUIC only; counts-only telemetry (token COUNT yes; arrays and content-derived hashes, including the pin commitment, never) |
| Malformed / partial / forged pin carrier at session open | Strict `deny_unknown_fields` schema parsed by the session-open reader only (never the frame router); first-failure admission table (schema → signature/sender binding → carriage → profile → params → commitment bind → prefill fit → clamped deadline), signature = Ed25519 over the signature-omitted canonical JSON with the sender bound to the QUIC-authenticated PeerId's key; oversize carriers refused before parse (256 KiB frame limit, transport-enforced); fixed open order `lease? → pin? → offer`, deviations are bad_frame-class session ends; the carrier's `session_id` must equal the offer's (mismatch ends the session before any accept) |
| Carrier replay/reorder at open (ADR-036 review focus) | The carrier seeds the session gate's seq baseline and nonce ledger (frame reuse → `replayed_request`); the ledger is per-session by contract — cross-session replay requires the coordinator's own signing key, which is the authorized sender; rounds in carrier sessions are ≥ 1 by construction (the carrier's `round: 0` is the session-open marker) |

### 4.3 Local node

| Attack | Mitigation (v0.1) |
|---|---|
| Sidecar reachable from LAN | 127.0.0.1-only bind + random bearer secret |
| Key theft from disk | Windows Credential Manager/DPAPI; never in webview or config files |
| Malicious local app using the gateway | Gateway is loopback-only; v0.1 accepts any local client (documented limitation; local auth token is a candidate hardening) |
| Log leakage of prompts | `modelswarm-telemetry` redaction is mandatory; QA test asserts absence |
| Conversation KV outliving its session (ADR-036 carrier sessions) | The carrier session's family engine is constructed per-session with the conversation as the prefilled root and torn down at session end (drop-counted at the engine seam); the coordinator's self-check engine is dropped at turn end; the cache-cold discipline (ADR-032 §3) and the routed dribble-residence lifetime-ceiling advisory apply; `prompt_tokens` never enter logs, telemetry, rejections, receipts, or the webview (token counts only) |
| Malicious GGUF/runtime | Only catalog-approved, digest-pinned artifacts from HF; pinned llama.cpp build with checksum |

### 4.4 Supply chain

Catalog immutability (ADR-005) means a node never executes a model or runtime
that was not resolved to a full commit hash + digest by an admin and validated
by test nodes. Dependency audits (cargo audit / npm audit) run in CI from
Phase 1 onward.

### 4.5 Desktop auto-update chain (added 2026-10-13; review:
`docs/reviews/review-updater-security-2026-10-13.md`)

Surface: Tauri 2 updater plugin (`tauri-plugin-updater` 2.13.3, locked) driven
from Rust in the desktop shell. Manifest endpoint is the tracker site's static
`https://modelswarm.deepflux.space/downloads/latest.json` (TLS-only,
build-embedded); the minisign public key is embedded in `tauri.conf.json` at
build time. The manifest itself is NOT signed — only the update package is.

| Attack | Mitigation |
|---|---|
| Compromised site serves a crafted manifest AND a malicious package | The package signature is verified over the DOWNLOADED BYTES against the build-time-embedded public key before anything installs (`verify_signature` → `PublicKey::verify`, plugin `updater.rs`). The `.sig` travels in the site-controlled manifest, but its validity is anchored client-side: a package we never signed can never install. A missing/empty signature fails manifest parse or decode |
| Version confusion (inflated manifest `version` + older genuine artifact) | `requireSignedVersion: true`: the minisign trusted comment — covered by the signature's global signature — must name the announced version (`SignedVersionMismatch` otherwise; no version in the comment → `MissingSignedVersion`) |
| Downgrade to an old signed release | No custom comparator and `allowDowngrades` unset → the plugin's default rule applies: `release.version` must be strictly greater than the running version. A client at version N can never be moved to any version ≤ N |
| Endpoint/TLS downgrade of the check | Endpoints validated https-only in release builds; `dangerous*` flags set nowhere; pubkey and endpoints come from build-embedded config only — no runtime override, no env override, and no webview capability grants the plugin's invoke commands (only status lines cross into the UI) |
| Dev opt-out abuse | Only the exact env value `MSP_NO_AUTOUPDATE=1` skips checks; it disables checking, never verification |
| Installer races a live engine binary | `on_before_exit` closes every retained kill-on-close winjob handle (all supervised engine children die) after signature verification and before the NSIS installer spawns |

Residual risks, accepted for v0.2.x (invite-only tester population):

1. **Update withholding.** A compromised origin (Vercel account/DNS+TLS) can
   serve a stale manifest indefinitely, pinning clients at their current
   version. Inherent to an unsigned (TLS-only) manifest.
2. **Re-serving a yanked/future genuine release.** The origin can re-serve any
   version we genuinely signed that is NEWER than the client's current one
   (e.g. a pulled 0.2.24 to a client on 0.2.23). No minimum-version/rollback
   fence exists; one (signed manifests or a baked-in version floor) is
   REQUIRED before wider distribution (v1).
3. **Download-side memory DoS.** The plugin buffers the package in memory
   before verification with no size cap or timeout; a compromised origin can
   OOM-crash the desktop app (local DoS only — no code execution; the
   signature gate is unaffected). A wall-clock timeout and byte cap on the
   download phase are advised.
4. **Key lifecycle.** Rotation requires shipping a client with a new embedded
   pubkey (manual install); until every deployed client has updated past the
   rotation release, the old public key must be treated as still-valid —
   rotation lead time is one full adoption cycle. Release-train obligations
   (standing): sign every installer at owner staging with
   `--app-version <VERSION>` (mandatory under `requireSignedVersion`); the
   private key never enters the repo, CI, or logs; rotation is a manual client
   ship recorded in the release checklist.

### 4.6 Circuit relay + admission tickets (added 2026-10-15; ADR-038;
review: `docs/reviews/review-adr038-admission-security-2026-10-15.md`)

Surface: the operator-hosted circuit relay (`crates/modelswarm-relay`),
loopback by default, `--public` on explicit operator opt-in. Posture per
ADR-038: open-by-default admission by tracker-signed `RelayAdmissionTicket`
(content-blind, 600 s TTL, minted on every successful heartbeat), enforced
FAIL-CLOSED at the two libp2p-relay accept sites (one gate limiter FIRST in
both rate-limiter vectors), under finite frozen caps (600 s reservation,
1800 s / 16 MiB per circuit). **Gate D12** (`docs/reviews/
architecture-gate-2026-10-09.md`) closes CONDITIONALLY: this §6.1 review
approves the landed code; D12 is closed only when T/R's §6.2 sign-off exists
AND the owner-gated §6.3 real-network verification entry
(`docs/verification/relay-admission-<date>.md`) exists against the REDEPLOYED
gated relay — until then the deployed F2B relay must not run `--public` on
the old ungated binary.

The §6.1 adversary matrix, adjudicated against the landed code (evidence in
the review file):

| Attack | Mitigation (as landed, 2026-10-15) |
|---|---|
| S1 ticketless flood, incl. clients that skip the pre-flight | Gate predicate on the reservation AND circuit accept paths (upstream `.all()` short-circuits on the gate's `false` ⇒ `resource_limit_exceeded(201)` before any token bucket or reservation lookup); proven in-process with a real libp2p client that never presents; renewals re-hit the gate every ≤ 450 s |
| S2 forged signature | Ed25519 over the canonical signature-omitted JSON against the pinned hub key (`protocol/keys/hub-public.hex`, `--tracker-key` override); fail-closed boot — no resolvable key refuses startup (exit 2) in EVERY mode |
| S3 farmed/stolen ticket | Check 7 peer binding: only the named PeerId's Noise-authenticated key can present it (a stolen ticket is useless to a third party); check 8: `installation_id` must re-derive from the authenticated PeerId's embedded key (ADR-020); farming = enrolled sybils, bounded by enrollment (`DEVICE_APPROVAL_CAP` 250 / admin approval) + per-peer caps (8 reservations, 128 circuits, 30 creations/2 min, 16 MiB/30 min per circuit) |
| S4 tracker-key compromise | Same root-of-trust exposure as catalog + leases (§4.1 residual — not a new class); NEW instant relay-side lever: restart with `--tracker-key` kills every outstanding ticket at once (residual 1 below) |
| S5 admission-table exhaustion | 4096 entries keyed by PeerId, memory-only, full expired-sweep on every admit (a full sweep precedes every `table_full` refusal); filling it requires 4096 DISTINCT live validly-ticketed PeerIds — enrollment-bounded, and one installation's tickets can never occupy more than one slot |
| S6 clock-skew games | ±120 s skew window at verify (check 6) + admission deadline `expires_at + 120 s` (T4); one shared clock feeds verify, admit, and the gate; pinned by the expiry vector and the virtual-clock re-close proof |
| S7 pre-flight injection | Strict frozen protobuf (unknown/duplicate/wrongly-typed/wire-type-3+ fields refused), exactly one frame each way, announced length validated ∈ [1, 4096] BEFORE body allocation, 15 s deadline per exchange, mss-bounded negotiation; the admission table keys on the Noise-authenticated presenting PeerId, never ticket input |
| S8 shared-NAT rate-limiter token theft | The gate limiter is FIRST in both limiter vectors (index-0 insert, test-pinned); upstream short-circuit means unadmitted peers never call `try_next` on the per-peer/per-IP token buckets honest peers behind shared NATs depend on |
| S9 restart/state | Admission state is memory-only and never persisted; a restart empties the table and every peer re-presents one cheap frame; reservations/circuits die with the relay's connections (no state to forge or replay across restarts) |

Residual risks (accepted for the open-relay posture, recorded 2026-10-15 —
deterrence-grade, in the AGENTS.md sense):

1. **Tracker-key compromise = unlimited relay admission until rotation.**
   The hub Ed25519 key can mint tickets for any PeerId the holder controls;
   each ticket is capped at TTL + skew, but issuance is unbounded while the
   compromise lasts. Recovery: rotate the key at the tracker, restart every
   relay with `--tracker-key <new>` (INSTANT global ticket invalidation —
   the one lever the ADR adds); full rotation additionally ships new node
   releases because deployed nodes pin the OLD public key for
   catalog/lease verification (the §4.1/§4.5 key-lifecycle lead time —
   rotation lead time is one full client adoption cycle). Stated runbook
   order: generate new key → tracker redeploy (re-signs catalog + leases +
   tickets) → relay `--tracker-key` restarts → node release train.
2. **Ticket farming via enrolled installs — bounded, not prevented.** A
   determined operator of many enrolled installations (sybil ring) earns one
   admitted PeerId per install; damage per install is bounded by the §3.5
   caps (worst sustained burn ≈ 32 Mbps per admitted rogue — the operator's
   pipe, not the protocol, is the global bound). Enrollment controls
   (`DEVICE_APPROVAL_CAP`, admin approval, enrollment rate limits) are the
   actual bound; the relay adds no per-identity economics in v1.
3. **The relay operator sees flow metadata and relays PLAINTEXT frames.**
   Who reserves, who circuits to whom, timing, and sizes are visible to the
   operator (admission/relay log discipline: counts, PeerIds, reasons only
   — no ticket bytes, no payload), and QUIC terminates at the relay, so a
   hostile relay fork can READ relayed frames (our relay is content-blind
   by construction; a fork need not be). The standing mitigation is the
   msp-v1 §6.1 end-to-end handshake over relayed circuits — the queued
   session-layer decision (F2B §7.2); until it lands, relayed paths are
   relay-vouched and labeled as such, and prompts/completions still never
   reach the tracker.
4. **Expiry-only revocation tail ≤ 12 min.** No relay-side revocation
   channel exists (T8: the relay never contacts the tracker); an
   installation revoked at the tracker keeps its LAST ticket valid until
   `expires_at + 120 s`, during which the per-peer caps bound the damage.
   Key rotation (residual 1) is the only instant lever.
5. **Tracker outage > TTL degrades NEW relayed circuits honestly.** No
   fresh tickets ⇒ new relay legs earn a verbatim
   `permission_denied(202)/expired` (or the gate's `201` when the client
   has nothing to present); established circuits live out their caps,
   direct paths are unaffected — the §8 "tracker outage → streams
   continue" discipline holds for established streams with this labeled
   degradation for new relayed ones. (Related: an ADMITTED rogue behind a
   shared NAT still shares that NAT's per-IP token bucket with honest
   peers — inherent to per-IP accounting; the per-peer bucket bounds the
   rogue itself.)

## 5. Explicit non-guarantees (v0.1)

1. **No remote attestation.** A patched binary can claim to host while not
   hosting, or serve while paused. The capability-token system raises effort
   and creates auditable evidence; it does not eliminate lying.
2. **No confidential computing.** The serving peer reads every prompt it
   serves — and, since ADR-036 (owner directive 2026-10-13, default-on
   carriage), every verifying peer of an elected family-verified chat
   reads the whole conversation it verifies. The UI discloses both on
   their always-visible lines; the swarm is invite-only in v0.1.
3. **No Sybil resistance** beyond invite-only accounts + per-account limits.
4. **No universal NAT traversal.** Direct failure is possible and must be
   reported honestly; relayed paths exist (F2B) and are admission-gated
   (ADR-038, §4.6) but carry the relay-vouched trust and metadata/plaintext
   residuals stated there; DCUtR/hole-punching remains a later milestone.
5. **No protection of the local machine from its own user** — a user who
   controls the OS controls the node.
6. **No update-rollback fence** — a compromised update origin can withhold
   updates or re-serve any previously-signed version newer than the client's
   (§4.5); versions at or below the running one are refused.

## 6. Abuse tests and where they live

Hub-side abuse tests are frozen in `docs/acceptance/phase-1.md` (§F). P2P and
token abuse tests land with the phases that build those surfaces —
`docs/acceptance/phase-3.md` (frame replay, wrong signature, wrong profile,
oversized prompts, cancellation, disconnects, slow consumers),
`phase-5.md` (token copy/forgery/expiry, clock skew, stale lease, patched
metrics), `phase-7.md` (full matrix from `docs/build-plan.md`) — plus a
prompt-redaction gate in every phase whose code first touches prompt text
(Phase 3 transport, Phase 4 gateway). The per-phase index is
`docs/acceptance/README.md`. Items that must exist before release:

- Replay of signed hub requests and P2P frames.
- Copied/modified capability token (wrong peer, wrong profile, expired).
- Oversized prompt, oversized frame, deep nesting (parser abuse).
- Prompt-privacy audit of hub DB and all node logs.
- Draining/paused host still trying to consume.
- Metrics inflation attempt (advertise huge decode rate; verify scheduler
  ignores self-reports when measurements disagree).
- ADR-036 carrier abuse (added 2026-10-14): golden-vector replay of all
  three pin-carrier fixtures through the admission table, the pairwise
  first-failure order sweep, the fixed open-sequence order (4 valid
  shapes + deviation classes + oversize-before-parse), carrier nonce/seq
  replay over the real staged transport, codes-only reject frames, the
  commitment's pure self-consistency, the offer↔carrier session-id
  binding, the both-keys discriminant rule, and the engine-teardown
  KV-residency bound
  (`crates/modelswarm-node/tests/verifydrafts_pin_carrier.rs`,
  `crates/modelswarm-node/tests/verifydrafts_pin_carrier_security.rs`,
  `crates/modelswarm-session/tests/pin_carrier_property.rs`,
  `crates/modelswarm-session/tests/verifydrafts_vectors.rs`; the
  desktop-level redaction and disclosure pins live in
  `crates/modelswarm-desktop/src/app.rs` tests under `tauri-shell`).
- ADR-038 admission abuse (added 2026-10-15): the §6.2 suites are the
  abuse tests for §4.6's S1-S9 rows — the in-process fail-closed proofs
  (ticketless reservation AND circuit refusal, admit-on-ticket, virtual-
  clock re-close), the twelve-row forge corpus re-derived against the
  frozen status+reason pairs with the first-failure order pinned
  (syntax ≺ signature ≺ peer binding ≺ installation ≺ capacity), the
  frame-vector malformed corpus (strict protobuf), size-before-allocation
  refusal at the frame reader, boot-discipline refusals, the frozen-cap
  cut proofs, the transport presentation suite (pre-flight before
  reserve/connect, 202 ⇒ no hop stream with the verbatim label, `na`
  third-party interop, renewal re-presentation window), the node cache
  suite (deploy skew, malformed-drop, replace-never-accumulate, no
  freshness policy), and the TS↔Rust ticket byte-parity gate
  (`crates/modelswarm-relay/src/main.rs` + `src/admission.rs` tests,
  `crates/modelswarm-transport/src/relay_client.rs` +
  `libp2p_backend.rs` tests, `crates/modelswarm-node/src/relay_ticket_cache.rs`
  tests, `apps/tracker/tests/relay-ticket.test.ts`). Known drift, ruled
  acceptable 2026-10-15 (binding; see the §6.1 review): the golden accept
  vector's `installation_id` placeholder cannot pass check 8's real
  ADR-020 derivation — the vectors pin checks 1-6, binding is pinned by
  re-derived fixtures; regeneration with a real (peer_id,
  installation_id) pair is REQUIRED in the next Protocol vector revision
  (`relay-ticket-forge-1.json`'s `derived_installation_id` field
  especially — it currently records the placeholder as if real).
