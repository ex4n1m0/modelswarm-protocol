# Handoff — Windows Product Engineer: auto-update client (2026-10-13)

Owner request 2026-10-12: "clients update and restart when we ship a new
client." Settled design: Tauri's built-in updater machinery with OUR
signing keypair — Windows installers stay unsigned (code-signing remains
the separate, unpaid G6 stop), update packages carry their own
minisign-class signatures verified client-side against the embedded
public key.

**One correction to the task brief, resolved in favor of repo reality:**
the brief said "tauri v1 (tauri-cli 1.2.3)"; the repo is **Tauri 2**
(`tauri = "2"` in `crates/modelswarm-desktop/Cargo.toml`, config schema
`schema.tauri.app/config/2`, CI pins `tauri-cli ^2` at
`.github/workflows/desktop-installer.yml:113`). In Tauri 2 the updater is
the `tauri-plugin-updater` plugin (not core) with config under
`plugins.updater` — that is what this change implements. Consequences
called out below where the v1/v2 mechanics differ (no `dialog` key
exists in v2: the plugin never shows a built-in dialog, so "we render
our own status" is inherent, and relaunch is the plugin's NSIS `/R` flow
on Windows).

## 1. Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-desktop/tauri.conf.json` | `plugins.updater` block: `pubkey` = the owner-provided minisign PUBLIC key (embedded verbatim; public by design), one `endpoints` entry `https://modelswarm.deepflux.space/downloads/latest.json?target={{target}}&arch={{arch}}&current_version={{current_version}}` (static manifest; templating carried as QUERY parameters — ignored by static serving today, ready for a dynamic server without a client change), `requireSignedVersion: true` (anti-downgrade; see §6), `windows.installMode: "passive"` (progress bar only, no wizard; the plugin passes NSIS `/P /UPDATE` and `/R` so the app restarts). **No top-level `version` key** — the single-source rule is untouched. |
| `crates/modelswarm-desktop/Cargo.toml` | Optional `tauri-plugin-updater = "2"` (locked 2.13.3) wired into the `tauri-shell` feature. |
| `crates/modelswarm-desktop/src/app/updater.rs` (new, ~870 lines with tests) | The whole client half: the pure status state machine (`UpdaterStatus` × `UpdaterEvent` → `step`), the honest `line()` wording the UI renders verbatim, the `get_status` view shape, the supervisor (check on start + every 6h; `MSP_NO_AUTOUPDATE=1` → `Disabled`, no check ever), the download→verify→install cycle with throttled progress, `on_before_exit` engine-kill hook, and 10 unit tests. |
| `crates/modelswarm-desktop/src/app.rs` | `mod updater;`; `Inner.updater: UpdaterStatus` (written only by the supervisor, read only for `get_status`); `get_status` gains `"updater": inner.updater.view()`; `run()` registers `tauri_plugin_updater::Builder::new().build()` (NO webview capability added — driven from Rust; only status lines cross into the UI) and spawns the supervisor in `setup`. |
| `crates/modelswarm-desktop/ui/index.html` | Header status note `<span id="update-note" role="status">` next to `conn-note`; `refresh()` renders `s.updater.line` VERBATIM (no mirror logic — same rule as the census chips). A missing payload keeps the `update: …` placeholder. |
| `crates/modelswarm-desktop/ui-harness/updater.html` (new) | Stub harness (monitor/census pattern): loads the REAL `ui/index.html` into a same-origin 1152×648 iframe with the stubbed bridge, asserts the line renders verbatim in all 8 phases + missing-payload degradation + header fit + "conn still live" (a failure never blocks the app). |
| `crates/modelswarm-winjob/src/lib.rs` | `close_all_retained_jobs()` (production seam, was test-hook-only): closes every retained kill-on-close job handle, terminating ALL supervised engine children synchronously — called from the updater's `on_before_exit` so the NSIS installer never races a live engine binary it is about to replace. New test `close_all_retained_jobs_kills_every_supervised_child` (two children → one call → both dead; idempotent). |
| `installer/tauri/nsis-notes.md` | Stop #2 updated: the updater is no longer "disabled until signed artifacts exist" — it is enabled on the package-signature design above; the code-signing stop is unchanged. |
| `Cargo.lock`, `crates/modelswarm-desktop/gen/schemas/*` | Lock entries for the plugin tree; tauri-build regenerated schemas (committed generated artifacts, as before). |

## 2. Exact commands and outcomes

