# Handoff — Windows Product Engineer: ADR-032 light-up items (i) RC-L1 + (ii) the desktop swap (2026-10-12)

Owner-approved light-up package (ADR-032 §7 disposition): this change lands
items **(i)** the profile-bound production `FamilyLeaseSource` (Security
RC-L1, `c1fb927` required change) and **(ii)** the desktop composed-daemon
swap + feature enable. Items (iii) the item-2 exactness reruns (another
seat) and (iv) the release approval (owner) are untouched. The workspace
version is NOT bumped (the release train owns it).

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-node/src/verifydrafts/composed.rs` | RC-L1: `FamilyLeaseRefusal` (the msp-v1 gate's classification as stable labels + Display mirroring its log reasons), the trait widened to `lease_for(peer_id, served_profile)`, the production `RosterFamilyLeases` source (hub-key verification + profile/consume/slots/peer bindings, returning the token's `expires_at` as unix ms), the composed loop refusing with the refusal's label, unit tests per binding. |
| `crates/modelswarm-node/src/verifydrafts/mod.rs` | Export the new types; doc updates. |
| `crates/modelswarm-node/src/verifydrafts/engine_source.rs` | `capi_family_source(engine_dirs, …)` — the daemon-composition constructor for `FamilyEngineSource::Capi` (candidate bundles chosen by file check BEFORE any load — ADR-031 §2's one-bundle-per-process guard means no load-then-fallback; adapter load under `spawn_blocking`; backend detected from the bundle) + `disclosure_json` (the status surface) + tests incl. the fail-closed missing-bundle pin. |
| `crates/modelswarm-node/src/lib.rs` | `NodeHandle.engine_identity` — the supervised engine's pinned tag/build-hash, so the family bridge labels its in-process engine with the pin the sidecar verified against (no zero placeholder). |
| `crates/modelswarm-node/Cargo.toml` | `time` gains `parsing` (lease `expires_at` → unix ms at the family gate); no version bump. |
| `crates/modelswarm-node/tests/verifydrafts_composed.rs` | One shared test-hub helper; the demux happy path now rides the PRODUCTION roster source; new `family_session_refused_on_wrong_profile_lease` (the security disposition's required wrong-profile pin, live over QUIC). |
| `crates/modelswarm-node/tests/verifydrafts_capi_composed_e2e.rs` (new) | The env-gated REAL-engine composed E2E (`verifydrafts_capi_e2e.rs` pattern, `--test verifydrafts_capi_composed_e2e`): composed daemon + roster leases + Capi engine + msp-v1 arm on one listener, RC-L1 refusal arm, exactness pins vs the HTTP ground truth. |
| `crates/modelswarm-desktop/Cargo.toml` | Features `verifydrafts` + `verifydrafts-capi` (chaining `modelswarm-node/verifydrafts{,-capi}`); **`tauri-shell` now chains `verifydrafts-capi`** so `cargo build -p modelswarm-desktop --features tauri-shell` ships the family ON; `tempfile` dev-dep for the prompt-file test. |
| `crates/modelswarm-desktop/src/app.rs` | The swap (below), `FamilyStatus` + `get_status.family`, `family-prompt-tokens.txt` strict parser, engine-dir preference, tests. |
| `crates/modelswarm-desktop/ui/index.html` | One honest status line: family on (backend, batch window, roster size) / off (exact reason). |
| `Cargo.lock` | `tempfile` dev-dep edge only. |

## 2. RC-L1 — the mapping to the serving.rs discipline

`crates/modelswarm-node/src/serving.rs` `LeasePolicy::check` (ADR-026,
per msp-v1 request) → the family gate at session open
(`composed.rs::validate_family_lease`, called through
`RosterFamilyLeases::lease_for(peer, served_profile)`):

| msp-v1 per-request check (`serving.rs:1076-1106`) | Family gate (RC-L1) | Refusal label |
|---|---|---|
| empty/`f0-unverified` token → "no lease presented" | no roster entry for the authenticated peer | `no_lease` |
| `from_wire` decode → "bad lease" | decode failure | `invalid_lease` |
| `verify(hub_key, now)` — signature, `msp1:` form, expiry ±120 s skew, lease cap → "lease invalid" | identical call, same pinned hub key (`LeasePolicy::production()` in the desktop) | `invalid_lease` |
| `lease.model_profile_id != served_profile` → "lease is for another profile" | identical, against the daemon's pinned profile | `profile_mismatch` |
| `!can_consume \|\| slots == 0` → "lease grants no consume right" | identical | `no_consume_right` |
| `lease.peer_id != remote_peer_id` → "lease is not bound to this peer" | identical, against the QUIC-authenticated PeerId (defense in depth: the roster is keyed by peer, a misfiled/stolen token still refuses) | `peer_mismatch` |

The returned expiry is the token's own `expires_at` in unix ms — the
ADR-026-capped operative bound the gate's stage-5 check enforces for the
session. The family wire carries no lease field (ADR-032 §3; the
offer-form-v3 oracle ruling), so the ROSTER is where the token comes from;
`RefuseFamilyLeases`/`StaticLeaseExpiry` remain the test/stub seam
(`StaticLeaseExpiry` is now documented binding-free, test-only).

Tests: unit tests per broken binding (profile mismatch, expired beyond
skew, `can_consume` false, slots 0, wrong peer, foreign-hub signature,
garbage token) + the empty-roster fail-closed pin + label stability; the
live QUIC pin `family_session_refused_on_wrong_profile_lease` (telemetered
`verifydrafts.family_refused reason=profile_mismatch`, engine never
consumed); the live real-engine refusal arm in the composed E2E.

## 3. The desktop swap design

`set_hosting_inner` (MSP_LISTENER=1 path): when `verifydrafts` is compiled
in, the single spawn of `serve_sessions` becomes
`serve_sessions_composed` — ONE listener, BOTH vocabularies, ONE
`Admission` gate, msp-v1 serving unchanged (`serve_one_seeded`), family
sessions behind the RC-L1 roster source (`LeasePolicy::production()`).
Engine source under `verifydrafts-capi`: `capi_family_source` over the
installed bundles (GPU bundle first iff the sidecar serves Vulkan,
ADR-024/E0; CPU otherwise; GPU set as detect-only fallback; the winner is
picked by file check and loaded ONCE — ADR-031 §2 one-bundle guard),
labeled with `NodeHandle.engine_identity` (the sidecar's verified pin).
Feature chain: `tauri-shell → verifydrafts-capi → modelswarm-node/verifydrafts-capi (+ libp2p-backend via the dep spec)` — the release build path produces the family ON; workspace default builds are unaffected (desktop without `tauri-shell` pulls no node crate; verified by the unchanged 469/0/6 default count).

**Disclosure honesty:** the `SessionAccept` disclosure is overridden per
session by the composed loop with the engine's real
`capabilities()` (RC-8, unchanged); `get_status.family.disclosure` reports
the same real geometry via `disclosure_json` — never a hand-set field.

**Fallback behavior (never silent):** every path that cannot serve the
family keeps the plain msp-v1 bridge AND records why, in telemetry
(`family.fallback`) and in `get_status.family` (`state:"off"` + reason):
- `family_prompt_unpinned` — no `family-prompt-tokens.txt` (the G6
  out-of-band pairing file; see risks);
- `engine_bridge_failed` — no C-API bundle among candidates, model load
  failure, or ABI canary failure (local error detail in the log only);
- `engine_bridge_not_compiled` — a `verifydrafts`-without-capi dev build;
- `family_not_compiled` — a tauri-shell build without the family tier;
- `listener_off` — MSP_LISTENER not set (no listener, no family);
- `verifier_identity_unavailable` — identity load failure.

## 4. Exact commands and outcomes (final tree)

```text
cargo fmt --all --check                                              OK
cargo clippy --workspace --all-targets -- -D warnings                clean
cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings        clean
cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend -- -D warnings   clean
TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings   clean
cargo test --workspace                                               469/0/6   (baseline EXACT)
cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend                    520/0/7   (514 + 6)
cargo test --workspace --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend                 539/0/13  (532 + 7; ignored 12→13, the new E2E)
cargo test --workspace --features modelswarm-node/libp2p-backend     492/0/7
cargo test --workspace --features modelswarm-runtime/capi-adapter    484/0/10
TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --features tauri-shell                   11/0/0    (8 + 3 family tests)
```

## 5. E2E verdict (owner-local, real engine, this machine)

Staged pins verified against `runtime-pins.json` (b11407, canonical
`353c4aab…`); model = SmolLM2-135M-Instruct Q4_K_M from the store;
single-machine loopback, Windows x64 — NOT LAN, NOT a product claim.

`--test verifydrafts_capi_composed_e2e` (the swap path: composed daemon,
roster leases, Capi engine, msp-v1 through the same listener) — PASS:

- RC-L1 wrong-profile lease: refused with the honest session end, no
  accept frame, engine never touched;
- msp-v1 arm over the same listener: "Hello swarm" streamed;
- disclosure from real geometry: cpu, batch_verify true, window 32;
- round 1 (true window of 8): full accept + ground-truth bonus,
  `verify_pass_ms` 14, round wall 23 ms;
- round 2 (corruption at index 4): exact divergence + ground-truth
  replacement, `verify_pass_ms` 12, round wall 21 ms;
- committed stream 14/14 tokens == HTTP ground truth token-for-token;
  total wall 3406 ms including staging + the HTTP ground-truth arm.

`--test verifydrafts_capi_e2e` (plain-QUIC venue, regression re-run) —
PASS: verify_pass_ms 12/11, round wall 19/18 ms, same exactness pins.
Composed-path overhead vs plain QUIC ≈ 2-4 ms per round (demux + lease
gate + per-session engine construction) on this machine.

## 6. Assumptions

1. The desktop's roster starts EMPTY (no tracker surface distributes peer
   lease tokens today): family sessions refuse with `no_lease` until
   seeded — that is the ADR-026 discipline working, not a defect.
2. The family session prompt stays out-of-band (G6, recorded): the desktop
   reads `<data_dir>/family-prompt-tokens.txt` (strict token-id list;
   absent/junk = unpinned = fail-closed). Coordinators must chain on the
   same tokenization; closing this properly is the freeze-ADR item.
3. The family engine loads a SECOND in-process copy of the model next to
   the llama-server sidecar (the ADR-032 architecture); the adapter is
   only constructed when a prompt is pinned, so ordinary hosting users
   pay nothing until they opt into family verification.
4. `expires_at` (not `lease_expires_at`) is the session's stage-5 lease
   bound — the token's ADR-026-capped operative expiry.

## 7. Unresolved risks / notes for the release train

- **RAM/VRAM doubling while family verification is active** (sidecar +
  in-process C-API model): coordinate with the Resource Engineer's
  governor (the 70% ceiling, gate D11) before the release notes claim a
  configuration; the prompt gate keeps it opt-in for now.
- **The roster feeder does not exist yet**: a coordinator can never open a
  family session on a shipped desktop until tracker-side lease
  distribution (or manual seeding) lands. The UI says so
  (`lease_roster_peers: 0 → refusing family sessions until …`).
- **ADR-032 §7 disposition record**: items (i)+(ii) are now discharged in
  code; the owner/Protocol seat should append the one-line completion
   record when the flag-flip release is cut (I did not edit the ADR —
  not my path).
- Security seat re-check at the light-up gate (per `c1fb927`): the
  wrong-profile refusal is pinned at three levels (unit, live QUIC
  integration, real-engine E2E).
- The `A-2`/`A-3` advisories (stash bound, dribble residence) routed
  earlier remain open with their owners; nothing here widens them.

## 8. Suggested next task

Integrator: review + merge; then the release train runs the §7 item-(iii)
exactness reruns on this build and cuts the release (version bump +
`docs/verification` record), appending the light-up completion line to
ADR-032 §7.
