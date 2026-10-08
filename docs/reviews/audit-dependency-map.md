# ModelSwarm Dependency + Supply-Chain Map — 2026-10-09

Audit of frozen baseline `730cebf`, product v0.2.20, branch
`audit/master-prompt-2026-10-09`. Fully static: only Read/Grep/Glob/ls/git
were used (no cargo/npm execution, per audit constraints). Every finding
carries file:line evidence. Companion handoff:
`docs/reviews/handoff-dependency-auditor-2026-10-09.md`.

Headline counts: 18 Rust workspace members (17 crates + `apps/modelswarm-sim`),
680 locked crates.io packages, 137 locked npm packages, 1 pinned llama.cpp
engine release (`b11407`) across 4 platforms + 1 GPU variant, 6 GitHub Actions
used (all tag-pinned, none commit-pinned), 15 candidate catalog profiles,
6 golden vectors, 0 git/path/[patch] Cargo dependencies outside the workspace.

---

## 1. Rust workspace

### 1.1 Workspace root (`Cargo.toml`)

- Members: 17 `crates/*` + `apps/modelswarm-sim` (`Cargo.toml:3-22`).
- Version single-sourcing: `[workspace.package] version = "0.2.20"`
  (`Cargo.toml:27-28`); every crate inherits via `version.workspace = true`
  **except `modelswarm-winjob`** (`crates/modelswarm-winjob/Cargo.toml:3`,
  standalone `version = "0.1.0"` — deliberate isolation of the one
  unsafe-allowed crate, but a version island; see F-13).
- `tauri.conf.json` omits `version` on purpose; Tauri falls back to the
  package version, and CI guard `rust.yml:64-81` enforces both that and
  filename/version agreement with `apps/tracker/app/page.tsx`.
- Lints: `unsafe_code = "forbid"` workspace-wide (`Cargo.toml:34-35`);
  only `modelswarm-winjob` overrides to `allow`.
- No `[patch]`, no `[replace]`, no git dependencies anywhere in the
  workspace manifests or `Cargo.lock` (all 662 external sources are
  `registry+https://github.com/rust-lang/crates.io-index`).

### 1.2 Internal crate dependency graph (from `[dependencies]` sections)

Arrow = "depends on". `(opt)` = feature-gated optional; `(win)` =
`[target.'cfg(windows)'.dependencies]`; `(mock)` = feature-enabled mock.

```
modelswarm-types        → (none)
modelswarm-identity     → types
modelswarm-transport    → identity
modelswarm-tracker-api  → identity
modelswarm-eligibility  → types
modelswarm-runtime      → types
modelswarm-scheduler    → types
modelswarm-speculation  → (none)
modelswarm-store        → (none)
modelswarm-gateway      → runtime
modelswarm-telemetry    → (none)
modelswarm-relay        → (none; standalone libp2p host)
modelswarm-winjob       → (none; windows-sys only)
modelswarm-bench        → runtime(mock), scheduler, speculation, types
modelswarm-session      → gateway, identity, runtime, speculation, transport
modelswarm-node         → transport(opt,libp2p-backend), eligibility(opt),
                          gateway, identity, runtime, store, telemetry,
                          tracker-api, types, winjob(win)
                          [node/Cargo.toml:12,20-24]
modelswarm-desktop      → transport (UNCONDITIONAL, libp2p-backend on),
                          winjob(win)
                          + optional under tauri-shell: node, tracker-api,
                          store, gateway, identity, telemetry
                          [desktop/Cargo.toml:34-38]
apps/modelswarm-sim     → identity, transport, runtime(mock), session,
                          speculation
```

Layering is clean: `types`/`speculation`/`store`/`telemetry`/`winjob`/`relay`
are leaves; nothing depends on `node`, `desktop`, `sim`, or `relay` (they are
tops). One asymmetry worth knowing for M1/M2: `desktop` hard-depends on
`modelswarm-transport` with `libp2p-backend` enabled
(`crates/modelswarm-desktop/Cargo.toml:38`), so even the stub desktop build
(non-`tauri-shell`) compiles the full libp2p/quinn stack.

### 1.3 External dependencies per crate (version reqs from manifests)

