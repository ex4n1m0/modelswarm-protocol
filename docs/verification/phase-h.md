# Phase H Verification — Real-model Windows client

Owner-approved scope (2026-10-05): one model, one function, installer for
multi-machine testing. Decisions: Qwen2.5-0.5B-Instruct Q4_K_M; first-run
download (ADR-008, never bundled).

## What shipped (commits 6bd199a, 73ae932, e7f2213, 0ad8450, +H5)

- **H1 — profile**: `msp1:eb0a0d21a61daa85e9c4f382c45ebcc13c6d346cc5e402e22a4b4e38bcd8120c`
  registered + promoted on production tracker (catalogVersion 3). Identity
  hashes resolved from the GGUF itself (ADR-022); cross-language parity on
  the real 491,400,032-byte artifact: Node resolver ≡ Rust re-derivation
  (env-gated test `real_artifact_hashes_match_manifest`, run locally, PASS).
- **H2 — artifacts**: streamed download + progress + atomic finalize +
  sha256-while-streaming + corrupt-redownload + ADR-022 identity re-check
  (canned-server tests). Catalog envelopes Ed25519-verified against
  `protocol/keys/hub-public.hex` (tamper/wrong-key fail closed). Store
  migration 2 (`artifacts`).
- **H3 — engine**: pinned llama.cpp `b11407` CPU-x64 (zip sha256
  `353c4aab…`, 51 file hashes in runtime-pins.json, verified at launch).
  Supervisor: loopback spawn, per-run API key, `--no-webui`, logs discarded
  (prompt-echo policy), bounded restarts, dies with node. Adapter: exact
  token-id recovery via logprob bytes + GPT-2-byte-decoded vocab (the
  detext path mis-recovers 2.1% of vocab = byte-fallback tokens; the exact
  path has none), default sampling → temperature 0 (determinism contract),
  `propose()` greedy self-continuation, EOS-aware streaming.
  **Real-engine gate (local):** deterministic greedy, exact ids, measured
  **70.3 tok/s**, clean shutdown.
- **H4 — desktop**: in-process node, 8 IPC commands (state-only returns),
  roster heartbeat with honest `/ip4/0.0.0.0/tcp/0` (peers see direct-connect
  fail — beta boundary labeled in UI), ChatML chat through the loopback
  gateway with measured tok/s badge, privacy gate persisted, live UI.

## Honest limits (stated, not hidden)

- Remote execution over a transport listener is NOT wired: machines host and
  serve locally; the roster is visible cross-machine. Cooperative/remote
  modes arrive with the Phase F session-listener wiring.
- Unsigned builds everywhere (labeled); SmartScreen warning expected.
- No resume on interrupted downloads (restart transfer).
- Cross-CPU argmax determinism of llama.cpp is expected but unproven
  cross-machine; verification makes any divergence a rejection, not a
  correctness break.

## Real-model E2E matrix (how to reproduce)

Local gates run 2026-10-05 on the dev machine:
`MSP_REAL_GGUF=…qwen2.5-0.5b-instruct-q4_k_m.gguf MSP_LLAMA_SERVER=…engine-pruned/llama-server.exe cargo test -- --ignored`
→ types parity, engine smoke: PASS. CI job `real-model-e2e` (H5) re-runs
both on a windows runner with a fresh HF download.

## Multi-machine test checklist (owner)

1. Install `ModelSwarm…-setup.exe` on each machine (same LAN/VPN).
2. Accept the privacy gate; Save tracker (default is production).
3. Download model (progress bar → verified).
4. Toggle Hosting ON on every machine — roster shows all installations.
5. Chat on any machine: replies tagged `local single · N tok/s (measured)`.
