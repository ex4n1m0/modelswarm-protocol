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

## I7 — v0.2.4: complete installs on every OS + closed-loop device approval

Owner directive (2026-10-06): every OS installer must carry everything the
app needs, setup must be automatic, and state must be legible to the user.

**Packaging.** The v0.2.3 engine fix was Windows-shaped: on deb/AppImage the
resources land in `/usr/lib/<name>/`, on dmg in `Contents/Resources` — never
next to the exe. `engine_binary_path()` now resolves via Tauri's
`resource_dir()` (with exe-relative dev fallbacks), the v0.2.3 `*.dll/*.exe`
resource glob (which matched NOTHING on Linux/macOS) is back to `engine/*`,
the engine dir is fully gitignored (no dotfiles can ship), and WebView2
`downloadBootstrapper` is explicit so fresh Windows machines self-install.
**CI now opens every built artifact** (7z / dpkg-deb / appimage-extract /
hdiutil) and FAILS unless the pinned engine binary is inside — first run
caught its own SIGPIPE bug in the deb check; fixed, all three OSes green.

**First run is now closed-loop.** Hosting start auto-enrolls: device/start →
pairing code shown in Setup ("code EA6SSRMF — ask the owner to approve it at
modelswarm.deepflux.space/verify") → device/complete polled every 30 s →
session → roster register; 401s re-enroll, everything logged (`enroll.*`,
`roster.error`). New owner-controlled approval page `/verify` +
`POST /admin/devices/approve {userCode}` (X-MSP-Admin gated; knowing the
code alone approves nothing — verified live with a wrong token: 403).
msp-v1 §3.2 documents the endpoint.

**Production bug found and fixed by this work**: `/auth/device/start` 500'd
with an EMPTY body on real Postgres — `upsertInstallation`'s peer_keys
insert targeted `ON CONFLICT (installation_id)` but that table's only unique
constraint is `(installation_id, added_at)`; the statement could never plan
against the declared schema (42P10). The MemoryStore suite cannot see SQL
breakage, so it survived from Phase B. peer_keys is now `ON CONFLICT DO
NOTHING` (append-only history; current key lives in installations), and the
env-gated real-Pg suite the tracker README always claimed now exists
(`tests/pg-enrollment.test.ts`: start → retry → approve-by-code → complete →
register on Postgres; also caught `insertProfile` sending NULL provenance
against a NOT NULL DEFAULT column). Live proof after deploy: the running
client went from `enroll.error` to `enroll.pending user_code=EA6SSRMF` on
its next 30 s retry, and the UI shows the code + verify URL + honest roster
note. Approval is the owner's one action at /verify; the device joins the
roster within 30 s of it.

**Per-OS first-run notes** are on the download card (SmartScreen, Gatekeeper
right-click-open/xattr, deb apt-install, AppImage chmod). Same version
number on every OS = same build (all-OS rule restored).

## I8 — v0.2.5: the §2.3 envelope fix (found by live enrollment)

v0.2.4's client could never register: `SignedEnvelope` serialized
`installation_id`/`body_digest` (snake_case) while msp-v1 §2.3 and the
tracker's strict zod schema define `installationId`/`bodyDigest`
(camelCase) — the signature payload had the same mismatch, so every
Rust-client signed request died as `401 unsigned_request` ("missing or
malformed MSP1 authorization envelope"). Invisible since Phase B: the TS
and Rust sides were never integration-tested against each other.

Live diagnosis trail (all reproducible): running client logged
`enroll.error` with an empty-body parse failure → Vercel function logs
showed the Postgres 42P10 (I7 fix) → after that fix, probe showed
`401 unsigned_request` → header dump + zod schema comparison found the
casing split. After the fix, `device/complete` returns the correct
`403 pending` (probe, 2026-10-05 ~17:55Z). Rebuilt + republished all OSes
as v0.2.5 (the v0.2.4 installers carried the broken envelope).
`finish()` now names the HTTP status on unparseable bodies — the empty
500 previously surfaced as a bare serde EOF error.

## I9 — v0.2.6: empty-reply poisoning (found while verifying I8)

A single failed generation left an EMPTY assistant turn in the ChatML
history; the model then imitated it — every later reply returned 0 tokens
in ~60 ms until an app restart (observed live; the same gateway answered
fresh-history requests with 256 tokens at 193 tok/s simultaneously).
Empty replies no longer enter the rendered history. Also recorded from
tonight's E2E: force-killing the app (taskkill /F) orphans the engine
child — the supervisor needs a graceful exit; several overlapping
node/engine instances during testing produced a transient no_eligible_peer
window that fully recovered after cleanup + restart (verified across four
paths: app gateway, node CLI, H3 real-engine selftest at 143 tok/s, and
direct engine HTTP).

## I10 — v0.2.7/v0.2.8: 16:9 window, one-click device approval, single-host census

Owner report (2026-10-06): window should be 16:9; the website census shows
no model even though one is hosted. Diagnosis: the census was honest —
`GET /api/v1/stats` returned `peersOnline: 0, models: []` because the
installation never got past device approval (code KR2RX8U5 sat pending; no
`/admin/devices/approve` or `/peers/register` in the Vercel logs). A
one-machine swarm is supported by design: it serves locally and, once
registered, must appear as `peersOnline: 1` with its model as the sole
online entry — now pinned by a regression test
(`stats.test.ts` → "reports a single registered host as its model's sole
online peer").

v0.2.7 (5ef4dc0): window 800×600 → 1152×648 (16:9); `?code=` prefill on
/verify (focus moves to the owner-token field); tauri-plugin-opener with a
capability scoped to `https://modelswarm.deepflux.space/*`.

v0.2.8 (0f0ec63): the 0.2.7 webview-side `__TAURI__.opener.openUrl` click
produced NO visible effect on the shipped binary (no browser window or
tab anywhere; the OS path itself was proven fine — `start <url>` opened
Edge immediately, so the failure was in the webview JS/IPC layer). The
button now invokes a Rust command, `open_approval_page`, that
prefix-validates the URL (`https://modelswarm.deepflux.space/verify`) and
calls the opener API directly; errors surface in the UI banner.

Evidence (owner machine, shipped installers, 2026-10-06):

- Gates: `cargo fmt --check`, clippy workspace + `-p modelswarm-desktop
  --features tauri-shell` (`-D warnings`), `cargo test --workspace` (46
  suites ok), tracker `tsc --noEmit` + `vitest run` (93 passed) +
  `npm run build` — all green for both versions.
- CI 37399820790 (0.2.7) and 37402698707 (0.2.8): all three OS builds
  green incl. the engine-inside-artifact checks.
- Live: counted download route 302, downloaded Windows exe sha256
  `1df82797…` matches SHA256SUMS.txt and the site (0.2.8).
- E2E 0.2.8 on the owner machine: install → launch → window client area
  exactly 1152×648 (outer rect 1167×685 incl. chrome) → select SmolLM2
  135M → gateway `127.0.0.1:11435` LISTENING, enrollment pending
  (code T3PW8EG8) → click "approve this device…" → **Edge opens
  `https://modelswarm.deepflux.space/verify?code=T3PW8EG8` with the
  pairing code prefilled and the owner-token field focused** (AX tree of
  the Edge window, pid 25120). Gateway serving verified
  (`/v1/chat/completions`, 16 completion tokens).

Still pending (owner action, by design): the admin-token paste on /verify.
The app polls `device/complete` every 30 s; within one poll of approval it
registers, heartbeats, and the census shows `1 peer online` +
`SmolLM2 135M · 1 online`.

## I11 — v0.2.9: no console window on Windows launch

Owner report: a command window opens alongside the app on every launch.
Root cause: `modelswarm-desktop.exe` was a console-subsystem binary — the
standard Tauri `#![cfg_attr(not(debug_assertions), windows_subsystem =
"windows")]` attribute was missing from `main.rs`, so each launch got a
Windows Terminal window titled with the exe path (observed live: a
WindowsTerminal process spawned the same second as the app). The
llama-server child was already spawned with CREATE_NO_WINDOW
(`engine.rs` launch), so no second console appears when hosting starts.

Evidence (owner machine, shipped installer, 2026-10-06):

- Gates: fmt, clippy workspace + tauri-shell (`-D warnings`), workspace
  tests, tracker tsc/vitest/build — all green; CI 37405016190 green on
  all three OSes; live download route 302, Windows exe sha256
  `76274767…` matches SHA256SUMS.txt + site.
- Launch via Start-Process (shortcut-equivalent): the ONLY new visible
  window is the app itself — no Windows Terminal, no window titled with
  the exe path (EnumWindows before/after).
- Hosting start (model select → engine spawn): gateway 11435 LISTENING,
  still no console window.
- One-click approval re-proven on 0.2.9: click → Edge at
  `…/verify?code=GDE4FB4D`, pairing code prefilled, owner-token field
  focused (AX tree of the Edge window).