- `cargo fmt` then `cargo fmt --check` — clean.
- `cargo clippy --workspace -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --features tauri-shell --all-targets -- -D warnings` — clean (the CI overlay; local `engine/` is staged so the plain run passes too).
- `cargo test --workspace` — **495 passed / 0 failed / 6 ignored** (baseline 494/0/6; +1 = the new winjob seam test).
- `cargo test -p modelswarm-winjob` — 3/0/0 (all three job tests, including the new one: real `ping` children assigned to jobs, closed, verified dead via `tasklist`).
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --features tauri-shell` — **40 passed / 0 failed / 0 ignored** (was 30; +10 updater tests).
- Rendered status-line check — see §4.
- Live redirect probe: `curl -sI https://modelswarm.deepflux.space/api/download/ModelSwarm-Setup-0.2.21-windows-x64.exe` → `302` → `Location: https://modelswarm.deepflux.space/downloads/ModelSwarm-Setup-0.2.21-windows-x64.exe`; `curl -L --range 0-1023` on the same URL → `206`, 1024 bytes. The counting route works end-to-end today.
- Throwaway-key signing rehearsal (key + artifact in `%TEMP%`, deleted after): `npx -y @tauri-apps/cli@2 signer generate -w ./throwaway.key -f --ci` then `npx -y @tauri-apps/cli@2 signer sign -f ./throwaway.key --app-version 0.2.22 artifact.exe` → `.sig` produced whose base64 decodes to `trusted comment: timestamp:… file:artifact.exe version:0.2.22` — the exact field `requireSignedVersion` checks.

## 3. Status-machine behavior (what is pinned by tests)

States: `Disabled | Idle | Checking | UpToDate | Available{v} |
Downloading{v,done,total} | Ready{v} | CheckFailed{reason} |
InstallFailed{reason}`. Lines, exactly: "update available → v0.2.22" /
"downloading v0.2.22… 42%" / "v0.2.22 ready, restarting…" / "update check
failed — <reason>" / "update failed — this version keeps running
(<reason>)" / "updates: off (MSP_NO_AUTOUPDATE)".

- Happy path walks Idle→Checking→Available→Downloading→Ready with those
  exact lines (`happy_path_reaches_ready_with_exact_honest_lines`).
- **`Ready` is terminal**: on Windows the plugin's `install()` runs
  `ShellExecuteW` on the NSIS updater and then `std::process::exit(0)`
  — it NEVER returns — so Ready is written BEFORE `install()` and every
  later event (even a late timer tick) is ignored
  (`ready_is_terminal_even_against_a_late_timer_tick`).
- Check failure is honest and non-blocking: `CheckFailed` + the next
  cycle's `CheckStarted` moves on — no crash loop, no dead end
  (`check_failure_is_honest_and_never_blocks_the_next_cycle`).
  Structurally, the supervisor only ever WRITES status; nothing in the
  app gates on it.
- Download/install failure keeps the running version: the plugin
  verifies the minisign signature at the END of `download()` and only
  then installs; a failure writes `InstallFailed`, late `Progress` /
  `InstallBegin` events are IGNORED — no fabricated download or
  readiness (`install_failure_keeps_the_running_version…`).
- Progress is ignored outside an in-flight update
  (`progress_is_ignored_outside_an_in_flight_update`).
- `MSP_NO_AUTOUPDATE=1` is terminal and ONLY the exact value "1" opts
  out — a typo stays ON so a client can never be silently stranded
  (`disabled_is_terminal_and_only_the_exact_env_value_opts_out`).
- Cross-product sweep: every state×event stays a valid state with a
  non-empty line, and downloading/ready are only ever reached from an
  in-flight update (`every_transition_stays_a_valid_honest_state`).
- The `get_status` payload shape (`phase/version/progress/reason/line`,
  5 keys) is pinned, as is the 6h interval, the 160-char reason bound,
  and the ENTIRE `tauri.conf.json` updater block including the pubkey
  (checked to be a minisign key by its decoded comment header), the one
  https endpoint with all three templates, `requireSignedVersion`, and
  passive installs — plus a mirror of the CI no-top-level-version guard
  (`status_view_shape_is_pinned`, `check_interval_is_six_hours…`,
  `tauri_conf_pins_the_update_channel`).

