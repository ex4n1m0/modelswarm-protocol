# Phase D Verification Report

Date: 2026-10-05 · Commits: `e9b48a8` (acceptance algorithms, early) + this
commit (protocol integration).

## Delivered

The exact two-peer speculative protocol runs end-to-end over real loopback
TCP between three roles (coordinator, proposer, verifier):

- Signed §6.1 handshakes (ADR-018 staging); cooperative namespace as
  snake_case JSON riding the authenticated sessions (`transport::send_json`).
- Prefill with KV commitments and a genesis prefix hash; reconnect resumes
  from the last committed prefix (D4).
- Rounds: proposal → verification (target decode with default sampling,
  `verify_greedy`) → coordinator-built `PrefixCommit` (exact state-machine
  chain rules) Ed25519-signed and sent to **both** peers; each verifies
  signature + sender-key binding before touching state; ack reasons typed.
- Adaptive bounded window controller (×1.5 up / ÷2 down, cap 16).
- Fallback (ADR-013): `acceptance_collapse` (<0.15 trailing rate),
  `rtt_spike` (sustained 2-round overrun, armed after warm-up),
  `peer_lost`; fallback tail decodes via the exact plain decoder and lands
  through one final signed commit.
- Two-phase signed receipts (D7): verifier signs, coordinator countersigns
  the receipt digest; mismatch/no-ack are explicit close reasons.
- `SpeculativeExecutor` implements the gateway's `InferenceExecutor` with
  peer rotation per re-invocation (ADR-007 contract honored).

## Gates (docs/acceptance/phase-d.md)

| Gate | Evidence |
|---|---|
| D1 greedy equality | 180-case sim matrix (20 seeds × windows {2,4,8} × accuracy {0,0.5,1.0}): `tokens_equal_to_single` true in every case **including fallback paths**; 3× rerun stable; plus the 200-case pure sweep from `e9b48a8` |
| D2 sampled | Exact rejection-sampling proven at unit level (200k-draw distribution tests, `e9b48a8`); no runtime exposes distributions yet — E2E path is greedy, consistent with ADR-019's Phase-D entry condition |
| D3 rollback | KV-commitment digests checked at prefill and resume; state machine rejects divergence (replay tests) |
| D4 session safety | 7 E2E tests: disconnect mid-round → explicit fallback, state intact; reconnect replays last commit → `duplicate`, no advance; bad-signature commit → `bad_signature` ack, no advance |
| D5 window sweep | Machinery in the matrix (windows 2/4/8 + adaptive); records carry acceptance stats |
| D6 fallback | Injected collapse (accuracy 0.0) → `acceptance_collapse` recorded, output exact; sustained-overrun → `rtt_spike` |
| D7 receipts | Signed two-phase exchange verified; mismatch/close-without-ack explicit |
| D8 posture | **No performance claim.** All numbers TEST-ONLY mock/loopback (ADR-019). The real-hardware "beats fastest single" question stays open pending the research runtime + approved GGUF profile (stop conditions intact) — correctness-proven, performance-unproven, per the plan's own negative-result allowance |

## Integrator-run gates

`cargo fmt --all --check` PASS · `cargo clippy --workspace --all-targets
-- -D warnings` exit 0 · `cargo test --workspace` **237 passed / 0 failed**
· `sim spec 7 8 0.0` → acceptance 0.0, fallback engaged, tokens exact,
receipts verified (one JSON line, machine-readable).

## Accepted interpretations (from the handoff, recorded)

1. Serve APIs take an expected requester key + max-sessions (state retained
   across connections — enables D4); the staged backend authenticates
   client→server only, so the verifier key comes from the caller's
   directory (ADR-018 gap, noted).
2. `VerificationResult.correction` is always present (correction-or-bonus)
   — the only reassemblable contract.
3. Final-round window capping (never a truncated commit).
4. `rtt_spike` requires a sustained 2-round overrun (literal single-round
   reading flaked on Windows scheduler jitter; output exact either way).
5. Executor serves the default-sampling pinned session; request sampling
   ignored in single/greedy scope (documented).
6. D7 refusal semantics are report-level outcomes, not held-open sockets.

## Notes

- The agent observed my concurrent tracker-api edits mid-run (expected —
  parallel integration) and correctly did not touch them; its final full
  workspace run included my committed code: 237/0.
- Loopback timing sensitivity documented: heavy load can trip `rtt_spike`
  conservatively — never correctness.
