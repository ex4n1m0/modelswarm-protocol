# Handoff — Test and Release Engineer: the v0.2.21 release train (2026-10-12)

Owner approval on record (2026-10-12): "all approved and you may release
2.2.1 to the website also" — v0.2.21, ALL-OS per the standing rule. This
file is the release-checklist execution record: every command, outcome,
and hash. VERCEL DEPLOY was deliberately NOT run (owner/Integrator step).

## 0. Release-train commits (main)

| Commit | Content |
|---|---|
| `39dea6f` | fix(runtime)+ci: ffi.rs unsafe wrap + the new `check-nonwindows-capi` CI gate |
| `034ba17` | v0.2.21: version bump + ADR-032 §7 item-(iii) record + release notes |
| `0a8be57` | fix(node): `_telemetry` rename (second gate find) — the artifacts build |
| `4258b3c` | site-staging: SHA256SUMS.txt retargeted (final gates push) |

`zcode-master-prompt.md` (pre-existing untracked, not mine) excluded.

## 1. Blocker found and fixed en route (both cross-path, Integrator-approved)

The 3-OS desktop-installer matrix was RED at the light-up commits
(`55d802e` run `38238154483`, `2ffc8a7` run `38242527580`):
`crates/modelswarm-runtime/src/capi/ffi.rs:905` called
`libloading::Library::new` without an unsafe block — unsafe since
libloading 0.8.9 (lockfile-pinned). Windows compiled because the
`cfg(not(windows))` branch is cfg'd out there, and EVERY other CI surface
(rust.yml `check` incl. the capi-adapter feature steps) runs on
windows-latest: the non-Windows half of the workspace's only unsafe module
had never compiled anywhere until the light-up chained
`tauri-shell -> verifydrafts-capi -> capi-adapter` into the installer
matrix. Third strike of the windows-only-CI blind-spot class (wire bugs →
feature unification → OS).

1. `39dea6f` (Runtime-owned path): unsafe wrap + safety comment mirroring
   the Windows branch (path = canonicalized library inside the sha256-
   verified runtime-pins.json bundle; constructor execution accepted
   identically to `load_with_flags`). Compile-only, zero behavior change.