| Crate | External deps (req) |
|---|---|
| types | serde 1(+derive), serde_json 1, sha2 0.10, hex 0.4 |
| identity | ed25519-dalek 2(+rand_core), rand_core 0.6(+getrandom), sha2 0.10, hex 0.4, base64 0.22, bs58 0.5, serde 1, serde_json 1, time 0.3 |
| transport | tokio 1(full), serde 1, serde_json 1, bytes 1, ed25519-dalek 2, base64 0.22, libp2p 0.57 (opt), futures 0.3 (opt) |
| tracker-api | serde 1, serde_json 1, tokio 1, reqwest 0.12(+json,http2,rustls-tls), ed25519-dalek 2, base64 0.22, sha2 0.10, hex 0.4 |
| eligibility | ed25519-dalek 2, base64 0.22, serde 1, serde_json 1, time 0.3; dev: rand_core 0.6 |
| runtime | async-trait 0.1, serde 1, serde_json 1, thiserror 2, tokio 1, reqwest 0.12(+json, no TLS — loopback), sha2 0.10 |
| scheduler | serde 1, thiserror 2 |
| speculation | (none) |
| store | rusqlite 0.32(+bundled), time 0.3; dev: tempfile 3 |
| gateway | async-trait 0.1, axum 0.8, futures-util 0.3, serde 1, serde_json 1, thiserror 2, tokio 1, tokio-stream 0.1, uuid 1(+v4); dev: http-body-util 0.1, tower 0.5 |
| telemetry | (none — zero-dependency redaction logger) |
| bench | hex 0.4, serde 1, serde_json 1, sha2 0.10, thiserror 2; dev: tokio 1 |
| node | async-trait 0.1, axum 0.8, futures-util 0.3, reqwest 0.12(+json,stream,rustls-tls), sha2 0.10, hex 0.4, serde 1, serde_json 1, thiserror 2, tokio 1, ed25519-dalek 2 (opt), rand_core 0.6 (opt), time 0.3 (opt); win: winjob; dev: rusqlite 0.32(bundled), tempfile 3 |
| winjob | windows-sys 0.59 (Win32_Foundation/Security/JobObjects/Threading) |
| relay | libp2p 0.57 (tokio,quic,noise,yamux,identify,ping,relay,macros), tokio 1, futures 0.3; dev: env_logger 0.11, log 0.4 |
| desktop | tauri 2 (opt, wry), tauri-plugin-opener 2 (opt), tauri-plugin-single-instance 2 (opt), tokio 1 (opt), serde 1 (opt), serde_json 1 (opt), reqwest 0.12(+json,rustls-tls, opt), futures-util 0.3 (opt); build: tauri-build 2 (opt) |
| sim | async-trait 0.1, tokio 1(full), serde_json 1, anyhow 1 |

Only two crates use `anyhow` at all (`sim` only) — workspace standard is
`thiserror`. `tracing` appears in `Cargo.lock` (0.1.44) purely transitively
(libp2p/tauri); no workspace crate declares it.

### 1.4 Load-bearing pins from `Cargo.lock` (680 packages total)

| Package | Locked | Notes |
|---|---|---|
| libp2p | 0.57.0 | 19 libp2p-* subcrates; QUIC via quinn 0.11.12 / quinn-proto 0.11.19 |
| tokio | 1.53.2 | single copy |
| rustls | 0.23.45 | via reqwest rustls-tls + libp2p-tls |
| reqwest | **0.12.28 + 0.13.5** | 0.12.28 = all workspace crates; **0.13.5 pulled solely by tauri 2.12.1** (F-3) |
| serde / serde_json | 1.0.229 / 1.0.151 | single copies |
| ed25519-dalek | **2.2.0 + 3.0.0** | 2.2.0 = identity/eligibility/transport/tracker-api/node; **3.0.0 pulled by libp2p-identity 0.3.0** (F-4) |
| sha2 | 0.10.9 + 0.11.0 | 0.11.0 only via ed25519-dalek 3.0.0 / libp2p-identity 0.3.0 |
| rusqlite | 0.32.1 (bundled SQLite) | store + node dev |
| tauri | 2.12.1 | tauri-build 2.7.1, plugins 2.7.0/2.5.2, wry 0.57.0, webview2-com 0.39.1 |
| windows / windows-sys | 0.62.2 / **0.45.0, 0.52.0, 0.59.0, 0.61.2** | winjob on 0.59; others transitive (F-5) |
| anyhow / thiserror | 1.0.104 / 2.0.21 (+1.0.69 transitive) | thiserror 1 only via gtk/jni/ndk chain |
| tracing | 0.1.44 | transitive only |
| axum / hyper / http | 0.8.9 / 1.11.1 / 1.5.0 | gateway + node loopback servers |
| uuid / base64 / bs58 / time / hex | 1.27.0 / 0.22.1(+0.21.7,0.23.1) / 0.5.1 / 0.3.55 / 0.4.3 | base64 triple-tracked (transitive) |
| proptest / tempfile | 1.11.0 / 3.27.0 | dev-only |

