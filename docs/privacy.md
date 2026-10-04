# ModelSwarm Privacy Model

Status: Phase 0 draft. Feeds the first-run privacy disclosure (Phase 6) and
the QA privacy audit (Phase 7).

## 1. Where data lives

| Store | Contains | Never contains |
|---|---|---|
| Node SQLite (`ms-store`) | Installation state, license acceptances, profile cache, EWMA performance observations, job accounting (timings/outcomes only) | Prompts, completions, conversation text |
| Hub Postgres | Accounts, installations, public keys, profile catalog, leases, challenges, capability-token records, signed job-result events (timings/outcomes) | Prompts, completions, HF tokens, raw IPs beyond lease lifetime |
| Node logs | Structured events with mandatory redaction via `modelswarm-telemetry` | Prompt/completion text, keys, tokens, bearer secrets |
| Windows credential store | Ed25519 private key, internal sidecar secret, (optional) HF token | — |
| Hugging Face | Receives authenticated artifact downloads only, direct from the node | Prompts, completions, swarm metadata |

## 2. Prompt lifecycle

1. A local application POSTs a chat completion to `127.0.0.1:11435`.
2. The gateway normalizes it in RAM into the frozen P2P schema.
3. It is transmitted inside a libp2p Noise-encrypted stream to the serving peer.
4. The serving peer proxies it over loopback to llama.cpp and streams tokens
   back through the same encrypted stream.
5. When the stream ends, plaintext is gone: no persistence at either peer,
   no logging at either peer, no hub visibility at any point.

Redaction is enforced centrally in `modelswarm-telemetry`; crates do not implement
their own logging. QA asserts absence of prompt text in hub DB, node SQLite,
and logs (Phase 7 acceptance).

**User-facing disclosure:** the serving peer — another person's computer — can
technically read any prompt it serves. This is stated plainly at onboarding.
Mitigation posture: invite-only swarm in v0.1, encrypted transport, no
confidential-computing claims.

**Planned change (follow-up track, gated):** the experimental cooperative
modes in `docs/cooperative-plan.md` would deliver the prompt to a micro-swarm
of 2–8 peers instead of one, multiplying exposure. If and when that track
passes its prerequisite gate, this document must be updated with the exact
recipient set per mode and the onboarding disclosure reworded before any
cooperative request is served.

## 3. Secrets handling

- Ed25519 private key: generated on-device, stored via DPAPI/Credential
  Manager, exfiltrated never; the Tauri webview never receives it.
- Sidecar bearer secret: random per launch, loopback only.
- Hub session credentials: short-lived, OS-protected storage.
- Hugging Face token: optional, only for gated models, stored locally via the
  same OS-protected path, sent only to huggingface.co over HTTPS, never to the
  hub, never logged. The plan-level promise holds: "user supplies own HF
  authorization; tracker never stores token."

## 4. Network metadata

The hub sees: account id, installation id, peer ids, profile ids, addresses
advertised during a lease, timing/outcome events. This is routing metadata
required by the rendezvous function. Lease rows expire; historical metadata
beyond accounting aggregates is not retained longer than the operations runbook
requires (retention constants set in Phase 1 migrations).

## 5. Telemetry

v0.1 ships **no crash reporting or product analytics**. Diagnostics are local
files the user can inspect and export. If telemetry is ever added, it requires
a new ADR and explicit opt-in.
