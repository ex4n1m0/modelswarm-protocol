# Handoff — Dependency Auditor — 2026-10-09

Per `AGENTS.md` handoff format. Deliverable:
`docs/reviews/audit-dependency-map.md`.

## 1. Changed files and why

1. `docs/reviews/audit-dependency-map.md` (NEW) — the full dependency +
   supply-chain map, findings, and proposed actions.
2. `docs/reviews/handoff-dependency-auditor-2026-10-09.md` (this file).

No other file was created, modified, staged, committed, or pushed. No cargo,
npm, vercel, or network-mutating command was run (constraint: the validation
suite holds `target/`; node_modules untouched; `target/node_modules` never
entered).

## 2. Scope

Complete internal + external dependency and supply-chain map of the frozen
baseline (`730cebf`, product 0.2.20, branch `audit/master-prompt-2026-10-09`):
Rust workspace graph + Cargo.lock pins, runtime-pins.json engine pinning,
toolchain/CI consistency, tracker npm chain, installer/engine staging,
website/deploy chain, protocol/catalog assets, license pipeline, Windows-only
vs cross-platform split, unused-dep spot-checks.

## 3. Exact commands/static sources used

Static analysis only — Read/Grep/Glob/ls/git plus two read-only `node -e`
parsers over `Cargo.lock` and `apps/tracker/package-lock.json` (no installs,
no builds):

- `cat`/Read: root `Cargo.toml`, all 17 `crates/*/Cargo.toml`,
  `apps/modelswarm-sim/Cargo.toml`, `rust-toolchain.toml`,
  `runtime-pins.json`, all 5 `.github/workflows/*.yml`,
  `apps/tracker/package.json`, `next.config.mjs`, `page.tsx`,
  `app/api/download/[file]/route.ts`, `tauri.conf.json`,
  `crates/modelswarm-desktop/build.rs`, `installer/stage-engine-windows.mjs`,
  `installer/tauri/nsis-notes.md`, `catalog/schema.json` +
  `schema-v2.json`, sample `catalog/candidate-profiles/*.json`,
  `protocol/vectors/*`, `protocol/keys/hub-public.hex`,
  `apps/tracker/keys/hub-public.hex`, `.gitignore`,
  `docs/architecture.md` §13, `AGENTS.md`, `zcode-master-prompt.md`.
- `grep`: reverse-dependency and consumer discovery (runtime-pins readers,
  licenseId/licenseEvidence consumers, hf URL construction in
  `crates/modelswarm-node/src/artifact.rs:282` and
  `crates/modelswarm-desktop/src/app.rs:711`, hub-key `include_str!` sites
  `catalog.rs:13` / `serving.rs:939`).
- `node -e` (read-only parsers): Cargo.lock package/reverse-dep extraction
  (680 packages, all `registry+https://github.com/rust-lang/crates.io-index`,
  zero git/path/patch sources); package-lock version extraction.

No test evidence is attached because this audit is read-only by mandate; the
"command evidence" is the grep/read inventory above plus file:line citations
in the deliverable.

## 4. Findings (severity + evidence)

Full detail in `audit-dependency-map.md` §10. Summary:

**CRITICAL — none.** No unverified external download in any production path
(engine + weights hash-verified end to end), no git/path deps, no [patch],
no committed secrets, hub keys in repo are public halves only.

**HIGH**
- F-1 CI actions tag-pinned, not commit-pinned — every `uses:` in all five
  workflows floats (checkout@v4, rust-cache@v2, setup-node@v4,
  upload-artifact@v4, dtolnay@*), including the workflow that builds the
  shipped installers (`desktop-installer.yml`).
- F-2 Release-pipeline toolchain drift — `@stable` in
  desktop-installer.yml:42, real-model-e2e.yml:16,
  windows-package-dry-run.yml:18 vs the 1.99.0 pin
  (rust-toolchain.toml:5, rust.yml:26); plus floating
  `cargo install tauri-cli --version '^2'` (desktop-installer.yml:113).
  Mitigated: the rust-toolchain.toml override forces 1.99.0 for builds.

**MEDIUM**
- F-3 Two reqwest stacks in the shipped desktop binary (0.12.28 workspace +
  0.13.5 via tauri 2.12.1) — contradicts the unification note in
  `crates/modelswarm-runtime/Cargo.toml:14-16`.
