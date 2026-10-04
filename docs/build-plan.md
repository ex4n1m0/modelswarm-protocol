# ModelSwarm Prototype: ZCode + GLM-5.3 Build Plan

## Executive direction

Build the first prototype as a **Windows-first Rust application plus a lightweight Vercel rendezvous hub** at `modelswarm.deepflux.space`. The hub discovers and authenticates peers; model prompts and generated tokens travel directly between peers whenever possible. Do not route inference or Hugging Face model files through Vercel.

Use ZCode with GLM-5.3 in its long-horizon/Goal workflow, but divide implementation into bounded phases and specialist subagents. ZCode is designed to retain workspace files, terminal results, browser context, and Git state throughout long coding tasks, making it suitable for an iterative plan-code-test-review workflow.[^1]

The prototype must enforce one central rule:

> A peer may submit inference to a model swarm only while it is actively hosting the exact same approved model profile.

“Exact same” means the same Hugging Face repository, full revision hash, GGUF file, artifact hash, tokenizer/chat template, quantization, context policy, and compatible runtime profile—not merely the same model name.

## Architecture decision

```text
                          modelswarm.deepflux.space
                    Vercel web app + tracker API + database
                   accounts · catalog · peer rendezvous · policy
                                  │
                    HTTPS heartbeat / lookup / signaling
                                  │
               ┌──────────────────┴──────────────────┐
               │                                     │
       Windows ModelSwarm Node A              Windows ModelSwarm Node B
       Rust + llama.cpp sidecar                Rust + llama.cpp sidecar
       host + client + scheduler               host + client + scheduler
               │                                     │
               └──── direct encrypted P2P stream ────┘
                    prompts · tokens · cancellation

        Hugging Face Hub ── direct model download ──► each Windows node
```

### Control and data planes

| Plane | Responsibility | Location |
|---|---|---|
| Catalog | Approved models, exact revisions, files, licenses, hashes, minimum runtime | Vercel hub/database |
| Identity | User account, installation identity, Ed25519 peer key | Hub plus local Windows credential storage |
| Rendezvous | Peer registration, short-lived heartbeat, candidate lookup, connection signaling | Vercel hub |
| P2P transport | Encrypted direct peer connection, request, stream, cancellation | Rust nodes |
| Inference | Load GGUF model, prefill, decode, stream tokens | Bundled `llama.cpp` process |
| Model acquisition | Download approved artifacts and verify revision/hash | Directly from Hugging Face to node |
| Scheduling | Filter eligible peers and select the lowest predicted completion time | Requesting Rust node |
| Accounting | Record hosting proof, jobs served/consumed, failure and latency observations | Local node plus signed hub events |

### Vercel boundary

Vercel can host the website, REST tracker, database integration, and initial signaling. Vercel now supports WebSockets in public beta, but each connection remains pinned to a function and ends when the function reaches its duration limit; this makes WebSockets useful for signaling, not an ideal permanent P2P relay. Function duration remains bounded by plan, so the architecture must not require a tracker invocation to stay alive for the lifetime of every desktop node.[^2][^3][^4][^5]

The first release should therefore use:

- HTTPS heartbeats every 15–30 seconds with a 60–90 second peer lease.
- REST peer lookup and short signaling exchanges.
- Direct QUIC/libp2p traffic after rendezvous.
- A separate public libp2p relay service in a later phase for peers that cannot connect directly.

NAT traversal commonly needs a public relay to coordinate a relayed connection and DCUtR hole punch. The Vercel deployment should never be presented as a full relay unless tests prove it can sustain the required binary traffic, duration, bandwidth, and abuse controls.[^6]

## Prototype scope

### Must ship in v0.1

- One signed Windows `setup.exe` or unsigned internal-test installer, clearly labeled.
- Rust desktop application/daemon with a minimal UI.
- One canonical GGUF model profile working end to end.
- Optional second and third profiles only after the first passes all tests.
- Direct Hugging Face download with progress, pause/resume, disk-space validation, checksum verification, and clear license display.
- Tracker deployment at `modelswarm.deepflux.space`.
- Peer registration, expiring heartbeats, lookup, and direct connection negotiation.
- OpenAI-compatible local endpoint, initially `http://127.0.0.1:11435/v1/chat/completions`.
- Dynamic routing based on measured RTT, queue delay, prefill rate, decode rate, and recent reliability.
- Streaming output, timeout, cancellation, circuit breaker, and retry before the first emitted token.
- Enforcement that a client can consume only the exact profile it is hosting.
- Local logs with secret and prompt redaction.
- Automated Rust tests plus a three-node end-to-end test on Windows.

### Explicit non-goals

- Splitting one autoregressive token across 100 replicas.
- KV-cache transfer or prefill/decode disaggregation.
- Blockchain, payment tokens, or a public marketplace.
- Anonymous public access.
- P2P redistribution of model weights.
- Automatic support for arbitrary Hugging Face repositories.
- Linux/macOS installers.
- A distributed DHT in v0.1; the Vercel tracker is the bootstrap authority.
- Seamless mid-generation migration between peers.