Supervisor wiring: spawned once in `run().setup`; `tokio::time::interval`
fires its first tick immediately (check on start), then every 6h; after
each cycle a `Ready` state breaks the loop (Windows normally never
returns from `install()`; if a future plugin build ever did, no second
download can happen). Non-Windows: after `install()` returns, the cycle
calls `handle.restart()` (never returns). Children-on-restart: the
composed daemon runs IN-PROCESS in the shell, so every path of app death
closes the retained winjob handles and kills the engine — pinned by
`job_handle_close_kills_assigned_child` (existing) — and the update path
specifically kills them EARLY via `close_all_retained_jobs()` in
`on_before_exit` so NSIS never replaces files a live engine is executing
(pinned by the new winjob test).

## 4. Rendered status-line check (stub harness)

Committed harness: `crates/modelswarm-desktop/ui-harness/updater.html`
(browser pattern of monitor/census). This session's host could NOT run
headless Edge (`msedge --headless=new` produces zero output here —
`--version`, `--dump-dom`, `--screenshot`, and a beacon probe all came
back empty; the prior monitor-handoff runs used the owner's interactive
session). The rendered check was therefore executed with jsdom over the
REAL `ui/index.html` with the same stub bridge the harness uses
(`jsdom@25` installed in `%TEMP%\msp-jsdom`, outside the repo): **9/9
PASS** — all 8 phase lines render verbatim in `#update-note` with
`role="status"` and `conn-note` still "live" (app not blocked), and the
missing-payload case keeps the `update: …` placeholder. The harness's
header-geometry assertions (fit at 1152px) still need one headless-Edge
run on a machine where it works — suggested for T&R's next UI pass; the
worst-case line is bounded (`short_reason` caps at 160 chars, flex-wrap
keeps the header from clipping).

## 5. Release-train: exactly what it must produce

Per version bump (all OSes together), AFTER CI builds the installers:

1. **Sign each installer at LOCAL staging where the private key lives**
   (the key never enters the repo or CI):
   `npx -y @tauri-apps/cli@2 signer sign -f <private-key-path> --app-version <VERSION> <installer-file>`
   (or the equivalent `cargo tauri signer sign` from the CI-pinned
   tauri-cli ^2). Two corrections vs the draft command in the task
   brief, both load-bearing: **@2, not @1.2.3** (this repo is Tauri 2;
   the v1 CLI's signature format lacks what v2 verifies), and **`-f`
   for a key PATH** (`-k` takes the key STRING). **`--app-version` is
   MANDATORY** because the client config sets `requireSignedVersion:
   true` — without it the trusted comment carries no version and the
   client rejects the update (verified by rehearsal: the `.sig` decodes
   to `trusted comment: … version:0.2.22`). Output: `<file>.sig`
   containing the base64 of the whole minisign file.
2. **`latest.json`** (static file at
   `apps/tracker/public/downloads/latest.json`, Tauri v1/v2-compatible
   static format; the client looks up `{os}-{arch}` keys — verified in
   plugin source `get_urls`, which tries `{os}-{arch}-{bundle}` then
   `{os}-{arch}`):
   ```json
   {
     "version": "<VERSION>",
     "notes": "<release notes>",
     "pub_date": "<ISO 8601, e.g. 2026-10-13T00:00:00Z>",
     "platforms": {
       "windows-x86_64": { "signature": "<contents of ModelSwarm-Setup-<VERSION>-windows-x64.exe.sig as printed>", "url": "https://modelswarm.deepflux.space/api/download/ModelSwarm-Setup-<VERSION>-windows-x64.exe" },
       "linux-x86_64":   { "signature": "<…AppImage.sig>", "url": "https://modelswarm.deepflux.space/api/download/ModelSwarm-<VERSION>-linux-amd64.AppImage" },
       "darwin-aarch64": { "signature": "<…dmg.sig>", "url": "https://modelswarm.deepflux.space/api/download/ModelSwarm-<VERSION>-macos-arm64.dmg" }
     }
   }
   ```
   The `signature` value is the base64 string the CLI prints (base64 of
   the full two-line minisign file) — that is what the plugin decodes.
3. **Manifest URL verdict: the counting-route 302 WORKS — use
   `/api/download/<file>` URLs in the manifest.** Evidence: (a) plugin
   source `download()`/`check()` build a plain `reqwest::ClientBuilder`
   with no custom redirect policy; reqwest's default is
   `Policy::limited(10)` (`reqwest-0.12 src/redirect.rs:161`), and the
   post-redirect final status is what the plugin checks; (b) live probe
   above (302 → 206 via follow). One redirect is consumed; the route is
   per-IP rate-limited at the public class, which a 6h-cadence updater
   cannot hit. The website buttons keep the same route; no `/downloads/`
   direct URLs are needed.
4. **Do NOT set `bundle.createUpdaterArtifacts`** in tauri.conf.json:
   it would make CI itself generate `.sig` files and REQUIRE the
   private key (`TAURI_SIGNING_PRIVATE_KEY`) inside CI, which the key
   custody model forbids. Post-build local signing (step 1) replaces it.
5. First updater-bearing release is the NEXT version bump: every
   currently installed client (≤0.2.21) has no updater and moves to it
   manually via the site; from there on updates are automatic.
6. Linux note: the updater's Linux install replaces the current
   executable — correct for AppImage installs; deb-managed installs
   should keep updating via the deb (the manifest still carries the
   AppImage URL; a deb-hosted client would self-replace outside dpkg's
   knowledge — flagged as a known v0 limitation, listed in §7).

## 6. For the Security review of the update chain

- **Trust root:** the minisign public key embedded in
  `tauri.conf.json`; signature verification is in the plugin's
  `download()` before any install, cannot be disabled
  (`verify_signature` → `PublicKey::verify` with the embedded key).
- **`requireSignedVersion: true`** (enabled here from day one — no
  legacy signed releases exist to break): the signature's trusted
  comment (itself covered by the signature) must name the version the
  manifest announces, blocking the "inflated version + older genuine
  artifact" confusion attack on a compromised manifest. Cost: the train
  MUST pass `--app-version` when signing (§5.1) or every update is
  rejected.
- **Manifest itself is unsigned** (TLS-only): the tracker/Vercel origin
  can serve a crafted `latest.json`. Blast radius without
  requireSignedVersion would be downgrade-to-genuine-old; with it, a
  forged manifest can at most re-serve a version that was genuinely
  signed under that exact version string (still a downgrade to a real
  release the attacker has — inherent to static-manifest Tauri v2
  updates; `allowDowngrades` stays false so the announced version must
  still compare NEWER, which bounds the attack to versions > current
  that were ever signed — i.e. genuine releases). Vercel account
  compromise is the residual risk; key rotation = ship a new client
  with a new pubkey (manual install) — documented, not solved.
- **No webview surface:** the updater plugin is registered WITHOUT any
  capability grant; no JS command can trigger checks/installs; only
  status lines cross into the UI. CSP untouched (the updater talks from
  Rust, not the webview).
- **Restart hygiene:** engine children are killed pre-install via the
  winjob seam (test-pinned); the NSIS `/UPDATE /R` flow restarts the
  app per-user, no elevation.
- The pubkey is committed deliberately (public by design); the private
  key must NEVER enter the repo, CI, or logs — signing stays at owner
  staging (§5.1). `dangerous_accept_invalid_certs` /
  `dangerous_insecure_transport_protocol` are NOT set anywhere.

## 7. Assumptions, unresolved risks, next task

Assumptions: (1) `latest.json` will be served from the tracker's
`public/downloads/` (same static surface as the installers — Vercel
serves query-stringed static paths identically; verified only by the
live 302/206 probe of the sibling artifact, not by deploying a real
latest.json). (2) All-OS simultaneous releases continue (single version
bump), so one manifest serves every platform key. (3) The owner's
private key was generated with the Tauri v2 toolchain (the pubkey's
minisign format matches); if it predates the version-in-trusted-comment
CLI, the first train sign must use the current CLI as documented.

Risks: (1) The updater's real round-trip is validated only by the train
+ a staged manifest — as the task specified, no fake E2E was built; the
first 0.2.22→0.2.23 cycle is the live proof. (2) deb-managed Linux
installs self-replace outside dpkg (§5.6). (3) Hosting sessions drop on
the update restart — v0.1-acceptable, the status line says "ready,
restarting…" but a gentler "wait for session drain" policy is a future
owner decision. (4) The harness's geometry assertions await one working
headless-Edge run (§4). (5) `pub_date`/`notes` are advisory only in
this plugin version.

Suggested next task: Test and Release Engineer mirrors the updater
config pin as a CI guard (optional; the Rust-side
`tauri_conf_pins_the_update_channel` already fails the desktop
tauri-shell suite on drift) and stages a signed dummy `latest.json`
against a scratch build to exercise the first real round-trip before
0.2.22 ships.

## 8. A-1 closure — bounded download (2026-10-13, after the security review)

Commit `27c854b`, closing advisory A-1 of
`docs/reviews/review-updater-security-2026-10-13.md` (unbounded in-memory
package buffering + no download timeout → a compromised origin could
OOM-crash the client during download; `on_chunk` cannot cancel). One file
changed: `crates/modelswarm-desktop/src/app/updater.rs`.

**Mechanism.** `download_with_progress` now runs the plugin's
`update.download(...)` inside `drive_bounded_download`, which enforces
both bounds:

- **Wall clock**: `tokio::time::timeout` over the whole download phase
  (connect + stream + buffering) — the bound that always bites, since a
  drip-feed origin under the byte cap can otherwise hold the download
  open (and grow its buffer) indefinitely.
- **Byte cap, mid-stream**: `ChunkIngress` (the `on_chunk` body) counts
  every chunk against the cap and fires a one-shot channel on the FIRST
  breach; a `tokio::select!` between the download future and that signal
  resolves on the breach and **drops the download future** — dropping the
  future is the cancel `on_chunk` cannot perform; the drop closes the
  plugin's response stream and frees the partial buffer. A tail-race
  check re-reads the counter when the future resolved `Ok` in the same
  poll in which it crossed the cap (and the select-reported at-breach
  count wins over the later counter — one chunk can land between signal
  and drop). No progress is ever claimed past the cap.

**The constants chosen.**

| Constant | Value | Rationale |
|---|---|---|
| `DOWNLOAD_TIMEOUT` | 10 min (`600 s`) | Any legitimate package (≤ the cap) completes inside 10 min on a link sustaining ≥ ~0.9 MiB/s; slower is an honestly-reported failed update, running version intact. (The review sketched 30 min; 10 min is tighter and still ~an order of magnitude above observed install times.) |
| `DOWNLOAD_BYTE_CAP` | 512 MiB (`536,870,912` B) | Far above any installer the train ships (engines included); enforced per chunk, so exactly-at-cap is still accepted. |

**Honest-status strings (pinned verbatim by tests):**

- timeout: `update failed — this version keeps running (download timeout)`
- cap: `update failed — this version keeps running (download exceeded the 512 MiB cap)`
- plugin errors keep the prior shape: `… (download: <reason>)`.

Both A-1 edges land in the existing terminal-state discipline:
`InstallFailed`, never a crash, never a restart loop — the 6h timer still
starts the next cycle, `install()` never receives the bytes, and the
running version keeps running.

**Tests added (5; pure/injected-stream discipline, no plugin
construction):**

1. `download_bounds_are_the_reviewed_a1_constants` — pins 10 min / 512
   MiB so drift is a deliberate re-review.
2. `chunk_ingress_throttles_progress_and_signals_the_breach_once` —
   percent-boundary throttle (silent chunk), one boundary report at
   exactly-at-cap, breach signalled exactly once, no progress past the
   cap.
3. `a1_byte_cap_aborts_the_stream_and_no_install_is_attempted` —
   injected 12 × 64 MiB chunk stream aborts AT the breaching 9th chunk
   (576 MiB — the last three chunks never arrive); exact cap line;
   `InstallBegin` ignored; next `CheckStarted` still moves to Checking.
4. `a1_wall_clock_timeout_aborts_a_slow_stream_and_no_install_is_attempted`
   — injected drip-then-silent stream (8 MiB then nothing, under the cap)
   times out under a 50 ms injected bound; exact timeout line; same
   no-install / next-cycle pins.
5. `a_download_within_both_bounds_completes_untouched_and_reaches_ready`
   — exactly-at-cap (8 × 64 MiB = 512 MiB) passes through untouched and
   is the only road to `Ready`.

**Gates (re-run for the closure):** `cargo fmt --check` clean;
`cargo clippy --workspace -- -D warnings` clean;
`TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p
modelswarm-desktop --features tauri-shell --all-targets -- -D warnings`
clean; `cargo test --workspace` **495 / 0 / 6** (baseline exact);
tauri-shell desktop suite **45 / 0 / 0** (was 40; +5), plus 20/20 re-runs
of the cap test against the select-order race and 5 consecutive clean
full-suite runs.

Residual (unchanged from the review): a breach aborts only after the
crossing chunk has been observed, so a bounded sliver past the cap
(chunk-granularity) can be buffered before the drop — the process-wide
OOM vector is closed, the sliver is not zero. A-2 (redirect scheme), A-3
(exit-hook replacement), A-4 (post-hook `ShellExecuteW` failure), A-5
(unsigned installers) remain as recorded in the review.