- F-4 Two ed25519-dalek majors (2.2.0 workspace vs 3.0.0 via
  libp2p-identity 0.3.0) + sha2 0.10/0.11 split in the P2P binary.
- F-5 Four windows-sys copies (0.45/0.52/0.59/0.61.2) + windows 0.62.2.
- F-6 Version triple-tracking: tracker package.json 0.1.0 (inert — nothing
  reads it; page.tsx:11-19 hardcodes 0.2.20; CI guard rust.yml:64-81 checks
  Cargo↔page.tsx only) + hand-maintained inline installer sha (page.tsx:19).
- F-7 npm caret ranges; lockfile v3 is the real pin (next 15.5.27, react
  19.3.0, pg 8.23.1, zod 3.25.76, @noble/ed25519 2.3.0, vitest 5.0.3,
  typescript 5.9.3, ajv 8.20.0).
- F-8 Engine archive URL/SHA duplicated in desktop-installer.yml:24-37 vs
  runtime-pins.json (currently identical; per-file verify is fail-closed).
- F-9 Six candidate profiles use floating `/blob/main/` license-evidence
  URLs.
- F-10 License metadata URL-only in schema-v2 (v1's `licenseId` dropped —
  schema.json:53 vs schema-v2.json:81); bartowski repack candidates cite
  the repack LICENSE, not the upstream model license.
- F-11 postgres:16-alpine CI service image not digest-pinned
  (tracker.yml:36,77) — test-only.
- F-12 Desktop stub build still compiles libp2p (unconditional transport
  dep with libp2p-backend, desktop/Cargo.toml:38).

**LOW**
- F-13 Duplicate hub-public.hex (protocol/keys + apps/tracker/keys);
  WebView2 bootstrapper + tauri-cli NSIS downloads not hash-verified by us.
- F-14 winjob version island 0.1.0 (winjob/Cargo.toml:3).
- F-15 Transitive duplication noise (thiserror 1/2, base64 ×3, tracing
  never used first-party — telemetry is zero-dep by design).
- F-16 ajv (validate:vectors) is a devDependency — fine while validation
  stays CI-only.

Positive findings worth preserving: engine supply chain is exemplary
(compile-time-embedded pins, per-file SHA-256 at staging, CI, and every
launch; canonical build-hash identity consistent across runtime-pins.json,
candidate manifests, and node `id()`); golden-vector + Rust↔TS wire-compat
CI pins the cross-language crypto skew (dalek 2.2.0 vs @noble/ed25519 2.3.0);
version single-sourcing between workspace Cargo.toml and tauri.conf.json is
CI-enforced; no redistribution of weights anywhere.

## 5. Assumptions made

- Lockfile presence ⇒ not necessarily linked into shipped binaries (tauri
  tree is feature-off for node builds); binary-level inclusion unverified
  because cargo was forbidden.
- Vercel deploy settings live outside the repo (dashboard) per
  architecture §13; no vercel.json exists to audit.
- tracker package.json "0.1.0" predates the single-sourcing rule and is
  inert (verified by grep: no reader).
- The `@stable` dtolnay steps do not actually float the build toolchain
  (rust-toolchain.toml override applies), based on rustup's documented
  precedence — not empirically re-verified in CI logs.

## 6. Unresolved risks

- Tag-mutable CI actions remain the top exploitability window into release
  artifacts (F-1) until commit-pinned.
- Whether a future tauri or libp2p bump unifies reqwest/ed25519-dalek is
  upstream-controlled; tracked as accepted-until-upgrade (F-3/F-4).
- License auto-selection (M3) cannot gate on machine-readable license data
  today (F-10) — needs an ADR before M3, and owner sign-off is required for
  any license-restricted distribution (master prompt §17).

## 7. Suggested next task for the integrator

Merge this read-only audit as-is (no code changes to integrate). Then, in
order: (1) commit-pin CI actions + align `@stable` steps and tauri-cli to
exact versions (F-1/F-2 — small, workflow-only, Test and Release Engineer
owns `.github/workflows/`); (2) single-source the desktop-installer archive
matrix from runtime-pins.json (F-8 — needs Tracker/Windows Product
coordination); (3) open an ADR for a machine-readable catalog license field
before M3 (F-10 — Protocol Architect). Defer F-3/F-4/F-5 to the next
dependency-bump window; note them in `docs/risk-register.md`.
