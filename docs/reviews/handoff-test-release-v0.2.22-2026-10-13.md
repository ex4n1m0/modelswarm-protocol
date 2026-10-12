# Handoff — Test and Release Engineer: v0.2.22 release train (2026-10-13)

Owner-approved continuous-release mandate: cut v0.2.22 (all-OS, one
version bump). The two live tester clients update manually to this one
build, then self-update forever after. Feature set since 0.2.21:
picker census chips (094b537), resource monitor panel (10d8ff1),
ADR-034 one-click family pairing (905b362), config-writer fix
(6c249be), auto-updater client (ea47032 + hardening 5014481 + bounded
download 27c854b — the A-1 fix is IN this release).

## 1. Changed files and why

| File | Change |
|---|---|
| `Cargo.toml` | Version bump 0.2.21 → 0.2.22 (the single source; tauri.conf.json carries no version — guard-verified on every push). Plus `exclude = ["tests/updater-manifest-verify"]` so the rehearsal tool lives under T/R-owned `tests/` outside the product workspace. |
| `Cargo.lock` | Member versions refreshed via `cargo update -w` — exactly 17 entries 0.2.21→0.2.22, zero dependency-edge changes. |
| `docs/verification/v0.2.22-release-notes-2026-10-13.md` | Honest user-facing notes: census chips, monitor panel, family pairing (probe labels `pairing_confirmed`/`pin_mismatch`), config-fix, SELF-UPDATER (6h + on-start, client-side minisign verification against the built-in key, restart-to-install, bounded download 10 min / 512 MiB) — "this is the last manual install"; standing opt-in gates stated (family needs `MSP_LISTENER=1` + engine bundle; direct/LAN only, no relay); RC-1 obligations recorded; no performance claims made. |
| `tests/updater-manifest-verify/` (new) | The standing rehearsal tool (minisign-verify `=0.3.0`, the plugin's locked version): replays the exact tauri-plugin-updater 2.13.3 chain over the staged manifest — embedded-pubkey decode, base64-of-`.sig` → `Signature::decode`, `verify(bytes, sig, allow_legacy=true)`, `requireSignedVersion` emulation, `/api/download` NAME_PATTERN + staged-file existence; hand-rolled SHA-256 pinned by known vectors (empty/abc/1000×a; cross-checked against `hashlib`). Workspace-excluded, never compiled by CI guards. |
| `crates/modelswarm-winjob/src/lib.rs` | **CROSS-PATH (Windows Product review requested):** `cfg(windows)` on the test helper `job_test_lock` — compile-only, `#[cfg(test)]` code only, zero shipped-binary bytes change. See §3. |
| `apps/tracker/public/downloads/SHA256SUMS.txt` | Retargeted to the four 0.2.22 installers (tracked surface; binaries/sigs/manifest stay gitignored). |
| `apps/tracker/app/page.tsx` | **CROSS-PATH (Tracker Engineer review requested):** landing download section retargeted 0.2.21→0.2.22 + one honest self-update clause + "AppImage (self-updating)" label — the section hardcoded 0.2.21 links which 404 once the 0.2.21 binaries are removed from staging. Copy rules kept (no engine/format names beyond the existing set, honest tense, no status pills). |
| `.gitignore` | `downloads/*.sig` + `downloads/latest.json` added (owner-local deploy staging, same rule as the binaries). |
| `docs/verification/updater-manifest-rehearsal-2026-10-13.md` | The Security-review-required staged-manifest rehearsal record (see §4). |

Commits: `aa90731` (bump + notes + tool) → `b1c2234` (winjob gate fix) →
`d1c2dc0` (staging + SHA256SUMS + rehearsal + page retarget; guards
green here — the established two-push window). Zero production-code
delta between the artifact-build commit `aa90731` and final head
`d1c2dc0` (the only code change, `b1c2234`, is `cfg(test)`-only).

## 2. Exact commands and outcomes

Local gates (before the bump push, at 7e07187+bump):
`cargo fmt --all --check` clean; `cargo clippy --workspace
--all-targets -- -D warnings` clean; `cargo test --workspace`
**495/0/6** (baseline exact); `TAURI_CONFIG='{"bundle":{"resources":[]}}'
cargo test -p modelswarm-desktop --features tauri-shell` **45/0/0**
(the new baseline — was 40 at ea47032..7e07187). Tracker (after the
page edit): `npm run typecheck` clean; `npm run test` 136 passed /
2 skipped; `npm run build` green.

CI (GitHub Actions, observed):
- **desktop-installer 38283004626** (workflow_dispatch on `aa90731`;
  the bump push itself touches no desktop/node path — the run history's
  in-flight 38281532370 belonged to 7e07187/0.2.21 and was not used):
  **SUCCESS** — windows nsis + ubuntu deb/appimage + macos arm64 dmg,
  engine b11407 hash-verified per job and verified INSIDE each
  artifact.
- **rust 38283538702** (b1c2234): check-nonwindows-capi SUCCESS;
  guards FAILURE = exactly the expected staging-window line
  ("SHA256SUMS.txt does not match version 0.2.22") and nothing else;
  check cancelled by the next push (concurrency) — superseded below.
- **Final commit `d1c2dc0`: rust 38285433454 SUCCESS (check +
  check-nonwindows-capi + guards all green), tracker 38285433492
  SUCCESS, windows-package-dry-run 38285433502 SUCCESS.**

Artifacts: `gh run download 38283004626`; hash chain verified THREE
ways (CI job-log sha256sum == per-job artifact SHA256SUMS.txt == local
re-hash == staged copies, `sha256sum -c` all OK):

| file | sha256 |
|---|---|
| `ModelSwarm-Setup-0.2.22-windows-x64.exe` | `b38dd07de95adbc65d2958951bafe07cd11ee4fb24776d51120d4563a68ebc89` |
| `ModelSwarm-0.2.22-linux-amd64.deb` | `2848ede9822cd96aea4fd0866f8a41eebe5846b4e1911b61d884b95f5e2736ac` |
| `ModelSwarm-0.2.22-linux-amd64.AppImage` | `74a89987086504b65d05a257a57e537d1f18033730144e2304e4169d152c5c02` |
| `ModelSwarm-0.2.22-macos-arm64.dmg` | `012f0046c60f164135312b78e48e944e169b8f07e676321c4ebee6768e6646d0` |

Signing (owner-local; key file path only ever appeared on command
lines — contents never printed, never committed):
`npx -y @tauri-apps/cli@2 signer sign -f "C:\Users\igorc\
ModelSwarm-updater.key" --app-version 0.2.22 <file>` ×4 — all OK
(non-interactive; passwordless key). `.sig` trusted comments carry
`file:<canonical name>` + `version:0.2.22`.

## 3. Gate find: pre-existing Linux breakage from ea47032 (fixed)

The 0.2.22 bump push was the first since ea47032 whose rust.yml run
wasn't cancelled by a subsequent push before the ~20-min ubuntu job
finished — and check-nonwindows-capi promptly failed: `job_test_lock`
(winjob test serializer, added ea47032) lacked `cfg(windows)` while
all three callers are windows-gated → dead code under `-D warnings` in
the non-Windows lib-test compile. **Every rust run since ea47032
(38277790548, 38279076177, 38281532443) was cancelled mid-flight by
the next push, so main had been carrying this silently.** Fix `b1c2234`
(`cfg(windows)` on the helper): compile-only, test-module-only;
local fmt/clippy/`cargo test -p modelswarm-winjob` 3/0/0 green; Linux
proven by CI (SUCCESS on b1c2234 and on final d1c2dc0). Landed under
the same release-gate authority as the 0.2.21 train's runtime/node
compile fixes — **Windows Product review requested** (this handoff).
Process note for the Integrator: pushes at <20-min spacing hide this
gate entirely; consider making check-nonwindows-capi its own
concurrency group (T/R can do this on request — it is a CI-only
change, no product code).

## 4. Signed-manifest rehearsal (Security review requirement)

Record: `docs/verification/updater-manifest-rehearsal-2026-10-13.md`.
(a) `latest.json` serves from the staging tree over HTTP with the
plugin's full query-param URL shape; fetched == staged. (b) ALL THREE
platform signatures VERIFY — `cargo run --manifest-path
tests/updater-manifest-verify/Cargo.toml` — against the
tauri.conf.json-embedded key, which is byte-identical to
`C:\Users\igorc\ModelSwarm-updater.key.pub` (key id 38FC98CB4B064F37),
over the staged artifact bytes, signed version == 0.2.22; negative
controls bite (tampered byte → `InvalidSignature`; manifest version
0.2.23 → mismatch on all platforms; restore re-verified by hash +
final PASS). (c) `/api/download/<file>` resolves on the real Next
route (MemoryStore dev server): 302 → `/downloads/<file>` → 200 with
exact content-lengths; ranged GET 206 — matching the live 0.2.21
302→206 production probe. `latest.json` format note: the `signature`
field is base64-of-`.sig`-file (what this tauri-cli emits as the
single-line `.sig` and what the plugin's `base64_to_string` +
`Signature::decode` chain requires); windows-x86_64 → exe,
linux-x86_64 → AppImage (the self-replacing rider), darwin-aarch64 →
dmg; the deb stays site-only.

## 5. RC-1 key-lifecycle obligations (standing, now in the train record)

1. `--app-version <VERSION>` on every signer invocation, every train —
   mandatory or every client rejects the update.
2. Private key custody is owner-local only: never the repo, never CI
   (no `TAURI_SIGNING_PRIVATE_KEY` anywhere in `.github/`), never logs.
3. Rotation = a manual client ship with a new embedded pubkey; the old
   public key stays valid on deployed clients for one full adoption
   cycle — treat the old private key as compromised-and-live until
   then.
Desktop tauri-shell test baseline is now **45/0/0** (was 40); workspace
baseline 495/0/6 unchanged.

## 6. Assumptions

1. The embedded pubkey is the owner's real channel key (decodes to a
   valid minisign Ed25519 key; `.pub` file matches byte-for-byte) and
   the private half is the matching owner-local key (all four
   signatures verify against it — proven, not assumed).
