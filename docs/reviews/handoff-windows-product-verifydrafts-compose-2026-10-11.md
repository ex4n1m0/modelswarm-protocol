# Handoff — Windows Product Engineer: ADR-032 light-up completion (G4 engine bridge + composed daemon), features `verifydrafts-capi` / `verifydrafts`+`libp2p-backend`

Date: 2026-10-12 (task dated 2026-10-11; file name per the task directive).
Owner seat: Windows Product Engineer (`crates/modelswarm-desktop`,
`crates/modelswarm-node`, `crates/modelswarm-winjob`, `installer/`).
Base: `cdaf378` (the QUIC venue). Consumes Runtime's `339ae9e`
(CapiVerifyEngine) as-is; `modelswarm-runtime` and `modelswarm-session`
were NOT touched (zero lines).

## 1. Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-node/Cargo.toml` | New OFF-by-default feature `verifydrafts-capi = ["verifydrafts", "modelswarm-runtime/capi-adapter"]` — enabling node/verifydrafts+adapter wires the bridge through ONE flag (the task's requested feature dependency). No new dependency (modelswarm-runtime already a node dep); `Cargo.lock` unchanged. |
| `crates/modelswarm-node/src/verifydrafts/engine_bridge.rs` (new, cfg `verifydrafts-capi`) | **The G4 bridge**: `CapiBatchVerifyEngine` — the architect's `BatchVerifyEngine` impl for Runtime's production `CapiVerifyEngine`, hosted in the node (the crate that depends on both sides of the recorded trait cycle; Runtime's `tests/capi_verify.rs` `CapiVerifyEngineAsTrait` pattern, production-hosted). `CapiBatchVerifyEngine::new` also commits the daemon-pinned session PROMPT as the PREFILLED root (the family wire/gate never carries a prompt — see §4 gap G6; a raw engine without prefill refuses every window). `capi_disclosure` = Runtime's `to_disclosure` reference mapping, hosted where both types are nameable. 3 unit tests: the disclosure-mapping pin (both backends, label agreement, schema validity), the WINDOW_CAP=32 cross-crate agreement pin, and the real-trait compile-proof binding. |
| `crates/modelswarm-node/src/verifydrafts/engine_source.rs` (new, cfg `verifydrafts`) | **The injection seam** (`FamilyEngineSource`, the TimingEngine seam's pattern — the serve paths stay engine-agnostic behind `Arc<dyn BatchVerifyEngine>`; this is the one place that knows which engine backs them): `Shared(Arc<dyn BatchVerifyEngine>)` = mock/stub venue; `Capi { adapter, backend, profile_id, prompt_tokens }` (under `verifydrafts-capi`) = a fresh prefilled engine PER family session on the shared adapter. `engine_for()` / `disclosure()`. 1 unit test (Shared yields the injected engine, capabilities flow, passes work through the seam). |
| `crates/modelswarm-node/src/verifydrafts/composed.rs` (new, cfg `verifydrafts` + `libp2p-backend`) | **The composed daemon** (`serve_sessions_composed`): ONE QUIC listener, BOTH vocabularies (the msp-v1 `SpecMessage` bridge and the VerifyDrafts family), ONE `Admission` gate (serving.rs's session/per-peer caps, applied identically to both paths), ONE ADR-026 lease discipline. Demux by FIRST frame with zero msp-v1 added latency: typed recv returns msp-v1 frames immediately; raw family frames are stashed by the typed recv and collected on the 1 s poll boundary via the stash-first raw recv; the peeked frame is handed to the winning path pre-read (`serve_one_seeded` / the new `PrefacedQuicWire`) — never decoded twice. Family path per session: lease derived AT OPEN from the QUIC-authenticated PeerId through the `FamilyLeaseSource` seam (fail-closed default `RefuseFamilyLeases`; `StaticLeaseExpiry` for tests/roster), engine from `engine_for()`, and the accept's `EngineDisclosure` OVERRIDDEN with the engine's real `capabilities()` (honest geometry, never the hand-set field). 2 unit tests (fail-closed + static lease sources). |
| `crates/modelswarm-node/src/serving.rs` | Behavior-identical refactor only: `serve_one`'s loop extracted into `pub(crate) serve_one_seeded(..., first: Option<WireMessage>)` so the composed demux can hand it the peeked first frame (`None` = today's read-here path, byte-identical); `send_error` made `pub(crate)` for the composed admission refusal. No behavioral change (default suite count unchanged, all serving/remote tests green). |
| `crates/modelswarm-node/src/verifydrafts/mod.rs` | Module declarations + re-exports for the three new modules; the venues doc section gains the composed-daemon + engine-seam description. |
| `crates/modelswarm-node/src/verifydrafts/serve.rs` | Doc-only: `serve_verifydrafts_quic`'s admission note now points at the composed wiring (the limitation it recorded is closed by this change). |
| `crates/modelswarm-node/tests/verifydrafts_composed.rs` (new) | 2 integration tests over the REAL QUIC transport: **the required demux pin** — a SpecMessage session (InferenceRequest → TokenDelta/Usage/Completed through a lease-checked FixedExecutor) and a VerifyDrafts session (negotiate + engaged round) COEXIST on one listener, with the family's disclosure asserted to be the INJECTED engine's geometry (window 8, batch true — not the config's hand-set `batch_verify: false`); and the fail-closed lease seam — no lease ⇒ the family session ends before any engine work (script never consumed), the client surfaces `ClientOpenError::Io` (fallback-to-single), telemetry carries `verifydrafts.family_refused` / `no_lease`. |
| `crates/modelswarm-node/tests/verifydrafts_capi_e2e.rs` (new, cfg `verifydrafts-capi`) | The env-gated `#[ignore]`d REAL-engine end-to-end (Runtime's `capi_verify.rs` staged-environment pattern): pins sha256-verified, llama-server HTTP arm for ground truth on the same binaries, then `FamilyEngineSource::Capi` → `engine_for()` (bridge + pinned-prompt prefill) → `serve_verifydrafts_quic` → real QUIC → `VerifyDraftsClient`. Pins: the disclosure the client receives is the engine's real geometry; a true greedy window fully accepts with the ground-truth BONUS; a corrupted window rejects at the exact index with the ground-truth replacement; the committed stream equals HTTP truth token-for-token; every round's `verify_pass_ms` passes the RC-2 cross-check. Owner-local command in the file header. |
| `.github/workflows/rust.yml` | Two steps mirroring the existing precedent: `Clippy/Tests (verifydrafts + capi combo)` over `--features modelswarm-node/verifydrafts-capi` — without them the bridge and its mapping ship uncompiled in exactly the combination production will flip on. **Cross-boundary note:** T/R-owned file, flagged for review (the wiring handoff's precedent). |
| `docs/reviews/handoff-windows-product-verifydrafts-compose-2026-10-11.md` | This file. |

NOT changed: `modelswarm-desktop` (ZERO lines — the composed seam is
daemon-level; the desktop's `serve_sessions` call site swaps to
`serve_sessions_composed` at the flag-flip, a one-line plumb then),
`modelswarm-runtime`, `modelswarm-session`, `modelswarm-winjob`,
`installer/`, `Cargo.lock`.

## 2. Exact commands run and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` | clean (after one `cargo fmt --all` normalization) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (default) |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi -- -D warnings` | clean (the combo; note `verifydrafts-capi` implies `verifydrafts`) |
| `cargo test --workspace` | **462 passed / 0 failed / 6 ignored — exactly the recorded baseline** (all new code feature-gated OFF; the serving.rs refactor is behavior-identical) |
| `cargo test --workspace --features modelswarm-node/verifydrafts` | **505 passed / 0 failed / 7 ignored** = the 500/0/7 baseline + exactly this change's 5 tests that compile in this config (2 composed unit + 1 seam unit + 2 composed integration) |
| `cargo test --workspace --features modelswarm-node/verifydrafts-capi` | **523 passed / 0 failed / 12 ignored** = 505 (above) + the capi-adapter suite (its own delta vs default measured 477−462 = +15 passed / +4 ignored) + this change's 3 bridge unit tests + the 1 env-gated ignored e2e (15+4+3+1 accounted exactly) |
| `cargo test --workspace --features modelswarm-runtime/capi-adapter` (attribution only) | 477/0/10 |
| QUIC venue suites ×3 (`verifydrafts_quic`, `verifydrafts_wiring`, `verifydrafts_composed`) | 4/0 + 5/0 + 2/0 each run — zero flakes |

## 3. Real-engine end-to-end (owner-local, the task's requested run)

`cargo test -p modelswarm-node --features verifydrafts-capi --test
verifydrafts_capi_e2e -- --ignored --nocapture` with the pinned CPU set
(`%LOCALAPPDATA%\ModelSwarm\engine`, sha256-verified against
`runtime-pins.json`) + SmolLM2-135M-Instruct Q4_K_M — **PASS, twice**
(first run caught a test-authoring slice bug, fixed, then 2× green; no
`llama-server` left running after either):

- disclosure from real geometry: backend `cpu`, `batch_verify true`,
  `batch_window_max 32` (== ADR-032 WINDOW_CAP) — the client received
  exactly the engine's caps through the wire;
- round 1 (true greedy window): full acceptance + the ground-truth
  bonus token; engine-measured `verify_pass_ms` 13 / 12, round wall
  19 / 20 ms; cross-check `Ok`;
- round 2 (corrupted window): rejection at the exact divergence index
  (4) with the ground-truth replacement; `verify_pass_ms` 12 / 12,
  wall 18 ms; cross-check `Ok`;
- committed stream after both rounds: 14 tokens, token-for-token equal
  to the HTTP arm's greedy truth; requester-measured EWMA fed.

Environment-labeled numbers only (single-machine loopback, Windows x64,
not LAN, not a product claim).

## 4. Gaps and decisions recorded (the task's stop-and-record protocol)

- **G6 — the family has no prompt surface (NEW, recorded):**
  `BatchVerifyEngine` has no prefill/prompt method, and neither the
  offer wire nor the gate carries the session prompt — but a real
  engine must hold the SAME tokenized prompt in KV as the coordinator's
  session. Resolution WITHIN this seat: the seam pins the prompt from
  daemon configuration (`FamilyEngineSource::Capi { prompt_tokens }`,
  empty refused) and the bridge commits it via
  `CapiVerifyEngine::prefill` at construction — the same out-of-band
  pinning family as profile id + sampling params (wiring handoff G3).
  If the architect amends the offer to echo a prompt/params commitment
  (G3's likely fix), the seam is the single place to feed it.
- **G3-family (unchanged):** profile/params (and now prompt) binding at
  negotiation still waits on the Protocol Architect's freeze-ADR
  amendments; the `WirePrefixCommit` mirror from `f9ec2ff` remains the
  bridge until they land.
- **Lease seam (design decision):** "ADR-026 applied to BOTH paths
  equally" is realized as — msp-v1: per-request hub-signed
  `capability_token` (unchanged); family: lease derived AT SESSION OPEN
  from the QUIC-authenticated PeerId via `FamilyLeaseSource`, feeding
  the gate's stage-5 `expired_token` check for the session's lifetime.
  The fail-closed DEFAULT refuses all family sessions until the daemon
  wires a real source (the eligibility/roster store); there is no
  frozen family reject code for "no lease", so the honest wire answer
  is the session end (client fallback-to-single), telemetered
  `verifydrafts.family_refused`/`no_lease`.
- **Admission refusal frame (recorded):** a session refused by the
  shared `Admission` cap gets the msp-v1 `overloaded` `StreamError`
  (today's frame) before its vocabulary is known; a family client
  cannot parse it and sees the session close — honest, and the
  refusal is telemetered identically for both vocabularies.
- **Feature visibility caveat:** the composed module and
  `tests/verifydrafts_composed.rs` are cfg'd on
  `all(verifydrafts, libp2p-backend)`. Workspace runs get
  `libp2p-backend` via desktop dep-spec unification (the same fact the
  prior handoffs' counts rely on), so the CI feature steps compile
  them; a bare `-p modelswarm-node --features verifydrafts` run does
  NOT (the module is simply absent — clean, but worth knowing).
- **Runtime API gap: none.** The bridge consumed `CapiVerifyEngine`,
  `VerifyEngineCaps`, `VerifyBackend`, `prefill`, and the
  `to_disclosure` mapping reference exactly as shipped; nothing
  blocked the bridge (answering the task's explicit question).

## 5. Assumptions made

1. Default builds keep today's behavior byte-identically: verified by
   the unchanged 462/0/6 default count and the seed-parameter refactor
   being pass-through (`None`) on the msp-v1 path.
2. Family sessions may hold their admission slot through the
   first-frame classification wait (≤300 s, mirroring the msp-v1
   between-requests budget — equal treatment was the requirement).
3. `engine_for` runs per family session AFTER the lease check
   (fail-closed first, engine cost second); a failed engine load
   refuses the session (`engine_unavailable`) rather than serving with
   a fallback disclosure — an honest `batch_verify: false` peer is a
   CONFIGURATION choice (Shared with the HTTP disclosure), not a
   degraded-mode fallback.
4. The composed loop's disclosure override (`config.engine =
   engine.capabilities()`) is authoritative over any hand-set field —
   pinned by the coexistence test (window 8 from the injected engine,
   not the config default).
5. The e2e keeps the CPU bundle (mirrors Runtime's main suite);
   Vulkan is pinned within-backend by Runtime's own suite and the
   mapping test covers both labels — a Vulkan-staged e2e variant would
   be a follow-up, not a gate.

## 6. Unresolved risks

1. The desktop still spawns `serve_sessions` (single vocabulary). At
   the flag-flip it swaps to `serve_sessions_composed` with a real
   lease source — the one-line plumb deferred by design (task item 3).
2. `FamilyLeaseSource` has no production implementation yet (the
   default refuses everything); wiring it to the eligibility store is
   the daemon-integration task.
3. A family session's classification adds ≤1 s latency (the poll
   boundary) before its offer is answered — bounded, but visible;
   acceptable for the light-up tier.
4. Cross-machine behavior (NAT, real RTT) remains untested — same
   posture as the QUIC venue handoff (F2B is the owner-gated
   cross-machine proof).
5. The e2e's exactness pins are build-sensitive (ADR-032 §7 standing
   rule): re-run on every engine bump, exactly like Runtime's.

## 7. Suggested next task for the integrator

1. T/R review of the two new CI steps (this change's only
   cross-boundary edit).
2. Route G6 (+ G2/G3, still open from the wiring handoff) to the
   Protocol Architect: one freeze-ADR ruling covering the family's
   commit wire form AND the offer's missing profile/params/prompt
   binding — the seam is ready to consume whatever echo lands.
3. Owner-gated flag-flip package (per ADR-032 §7): swap the desktop's
   spawn to `serve_sessions_composed` + a real `FamilyLeaseSource`,
   flip the feature defaults, and run the item-2 exactness-pin reruns
   — everything code-side for the light-up tier is now complete
   (wire tier `efa79a6`, serve loop `f9ec2ff`, engine `339ae9e`, QUIC
   venue `cdaf378`, bridge + composition this change; real-engine e2e
   green twice).
