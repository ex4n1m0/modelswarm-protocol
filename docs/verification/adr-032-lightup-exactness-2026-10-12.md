# ADR-032 §7 item (iii) — light-up exactness reruns on the production build (2026-10-12)

Record owner: Test and Release Engineer (release train v0.2.21). This file
consolidates the flag-flip package's item-(iii) evidence — "the item-2
exactness reruns on the production build" — required by ADR-032 §7's
disposition (`01f6c90`) as a condition of the light-up release. Item (iv)
(owner release approval) is recorded in the release-train handoff
(`docs/reviews/handoff-test-release-v0.2.21-2026-10-12.md`): the owner
approved the v0.2.21 release and the website publication on 2026-10-12.

Primary sources (both read in full, numbers transcribed verbatim):

- `docs/reviews/handoff-windows-product-lightup-2026-10-12.md` (commit
  `55d802e`) — the composed-daemon swap + RC-L1 lease source.
- `docs/reviews/handoff-windows-product-family-lease-2026-10-12.md`
  (commit `2ffc8a7`) — the lease PRESENTATION path (empty-roster pairing).

## 1. What was run

The composed real-engine E2E
(`crates/modelswarm-node/tests/verifydrafts_capi_composed_e2e.rs`, env-gated
`--ignored` tier) against the PRODUCTION build path: the desktop
`tauri-shell` release configuration with the family compiled ON
(`tauri-shell → verifydrafts-capi → modelswarm-node/verifydrafts-capi
(+ libp2p-backend)`, the exact feature chain
`cargo build -p modelswarm-desktop --features tauri-shell` produces), one
listener demultiplexing BOTH vocabularies, msp-v1 serving unchanged.

Three green executions on the same machine, two configurations:

| # | Commit | Lease source | Verdict |
|---|---|---|---|
| 1 | `55d802e` | roster-seeded | PASS |
| 2 | `2ffc8a7` | PRESENTED (roster never seeded) | PASS (total 3274 ms) |
| 3 | `2ffc8a7` | PRESENTED (roster never seeded) | PASS (total 3250 ms) |

Plain-QUIC venue regression (`verifydrafts_capi_e2e`) re-run green at both
commits (see §4).

## 2. Environment and provenance (honesty labels)

- Hardware: single owner machine, Windows x64, 24 logical CPUs.
- Runtime build: llama.cpp pinned tag `b11407`, canonical CPU bundle
  (`llama-b11407-bin-win-cpu-x64.zip`, archive sha256
  `353c4aab423bff5cb6dc0e1d32e41a447990ab3d69a411b7f58ff48187c91a03`),
  every staged engine file sha256-verified against `runtime-pins.json`.
- Model profile: SmolLM2-135M-Instruct Q4_K_M from the local store.
- Venue: single-machine loopback. **This is NOT a cross-machine claim, NOT a
  LAN claim, and NOT a product performance claim** — it is an exactness
  (lossless-correctness) verification of the production build path. All
  millisecond figures below are lab observations on this machine, labeled
  as such, and are not cited as product performance numbers.
- Build profile: the desktop release path named above; the C-API in-process
  adapter loads the same pinned `llama.dll` the sidecar serves from
  (`NodeHandle.engine_identity` label).

Environment note added by the release train (2026-10-12): during this
release the 3-OS installer matrix exposed a latent compile defect in the
NON-Windows half of the C-API adapter (`libloading::Library::new` is
`unsafe` in 0.8.9; `ffi.rs` now wraps it with a safety comment, and
`rust.yml` gained a per-push `check-nonwindows-capi` ubuntu clippy job).
Consequence for this record's scope: the real-engine exactness evidence
below was executed on WINDOWS; on Linux/macOS the C-API surface is
COMPILE-verified (the new per-push gate plus the green installer matrix
builds) but NOT engine-verified. No non-Windows engine-verification claim
is made anywhere in this record.

## 3. Pinned outcomes (all three composed executions)