## Recommended stack

| Component | Choice |
|---|---|
| Core app | Rust stable, Tokio, Axum/Hyper, Serde, Reqwest |
| P2P | `rust-libp2p` with QUIC, Noise/TLS, Identify, Ping; add Relay/DCUtR after direct paths |
| Identity | Ed25519 keypair generated per installation |
| Hashing | SHA-256 for downloadable-file verification; BLAKE3 for internal IDs if desired |
| Local storage | SQLite via SQLx |
| Secrets | Windows Credential Manager/DPAPI abstraction |
| Inference | Bundled, version-pinned `llama.cpp` server sidecar |
| Desktop UI | Tauri 2 shell with a small web UI, or `egui` if an all-Rust UI is preferred |
| Installer | Tauri NSIS `setup.exe`; add MSI later |
| Hub frontend/API | Next.js/TypeScript on Vercel |
| Hub database | Managed Postgres such as Neon through Vercel Marketplace |
| API validation | Zod or equivalent at the hub; Rust typed validation in the node |
| CI | GitHub Actions with Windows build/test/signing workflow |

Tauri can generate Windows NSIS setup executables and MSI packages; MSI creation requires Windows/WiX. Its updater requires signed update artifacts, and signature verification cannot be disabled, which is appropriate after the prototype key-management process is established.[^7][^8]

`llama.cpp` offers prebuilt installation options, can run GGUF models from Hugging Face, and exposes an OpenAI-compatible server. Keep it as a pinned sidecar in v0.1 instead of attempting to build inference kernels in Rust.[^9][^10]

## Model catalog policy

Begin with a single reference profile, then add two sizes in the same family:

| Stage | Suggested profile | Purpose |
|---|---|---|
| v0.1 | Qwen3 4B GGUF `Q4_K_M` | Broad hardware compatibility and end-to-end validation |
| v0.1.1 | Qwen3 1.7B GGUF | Low-resource onboarding and network testing |
| v0.2 | Qwen3 8B GGUF `Q4_K_M` | Higher-quality tier for stronger PCs |

Hugging Face’s `ggml-org` collection includes Qwen3 GGUF repositories at 1.7B, 4B, and 8B scales. The Qwen3 4B GGUF page identifies an Apache-2.0 license and documents `llama.cpp`, Windows, Ollama, and `Q4_K_M` usage. Final catalog entries must nevertheless be reviewed individually for licensing, authorship, conversion provenance, and runtime compatibility; Hugging Face stores license information in model-card metadata and supports custom licenses.[^11][^12][^13][^14]

### Immutable profile example

```json
{
  "profileId": "msp:qwen3-4b:q4_k_m:v1",
  "displayName": "Qwen3 4B Q4_K_M",
  "hfRepo": "ggml-org/Qwen3-4B-GGUF",
  "hfRevision": "FULL_40_CHARACTER_COMMIT_HASH",
  "filename": "EXACT_APPROVED_FILENAME.gguf",
  "sha256": "EXPECTED_FILE_SHA256",
  "quantization": "Q4_K_M",
  "runtime": {
    "name": "llama.cpp",
    "minBuild": "PINNED_AND_TESTED_BUILD"
  },
  "contextTokens": 8192,
  "maxOutputTokens": 2048,
  "licenseId": "apache-2.0",
  "status": "active"
}
```

Do not place placeholder values into production. The catalog-generation script must resolve and store the full Hugging Face commit hash, exact filename, byte size, artifact digest, and license evidence.

### Update semantics

“Keep models up to date” must not mean silently tracking `main`. Hugging Face supports downloads pinned to a branch, tag, or full commit hash, while unpinned downloads default to the latest `main` revision. Silent drift would divide apparently identical peers into incompatible replicas.[^15]

Use coordinated channels instead:

1. The hub checks approved repositories for new revisions.
2. An administrator promotes a tested revision to `candidate`.
3. Test nodes download and validate it.
4. The hub publishes a new immutable profile ID.
5. Nodes download it in the background but continue serving the old profile.
6. Once enough hosts are ready, the hub marks the new profile `active` and the old one `deprecated`.
7. Requests never cross profile IDs.
8. Old files are removed only with user consent and after no active swarm depends on them.

The Rust `hf-hub` client provides asynchronous repository and file operations, downloads, local caching, retries, and Xet-backed transfers, so it is a suitable starting dependency. If reliability problems appear, the node may invoke the official `hf` CLI as an isolated fallback, but it should not require Python for normal installation.[^16][^17]

## Eligibility rule

A user may request model profile `P` only if that same installation currently qualifies as an active host for `P`.

### Qualification conditions

- The exact approved artifact exists locally and passes the expected digest.
- The approved runtime starts the model successfully.
- A local challenge prompt completes within a model-specific deadline.
- The node has an active tracker lease for that profile.
- The node advertises at least one serving slot.
- The node accepts inbound requests directly or through an approved relay path.
- The node is not draining, paused, suspended, or below the reliability threshold.
- The user has accepted the model’s license and network privacy warning.

