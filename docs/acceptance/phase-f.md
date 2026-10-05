# Phase F Acceptance Tests — Public Hostile Swarm & Transport Hardening

Gate (revision): malicious test peers cannot corrupt accepted output, forge
usage, replay commits, or wedge sessions indefinitely; verifier trust
assumptions explicit. Evidence → `docs/verification/phase-f.md`.

## Adversarial suite (simulator-driven)

- **F1** Fabricated proposals (plausible garbage tokens) → verification
  rejects; lossless contract intact.
- **F2** Lying logprobs → sampled acceptance still provably correct
  (verifier uses only target distributions); liar's receipts accumulate →
  suspension policy triggers.
- **F3** Prefix equivocation (two peers, different prefixes, same round) →
  both rejected and flagged; no state advance.
- **F4** Replay of commits/proposals across sessions and reconnects →
  rejected (requestId/nonce single-use; §6.6 rules).
- **F5** Withholding after prefill → deadline fires, slot released,
  reputation cost recorded; no coordinator stall.
- **F6** Latency-measurement manipulation → scheduler decisions use only
  requester-measured values (advertised values never raise a score).
- **F7** Malicious verifier accepting garbage → auditor redundant
  re-verification of sampled rounds detects divergence; quarantine + epoch
  bump; security overhead measured per ADR-13 records.
- **F8** Registration/lookup/session floods → rate limits hold; no 5xx.
- **F9** Malformed-frame fuzz at every P2P entry point (decode failures,
  oversized, truncated, deep nesting) → clean rejections, no panics.
- **F10** Prompt-retention: redaction gates re-run on both peers' logs.

## Transport hardening (ADR-018 migration)

- **F11** rust-libp2p backend lands behind the `PeerTransport` trait: QUIC +
  Noise + Identify; the D/E correctness suites re-run green over it.
- **F12** ADR-020 migration: `peer_id` = identity-multihash derivation
  everywhere; tracker validates the multihash↔pubkey binding; goldens
  updated; placeholder equality removed.
- **F13** Local relay circuit exercised in the simulator (relay + DCUtR
  negotiation happy path); relayed-path cost penalty applied by the
  scheduler.

## Hard stops (recorded, require owner approval to lift)

- Public relay/hosting deployment (paid infrastructure).
- Any DNS or production-credential change.
