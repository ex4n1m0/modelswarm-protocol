# Handoff — Windows Product Engineer, v0.2.23 desktop batch (2026-10-13, evening)

**Provenance note (Integrator):** this batch was dispatched to the Windows
Product Engineer agent, which died mid-run at ~39 minutes on a plan-quota
exhaustion (limit resets 2026-10-16) AFTER completing the code and tests
but BEFORE its gates/handoff. The Integrator audited the full diff
line-by-line, re-ran every gate independently, ran the UI harness, and
completes this handoff. All evidence below is the Integrator's own
execution unless noted.

## 1. Changed files and why

- `crates/modelswarm-desktop/src/app.rs` (+~1,064 lines):
  - **Settings toggle "allow swarm pairing"** (owner directive
    2026-10-13): persisted config key `allow_swarm_pairing`, DEFAULT ON;
    `bind_serving_listener` no longer reads `MSP_LISTENER` (the transport
    crate's internal ADR-018 guard env is SET by the desktop when the
    toggle allows — cross-crate seam flagged for Network Engineer: the
    guard should take an explicit opt-in parameter). New IPC
    `set_pairing_allowed` (refused while hosting), `get_status.pairing_allowed`
    is the UI's honest posture source. Honest-degrade `FamilyStatus`
    gains `detail` (exact bind error / resource projection) for every
    off path: `pairing_disabled`, `listener_bind_failed`,
    `resource_ceiling`, `serving_bridge_unavailable`.
  - **Family tier on by default + RAM honesty**: `compose_family_serving`
    now takes weights bytes + hardware and runs `family_resource_verdict`
    — projects the verify engine's second copy of the weights against
    the D11 70% ceiling (system RAM; VRAM when the serving backend is
    vulkan), degrade with exact numbers when it would breach; unobservable
    inputs (null sample fields) pass with a logged gap, never a guessed
    block. Advisory-grade until the Resource Engineer's M2 governor owns
    enforcement (coordination seam below). The compose now also returns
    the family engine source; the desktop keeps it as
    `Inner.family_engine` for the coordinator self-check.
  - **Coordinator self-verify (the 2026-10-13 live root cause)**:
    `family_selfcheck_window` runs every drafted window through a FRESH
    prefilled local batch-verify engine (`FamilyEngineSource::engine_for`
    — cache-cold by construction) BEFORE `client.round`. Verdicts:
    `Send` / `LocalDivergence{accepted}` (labeled `local_selfcheck`,
    counts only) / `Unavailable` (labeled `selfcheck_unavailable`) —
    fail-closed: a window that fails or cannot be checked locally NEVER
    hits the wire. Warm-cache greedy flips (tonight's 0/8
    divergence-at-0 live find) are caught locally.
  - **Telemetry gap fix**: `log_family_pairing` — the pairing DETAIL
    label (Err-branch reasons: `rejected`/`io`/`local_selfcheck`/…) now
    reaches the `family.probe.pairing` event (was dropped; found live
    2026-10-13 when the UI was the only place the reason existed).
  - **Chat auto-engage checkpoint** (owner directive "family on by
    default, zero buttons"): `family_engage_checkpoint` runs on every
    swarm chat turn — reads the coordinator-side capability cache
    (`Inner.family_peer_caps`, filled by `family_probe` from the
    SessionAccept disclosure — the ONLY existing capability surface; the
    roster carries no batch-verify field and inventing one is an ADR),
    evaluates the pinned reason ladder via the scheduler's frozen
    `should_engage_cooperative` (margin ≥ 0.05), telemeters
    `family.engage`, and exposes `family_engage.{engaged,reason}` on
    every swarm reply. **Protocol boundary (recorded, not improvised):**
    routing user chat content through a family session is blocked by
    ADR-034 rule 5 (the family wire is content-blind; arbitrary prompts
    need the owner-gated rule-8 commitment path) — the ladder's terminal
    label is `blocked_content_contract`; the plain swarm path serves
    every turn before the first output token regardless.
  - **Chat controls**: `clear_chat` IPC (resets the daemon chat_log +
    emits `chat-cleared`; the live-confirmed language-drift escape
    hatch) and `send_chat.temperature` (clamped [0,2], rides the
    EXISTING msp-v1 §6 sampling fields — no protocol change; pairing
    draft paths stay greedy-pinned).
- `crates/modelswarm-desktop/ui/index.html`: Setup toggle row; chat-box
  temperature slider + clear button + the "pairing stays greedy" note;
  peer-first picker (live-peer models only when any exist;
  downloaded/selected stay visible; "show all N (x more)" restores; the
  v0.2.19 hardware shortlist applies when no model has peers); honest
  remote-serve chip + family line with detail; per-reply family-engage
  note + temp in the meta line.
- `crates/modelswarm-desktop/ui-harness/pairing-chat.html` (NEW) +
  `census.html` (updated): the stub-Tauri 1152×648 rendered-geometry
  harness for this batch (the standing verify-rendered-layout rule).
- `crates/modelswarm-node/src/verifydrafts/mod.rs` (+6): desktop-facing
  re-exports only (`EngineDisclosure`, `BatchVerifyEngine`,
  `RuntimeError`, `SamplingParams`) so the desktop can name the
  self-check types without new direct crate deps. No semantic change.
- `crates/modelswarm-desktop/Cargo.toml` / `Cargo.lock`: optional
  `async-trait` under `verifydrafts` (the scripted test engine's trait
  impl attribute).

## 2. Exact commands and outcomes (Integrator re-run)

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy -p modelswarm-desktop --all-targets --features
  tauri-shell -- -D warnings` — clean (the shipped feature set).
- `cargo test --workspace` — 495 passed / 0 failed (baseline exact).
- `cargo test -p modelswarm-desktop --features tauri-shell` —
  **50 passed / 0 failed** (baseline 45; +5 new tests: the env-free
  toggle round-trip incl. unknown-key preservation, the off-detail
  shape, the self-check gate's four arms incl. the 0/8 warm-cache flip
  case, the telemetry detail fix incl. the no-arrays assertion, the
  engage reason ladder's full priority order).
- UI harness: `python -m http.server 4817` from the crate root →
  `ui-harness/pairing-chat.html` → **14/14 PASS** (title "ALL PASS"),
  including geometry: chat section 216px within the 552px column,
  transcript ≥ 80px, toggle-off rendering from `get_status`, exact
  degrade texts, `send_chat` receiving the slider temperature, and
  clear-chat emptying the transcript.

## 3. Test evidence

See §2 — every gate green, all evidence from this machine (A), run by
the Integrator after the agent death.

## 4. Assumptions

- The toggle applies at the NEXT hosting start (changing it while
  hosting is refused) — same discipline as `set_tracker`.
- Temperature is chat-only; the family session pins its own defaults and
  every pairing draft path stays greedy (the determinism contract).
- `MSP_LISTENER=1` remains set at machine scope on both live machines;
  it is now simply unread by the desktop product gate.

## 5. Unresolved risks / cross-crate seams flagged for routing

1. **Network Engineer**: the transport ADR-018 non-loopback guard still
   consults `MSP_LISTENER` internally; the desktop SETS it when the
   toggle allows. The guard should take an explicit opt-in parameter.
2. **Resource Engineer**: the 70% ceiling check is desktop-side and
   advisory-grade (one platform sample at hosting start); the M2
   governor owning enforcement (with the winjob seam) is the recorded
   follow-up. The doubling projection uses weights bytes only (KV/ctx
   overhead not yet modeled).
3. **Protocol Architect + owner**: chat cooperative routing is blocked
   at ADR-034 rule 5 (content-blind family wire). The rule-8 commitment
   path is the recorded route to lifting `blocked_content_contract` —
   an owner-gated ADR decision, not a code gap.
4. **No family-round timing EWMA exists yet** (`gate_unmeasured` in the
   ladder): the F15 recorder tracks the plain path only; measuring
   engaged family rounds needs the rule-8 path first (or a
   probe-sourced EWMA — future work).
5. The capability cache is probe-filled; a fresh install that never
   probes reads `capability_unknown` on every chat turn (honest, and
   the probe now rides the family-on-by-default tier automatically at
   hosting start — but auto-probing on hosting start was NOT built).

## 6. Suggested next task for the Integrator

Proceed with the v0.2.23 train (bump + notes + CI + staging + manifest +
deploy + the two live machines' first self-update cycle). Route seams
1–3 to the named seats with this handoff.