Other duplicated-version families in the lock (all transitive, cosmetically
noisy): getrandom 3, rand_core 3, hashbrown 3, syn 3, toml_edit 3, schemars 3,
curve25519-dalek 2, digest 2, indexmap 2.

### 1.5 Unused/suspicious spot-checks

Spot-checked every unusual declaration; **all are genuinely imported**:
reqwest in runtime (loopback client, `llamacpp.rs:99`), uuid in gateway
(`http.rs:19`), bytes in transport (`frame.rs:15`), axum in node
(`lib.rs:407`), futures-util in desktop (`app.rs:1498`). No declared-but-
unused dependency was found in the sampled set; nothing pulls a suspicious/
typosquatted name. The one soft note: `modelswarm-telemetry` has zero
dependencies by design (custom redaction, no tracing) — consistent with the
content-blind rules, but it means log-format evolution is entirely local code.

---

## 2. Runtime pins (`runtime-pins.json`)

What it pins: the llama.cpp engine release **`b11407`** for windows-x64
(CPU zip + **vulkan GPU variant**, ADR-024), linux-x64, macos-arm64,
macos-x64 — each with `archive_url`, `archive_sha256`, and **per-file
SHA-256 for every bundled binary** plus the full non-bundle file set.
`canonical_build_hash` = windows-x64 CPU archive sha
(`353c4aab…91a03`) and is the runtime identity reported on EVERY OS/backend
(`runtime-pins.json:2-4`). Model weights are never bundled.

How it is consumed:

| Reader | Role | Evidence |
|---|---|---|
| `crates/modelswarm-node/src/engine.rs:32` | `include_str!` compile-time embed; per-file hash verify before every launch; variant (`vulkan`) resolution per ADR-024 | engine.rs:7, 32, 39, 65, 182-207 |
| `crates/modelswarm-runtime/src/llamacpp.rs` | `EngineIdentity` (build_hash) surfaced as runtime `id()` | llamacpp.rs:35-41 |
| `installer/stage-engine-windows.mjs:22` | staging-time fail-closed SHA-256 verify of CPU + Vulkan sets into gitignored `crates/modelswarm-desktop/engine{,-vulkan}/` | stage-engine-windows.mjs:11, 36-69 |
| `.github/workflows/desktop-installer.yml:80-111` | CI download + archive sha + per-file verify (pwsh), then post-build "engine inside artifact" check | desktop-installer.yml:49-159 |
| `scripts/resolve-candidate.mjs` | stamps `runtime.build_hash` into every candidate manifest | resolve-candidate.mjs (header, ADR-022) |
| `.github/workflows/rust.yml:6,8` | pins file triggers the rust CI path filter | rust.yml:6 |

CPU/GPU: both pinned for Windows (vulkan variant, ADR-024: preferred at
launch when present, identity unchanged, CPU fallback). Linux/macOS ship
CPU-only pins (macOS arm64 archive includes the Metal backend dylibs in the
base set; no separate GPU variant). The archive URL+SHA values are
**duplicated** in the desktop-installer matrix (`desktop-installer.yml:24-37`)
vs runtime-pins.json — currently identical, and the per-file re-verify makes
drift fail closed, but it is a second copy to update (F-8).

