# ModelSwarm Protocol (MSP)

Windows-first prototype in which PCs hosting the **same exact local AI model profile**
form a dynamically routed inference swarm. A user points any OpenAI-compatible client
at one local endpoint (`http://127.0.0.1:11435/v1`); the local gateway discovers
eligible hosts via `modelswarm.deepflux.space`, measures candidates, selects the best
peer, and streams the answer over an encrypted direct P2P connection.

**Core rule:** an installation may consume swarm inference for profile P only while it
is actively and verifiably hosting the exact same profile P.

## Status

Phase 0 — design and scaffold. See `docs/verification/phase-0.md` for the current
verification report. No runtime functionality exists yet, by design.

**Plan of record:** the project has three planning documents, newest governs
where they conflict (formal map pending ADR-015):

1. `docs/build-plan.md` — original prototype plan (phases 0–7). Phase 0 done.
2. `docs/cooperative-plan.md` — cooperative-inference research (phases P0–P8),
   gated on the original prototype's completion.
3. `docs/competitive-revision-plan.md` — competitive-learnings revision
   (phases A–G): repositions ModelSwarm as network-aware micro-swarms of
   exact-profile peers cooperatively accelerating one request; renames the
   workspace layout; derives `ModelProfileId` from a manifest.

The revision's Phase A first response (repository audit, unchanged/redesign
map, p2ptokens recommendation, proposed ADR-009…016, task graph) is at
`docs/reviews/phase-a-first-response.md`. **No Phase A work has started** —
it begins only on approval of that response.

## Repository map

| Path | Purpose |
|---|---|
| `docs/` | Architecture, threat model, privacy model, ADRs, risk register, verification reports |
| `protocol/` | MSP v1 protocol draft (`msp-v1.md`), message schema (`messages.proto`) |
| `catalog/` | Immutable model-profile JSON schema + reviewed candidate profiles |
| `crates/` | Rust workspace libraries (core, crypto, catalog, runtime, p2p, scheduler, store, gateway, telemetry) |
| `apps/` | Windows node daemon, desktop UI (Phase 6), multi-peer simulator |
| `hub/` | Next.js tracker application deployed on Vercel (control plane only) |
| `tests/` | Integration and Windows E2E tests |
| `installer/` | Windows packaging (NSIS via Tauri, Phase 6) |
| `.github/workflows/` | CI: Rust checks, hub checks, packaging dry run |

## Development

```text
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Hub (Phase 1+): `cd hub && npm ci && npm run typecheck && npm run build`

Read `AGENTS.md` before contributing. The authoritative build plan lives in
`docs/build-plan.md`.