2. Vercel serving of `/downloads/latest.json` behaves like any static
   file (query params ignored) — proven locally over the same files;
   the production origin's behavior is the Integrator's post-deploy
   probe.
3. Testers update manually to 0.2.22 exactly once (site links or
   direct URLs); from 0.2.22 the updater chain takes over. A 0.2.21
   client fetching the new manifest sees "update available → v0.2.22".

## 7. Unresolved risks

1. **Vercel deploy pending (Integrator/owner).** Until it runs, the
   live site still serves 0.2.21 links (now 404 on the counting route
   against the NEW deployment's file set — but the CURRENT live
   deployment still bundles its own 0.2.21 files, so live links keep
   working until the deploy flips). Post-deploy probe (must pass
   before telling testers): `curl -sI https://modelswarm.deepflux.\
   space/api/download/ModelSwarm-Setup-0.2.22-windows-x64.exe` → 302
   to `/downloads/...`; plus `curl -s https://modelswarm.deepflux.\
   space/downloads/latest.json` returns version 0.2.22.
2. First REAL self-update cycle is exercised only when a 0.2.22 client
   sees a future 0.2.23 manifest (the rehearsal verified the exact
   chain offline; the review's §10 assumption 2 is discharged to the
   extent possible pre-deploy).
3. Cross-path edits awaiting owner-brokered review: winjob cfg fix
   (Windows Product), page.tsx retarget (Tracker Engineer).
4. RC-2 (v1 obligation, unchanged): no rollback fence; withholding and
   re-serving of genuine newer signed versions remain unbounded —
   recorded in threat-model §4.5/§5.6.

## 8. Suggested next task for the Integrator

Run the Vercel deploy of `apps/tracker` (staged: 4 installers + `.sig`
set + `latest.json` + SHA256SUMS in `apps/tracker/public/downloads/`),
then the two post-deploy probes above, then point the two testers at
the 0.2.22 installers. After that: broker the two cross-path reviews
(§7.3) and, if agreed, let T/R split check-nonwindows-capi into its
own concurrency group so push cadence can no longer hide it (§3).