### Capability token

After a successful hosting challenge, the tracker issues a short-lived, signed capability token:

```json
{
  "peerId": "12D3Koo...",
  "profileId": "msp:qwen3-4b:q4_k_m:v1",
  "canHost": true,
  "canConsume": true,
  "slots": 1,
  "expiresAt": "2026-10-04T15:00:00Z",
  "nonce": "..."
}
```

Every inference request includes this token, but the serving peer also verifies its signature, expiry, profile, requesting peer identity, and replay nonce. The token proves recent qualification—not long-term fairness.

### Anti-cheating boundary

The v0.1 rule is enforceable against casual misuse, not a determined attacker. A malicious client could patch its binary, fake metrics, or host only while consuming. The prototype should document this limitation and collect evidence for a later contribution policy such as minimum uptime, requests served, availability score, or earned service credits.

Do not equate “the GGUF file exists” with “the node is contributing.” A useful host must pass live challenges and be reachable. Authentication, authorization, rate limiting, payload limits, and resource quotas are essential because unbounded API operations can cause resource exhaustion or denial of service.[^18][^19]

## Tracker design

### Domain setup

1. Create a Vercel project named `modelswarm-hub`.
2. Add `modelswarm.deepflux.space` under **Settings → Domains**.
3. At the DNS provider for `deepflux.space`, create the exact CNAME target shown by Vercel.
4. Verify TLS and redirect any temporary `*.vercel.app` public link to the custom domain where practical.
5. Use lowercase `modelswarm.deepflux.space` in code and documentation; DNS is case-insensitive, but canonical lowercase avoids inconsistent URLs.

Vercel’s official guidance says subdomains use a CNAME and that each project may receive a unique target, so the value shown in the project must be used rather than guessed.[^20][^21]

### Tracker endpoints

```text
GET  /api/v1/health
GET  /api/v1/catalog
GET  /api/v1/catalog/{profile_id}
POST /api/v1/auth/device/start
POST /api/v1/auth/device/complete
POST /api/v1/peers/register
POST /api/v1/peers/heartbeat
POST /api/v1/peers/challenge/complete
GET  /api/v1/peers?profile_id=...&limit=...
POST /api/v1/rendezvous/offer
POST /api/v1/rendezvous/answer
POST /api/v1/events/job-result
POST /api/v1/peers/drain
```

### Core records

```text
users
installations
peer_keys
model_profiles
license_acceptances
peer_leases
peer_observations
hosting_challenges
capability_tokens
job_receipts
blocked_peers
release_channels
```

Store only metadata required for operation. Do not store prompts, completions, raw conversation histories, or Hugging Face access tokens. Vercel no longer provides a first-party Vercel Postgres product; connect a managed Postgres provider such as Neon through the Vercel Marketplace.[^22]

### Trust assumptions

- The tracker is authoritative for catalog and eligibility in v0.1.
- Peers are authoritative for their local queues but not automatically trusted.
- Requesters measure peers and maintain local reputation.
- Every advertisement expires quickly.
- Every signed request includes timestamp, nonce, profile ID, and body digest.
- The tracker can revoke installations and profile versions.
- Prompt transport uses end-to-end peer encryption; the tracker sees routing metadata but not prompt content.

## P2P request flow

1. The node starts and verifies its installation key.
2. It fetches the signed catalog and checks for an approved update.
3. It starts `llama.cpp` bound to loopback only.
4. It runs a local readiness challenge.
5. It registers public/local candidate addresses and sends a signed heartbeat.
6. The tracker grants a short capability token.
7. A local application sends an OpenAI-compatible request to the ModelSwarm gateway.
8. The gateway confirms it is eligible to consume that exact profile.
9. It requests candidate hosts from the tracker.
10. It filters incompatible, stale, blocked, overloaded, or policy-ineligible peers.
11. It probes the best candidates and selects the lowest predicted completion time.
12. It opens an encrypted P2P stream and sends the normalized request.
13. The serving peer validates identity, token, profile, limits, and available capacity.
14. The serving peer proxies to its loopback `llama.cpp` endpoint and streams normalized token events.
15. Both peers sign a minimal job receipt containing timings and outcome, not prompt text.
16. The requester updates its local EWMA performance observations.

### Initial scheduler

Use a transparent heuristic rather than machine learning:

```text
predicted_ms =
    measured_rtt_ms
  + advertised_queue_ms
  + prompt_tokens / measured_prefill_tokens_per_ms
  + expected_output_tokens / measured_decode_tokens_per_ms
  + failure_penalty_ms
  + stale_advertisement_penalty_ms
```

Prefer measurements observed by the requester over self-reported peer metrics. Keep a session on the same peer for follow-up turns when possible. Retry another peer only before any output token is exposed in v0.1; after output begins, report an interrupted stream rather than pretending continuation is deterministic.

