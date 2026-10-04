# ModelSwarm Threat Model

Status: Phase 0 draft, owned by the Architect with QA/Security review.
Scope: MSP v0.1 prototype (Windows nodes + Vercel tracker hub).

## 1. System under analysis

See `docs/architecture.md`. Surfaces an adversary can touch:

1. Hub REST API (`modelswarm.deepflux.space`) — unauthenticated + authenticated.
2. P2P streams between peers (QUIC/Noise, internet-reachable listeners).
3. Loopback llama.cpp sidecar (127.0.0.1, random port, bearer secret).
4. Local node files (SQLite, cached GGUF, logs) and Windows credential store.
5. The user's local OpenAI-compatible gateway on 127.0.0.1:11435.
6. Supply chain: GGUF artifacts, llama.cpp builds, npm/cargo dependencies.

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

### 4.3 Local node

| Attack | Mitigation (v0.1) |
|---|---|
| Sidecar reachable from LAN | 127.0.0.1-only bind + random bearer secret |
| Key theft from disk | Windows Credential Manager/DPAPI; never in webview or config files |
| Malicious local app using the gateway | Gateway is loopback-only; v0.1 accepts any local client (documented limitation; local auth token is a candidate hardening) |
| Log leakage of prompts | `modelswarm-telemetry` redaction is mandatory; QA test asserts absence |
| Malicious GGUF/runtime | Only catalog-approved, digest-pinned artifacts from HF; pinned llama.cpp build with checksum |

### 4.4 Supply chain

Catalog immutability (ADR-005) means a node never executes a model or runtime
that was not resolved to a full commit hash + digest by an admin and validated
by test nodes. Dependency audits (cargo audit / npm audit) run in CI from
Phase 1 onward.

## 5. Explicit non-guarantees (v0.1)

1. **No remote attestation.** A patched binary can claim to host while not
   hosting, or serve while paused. The capability-token system raises effort
   and creates auditable evidence; it does not eliminate lying.
2. **No confidential computing.** The serving peer reads every prompt it
   serves. The UI discloses this at onboarding; the swarm is invite-only in
   v0.1.
3. **No Sybil resistance** beyond invite-only accounts + per-account limits.
4. **No universal NAT traversal.** Direct failure is possible and must be
   reported honestly; relay/DCUtR is a later milestone.
5. **No protection of the local machine from its own user** — a user who
   controls the OS controls the node.

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
