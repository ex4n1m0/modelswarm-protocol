# ModelSwarm Architecture

Status: Phase 0 draft, owned by the Architect. Changes require an ADR.
Authoritative source for structure; wire-level detail lives in
`protocol/msp-v1.md`; security analysis in `docs/threat-model.md`.

## 1. Purpose and core rule

ModelSwarm lets Windows PCs that each host the **same exact local AI model
profile** behave as one reliable inference endpoint through discovery,
measurement, routing, and failover. One local OpenAI-compatible endpoint
(`http://127.0.0.1:11435/v1`) fronts a dynamically selected remote peer.

Core rule (enforced at tracker, requester, and serving peer):

> An installation may consume swarm inference for profile P only while it is
> actively and verifiably hosting the same exact profile P with at least one
> advertised serving slot.

"Exact" = same HF repository, full 40-char revision commit hash, GGUF filename,
artifact SHA-256, quantization, context policy, and compatible pinned runtime.

## 2. System overview

```text
                          modelswarm.deepflux.space
                    Vercel web app + tracker API + Postgres
              accounts · catalog · peer rendezvous · policy · leases
                                  │
                    HTTPS heartbeat / lookup / signaling
                                  │
               ┌──────────────────┴──────────────────┐
               │                                     │
       Windows ModelSwarm Node A             Windows ModelSwarm Node B
       Rust node + llama.cpp sidecar          Rust node + llama.cpp sidecar
       host + client + scheduler              host + client + scheduler
               │                                     │
               └──── direct encrypted P2P ───────────┘
                  (rust-libp2p QUIC + Noise)
                    prompts · token stream · cancellation

        Hugging Face Hub ── direct download ──► each Windows node
```

## 3. Planes and boundaries

| Plane | Responsibility | Location |
|---|---|---|
| Catalog | Approved profiles, exact revisions, digests, licenses, min runtime | Vercel hub + Postgres |
| Identity | User account, installation identity, Ed25519 peer key | Hub + local Windows credential storage |
| Rendezvous | Peer registration, heartbeats, candidate lookup, signaling | Vercel hub |
| P2P transport | Encrypted direct connection, request, stream, cancellation | Rust nodes |
| Inference | Load GGUF, prefill, decode, stream tokens | Local `llama.cpp` sidecar |
| Model acquisition | Download approved artifacts, verify revision/digest | HF → node, direct |
| Scheduling | Filter peers, pick lowest predicted completion time | Requesting node |
| Accounting | Hosting proof, jobs served/consumed, failure/latency stats | Local node + signed hub events |

**The hub never carries:** model files, prompts, completions, inference
traffic, or Hugging Face credentials. Vercel function lifetimes are bounded, so
no architecture component may require a long-lived hub connection; peers
heartbeat over HTTPS every 15–30 s with 60–90 s leases (ADR-001). WebSockets on
Vercel remain pinned to a function's lifetime and are suitable only for brief
signaling, never as a relay.

## 4. Node components (Rust workspace)

Naming and phases updated Phase A per ADR-010/015 (A–G phases).

| Crate | Role | Phase |
|---|---|---|
| `modelswarm-types` | Protocol version, typed IDs, manifest types (ADR-011), canonical validation | 0, B |
| `modelswarm-identity` | Ed25519 identity, signatures, nonces, lease verification | B |
| `modelswarm-transport` | libp2p QUIC/Noise transport, request protocol, streaming; relay/DCUtR in F (ADR-014) | C, F |
| `modelswarm-tracker-api` | Typed client for the tracker REST API | B |
| `modelswarm-eligibility` | Eligibility leases, capacity classes, audit epochs, suspension | B |
| `modelswarm-runtime` | Runtime trait + llama.cpp sidecar; HF artifact acquisition | C |
| `modelswarm-scheduler` | Candidate filtering, cost model v2 (ADR-013), mode selection | C+ |
| `modelswarm-session` | Cooperative session state machine, commits, rollback, peer replacement | D |
| `modelswarm-speculation` | Exact proposer/verifier algorithms, candidate trees | D–E |
| `modelswarm-store` | SQLite state, migrations, EWMA observations, accounting | B |
| `modelswarm-gateway` | OpenAI-compatible loopback API, SSE, retry policy | C |
| `modelswarm-telemetry` | Redacted structured logs, metrics | B+ |
| `modelswarm-bench` | Benchmark harness + network emulation (ADR-013 schemas) | C |
| `modelswarm-node` | Windows daemon composition | B+ |
| `modelswarm-desktop` | Tauri 2 UI | G |
| `modelswarm-relay` | Circuit-relay v2 host (ADR-014); F2A verified 2026-10-08, node-side usage (F2b Swarm migration) pending; internet exposure blocked until auth + per-circuit caps | F |
| `modelswarm-winjob` | Windows job objects: kill-on-close, total-RAM detection (the M2 governor seed; governor generalizes cross-platform) | G |