`rust-libp2p` Gossipsub may later distribute live capacity metadata, but it does not perform peer discovery itself; Kademlia plus Identify is a documented discovery combination. Keep the central tracker in the first build and introduce DHT/gossip only after the direct system is measurable and stable.[^23]

## Repository layout

```text
modelswarm/
├── Cargo.toml
├── rust-toolchain.toml
├── crates/
│   ├── ms-core/              # IDs, errors, protocol versions, canonical encoding
│   ├── ms-crypto/            # Ed25519, signatures, nonces, capability verification
│   ├── ms-catalog/           # model manifests, HF resolution, digest verification
│   ├── ms-runtime/           # runtime trait and llama.cpp implementation
│   ├── ms-p2p/               # libp2p transport and request protocol
│   ├── ms-scheduler/         # candidate filtering and scoring
│   ├── ms-store/             # SQLite and migrations
│   ├── ms-gateway/           # local OpenAI-compatible Axum API
│   └── ms-telemetry/         # metrics, structured redacted logs
├── apps/
│   ├── modelswarm-node/      # Windows background process
│   ├── modelswarm-desktop/   # Tauri UI
│   └── modelswarm-sim/       # local multi-peer simulation/load generator
├── hub/
│   ├── app/                  # Next.js pages and API routes
│   ├── lib/                  # auth, DB, catalog signatures, validation
│   ├── migrations/
│   └── tests/
├── protocol/
│   ├── msp-v1.md
│   ├── messages.proto
│   └── threat-model.md
├── catalog/
│   ├── schema.json
│   └── candidate-profiles/
├── installer/
├── scripts/
├── tests/
│   ├── integration/
│   └── e2e-windows/
├── docs/
│   ├── architecture.md
│   ├── adr/
│   ├── operations.md
│   └── privacy.md
└── .github/workflows/
```

## Subagent organization

ZCode should create or emulate these bounded specialist agents. Each agent works on a separate branch/worktree, writes a short handoff file, and may not modify another agent’s owned directories without approval.

| Agent | Ownership | Required output |
|---|---|---|
| Architect | `docs/`, protocol boundaries | ADRs, threat model, API contracts, dependency decisions |
| Rust Core | `crates/ms-core`, crypto, store | Versioned types, canonical encoding, key handling, tests |
| Tracker | `hub/` | Vercel app, database schema, catalog and rendezvous APIs |
| P2P | `crates/ms-p2p` | Direct encrypted connection, peer protocol, NAT test notes |
| Runtime | `crates/ms-runtime`, catalog | `llama.cpp` lifecycle, HF download, verification, health metrics |
| Gateway/Scheduler | gateway and scheduler crates | OpenAI-compatible stream proxy, scoring, retries, affinity |
| Windows UX | desktop and installer | Onboarding, model selection, tray controls, installer/update flow |
| QA/Security | tests, CI, security docs | E2E harness, fuzz/property tests, abuse tests, release checklist |

### Coordination rules

- The Architect freezes v0 protocol types before parallel implementation.
- Shared schemas are changed only through an ADR and compatibility review.
- Every subagent starts by reading `AGENTS.md`, `docs/architecture.md`, and relevant ADRs.
- Every subagent must run format, lint, unit tests, and its owned integration tests.
- Every handoff contains changed files, commands run, test evidence, assumptions, unresolved risks, and suggested next task.
- The integrator reviews diffs and merges one agent at a time.
- Agents must not invent API fields independently.
- No agent may claim success without executable test evidence.

## Master ZCode prompt

Paste the following into a new ZCode GLM-5.3 Goal task at the repository root. Use the highest practical reasoning setting and allow terminal/file changes only inside a clean Git worktree.

