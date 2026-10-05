# Phase I Verification — follow-ups (owner notes 2026-10-05)

## I1 — online counter (LIVE, c04dbb3)

`GET /api/v1/stats` → `{"peersOnline":N}`: distinct peers with a live,
non-draining lease; counts only (content-blind). Landing-page badge polls it
every 30 s with graceful fallback. Tests: zero / distinct / draining excluded
(one-way per msp-v1) / TTL lapse — 84/84 tracker tests green. Live-verified:
`{"peersOnline":0}` before any machine hosts.

## I2 — lower-RAM models (catalogVersion 7, 93b9a43)

| Tier | Profile | Artifact | Notes |
|---|---|---|---|
| ~250 MB RAM | SmolLM2 135M Instruct Q4_K_M `msp1:948d898a…` | 105,454,432 B | bartowski quant of the apache-2.0 model (provenance recorded; no official 135M GGUF repo exists) |
| ~500 MB | SmolLM2 360M Instruct Q8_0 `msp1:6897f192…` | 386,404,992 B | official HuggingFaceTB repo (only q8_0 published) |
| ~600 MB | Qwen2.5 0.5B Q4_K_M `msp1:eb0a0d21…` | 491,400,032 B | unchanged from Phase H |

Both resolver-resolved fail-closed; golden vectors added; Node + Rust parity
gates PASS. Registered + promoted on production.

## I4 — model-picker-first UX (this commit)

First page = verified-catalog model list (downloaded state shown). One click:
`select_model` → download-or-load (progress events) → node starts, registers,
heartbeat begins ("the swarm starts working") — with a stable local API on
`127.0.0.1:11435` (ephemeral fallback when taken) shown with a copyable curl
snippet, plus the test/chat box. Tracker config + stop-hosting remain.
Selection persists across restarts (config.json). Switching models requires
stopping hosting first (honest, explicit).

## I3 — Linux/macOS (next)

Per-platform runtime-pins (ubuntu-x64, macos-arm64 llama.cpp assets), engine
binary-name portability, tauri deb/appimage/dmg matrix CI. Tracked in
`phase-i-followups` memory + this doc's open items.

## Honest limits (unchanged from Phase H)

Local serving; roster visible cross-machine; remote execution awaits the
transport listener. Unsigned builds, labeled. All badge numbers measured.

## I6 — v0.2.3 Windows-first release (this commit)

Owner report (2026-10-05 23:56): "catalog unreachable", no version number in
the client, engine reported missing. Root causes found on the live build:

1. **v0.2.2 NSIS shipped without the engine** — built from a cleaned tree
   (`engine/` is gitignored and empty; the `resources: ["engine/*"]` glob
   fails silently) → 13.9 MB installer, every fresh install dead.
2. **`engine_binary_path()` never looked in `engine/`** — the exact subdir
   the NSIS bundle installs to; even engine-complete installs read as
   "Engine MISSING".
3. **No version anywhere** (title, UI, `get_status`) — builds were
   indistinguishable, which is how v0.2.1/v0.2.2 confusion happened.
4. **The UI swallowed every catalog error** as a bare "catalog unreachable"
   and nothing was logged. (The catalog itself was healthy: the same fetch
   path from current source verified live — 3 profiles, Ed25519 OK.)

Fixes in `crates/modelswarm-desktop`: engine lookup order `engine/` → flat →
`resources/`; `get_status` exposes `version` (tauri package_info) +
`engine_path`; empty persisted tracker falls back to the default;
`set_tracker` requires an absolute http(s) URL; `list_models` retries once
and surfaces the real error + URL; catalog and roster failures are logged to
`logs/node.jsonl` (`catalog.error`, `roster.error` — throttled); UI shows
version chip + footer version and the actual error text; `build.rs` warns
when a release build lacks the staged engine; new
`installer/stage-engine-windows.mjs` stages the pinned b11407 engine
(per-file SHA-256 vs `runtime-pins.json`, fail-closed).

Evidence: sandbox `/S /D=` install of the exact published artifact →
engine `pinned llama.cpp — present`, catalog renders 3 cards, `live` IPC,
title/header/footer read `0.2.3-internal-test-unsigned`; SmolLM2-135M
selected → 105,454,432 B downloaded (exact catalog size), gateway
`127.0.0.1:11435` serving the profile, chat measured **244.5 tok/s · 256
tok · 1047 ms**. Live site: counted route 302 → artifact, SHA-256 match
(`d0e16f8c…`), counter incremented. Owner machine upgraded in place.

**Windows-first (owner directive)**: Linux/macOS stay at v0.2.2 ("paused"
on the site) until the Windows client is verified; releases then resume
together.

Known boundary (now visible, not silent): roster registration 401s until
the installation is device-enrolled AND admin-approved (tracker ops gate);
hosting is local-only until the Phase F listener, as before.
