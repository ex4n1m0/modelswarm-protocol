# Audit — Rewrite Boundaries — 2026-10-09

Answers the master-prompt §8 question: does anything justify a rewrite?
**No.** Evidence: the salvage matrix shows zero REPLACE-at-component-scale,
zero QUARANTINE, one DELETE (a dead IPC command). The frozen core (lease
wire, envelope signing, manifest identity, catalog pinning, ADR-026 gate)
is byte-consistent end to end with CI parity — rewriting it would destroy
verified correctness for no gain. A full rewrite would additionally
discard the test-estate (301+313+7 Rust, 110 TS, 3-sided golden vectors,
wire-compat CI) that is this repo's strongest asset.

## Where "replacement" IS sanctioned (interface-preserving, incrementally)

| Boundary | What changes | Protected interface | Gate |
|---|---|---|---|
| `transport::libp2p_backend` at F2b | raw-QUIC implementation → libp2p Swarm (relay + DCUtR capable) | the public transport surface consumed by serving/remote (send/recv deadline semantics, `remote_peer_id()`) | ADR-018 gate re-runs; LAN re-proof |
| `llamacpp.rs` decode path | per-token POST loop → true SSE streaming | the executor event stream + exact-id recovery contract (fail-closed id+bytes) | regression-pinned greedy vectors; measured TTFT/ITL before/after |
| `CHALLENGE_PROMPT` constant | static replayable prompt → nonce-bound + digest-verified canary | challenge route shapes; content-blindness | tracker tests + wire-compat; E0 determinism dependency |
| `app.rs` monolith (2,301 ln) | extract modules (chat, lease, hardware, cards) behind the existing IPC command surface | IPC command names/shapes (UI compatibility) | tauri-shell clippy gate added FIRST so refactors are linted |
| `libp2p_backend` selection order | alphabetical-first → wired `select_microswarm` + EWMA | gateway call shape | shadow mode before action (expanded-mission §5) |

## Non-negotiable invariants any refactor must preserve

1. The project rule (exact-profile host-to-consume) enforced at tracker,
   requester, and serving peer (ADR-026 gate stays at session open).
2. Content-blind hub — no prompt/completion fields in any route, log,
   or migration, ever (artifact-pointer-only schemas).
3. Pinned revisions → immutable profile IDs; nothing resolves to a
   moving branch.
4. Loopback llama.cpp + bearer; loopback gateway default.
5. Retry only before the first output token; explicit interruption after.
6. Fastest-single comparator + honest negative results (release gates).
7. Canonical-JSON single implementation; shared schemas change only via ADR.

## Sequencing constraint

The streaming refactor (P1) precedes any M9 baseline freeze and any
cooperative-mode latency claim; the engine-adapter spike (P16) precedes
any lossless-mode speedup expectation; migration-0004 (P3) precedes the
M10 reputation chassis. These are the audit's hard ordering edges —
refactors elsewhere can proceed in parallel per the ownership map.