```text
You are the lead engineer and integrator for ModelSwarm Protocol.

MISSION
Build a Windows-first prototype in which PCs hosting the same exact local AI model profile form a dynamically routed inference swarm. A requester uses one local OpenAI-compatible endpoint. The local gateway discovers eligible hosts through modelswarm.deepflux.space, measures candidates, chooses the best host, connects peer-to-peer, and streams the response. The tracker coordinates discovery and policy but must not carry model files, prompts, or inference streams.

NON-NEGOTIABLE PRODUCT RULE
An installation may consume swarm inference for profile P only while it is actively and verifiably hosting the same exact profile P with at least one advertised serving slot. Enforce this at the tracker, requester, and serving peer. Treat the rule as prototype-grade deterrence, not perfect remote attestation.

TARGETS
- Primary language: Rust stable.
- Client OS: Windows 10/11 x86_64.
- Runtime: bundled, pinned llama.cpp server process bound to loopback.
- Model format: approved GGUF profiles downloaded directly from Hugging Face.
- Hub: Next.js on Vercel at modelswarm.deepflux.space with managed Postgres.
- P2P: rust-libp2p direct encrypted transport; tracker-assisted rendezvous.
- Local API: OpenAI-compatible chat completions with SSE streaming.
- Packaging: Windows NSIS setup.exe through Tauri 2.

CRITICAL ARCHITECTURE CONSTRAINTS
1. Vercel is the control plane, never the inference data plane.
2. Do not silently track Hugging Face main. Resolve every approved model to a full commit hash and exact artifact digest. A changed revision creates a new immutable ModelProfileId and separate swarm.
3. Do not redistribute model weights through ModelSwarm in v0.1. Download directly from Hugging Face and preserve license/attribution.
4. Bind llama.cpp to 127.0.0.1 only. Only the Rust node communicates with it.
5. Never store prompts or completions in the tracker. Redact them from logs.
6. Retry another peer only before the first output token in v0.1.
7. Direct connection failure must be explicit. Do not claim universal NAT traversal until a separate relay is implemented and tested.
8. Prefer a small, testable implementation over speculative DHT, blockchain, payments, KV-cache transfer, or token-level distributed inference.

WORKING METHOD
- First inspect the repository. Do not assume files exist.
- Create AGENTS.md, docs/architecture.md, protocol/msp-v1.md, docs/threat-model.md, and numbered ADRs before implementation.
- Produce a dependency and license inventory.
- Split work among bounded subagents: Architect, Rust Core, Tracker, P2P, Runtime, Gateway/Scheduler, Windows UX, QA/Security.
- Give each subagent explicit owned paths, inputs, acceptance tests, and forbidden scope.
- Use separate branches/worktrees when supported. Require HANDOFF.md from each subagent.
- Integrate sequentially. Resolve interface conflicts centrally; do not let agents create competing schemas.
- After every phase, run tests and record commands/results in docs/verification/<phase>.md.
- Use cargo fmt, cargo clippy with warnings denied for project code, cargo test, hub lint/typecheck/tests, and Windows E2E tests.
- Never disable a failing test merely to pass CI.
- Never put real secrets in source, fixtures, logs, or screenshots.
- Stop and ask for approval before creating paid cloud resources, changing DNS, requesting production credentials, publishing binaries, or accepting a third-party model license.

DELIVERY PHASES
Phase 0: architecture, threat model, protocol, repository scaffold, CI.
Phase 1: hub health/catalog APIs, database migrations, signed catalog, expiring peer leases.
Phase 2: Rust node identity, local store, llama.cpp supervision, one pinned model download and verification.
Phase 3: peer registration, heartbeat, lookup, direct encrypted request/response stream.
Phase 4: local OpenAI-compatible gateway, scheduler, cancellation, retry-before-first-token, circuit breaker.
Phase 5: hosting challenge and short-lived capability token enforcing host-to-consume.
Phase 6: Tauri Windows UI, onboarding, progress, start/stop hosting, tray status, NSIS installer.
Phase 7: three-node E2E tests, adversarial tests, observability, documentation, release candidate.

INITIAL MODEL POLICY
Implement one catalog profile first: an approved Qwen3 4B GGUF Q4_K_M artifact. Do not guess repository revision, filename, SHA-256, size, or llama.cpp build. Add a reproducible admin script that resolves these values from Hugging Face, outputs a candidate manifest, and requires explicit review/promotion. Add 1.7B and 8B profiles only after the 4B profile passes all acceptance tests.

DEFINITION OF DONE
- A fresh Windows VM installs ModelSwarm.
- The user selects the approved model, sees license/source/size, and downloads it directly from Hugging Face.
- The file and full revision are verified.
- llama.cpp starts locally and passes a challenge.
- The node appears at modelswarm.deepflux.space with a short lease.
- Three Windows nodes host the exact profile.
- A local client calls 127.0.0.1:11435/v1/chat/completions and receives a streamed answer from the dynamically selected remote peer.
- Taking the selected host offline before the first token causes a retry to another host.
- Pausing local hosting immediately prevents new swarm consumption after token/lease expiry.
- A node with a different revision or hash cannot join or consume that swarm.
- Prompts and completions do not appear in hub storage or normal logs.
- Build, tests, lints, installer generation, and documented E2E procedure pass.

Begin with Phase 0 only. Show the architecture plan, ADR list, proposed protocol messages, repository tree, subagent assignments, risk register, and exact acceptance tests. Do not implement Phase 1 until Phase 0 is reviewed.
```

## Phase prompts

Run each phase as a separate ZCode Goal after reviewing the previous phase. This limits context drift and makes rollback practical.

### Phase 0 — design and scaffold

```text
Execute Phase 0 from the approved ModelSwarm master specification.

Create only architecture/specification/scaffold work:
- AGENTS.md with ownership and handoff rules.
- Architecture, threat model, privacy model, protocol v1 draft, model-profile schema.
- ADRs covering Vercel boundary, llama.cpp sidecar, tracker-first discovery, Ed25519 identity, immutable HF revisions, capability tokens, retry semantics, and Windows packaging.
- Cargo workspace and hub skeletons that compile but contain no fake production logic.
- CI skeleton for Rust, hub, and Windows packaging dry run.
- A risk register and exact Phase 1 acceptance tests.

Have the Architect and QA/Security subagents independently review the design. Reconcile their findings. Run all scaffold checks. End with a Phase 0 verification report and a clean Git diff. Do not implement tracker behavior yet.
```

