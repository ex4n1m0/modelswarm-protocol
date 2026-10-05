# ModelSwarm Protocol (MSP)

Windows-first prototype in which PCs hosting the **same exact local AI model profile**
form a dynamically routed inference swarm. A user points any OpenAI-compatible client
at one local endpoint (`http://127.0.0.1:11435/v1`); the local gateway discovers
eligible hosts via `modelswarm.deepflux.space`, measures candidates, selects the best
peer, and streams the answer over an encrypted direct P2P connection.

**Core rule:** an installation may consume swarm inference for profile P only while it
is actively and verifiably hosting the exact same profile P.

## Status

Execution status 2026-10-05 (phases A–G of `docs/competitive-revision-plan.md`;
verification reports under `docs/verification/`):

| Phase | Status | Evidence |
|---|---|---|
| 0 scaffold + contracts | done | `phase-0.md` (commit 76397b6) |
| A audit & rebaseline | done | `phase-a.md` (9d5a3a3) |
| B protocol foundation + tracker | done | `phase-b.md` (e61b652, 070e8f6) |
| C runtime/scheduler/gateway/transport/bench | done | `phase-c.md` (e6834e3, f792fa0) |
| D exact two-peer speculation | done | `phase-d.md` (e9b48a8, ad675d2) |
| E multi-proposer token trees | done | `phase-e.md` (248aa05, e5420c6) |
| F hostile-swarm hardening | done (F11 fixed post-phase) | `phase-f.md` |
| G node + Tauri shell (unsigned) | done | `phase-g.md` (d27cf78) |

Workspace: **256+ tests, 0 failures, clippy `-D warnings` clean**; tracker:
78 vitest + typecheck/build/vector-parity green. The correctness contracts
hold end-to-end on loopback (greedy speculative == plain decoding under
adversarial drafts, proposers, and fallback; sampled distribution
preservation proven at unit level). **Every performance number is
TEST-ONLY mock/loopback** (ADR-019): no real-model claim exists; real
claims await the research runtime + an approved GGUF profile (stop
conditions intact). **Live: https://modelswarm.deepflux.space** (tracker, preview-grade env —
see `docs/deployment.md`). Known deployment gaps: production env vars
(Neon `DATABASE_URL`, stable `HUB_SIGNING_KEY`, `ADMIN_TOKEN`), code
signing (G stop), D/E-suite reparameterization over the libp2p backend.

**Plans:** `docs/build-plan.md` (original 0–7) → `docs/cooperative-plan.md`
(P0–P8) → `docs/competitive-revision-plan.md` (**A–G governs**; map in
ADR-015).

## Repository map

| Path | Purpose |
|---|---|
| `docs/` | Architecture, threat model, privacy model, ADRs, risk register, research specs, verification reports |
| `protocol/` | MSP v1 contract (`msp-v1.md`), message schema (`messages.proto`), golden vectors (`vectors/`) |
| `catalog/` | Model-profile schemas (v1; manifest-based v2 per ADR-011) + reviewed candidates |
| `crates/` | Rust workspace: `modelswarm-{types, identity, transport, tracker-api, eligibility, runtime, scheduler, session, speculation, store, gateway, telemetry, bench, node, desktop}` |
| `apps/tracker/` | Next.js tracker on Vercel (control plane only) |
| `apps/modelswarm-sim/` | Multi-peer simulator |
| `experiments/` | Benchmark manifests, frozen metric schemas, raw/processed results, reports |
| `tests/` | Integration and Windows E2E tests |
| `installer/` | Windows packaging (NSIS via Tauri, Phase G) |
| `.github/workflows/` | CI: rust, tracker, schemas, packaging dry run |

## Development

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tracker (Phase B+): `cd apps/tracker && npm ci && npm run typecheck && npm run build`
Vectors: `cd apps/tracker && npm run validate:vectors`

Read `AGENTS.md` before contributing. The authoritative build plan lives in
`docs/build-plan.md`.
