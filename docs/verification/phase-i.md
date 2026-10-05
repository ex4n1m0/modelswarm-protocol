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