### Phase 1 — tracker hub

```text
Implement ModelSwarm Phase 1 using the frozen Phase 0 contracts.

Assign the Tracker subagent ownership of hub/ and the Rust Core subagent ownership of shared protocol fixtures. Build:
- Next.js Vercel project.
- Managed-Postgres migrations for model profiles, peer identities, peer leases, challenges, capability tokens, observations, revocations, and license acceptance.
- GET health and signed catalog endpoints.
- Peer register, heartbeat, drain, and profile-filtered lookup endpoints.
- Strict request schemas, timestamp/nonce replay protection, rate limits, payload limits, and redacted structured logs.
- An admin-only catalog candidate/promote flow; no arbitrary user-added models.
- Lease expiry without relying on a long-lived process.
- Local integration tests using disposable Postgres.

Do not implement inference proxying or model hosting in the hub. The hub must never accept prompt/completion payloads. Add deployment documentation for modelswarm.deepflux.space, but stop before changing DNS or provisioning paid resources. Finish with tests and a handoff report.
```

### Phase 2 — Windows runtime

```text
Implement ModelSwarm Phase 2.

Assign Runtime and Rust Core subagents. Build the Windows Rust node with:
- Persistent Ed25519 installation identity protected through a Windows secret-storage abstraction.
- SQLite state and migrations.
- Signed catalog verification.
- Hugging Face candidate-manifest resolver/admin tool.
- Direct HF artifact download with progress, resume where supported, disk-space preflight, temporary files, atomic finalize, full revision pinning, and SHA-256 verification.
- License/source/size confirmation before download.
- Pinned llama.cpp sidecar acquisition or bundled-resource strategy with checksum verification.
- Child-process lifecycle, loopback-only binding, random internal auth secret, health checks, graceful stop, crash backoff, and metrics extraction.
- One approved Qwen3 4B Q4_K_M profile only.

Use mocks for Hub and llama.cpp in unit tests, then one opt-in real-model integration test. Never insert guessed hashes or revisions. End with evidence that a clean Windows machine can download, verify, start, query, and stop the local model.
```

### Phase 3 — direct P2P

```text
Implement ModelSwarm Phase 3 with the P2P and QA/Security subagents.

Build a versioned rust-libp2p request protocol over encrypted direct transport:
- Peer registration candidate addresses.
- Tracker lookup and rendezvous exchange.
- Handshake containing protocol version, peer ID, model profile ID, runtime compatibility, timestamp, nonce, and signed digest.
- Request admission and bounded queue.
- Streaming events: accepted, token_delta, usage, error, cancelled, completed.
- Backpressure, maximum message sizes, deadlines, cancellation, and connection cleanup.
- Ping/RTT measurements and direct-connect diagnostics.
- No tracker data-plane proxy and no false promise of NAT success.

Create a local simulator that launches at least three peers on distinct ports. Test malformed frames, replay, wrong signatures, wrong profile, expired leases, oversized prompts, cancellation, disconnects, and slow consumers. Produce a NAT/relay gap document before proposing relay work.
```

### Phase 4 — gateway and scheduler

```text
Implement ModelSwarm Phase 4 with the Gateway/Scheduler subagent.

Expose a loopback-only OpenAI-compatible API at 127.0.0.1:11435:
- GET /v1/models.
- POST /v1/chat/completions with stream true and false.
- Normalize requests into the frozen P2P schema.
- Discover exact-profile peers, filter stale/ineligible candidates, actively probe a bounded shortlist, and score expected completion time using measured RTT, queue, prefill, decode, reliability, and staleness.
- Use EWMA measurements persisted locally.
- Add session affinity.
- Add timeout, cancellation, circuit breaker, and retry only before first emitted token.
- Never log prompt or completion text.

Build deterministic scheduler tests and an E2E test showing routing changes when queue/latency conditions change. Report observed timings; do not fabricate benchmark claims.
```

### Phase 5 — host-to-consume enforcement

```text
Implement ModelSwarm Phase 5 with Rust Core, Tracker, and QA/Security subagents.

Enforce the rule that a peer can consume profile P only while actively hosting exact profile P:
- Tracker issues a short-lived signed capability only after active lease, exact artifact identity, live runtime challenge, available slot, and reachability checks.
- Requester refuses local submission without a valid capability.
- Serving peer verifies capability signature, audience/profile/peer binding, expiry, nonce, and revocation state.
- Pausing/draining hosting prevents new capabilities and invalidates consumption after the documented short grace window.
- Different quantization, revision, template, context policy, or digest means a different profile and must fail.
- Add per-peer request/concurrency/token limits.

Document that this is not cryptographic proof of continuous contribution. Add tests for replay, clock skew, copied tokens, stale leases, patched metrics, profile mismatch, and challenge timeout.
```

### Phase 6 — Windows UX

