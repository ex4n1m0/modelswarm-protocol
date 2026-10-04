# AGENTS.md — ModelSwarm Working Rules

This file governs every human and AI agent contributing to ModelSwarm Protocol (MSP).
Read it, `docs/architecture.md`, and every ADR under `docs/adr/` **before** writing code.

## Project rule (non-negotiable)

> An installation may consume swarm inference for model profile P only while it is
> actively and verifiably hosting the same exact profile P with at least one
> advertised serving slot.

"Exact" means: same Hugging Face repository, full revision commit hash, GGUF filename,
artifact SHA-256, quantization, context policy, and compatible pinned runtime — not
merely the same model name. This rule is enforced at the tracker, at the requesting
gateway, and at the serving peer. It is prototype-grade deterrence, not remote
attestation (see `docs/threat-model.md`).

## Hard architectural constraints

1. Vercel (`modelswarm.deepflux.space`) is the **control plane only**. It never carries
   model files, prompts, completions, or inference streams.
2. Never silently track a Hugging Face `main` branch. Every approved artifact resolves
   to a full commit hash + exact file digest. A changed revision is a **new immutable
   `ModelProfileId`** and a separate swarm.
3. No redistribution of model weights through ModelSwarm in v0.1. Downloads go directly
   from Hugging Face to the node, with license/attribution preserved.
4. `llama.cpp` binds to `127.0.0.1` only, with a random internal bearer secret. Only the
   Rust node talks to it.
5. Prompts and completions are never stored in the hub and are redacted from all logs.
6. Retry to another peer is allowed **only before the first output token** (v0.1).
7. Direct-connect failure must be reported explicitly. No claims of universal NAT
   traversal until a relay exists and is tested.
8. No speculative DHT, blockchain, payments, KV-cache transfer, or token-level
   distributed inference in v0.1.

## Phase gate

Each phase ends with `docs/verification/<phase>.md` containing exact commands and
observed results. The next phase does not start until the previous one is reviewed.
Phase 0 (this state of the repo) is design + scaffold only: **no tracker behavior, no
inference, no fake "working" code.**

## Follow-up track: cooperative inference (gated)

`docs/cooperative-plan.md` specifies an experimental follow-up program
(phases P0–P8: baseline freeze, transformer internals, speculative decoding,
multi-proposer trees, adaptive planner, search mode, adversarial resilience,
Windows integration). It does **not** modify the rules above; it adds:

- **Prerequisite gate.** No cooperative phase, crate, agent, or protocol
  message may be implemented until the original prototype demonstrably
  provides: three-node exact-profile hosting, exact-profile peer lookup,
  direct encrypted streams, a correctly streaming local gateway, a recorded
  fastest-peer baseline (TTFT/ITL/throughput/completion/bytes), working
  host-to-consume eligibility, and verified prompt/completion absence from
  tracker storage and logs. Until then, cooperative work is out of scope.
- When P0 begins, the Integrator appends its subagents and owned paths
  (`crates/ms-coop-protocol`, `ms-decode`, `ms-planner`, `ms-verification`,
  `ms-network-model`, `apps/ms-bench`, `apps/ms-coop-sim`, `research/`,
  `experiments/`) to the ownership map above under the same rules: ADR-gated
  shared-schema changes, `HANDOFF.md` per agent, sequential integration, and
  no invented fields outside `protocol/msp-cooperative-v1.md`.
- Scientific rule for the follow-up: every cooperative claim compares against
  the **fastest eligible single host at that moment**, never an average or
  slower host; slowdowns are reported as visibly as speedups.

## Ownership map

| Agent | Owned paths | Output required |
|---|---|---|
| Architect | `docs/`, `protocol/`, `AGENTS.md` | ADRs, threat model, protocol contracts |
| Rust Core | `crates/ms-core`, `crates/ms-crypto`, `crates/ms-store` | Versioned types, canonical encoding, key handling, tests |
| Tracker | `hub/` | Vercel app, migrations, catalog + rendezvous APIs |
| P2P | `crates/ms-p2p` | Direct encrypted transport, peer protocol, NAT notes |
| Runtime | `crates/ms-runtime`, `crates/ms-catalog`, `scripts/` | llama.cpp lifecycle, HF download, verification, metrics |
| Gateway/Scheduler | `crates/ms-gateway`, `crates/ms-scheduler` | OpenAI-compatible proxy, scoring, retries, affinity |
| Windows UX | `apps/modelswarm-desktop`, `installer/` | Onboarding, model management, tray, installer |
| QA/Security | `tests/`, `.github/workflows/`, security docs | E2E harness, fuzz/property tests, abuse tests, release checklist |

`apps/modelswarm-node` is owned by Rust Core; `apps/modelswarm-sim` is owned by QA/Security.

## Coordination rules

- The Architect freezes v0 protocol types (in `protocol/`) before parallel
  implementation begins. Shared schemas change only through an ADR.
- Every agent starts by reading this file, `docs/architecture.md`, and relevant ADRs.
- An agent may not modify another agent's owned paths without the Architect's written
  approval recorded in the ADR log for that change.
- Agents must not invent API fields; anything not in `protocol/msp-v1.md` or
  `catalog/schema.json` needs an ADR first.
- Every agent runs `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
  `cargo test --workspace`, and its owned integration tests before handoff
  (hub work: `npm run typecheck` + `npm run build` inside `hub/`).
- No agent may claim success without executable test evidence.
- Never disable a failing test to pass CI. Never commit real secrets, tokens, keys,
  prompts, or model weights.
- Stop and ask the human owner before: creating paid cloud resources, changing DNS,
  requesting production credentials, publishing binaries, or accepting a third-party
  model license.

## Handoff format

Every handoff is a file `docs/reviews/handoff-<agent>-<date>.md` containing:

1. Changed files (paths) and why.
2. Exact commands run and their outcomes.
3. Test evidence (command + summary).
4. Assumptions made.
5. Unresolved risks.
6. Suggested next task for the integrator.

The integrator merges one agent's branch at a time and resolves interface conflicts
centrally.