2. `0a8be57` (Windows-Product-owned path): `engine.rs::launch`'s
   `telemetry` param used only inside the `cfg(windows)` winjob block →
   unused-variable error under `-D warnings` on Linux; renamed to
   `_telemetry`. Found by the NEW gate itself (run `38244718360`) on its
   first execution. Compile-only, zero behavior change; the installer
   matrix never hit it (plain builds don't deny warnings).

Both landed under Integrator approval relayed by the coordinator
(2026-10-12, release-gate authority, precedent `bdbded3` ci+fix). ACTION
REQUESTED: Runtime Engineer + Security review the `ffi.rs` unsafe wrap;
Windows Product review the `_telemetry` rename.

## 2. CI hardening (owned path, same train)

`rust.yml` gained `check-nonwindows-capi`: ubuntu-latest, pinned 1.99.0 +
clippy, TAURI_CONFIG overlay,
`cargo clippy --workspace --all-targets --features modelswarm-runtime/capi-adapter -- -D warnings`,
per-push on all rust paths. Compile-only (no engine staging, no tests) —
closes the OS axis of the blind-spot class for the highest-risk feature.
It paid for itself immediately (§1 item 2).

## 3. Exact commands and outcomes (local, Windows 10.0.26300, toolchain 1.99.0)

```text
cargo fmt --all --check                                               CLEAN (after each edit)
cargo clippy -p modelswarm-runtime --all-targets --features capi-adapter -- -D warnings    CLEAN
cargo clippy -p modelswarm-runtime --all-targets --features capi-adapter --target x86_64-unknown-linux-gnu -- -D warnings   CLEAN
cargo clippy -p modelswarm-node --all-targets -- -D warnings           CLEAN
cargo clippy --workspace --all-targets -- -D warnings                  CLEAN
cargo test --workspace                                                469 passed / 0 failed / 6 ignored (baseline EXACT)
cargo update -w                                                       lock refresh: 18 member versions 0.2.20 -> 0.2.21, no dep changes
```

Cross-target check limitation (recorded honestly): a full-workspace
linux-target clippy from this Windows host is blocked by `ring` /
`aws-lc-sys` / `libsqlite3-sys` cc build scripts needing a cross C
compiler — environment-only; ubuntu runners have gcc, and the CI gate is
the authority.

## 4. Version surfaces audit

Single source bumped: workspace `Cargo.toml` `0.2.20 -> 0.2.21`
(`tauri.conf.json` carries no version — guard-checked). `Cargo.lock`
member versions refreshed. `grep 0\.2\.20` across tracked files: the only
product-facing surfaces were `Cargo.toml` / `Cargo.lock` /
`apps/tracker/public/downloads/SHA256SUMS.txt` (retargeted in §6); the
landing page has had no version/download references since `89f6b83`;
remaining doc mentions are historical records (audits, handoffs, ADRs)
and stay. No CHANGELOG file exists in the repo.

## 5. CI verdicts (release window)

| Run | Workflow | Commit | Verdict |
|---|---|---|---|
| `38244734948` | desktop-installer (manual dispatch — bump commits don't match its path triggers) | `034ba17` | SUCCESS 3/3 OS (first proof the ffi fix holds) |
| `38244718360` | rust | `034ba17` | check-nonwindows-capi FAILURE (the `_telemetry` find, fixed by `0a8be57`); guards FAILURE (expected pre-staging); run then superseded |
| `38244718230` | windows-package-dry-run | `034ba17` | SUCCESS |
| `38245849470` | rust | `0a8be57` | check SUCCESS; check-nonwindows-capi SUCCESS; guards FAILURE (expected pre-staging — SHA256SUMS still 0.2.20 at that commit) |
| `38245849555` | windows-package-dry-run | `0a8be57` | SUCCESS |
| `38245849525` | desktop-installer | `0a8be57` | SUCCESS 3/3 OS after one transient rerun: the first windows attempt failed "Fetch pinned engine" with a download checksum mismatch (no curl retry on that step; same pin verified green 11 min earlier and in every run since) — `gh run rerun --failed` → green. Hardening suggestion routed (§8). THE ARTIFACT BUILD. |
| `38249001205` | windows-package-dry-run | `4258b3c` (final) | SUCCESS |
| `38249001359` | tracker | `4258b3c` (final) | SUCCESS (check + integration-postgres + wire-compat) |
| `38249001299` | rust | `4258b3c` (final) | SUCCESS — check SUCCESS; check-nonwindows-capi SUCCESS; guards SUCCESS (workflow conclusion success) |

The two pre-staging guards failures are the established two-push staging
window (hashes cannot exist before artifacts build), acknowledged in the
release brief; the FINAL commit `4258b3c` is fully green across every
workflow that runs on it, and desktop-installer is green on `0a8be57` —
the commit that produced the staged artifacts (the staging commit changes
only the manifest + docs and matches no desktop-installer path trigger).

## 6. Staged artifacts + hash table

From run `38245849525` (commit `0a8be57`), downloaded via
`gh run download`; hash chain verified THREE ways before staging: CI
job-log `sha256sum` output == per-job artifact `SHA256SUMS.txt` == local
downloaded files; staged copies re-verified with `sha256sum -c` (all OK).
Tauri's underscored outputs renamed to the canonical dashed filenames the
`/api/download` surface and the guards expect. 0.2.20 binaries REMOVED
from the gitignored staging dir (all-OS-together rule — only 0.2.21 is
servable). Only the manifest is tracked.

| Staged file (apps/tracker/public/downloads/) | sha256 |
|---|---|
| `ModelSwarm-Setup-0.2.21-windows-x64.exe` | `32543f0999698fe939a0224af9ea7ca3e92d3b9a74db8e5c851c2da446666eff` |
| `ModelSwarm-0.2.21-linux-amd64.deb` | `83cf3841c574365a0654ef230c6a121f64bb5283a7dbd0795df5245fd1b5875d` |
| `ModelSwarm-0.2.21-linux-amd64.AppImage` | `946a7e56fb28ff93b96e577ded65c9f74f691b44ffac7fc89d3358a7704aa6b2` |
| `ModelSwarm-0.2.21-macos-arm64.dmg` | `6390fa7126785c14480e6a503ba73dcb1ab985e2b978897f485c64e40c9dcf02` |

Engine pin b11407 unchanged; every job hash-verified the archive + every
pinned file (runtime-pins.json) and verified the engine INSIDE each
artifact (NSIS/deb/AppImage/dmg unpack checks).

## 7. Release records shipped

- `docs/verification/adr-032-lightup-exactness-2026-10-12.md` — ADR-032
  §7 item (iii) discharged: composed real-engine E2E green x3 (one
  roster-seeded at `55d802e`, two presented-lease at `2ffc8a7`), all
  exactness pins, honest venue labels, beta boundaries, and the
  environment note (Windows exactness; Linux/macOS capi compile-verified
  only).
- `docs/verification/v0.2.21-release-notes-2026-10-12.md` — honest
  user-facing notes (what is new, never-worse engage gate, beta
  boundaries, no product performance claims).
- Owner/Protocol seat still owes the ADR-032 §7 completion line (not my
  path).

## 8. Assumptions, risks, suggested next tasks

- Assumptions: (a) the release brief's owner approval covers the website
  publication step I did NOT execute; (b) the two pre-staging guards
  reds are acceptable per the established staging window; (c) presenting
  hashes for a commit one below the manifest commit is sound because the
  staging commit touches no compiled code (guard design already encodes
  this: desktop-installer has no manifest path trigger).
- Risks / routed items: (1) Runtime + Security review of the `ffi.rs`
  unsafe wrap; Windows Product review of `_telemetry` (§1). (2)
  desktop-installer's engine-fetch curl has no `--retry` — one transient
  checksum mismatch cost a rerun mid-train; recommend hardening the
  fetch step (my path, deliberately not done mid-release). (3)
  desktop-installer and windows-package-dry-run use
  `dtolnay/rust-toolchain@stable` while rust.yml pins 1.99.0 — worth
  aligning (my path, next train). (4) The A-2/A-3 advisories and the
  RAM-doubling/governor coordination remain open with their owners — the
  release notes state the beta boundary honestly.
- Suggested next tasks: Integrator runs the VERCEL deploy (the staging
  dir is ready); owner/Protocol seat appends the ADR-032 §7 completion
  line; Runtime/Security/Windows-Product ack the two compile fixes; then
  the G6 prompt-pairing UX + governor coordination as the next product
  work.

## 9. Final-gate summary

FINAL COMMIT `4258b3c`: rust (check + check-nonwindows-capi + guards)
GREEN, tracker GREEN, windows-package-dry-run GREEN;
desktop-installer GREEN on the artifact build commit `0a8be57`. All four
0.2.21 installers staged with verified hashes; SHA256SUMS.txt tracked
and guard-validated; no binaries committed (guard-verified); Vercel
deploy left to the Integrator. v0.2.21 is RELEASED from the release
gate's perspective.