```text
Implement ModelSwarm Phase 6 with the Windows UX and QA/Security subagents.

Create a Tauri 2 Windows application around the Rust node:
- First-run privacy and risk disclosure.
- Login/device registration.
- Approved model catalog with source, license, revision, download size, disk requirement, and hardware suitability.
- Download progress, pause/resume/cancel, and integrity status.
- Hosting on/off, slots, bandwidth and concurrency limits.
- Swarm status, peers, queue, jobs served/consumed, and local API address.
- Clear states for unreachable, ineligible, draining, updating, and model mismatch.
- System tray controls and clean shutdown.
- NSIS setup.exe and uninstall behavior.
- No elevation unless proven necessary.

Keep private keys and tokens out of the webview. Add accessibility labels and actionable errors. Produce a signed-update design, but do not publish or sign production artifacts without explicit approval.
```

### Phase 7 — release candidate

```text
Execute ModelSwarm Phase 7 as an integration and security goal. No new product features.

Have QA/Security lead and all other agents review only their owned boundaries. Validate:
- Fresh Windows 10 and 11 installs.
- Three physical/VM nodes using one exact profile.
- Dynamic best-peer routing under injected queue and latency changes.
- Retry when the chosen host fails before first token.
- Clean interruption after first token.
- Host-to-consume rule and profile mismatch rejection.
- Tracker outage behavior and lease expiry.
- Direct-connect failure messaging.
- Prompt/log privacy.
- Installer, uninstall, rollback, update-signature design, SBOM, dependency licenses, vulnerability audit, fuzz/property tests, and reproducible build notes.

Fix release blockers only. Generate docs/verification/release-candidate.md containing exact commands, environments, observed results, known limitations, and remaining risks. Do not call the release production-ready unless every Definition of Done item has evidence.
```

## Acceptance matrix

| Test | Expected result |
|---|---|
| Same name, different GGUF hash | Peers never share a swarm |
| Same weights, different context policy | Separate profile or explicit rejection |
| Host pauses service | No new consume token after grace/expiry |
| Capability copied to another peer | Rejected because token is peer-bound |
| Tracker unavailable | Existing stream continues; no new peer discovery; clear degraded status |
| Selected peer fails before first token | Gateway retries next eligible peer once or within policy |
| Selected peer fails after tokens begin | Stream ends with explicit interruption; no fabricated continuation |
| Slow consumer | Backpressure prevents unbounded memory growth |
| Huge prompt or frame | Rejected before expensive work |
| Stale/self-inflated metrics | Active measurements and penalties lower peer rank |
| Model update appears on HF | New candidate profile; active swarm does not silently change |
| Gated model selected | User supplies own HF authorization and accepts terms; tracker never stores token |
| Prompt privacy check | No prompt/completion in tracker database or normal logs |
| Direct connection blocked by NAT | Actionable error; no claim that Vercel is relaying traffic |
| Three eligible peers with different load | Scheduler selects lowest predicted completion, not round-robin |

## Delivery sequence

1. Approve the architecture and exact non-goals.
2. Create a clean private Git repository and paste the master prompt into ZCode GLM-5.3 Goal mode.
3. Review Phase 0 before permitting code implementation.
4. Develop against `localhost` and a preview Vercel deployment.
5. Create the managed database and Vercel environment only after migrations/tests are ready.
6. Add `modelswarm.deepflux.space` and its exact CNAME when the hub health endpoint is stable.[^20]
7. Validate one model/profile end to end before expanding the catalog.
8. Add the 1.7B and 8B profiles as separate immutable swarms.
9. Run the three-node Windows release-candidate test.
10. Consider a separately hosted public relay only after collecting real direct-connect failure rates.

## Key risks

| Risk | Mitigation |
|---|---|
| Automatic HF updates split the swarm | Immutable profile IDs and staged promotion |
| Model license blocks redistribution/use | Direct HF download, per-profile license review, acceptance record |
| Vercel treated as permanent relay | Strict control-plane boundary and later standalone relay |
| NAT prevents direct links | Detect clearly; add libp2p Relay/DCUtR in a later milestone |
| Users host but refuse work | Live challenge, reachability requirement, minimum slots; credits later |
| Peers inspect prompts | Start invite-only, disclose risk, encrypt transport, never promise confidential computing |
| Malicious model/runtime artifact | Approved catalog, pinned revisions, checksums, signatures, provenance review |
| Sidecar exposed to LAN | Loopback-only bind plus internal random bearer secret |
| Scheduler trusts fake metrics | Active probes, local EWMA history, circuit breakers |
| Agent-generated interface drift | Frozen schemas, ADRs, owned paths, sequential integration |
| Windows SmartScreen distrust | Code-sign public builds; keep early binaries private and clearly labeled |
| Scope expansion stalls prototype | One model, tracker-first discovery, no marketplace/DHT/cache transfer in v0.1 |

## Final build principle

The prototype succeeds when many independent copies of one immutable model profile behave like one reliable endpoint through **discovery, measurement, routing, and failover**. It does not need to make one token stream use every computer, and it should not solve decentralized economics, universal NAT traversal, or arbitrary model compatibility in the first release.