Consistency check: candidate manifests' `runtime.build_hash`
(`353c4aab…`) equals `canonical_build_hash` — engine identity is single-
sourced correctly across catalog and node.

---

## 3. Toolchain

- `rust-toolchain.toml:5` pins `channel = "1.99.0"`, profile minimal,
  components rustfmt+clippy. Comment records why: a floating `stable` let a
  clippy lint promotion break main's gate (rust-toolchain.toml:2-4).
- `rust.yml:26` uses `dtolnay/rust-toolchain@1.99.0` (consistent, with the
  same components); `tracker.yml:104` also `@1.99.0` (cargo only).
- **Inconsistent:** `desktop-installer.yml:42`, `real-model-e2e.yml:16`,
  `windows-package-dry-run.yml:18` all use `dtolnay/rust-toolchain@stable`.
  The `rust-toolchain.toml` override still forces 1.99.0 for any cargo
  invocation inside the repo, so builds are de-facto pinned — but the
  `@stable` steps download a second, floating toolchain and contradict the
  documented pin rationale (F-2).
- Release pipeline adds `cargo install tauri-cli --version '^2' --locked`
  (`desktop-installer.yml:113`): a caret range, so the installer-producing
  tool floats within tauri-cli 2.x (its own `--locked` lockfile keeps the
  build internally reproducible, but the tool version itself drifts) (F-2).

---

## 4. Tracker (`apps/tracker`)

`package.json` deps are caret ranges; `package-lock.json` (lockfileVersion 3,
137 packages) is the real pin:

| Package | package.json req | Locked |
|---|---|---|
| next | ^15.3.0 | 15.5.27 (incl. `@next/swc-win32-x64-msvc` 15.5.27) |
| react / react-dom | ^19.0.0 | 19.3.0 / 19.3.0 |
| pg | ^8.23.1 | 8.23.1 |
| zod | ^3.24.0 | 3.25.76 |
| @noble/ed25519 | ^2.3.0 | 2.3.0 |
| vitest | ^5.0.3 | 5.0.3 |
| typescript | ^5.6.0 | 5.9.3 |
| ajv (dev) | ^8.20.0 | 8.20.0 |
| @types/node (dev) | ^22.0.0 | 22.20.5 |

Version skew: `package.json:3` = **0.1.0** while the product is 0.2.20.
Nothing reads it — the download page hardcodes the version strings
(`app/page.tsx:11-19`) and the CI guard checks page.tsx against the
**workspace Cargo.toml** version (`rust.yml:64-81`), never package.json. So
it is inert, intentional-looking-since-day-one drift, but it is a third
place a version number could live and mislead (F-6).

Crypto parallel: TS signs/verifies leases with `@noble/ed25519` 2.3.0 while
Rust verifies with `ed25519-dalek` 2.2.0 — cross-validated by the golden
vector `protocol/vectors/lease-hubkey-1.json` + wire-compat CI job
(`tracker.yml:72-147`), so the skew is pinned and tested, not accidental.

---

## 5. Installer + engine staging

- `installer/stage-engine-windows.mjs`: stages pinned CPU + Vulkan engine
  sets from local cache (`%LOCALAPPDATA%\ModelSwarm\engine{,-vulkan}`) into
  the gitignored desktop engine dirs; **every staged file SHA-256-verified
  fail-closed** against runtime-pins.json (lines 11, 36-69); stages the
  LLVM-OpenMP license beside `libomp.dll` for attribution (lines 32-34).
- `crates/modelswarm-desktop/tauri.conf.json`: bundles `engine/*` +
  `engine-vulkan/*` as resources (lines 36-39); NSIS target; per-user
  install; WebView2 via downloadBootstrapper (tauri.conf.json:40-44) — i.e.
  the installer fetches WebView2 from Microsoft at install time (external
  binary not hash-verified by us, standard Tauri behavior) (F-13).
- `crates/modelswarm-desktop/build.rs`: release-build gate panics if the
  engine was never staged (`MSP_ALLOW_NO_ENGINE=1` opt-out) — the v0.2.2
  engine-less-installer class of failure is now blocked at build time.