Apps: `apps/tracker` (control plane), `apps/modelswarm-sim` (multi-peer
simulator, Phase C).

## 5. Identity model

- **User account** on the hub (device authorization flow; no passwords in the
  node).
- **Installation identity**: one Ed25519 keypair generated on first run, stored
  via Windows Credential Manager/DPAPI abstraction, never in the webview, never
  in plaintext files (ADR-004). The libp2p `PeerId` derives from this key.
- **Every hub-mutating request** is signed: timestamp, nonce, installation id,
  body digest. The hub enforces replay windows.
- **Capability tokens** are short-lived hub-signed statements binding a peer to
  a profile with `canHost`/`canConsume` (ADR-006).

## 6. Model profiles and the catalog

A profile (schema: `catalog/schema-v2.json`, per ADR-011/022) is immutable:
the manifest-derived `profileId` (`msp1:` + hash over the GGUF-anchored
identity fields), `hf_repo`, `hf_revision` (full 40-char hash), `filename`,
`sha256`, byte size, `quantization`, `runtime {name, build_hash}` — plus
the ADR-022 identity-hash block and license evidence URL. The v1 shape
(`catalog/schema.json`, camelCase, `contextTokens`/`maxOutputTokens`,
`licenseId`) is dead-but-retained; `maxTokens` today uses the global
wire cap (ADR-028). `status ∈ {candidate, active, deprecated}`.

Lifecycle: hub detects new HF revision → admin promotes to `candidate` → test
nodes validate → hub publishes a **new immutable profile id** → nodes download
in background while still serving old → when enough hosts are ready, new
becomes `active`, old `deprecated`. Requests never cross profile ids. Old files
are deleted only with user consent after no active swarm depends on them.
Nothing ever resolves to a moving branch (ADR-005).

## 7. Eligibility and capability tokens

To qualify for profile P a node must: hold the exact artifact (digest
verified), start it with the approved runtime, pass a local challenge prompt
within deadline, hold an active tracker lease for P, advertise ≥1 serving slot,
accept inbound connections, not be draining/paused/suspended, and have accepted
the license + network privacy disclosure.

After a successful hosting challenge the tracker issues a short-lived signed
capability token (see `protocol/msp-v1.md` §Capability tokens). Every inference
request carries it; the serving peer independently verifies signature, expiry,
profile binding, and requesting peer identity. The token `nonce` is for
issuance-dedup and revocation only — per-request replay protection is the
single-use `requestId` (msp-v1 §5, §6.6). Pausing hosting stops new
token issuance and blocks consumption after the documented grace window.

This is deterrence against casual misuse, not remote attestation; a determined
attacker can patch the binary. Documented in `docs/threat-model.md` §Non-guarantees.

## 8. Runtime: llama.cpp sidecar

Bundled, version-pinned `llama.cpp` server process (ADR-002): spawned by the
Rust node, bound to `127.0.0.1` on a random port with a random internal bearer
secret, health-checked, gracefully stopped, restarted with crash backoff. The
node extracts prefill/decode rate and queue metrics from it. Only the Rust node
may talk to it; the GGUF file and runtime never leave the machine.

## 9. Discovery and rendezvous

1. Node verifies installation key, fetches and verifies the signed catalog.
2. Node registers candidate addresses (public observed + local) and heartbeats.
3. Requester asks the tracker for candidates filtered by profile id.
4. Tracker-assisted signaling (short offer/answer exchange) lets peers attempt
   a direct QUIC connection; if hole punching fails, the failure is reported
   explicitly as a direct-connect failure. A standalone circuit-relay v2 host
   now exists (`crates/modelswarm-relay`, ADR-014) and was verified 2026-10-08
   (F2A, `docs/verification/f2a-relay-2026-10-08.md`); nodes do not use it
   yet — that rides the F2b libp2p Swarm migration, and internet exposure is
   blocked until relay auth + per-circuit caps land. No universal
   NAT-traversal claims (ADR-014).

## 10. Gateway and scheduling

The gateway normalizes OpenAI chat-completion requests into the frozen P2P
schema, then scores eligible peers:

```text
predicted_ms =
    measured_rtt_ms
  + advertised_queue_ms
  + prompt_tokens / measured_prefill_tokens_per_ms
  + expected_output_tokens / measured_decode_tokens_per_ms
  + failure_penalty_ms
  + stale_advertisement_penalty_ms
```

Requester-measured values (EWMA, persisted locally) dominate self-reported
ones. Session affinity keeps a conversation on one peer when healthy. Retry to
another peer is allowed only before the first output token; after that the
stream terminates with an explicit interruption (ADR-007).

## 11. Failure handling

| Failure | Behavior |
|---|---|
| Tracker unreachable | Existing streams continue; no new discovery; UI shows degraded status |
| Host fails before first token | Gateway retries next eligible peer per policy |
| Host fails after first token | Stream ends with explicit interruption; no fabricated continuation |
| Slow consumer | Backpressure; bounded buffers |
| Oversized prompt/frame | Rejected before expensive work |
| Fake/self-inflated metrics | Active probing + local EWMA + circuit breakers de-rank |
| NAT blocks direct connect | Actionable error; no claim that Vercel relays |

## 12. Privacy boundaries

Prompts/completions exist in plaintext only: (a) in the requesting
application's original HTTP call, (b) in node RAM during normalization/proxy,
(c) in the serving peer's RAM and its loopback llama.cpp call. On the wire they
are libp2p-encrypted end to end. They are never written to disk, never sent to
the hub, never logged (redaction enforced in `ms-telemetry`). The serving peer
**can read prompts it serves** — disclosed to users; confidential computing is
out of scope. Details: `docs/privacy.md`.

## 13. Deployment topology

- **Tracker**: Next.js on Vercel (project `modelswarm-tracker`, formerly `modelswarm-hub`), custom domain
  `modelswarm.deepflux.space` (CNAME target read from the Vercel project
  settings, never guessed). Postgres via a managed provider (e.g. Neon) through
  the Vercel Marketplace.
- **Node**: Windows 10/11 x64; NSIS `setup.exe` via Tauri 2 (ADR-008);
  unsigned internal-test builds until a signing process exists.
- **Models**: downloaded directly from Hugging Face with pinned revisions.

## 14. Phase map

> Historical (original 0-7 plan). The executed phases are A-G per
> ADR-015's supersession map, plus H (real-model Windows client), I
> (follow-ups), and F0-F3 (P2P serving) — see `docs/verification/`.

| Phase | Scope |
|---|---|
| 0 | This document, ADRs, protocol draft, schemas, skeletons, CI, risk register |
| 1 | Hub: catalog/peer/lease/rendezvous APIs, migrations, signed catalog |
| 2 | Node: identity, store, HF download+verify, llama.cpp supervision |
| 3 | P2P: registration→rendezvous→encrypted request/stream, simulator |
| 4 | Gateway + scheduler + retry semantics |
| 5 | Capability tokens: host-to-consume enforcement end to end |
| 6 | Tauri UI, onboarding, tray, installer |
| 7 | Three-node E2E, adversarial tests, observability, release candidate |

## 15. Decisions index

ADR-001 Vercel control-plane boundary · ADR-002 llama.cpp sidecar ·
ADR-003 tracker-first discovery · ADR-004 Ed25519 installation identity ·
ADR-005 immutable HF revisions · ADR-006 capability tokens ·
ADR-007 retry-before-first-token · ADR-008 Windows packaging via Tauri/NSIS ·
ADR-009 revised positioning and differentiation · ADR-010 workspace
restructure · ADR-011 manifest-derived profile id · ADR-012 eligibility
lease · ADR-013 execution modes and cost model v2 · ADR-014 NAT traversal
roadmap · ADR-015 plan supersession map (A–G governs; superseded as
governing plan by ADR-027) · ADR-016 **conditionally reserved Phase A slot,
never issued** — closed as not-needed ("no code reuse adopted"), see
`docs/verification/phase-a.md`; numbering continued at 017 · ADR-017 node
persistence (rusqlite) · ADR-018 transport staging · ADR-019 runtime
adapters and benchmark honesty · ADR-020 identity derivations · ADR-021
research runtime decision · ADR-022 real profile registration
(GGUF-anchored hashes) · ADR-023 community model requests · ADR-024 GPU
engine variant · ADR-025 chat-template application point · ADR-026
hub-signed lease gate at session open · ADR-027 plan supersession — master
roadmap M0–M13 governs · ADR-028 tracker-surface and wire reconciliation ·
ADR-029 execution presets and the msp-cooperative-v1 namespace · ADR-030
receipts v2 (counter-signed work receipts, granted accounting).