---

## References

1. [ZCode Docs | GLM-5.3 Agentic Coding Guide - Z.ai](https://zcode.z.ai/en/docs/welcome) - Set up ZCode with GLM-5.3 for agentic coding workflows, long-horizon tasks, long-context development...

2. [Configuring Maximum Duration for Vercel Functions](https://vercel.com/docs/functions/configuring-functions/duration) - The maximum duration configuration determines the longest time that a function can run. run for up t...

3. [WebSockets - Vercel](https://vercel.com/docs/functions/websockets) - Vercel Functions can serve WebSocket connections, keeping a bidirectional connection open between a ...

4. [Vercel Functions Limits](https://vercel.com/docs/functions/limitations) - Large functions support up to 5 GB Beta. Maximum length of 128 characters. Last updated August 24, 2...

5. [Do Vercel Functions support WebSocket connections?](https://vercel.com/kb/guide/do-vercel-serverless-functions-support-websocket-connections) - Yes. WebSocket support for Vercel Functions entered public beta on June 22, 2026, and Python support...

6. [libp2p::tutorials::hole_punching - Rust - Inria](https://wide.gitlabpages.inria.fr/data-wallet-prototype/libp2p/tutorials/hole_punching/index.html) - Hole punching requires a public relay node for the two private nodes to coordinate their hole punch ...

7. [Windows Installer | Tauri](https://v2.tauri.app/distribute/windows-installer/) - Signing cross compiled Windows installers requires an external signing tool. ... msi Windows Install...

8. [Updater - Tauri](https://v2.tauri.app/plugin/updater/) - Tauri's updater needs a signature to verify that the update is from a trusted source. This cannot be...

9. [GGUF usage with llama.cpp - Hugging Face](https://huggingface.co/docs/hub/en/gguf-llamacpp) - Llama.cpp allows you to download and run inference on a GGUF simply by providing a path to the Huggi...

10. [ggml-org/llama.cpp: LLM inference in C/C++ - GitHub](https://github.com/ggml-org/llama.cpp) - Download and run a model directly from Hugging Face llama cli. The main goal of llama.cpp is to enab...

11. [Licenses - Hugging Face](https://huggingface.co/docs/hub/en/repositories-licenses) - The license can be specified in your repository's README.md file, known as a card on the Hub, in the...

12. [Model Cards - Hugging Face](https://huggingface.co/docs/hub/en/model-cards) - You can specify the license in the model card metadata section. The license will be displayed on the...

13. [ggml-org](https://huggingface.co/ggml-org/collections) - ggml-org on Hugging Face, the AI community building the future. Gemma 3 ggml-org/gemma-3-270m-it-GGU...

14. [.gitattributes · ggml-org/Qwen3-4B-GGUF at ...](https://huggingface.co/ggml-org/Qwen3-4B-GGUF/blame/becf9571b8497476e4b3cd12908c66cf456a57bc/.gitattributes) - Instructions to use ggml-org/Qwen3-4B-GGUF with libraries, inference providers, notebooks, and local...

15. [Download files from the Hub - Hugging Face](https://huggingface.co/docs/huggingface_hub/en/guides/download) - Download files from the Hub. The huggingface_hub library provides functions to download files from t...

16. [huggingface/hf-hub - GitHub](https://github.com/huggingface/hf-hub) - Rust client for the Hugging Face Hub API. hf-hub provides a typed, ergonomic interface for interacti...

17. [hf_hub - Rust - Docs.rs](https://docs.rs/hf-hub) - Async Rust client for the Hugging Face Hub API — the Rust counterpart to the Python huggingface_hub ...

18. [API2:2023 Broken Authentication - OWASP API Security Top 10](https://owasp.org/API-Security/editions/2023/en/0xa2-broken-authentication/) - Login attempts are subject to restrictive rate limiting: only three requests are allowed per minute....

19. [API4:2019 Lack of Resources & Rate Limiting - GitHub](https://github.com/OWASP/API-Security/blob/master/editions/2019/en/0xa4-lack-of-resources-and-rate-limiting.md) - It's common to find APIs that do not implement rate limiting or APIs where limits are not properly s...

20. [Adding & Configuring a Custom Domain - Vercel](https://vercel.com/docs/domains/working-with-domains/add-a-domain) - You can configure subdomains with a CNAME record. Each project has a unique CNAME record e.g. d1d4fc...

21. [Setting up a custom domain - Vercel](https://vercel.com/docs/domains/set-up-custom-domain) - 1. Check your existing domains · 2. Add the domain to your project · 3. Check what DNS records are n...

22. [Postgres on Vercel](https://vercel.com/docs/postgres) - Vercel Postgres is no longer available. If you had an existing Vercel Postgres database, we automati...

23. [libp2p_gossipsub - Rust](https://libp2p.github.io/rust-libp2p/libp2p_gossipsub/index.html) - Gossipsub is a P2P pubsub (publish/subscription) routing layer designed to extend upon floodsub and ...