- NSIS notes (`installer/tauri/nsis-notes.md`): updater disabled until
  signing exists (ADR-008); tauri-cli downloads the NSIS toolchain on first
  run — another unverified external download inside the release toolchain.
- External URLs downloaded anywhere: only `github.com/ggml-org/llama.cpp/
  releases/*` (engine, hash-verified), `huggingface.co` (weights + metadata,
  hash-verified per artifact_hashes), Microsoft WebView2 bootstrapper
  (unverified), tauri-cli's NSIS fetch (unverified). No other hosts.

---

## 6. CI supply chain (`.github/workflows/*.yml`)

All `uses:` across the 5 workflows:

| Action | Where | Pin style | Risk |
|---|---|---|---|
| actions/checkout@v4 | rust.yml:23,57; tracker.yml:18,53,98; desktop-installer.yml:41; real-model-e2e.yml:15; windows-package-dry-run.yml:17 | floating tag (v4) | MEDIUM — tag is mutable; runs in the release pipeline |
| dtolnay/rust-toolchain@1.99.0 | rust.yml:26; tracker.yml:104 | exact-version tag | LOW-MED — matches the toml pin; tag technically movable |
| dtolnay/rust-toolchain@stable | desktop-installer.yml:42; real-model-e2e.yml:16; windows-package-dry-run.yml:18 | floating | MEDIUM — see F-2; toml override rescues actual builds |
| Swatinem/rust-cache@v2 | rust.yml:29; tracker.yml:105; desktop-installer.yml:43; real-model-e2e.yml:17; windows-package-dry-run.yml:19 | floating tag | MEDIUM — cache-poisoning surface |
| actions/setup-node@v4 (node 22, npm cache) | tracker.yml:19,54,99 | floating tag; node major only | MEDIUM / LOW-MED |
| actions/upload-artifact@v4 | desktop-installer.yml:160; windows-package-dry-run.yml:24 | floating tag | MEDIUM |
| postgres:16-alpine (service container) | tracker.yml:36,77 | floating tag, no digest | LOW-MED — test infra only |

No workflow pins any action by commit SHA. For a repo whose desktop-installer
workflow produces the shipped (unsigned) installers, tag-pinning is the
single highest-leverage hardening step (F-1). CI-run `curl` downloads
(real-model-e2e.yml:21-27, desktop-installer.yml:53-79) are all
sha256-verified inline — good.

---

## 7. Website / deploy

- `apps/tracker/next.config.mjs` is minimal (reactStrictMode only) — no
  vercel.json in the repo; deployment is Vercel-CLI-driven from
  `apps/tracker/` with project settings held in the Vercel dashboard
  (architecture §13: project `modelswarm-tracker`, custom domain
  `modelswarm.deepflux.space`, managed Postgres via marketplace).
- Download counting: `apps/tracker/app/api/download/[file]/route.ts` —
  name-pattern guard (line 15), on-disk existence check (line 26), Postgres
  counter bump (line 30), 302 to `/downloads/<file>` (line 32). Not part of
  msp-v1; cannot proxy arbitrary paths.
- Installers are linked as: gitignored
  `apps/tracker/public/downloads/*.{exe,deb,AppImage,dmg}` (`.gitignore:41-46`,
  checksums tracked, binaries never committed — enforced by
  `rust.yml:58-63`), with `SHA256SUMS.txt` referenced and the Windows
  installer sha inlined on the page (`page.tsx:16-20`). The inline sha is a
  display duplicate that must be hand-updated per release (F-14).
- Hub key pinning: `protocol/keys/hub-public.hex` (13a8eead…) is
  `include_str!`-pinned into the node catalog + serving verification
  (`crates/modelswarm-node/src/catalog.rs:13`, `serving.rs:939`); an
  identical copy lives at `apps/tracker/keys/hub-public.hex` (duplication,
  F-13). Rotation requires a rebuild + redeploy of both sides. CI never uses
  production keys — wire-compat uses the fixture key pinned in the golden
  vector (`tracker.yml:93,146`).

---

## 8. Protocol / catalog assets

