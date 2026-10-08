# Integrator handoff — 2026-10-08 (post-review fix batch)

Owner-directed session: "do a review of this project and propose improvements"
followed by "do these". Four review agents (security, test/release, windows
product, architecture) audited the repo; their consolidated findings were
prioritized and implemented in commits `1702cfb..93bccb5` (7 commits).

## Changed files and why

1. **H1 — lease interop (the flagship-finding fix).** The F3 consume chain
   was broken end-to-end against production: the TS issuer never signed
   `lease_expires_at` (every real lease failed Rust `from_wire` at the
   ADR-026 gate) and the Rust client expected a response shape the route
   never sends (every `request_lease` failed deserialize; the desktop
   swallowed it). Files: `apps/tracker/lib/eligibility.ts`,
   `crates/modelswarm-tracker-api/src/lib.rs`,
   `protocol/vectors/lease-hubkey-1.json` (NEW cross-language golden
   vector + generator script), `protocol/msp-v1.md` §5 (truth),
   `crates/modelswarm-node/tests/live_tracker.rs` (harness now earns a
   lease and runs it through the production gate).
2. **CI re-green + new gates.** winjob test reaps its child
   (`zombie_processes`); toolchain pinned to 1.99.0 (a floating stable let
   a lint promotion break main for 3 pushes); rust.yml adds
   libp2p-backend clippy+tests (the serving path was never compiled in
   CI), desktop tauri-shell tests, protocol/catalog/pins path triggers,
   timeouts/concurrency, and a guards job (no committed binaries; version
   single-sourced; site filenames match). tracker.yml adds a wire-compat
   job: the Rust harness runs against a locally-started tracker over
   Postgres (candidate→promote seeding; full signed flow).
3. **Serving hardening (H2/M2/M3).** §6.4 wire clamps + prompt byte cap in
   `serving.rs` (was: a lease-holder could pin the engine 49 days);
   admission control (8 sessions / 2 per peer / request-id dedup 600 s);
   logs and StreamError messages carry variant names + stable codes only.
4. **Node composition (M1/F5).** Gateway binds BEFORE the engine spawns
   (port-conflict fallback no longer double-spawns/leaks llama-server);
   executor decode failures map to a closed error-code table via
   telemetry (replacing the eprintln + `peer_hint:"local:{e}"` channel
   that carried external-process error text across the P2P wire); engine
   lifecycle events reach node.jsonl; **data dir split from install
   root** (`%LOCALAPPDATA%\ModelSwarm\Data`, one-time migration, unit
   test) so NSIS uninstall cannot delete identity/models.
5. **Desktop honesty (F1/F3/F4).** Gateway error bodies render as ERROR
   turns (was: "no reply — rephrase"); swarm mode labels say who served
   (`swarm — remote peer 12D3…` vs `local (remote failed: reason)`);
   remote replies enter chat history; ONE process telemetry sink;
   `earn_lease` logs every failure reason; single-instance plugin;
   version single-sourced (tauri.conf.json carries none; workspace
   0.2.20); release builds panic without a staged engine
   (MSP_ALLOW_NO_ENGINE=1 opts out); listener chip reflects real state.
6. **Tracker hardening (H3/M5).** Production without a valid MSP_HUB_SEED
   throws at context build (was: silently signed with the repo-published
   dev key); rendezvous mailboxes bounded (24 h TTL + 64/mailbox, both
   stores, tested).
7. **Structural/docs.** Canonical JSON collapsed to one implementation
   (identity/eligibility shim `modelswarm-types`); reqwest unified at
   0.12; AGENTS.md ownership map covers every crate; architecture §14
   supersession note; tests/README truth.

## Exact commands run and outcomes

- `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo test --workspace` — **301 passed, 0 failed**.
- `cargo test --workspace --features modelswarm-node/libp2p-backend` — **313 passed, 0 failed** (3 ignored: env-gated real-engine/GPU/LAN).
- `cargo test -p modelswarm-desktop --features tauri-shell` — **7 passed**.
- tracker: `npm run typecheck`, `npm run build`, `npm run validate:vectors`
  (5/5), `npx vitest run` — **110 passed, 2 skipped**.
- **Production evidence**: `MSP_LIVE=1 cargo test -p modelswarm-node
  --features libp2p-backend --test live_tracker -- --ignored` — FAILED
  before the tracker deploy with `missing field 'lease_expires_at'`
  (empirical confirmation of H1), **PASSED after** (`enroll → register →
  heartbeat → challenge → lease earned + verified through the production
  gate → drain`).
- Deploy: `npx vercel deploy --prod` from apps/tracker (first attempt hit
  the known transient "Not authorized"; retry succeeded, aliased to
  modelswarm.deepflux.space).
- CI on push `93bccb5`: tracker workflow green incl. **wire-compat job
  passing on first run**; windows-package-dry-run green; rust +
  desktop-installer green (see run 37792918516 / 37792918213).

## Assumptions

- The lease signed-field change is backward-compatible in practice: no
  production lease could ever have parsed on the Rust side (verified
  live), so there is no legacy token to honor.
- Version 0.2.20 stays the shipped release number; the next release bump
  now happens in exactly one place (workspace Cargo.toml) and the guards
  job enforces the site filenames follow.
- The data-dir migration runs once per machine on first launch of a build
  containing it; downgrading after migration re-enrolls (documented in
  the code comment).

## Unresolved risks

- The wire-compat CI job is one run old; watch for flakiness (next start
  timing, Postgres cold start).
- The `MSP_ALLOW_NO_ENGINE` panic gate changes local release-build
  behavior — stage engines via `installer/stage-engine-windows.mjs`.
- Engine stderr remains discarded by policy (prompt-echo caution): load
  crashes still lack an in-log engine-side cause; capturing a redacted
  stderr tail is a recorded follow-up.
- CI on the LAN-proof env-gated tests still runs nowhere (needs the two
  physical machines by design).

## Suggested next tasks

1. **Gated LAN re-probe** (both A and B on builds ≥ these commits, both
   with MSP_LISTENER=1): the full A↔B earn→serve→verify chain should now
   pass — the original blocker (H1) is fixed and proven against
   production; record in `docs/verification/phase-f-lan-2026-10-07.md`.
2. **F2 relay binary** for B (owner decision already recorded: owner
   hosts the relay on their second desktop).
3. The expanded-mission roadmap review (9 subagents) completes in this
   same session; its consolidated document lands under `docs/reviews/`
   with an approval request before any governing-plan change.