- **Wrong-profile lease refused pre-engine** (RC-L1 arm; run 1 via the
  roster token, runs 2–3 via the PRESENTED token): the honest session end
  with no accept frame and the engine never constructed/consumed — the
  family gate classifies `profile_mismatch` (stable label
  `verifydrafts.family_refused`) BEFORE any engine work.
- **msp-v1 unchanged on the same listener**: the plain swarm arm streamed
  ("Hello swarm") through the same demuxed listener in every run.
- **Family session opened** (runs 2–3: on the PRESENTED lease with an
  EMPTY roster — the two-fresh-installations configuration): disclosure
  from the engine's REAL geometry — cpu backend, batch_verify true,
  window 32 — never a hand-set field.
- **Round 1 — full acceptance plus ground-truth bonus**: every draft token
  accepted, and the round's committed tokens equal the HTTP ground truth
  beyond the requested window. `verify_pass_ms` 12–14; round wall 21–23 ms.
- **Round 2 — exact divergence plus ground-truth replacement**: corruption
  at index 4 caught by the prefix-hash chain; the divergent suffix replaced
  by the verifier's own decode; the committed stream is token-identical to
  a corruption-free run. `verify_pass_ms` 12–14; round wall 21–23 ms.
- **Committed stream exactness**: 14/14 committed tokens token-for-token
  identical to the HTTP ground-truth arm in all three runs — the lossless
  `speculative_exact` contract holds on the production build.
- **Presentation adds no round overhead** (runs 2–3 vs run 1): verify-pass
  times sit in the same 12–14 ms band with and without the presented-lease
  carrier; total composed-run wall 3274/3250 ms (presentation) vs 3406 ms
  (roster path, includes staging differences) on this machine.

## 4. Regression arms

- Plain-QUIC venue (`verifydrafts_capi_e2e`) re-run green at both commits:
  `verify_pass_ms` 12/11 (run at `55d802e`) and 14/12 (run at `2ffc8a7`),
  round walls 18–21 ms, same exactness pins (14/14 == HTTP ground truth).
- Per-push gates at both commits (CI + local, per the handoffs): fmt clean;
  clippy `-D warnings` clean across default, verifydrafts,
  verifydrafts-capi, capi-adapter, libp2p-backend, and desktop tauri-shell
  (TAURI_CONFIG overlay); workspace tests 469/0/6 default (baseline
  EXACT), 526/0/7 verifydrafts, 545/0/13 capi, 492/0/7 libp2p-backend,
  484/0/10 capi-adapter, desktop tauri-shell 13/0/0.

## 5. Honest beta boundaries (unchanged by this record)

1. Opt-in gates: the serving side needs `MSP_LISTENER=1` AND a pinned
   family prompt file (`family-prompt-tokens.txt`, strict token-id list;
   absent/junk = fail-closed `family_prompt_unpinned`) AND a C-API engine
   bundle — ordinary hosting users pay nothing until they opt in.
2. RAM doubling while family verification is active (llama-server sidecar
   + the in-process C-API model copy). Coordination with the Resource
   Engineer's governor (the 70% ceiling, gate D11) is queued, not done.
3. No relay: direct/LAN reachability only; the NAT boundary stands (no
   universal-connectivity claim).
4. The tracker-fed lease roster does not exist yet; the PRESENTED lease
   (the same tracker-earned, hub-signed lease the msp-v1 path uses) is the
   operative admission path, and roster 0 is the normal configuration, not
   a refusal state.
5. The pairing surface is a diagnostic probe (`family_probe`: one real
   family session, no rounds, no prompt content), not a chat flow; driving
   family VERIFICATION inside chat is the cooperative-plan Windows
   integration phase.

## 6. Verdict

ADR-032 §7 item (iii) — the item-2 exactness reruns on the production
build — is DISCHARGED for the v0.2.21 release on the evidence above,
within the labeled venue (owner-local loopback, Windows, pinned b11407,
SmolLM2-135M Q4_K_M). Items (i) RC-L1 and (ii) the composed-daemon swap
were discharged by `55d802e`/`2ffc8a7` (see the two handoffs); item (iv)
owner approval is on record. The ADR-032 §7 completion line itself is the
owner/Protocol seat's edit (not made here).