| Asset | Status | Evidence |
|---|---|---|
| `catalog/schema.json` (v1, catalog-v1.json) | **legacy/draft — no live consumer** (contains `licenseId`, `status`, `maxOutputTokens`) | schema.json:13,53; zero code references |
| `catalog/schema-v2.json` (model-profile-manifest-v2.json, ADR-011) | **live**: TS zod mirror (`apps/tracker/lib/schemas.ts:6-11`), candidates route, catalog envelope, Rust tracker-api (`lib.rs:477`), CI vector validation (`scripts/validate-vectors.mjs:14`) | see grep table in §8 sources |
| `catalog/candidate-profiles/` | 15 `msp1:*` candidate JSON docs (manifest + display_name + status + provenance incl. licenseEvidenceUrl); the wire-compat job seeds one (msp1_948d…) | tracker.yml:113-137 |
| `protocol/vectors/` | 6 golden vectors: `lease-hubkey-1.json` (ADR-012/026 lease, fixture key, 2.2 KB), `manifest-basic.json`, `manifest-no-spec.json`, `manifest-qwen25-05b-q4km-real.json`, `manifest-smollm2-135m-q4km-real.json`, `manifest-smollm2-360m-q8-real.json` — all validated in CI (`tracker.yml:27`) | directory listing; lease-hubkey-1.json:1-8 |
| `experiments/schemas/{mode-result,run-manifest}.schema.json` | bench-record schemas consumed by `crates/modelswarm-bench` tests | bench/src/records.rs:170,273 |

License/attribution pipeline: schema-v2 records `licenseEvidenceUrl` in
provenance (schema-v2.json:81; zod mirror `schemas.ts:192`, optional, max
300). All 15 candidates carry one. **No redistribution of weights**: nodes
download directly from HF pinned revisions (`crates/modelswarm-node/src/
artifact.rs:282`, `crates/modelswarm-desktop/src/app.rs:711`), `*.gguf` and
`models/` are gitignored (.gitignore:35-38), installers bundle only the
engine (+ LLVM-OpenMP license). Gaps: (a) v1's `licenseId` (SPDX-ish id) was
dropped in v2 — only an evidence URL survives, so there is no machine-
readable license field today; (b) 6 of 15 candidates point evidence at
`/blob/main/` — a floating branch (violates the pinned-revision spirit;
artifact revisions themselves are pinned); (c) two candidates are
`bartowski/*` repacks whose evidence URL points at the repack repo's
LICENSE, not the upstream model license (attribution chain depth 1).

---

## 9. Windows-only vs cross-platform (M1/M2 input)

- **Windows-only**: `modelswarm-winjob` (windows-sys 0.59 FFI; the only
  `unsafe` in the workspace, deliberately quarantined), consumed by node and
  desktop under `cfg(windows)`; `webview2-com`/`windows` 0.62.2 (tauri
  windows backend); `winreg`, `uds_windows` (transitive). Local staging
  script `stage-engine-windows.mjs` is Windows-only by name; CI stages
  linux/mac itself.
- **Cross-platform core (no platform deps)**: types, identity, speculation,
  store (rusqlite bundled), telemetry, scheduler, eligibility, runtime,
  gateway, tracker-api, session, bench, relay, sim, transport (libp2p is
  optional-but-on for node/desktop), desktop (tauri matrix builds
  nsis/deb/appimage/dmg).
- M2 implication: the resource-governor seed is entirely inside winjob
  (kill-on-close + total RAM). All governor-relevant Windows API surface is
  confined to one crate behind a cfg gate — the platform-neutral crate the
  master prompt plans slots in cleanly; no other crate touches platform APIs
  directly.

---

## 10. Findings (severity-ranked)

**CRITICAL** — none. No unverified downloads in any production path, no git/
path deps outside the workspace, no [patch], no secrets or private keys
committed (`keys/` hold public halves only; CI seeds are fixtures), no
prompts/completions storage dependencies.

**HIGH**

- **F-1 — CI actions are tag-pinned, not commit-pinned.** Every `uses:` in
  all 5 workflows floats on a mutable tag (checkout@v4, rust-cache@v2,
  setup-node@v4, upload-artifact@v4, dtolnay@*). The desktop-installer
  workflow builds the installers users run; a moved/compromised tag flows
  straight into shipped artifacts. Evidence: §6 table.
- **F-2 — release-pipeline toolchain drift.** desktop-installer/real-model-
  e2e/windows-package-dry-run use `@stable` (desktop-installer.yml:42,
  real-model-e2e.yml:16, windows-package-dry-run.yml:18) against the
  documented 1.99.0 pin rationale (rust-toolchain.toml:2-4), and the
  installer job installs `tauri-cli --version '^2'` (desktop-installer.yml:
  113) — a floating tool in the release path. Builds remain de-facto 1.99.0
  via the rust-toolchain.toml override; the drift is real but mitigated.

**MEDIUM**

- **F-3 — two reqwest stacks in the shipped desktop binary.** Workspace
  crates pin 0.12.28; tauri 2.12.1 pulls 0.13.5 (Cargo.lock reverse-deps:
  only tauri → reqwest 0.13.5). Contradicts the "one HTTP stack" rationale
  recorded at `crates/modelswarm-runtime/Cargo.toml:14-16` — the unification
  holds for the node binary but not the desktop shell.
- **F-4 — two Ed25519 generations in the P2P path.** Workspace identity
  stack: ed25519-dalek 2.2.0; libp2p-identity 0.3.0 uses ed25519-dalek
  3.0.0 (+sha2 0.11.0). Node identity signatures and libp2p handshake
  signatures come from different dalek major versions in one binary.
  Interop is fine (same curve/algorithm) but audit surface doubles.
- **F-5 — four windows-sys copies** (0.45/0.52/0.59/0.61.2) plus windows
  0.62.2. winjob targets 0.59 (winjob/Cargo.toml:15). Transitive, but the
  Win32 boundary layer is compiled four times.
- **F-6 — version triple-tracking around the tracker.** package.json 0.1.0
  (inert), page.tsx hardcoded "0.2.20" strings + inline sha
  (page.tsx:11-19), workspace Cargo.toml 0.2.20. CI guard covers
  Cargo↔page.tsx only; package.json silently disagrees; the inline Windows
  sha must be hand-bumped every release.
- **F-7 — npm caret ranges; lockfile is the only real pin.** All tracker
  deps are `^` (package.json:15-31); any `npm update`/lockfile regen can
  jump next 15.5.x→15.6 etc. Lockfile v3 + `npm ci` in CI keeps deploys
  deterministic today.
- **F-8 — engine archive URL/SHA duplicated** in desktop-installer.yml
  matrix (lines 24-37) vs runtime-pins.json. Fail-closed per-file verify
  prevents silent drift, but two sources must be updated together.
- **F-9 — floating `main` in 6 candidate license-evidence URLs**
  (candidate-profiles 06e7f70f…, bc98fba0…, fc5a30ae…, 1e496b3f…,
  d45c55cc…, d4c851e6…): `/blob/main/LICENSE` violates the pinned-revision
  discipline even for evidence links.
- **F-10 — license metadata is URL-only.** schema-v2 dropped v1's
  `licenseId`; there is no machine-readable license field for M3's
  auto-selection gates (schema.json:53 vs schema-v2.json:81). bartowski
  repack candidates cite the repack's LICENSE, not the upstream model's.
- **F-11 — postgres:16-alpine CI image not digest-pinned** (tracker.yml:36,
  77). Test-only exposure.
- **F-12 — desktop stub still compiles libp2p.** Unconditional
  `modelswarm-transport` dep with `libp2p-backend` (desktop/Cargo.toml:38)
  pulls quinn/libp2p into even non-tauri-shell builds; heavier supply-chain
  surface than the feature flags imply.

**LOW**

- **F-13 — duplicated public-key + staging artifacts**: two identical
  `hub-public.hex` copies (protocol/keys, apps/tracker/keys); tauri-cli's
  NSIS toolchain + Microsoft WebView2 bootstrapper downloaded at
  build/install time without our own hash verification (tauri.conf.json:41-43).
- **F-14 — winjob version island** 0.1.0 vs workspace 0.2.20
  (winjob/Cargo.toml:3) — intentional isolation, worth a comment or
  inheritance if it ever ships outside the installer.
- **F-15 — transitive thiserror 1.0.69 / base64 ×3 / tracing unused-first-
  party** — cosmetic lock noise; no action needed until a dedupe pass.
- **F-16 — `ajv` needed by CI `validate:vectors` lives in devDependencies**
  (package.json:28) — correct as long as vector validation never runs in
  production deploy scripts; document that assumption.

### Confirmed drift/skew summary

| Concept | Where | Values | Verdict |
|---|---|---|---|
| Product version | Cargo.toml:28 / page.tsx:17 / package.json:3 | 0.2.20 / 0.2.20 / 0.1.0 | package.json drift, inert (F-6) |
| Engine identity | runtime-pins.json:4 / candidate manifests build_hash | 353c4aab… both | consistent |
| Engine archives | runtime-pins.json vs desktop-installer.yml:24-37 | b11407 both | consistent but duplicated (F-8) |
| Toolchain | rust-toolchain.toml:5 vs 3 workflows `@stable` | 1.99.0 vs floating | drift, mitigated by toml override (F-2) |
| Ed25519 impl | workspace dalek 2.2.0 vs libp2p dalek 3.0.0 | 2 majors | duplication (F-4) |
| HTTP client | reqwest 0.12.28 (workspace) vs 0.13.5 (tauri) | 2 minors | duplication in desktop only (F-3) |
| Rust vs TS lease crypto | dalek 2.2.0 vs @noble/ed25519 2.3.0 | cross-language | pinned + golden-vector tested — OK |

### Proposed actions (priority order)

1. Commit-pin all CI actions (checkout/rust-cache/setup-node/upload-artifact/
   dtolnay) and add a note in the release checklist; optionally pin
   postgres:16-alpine by digest (F-1, F-11).
2. Align the three `@stable` workflow steps to `@1.99.0` and pin tauri-cli
   to an exact version (`--version 2.x.y --locked`) (F-2).
3. Single-source the desktop-installer matrix archives from
   runtime-pins.json (generate the matrix or read the pins file) (F-8).
4. Fix the 6 `/blob/main/` evidence URLs to pinned revisions; consider
   reinstating a machine-readable license field via ADR before M3 (F-9/F-10).
5. Align package.json version with the workspace (or strip it) and derive
   page.tsx strings/sha from one source (F-6).
6. Record the dalek/reqwest duals as accepted-until-upgrade in the risk
   register; revisit when tauri or libp2p unify reqwest 0.13 / dalek 3
   (F-3/F-4).
7. For M1/M2: keep the governor platform-neutral by extending the winjob
   pattern (one cfg-gated crate per OS) — no other structural change needed
   (§9).

### Uncertainties

- Cargo.lock lists `tauri` (and thus reqwest 0.13.5, windows 0.62.2, wry)
  even though `tauri-shell` is feature-off by default — lockfile-level
  presence does not mean the shipped node binary links them; only desktop
  builds with `tauri-shell` do. Binary-level inclusion was not verified
  (no cargo runs allowed in this audit).
- Whether Vercel project settings pin a Node version / region / build
  command is outside repo visibility (no vercel.json); assumed managed in
  the dashboard per architecture §13.
- `npm ls`-style unused-dependency analysis was not possible without npm
  runs; TS-side spot-checks (ajv, @types/*) were done by grep of imports.

Static sources used: all `crates/*/Cargo.toml`, `apps/modelswarm-sim/
Cargo.toml`, `Cargo.lock`, `runtime-pins.json`, `rust-toolchain.toml`, all
`.github/workflows/*.yml`, `apps/tracker/package.json` +
`package-lock.json` + `next.config.mjs` + `app/**` (routes/page), installer/
scripts, catalog/*, protocol/vectors/*, protocol/keys/*, and grep of the
consuming crates (`engine.rs`, `llamacpp.rs`, `catalog.rs`, `serving.rs`,
`artifact.rs`, `app.rs`, `stage-engine-windows.mjs`, `resolve-candidate.mjs`,
`validate-vectors.mjs`).
