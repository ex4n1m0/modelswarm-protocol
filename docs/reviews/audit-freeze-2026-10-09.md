# Production Freeze Record — Master-Prompt Audit (2026-10-09)

Owner-triggered execution of `zcode-master-prompt.md` §5. This file records
the frozen production state before any audit-driven change. **No production
deployment was modified** (no `vercel deploy`, no DNS, no credentials used).

## Frozen state

| Item | Value |
|---|---|
| Audit date | 2026-10-09 |
| Production commit (main) | `730cebf` `fix(sim): wall-clock-tolerant fallback policy` |
| Workspace version | 0.2.20 (single-sourced in root `Cargo.toml`; enforced by `rust.yml` guards) |
| Rust toolchain | 1.99.0 (`rust-toolchain.toml`, matches CI pin `dtolnay/rust-toolchain@1.99.0`) |
| Protected audit branch | `audit/master-prompt-2026-10-09` (created from `730cebf`) |
| Baseline tag | `audit-baseline-2026-10-09` (annotated, at `730cebf`) |
| Live tracker | https://modelswarm.deepflux.space — HTTP 200, Vercel deployment `dpl_GnFUHxrpqRMH73qfrVuLfuv1vf6P` (observed 2026-10-09) |
| Deploy path | Vercel CLI from `apps/tracker/` — git push alone never deploys |
| Working tree at freeze | clean except untracked `zcode-master-prompt.md` and `docs/reviews/gap-engineering-study-2026-10-09.md` (both intentional audit inputs, not committed at freeze) |

## Migration-sensitive data (backed up by content manifest)

`apps/tracker/migrations/`:

| File | SHA-256 |
|---|---|
| `0001_init.sql` | `566cf769a1ebc18a70894147835cd9e000652f2b9a5924944c0dbfcd28080357` |
| `0002_downloads.sql` | `3792f73b2cb80ebb4b766ef94db6e0e3aff8e220257158f7e72a1de4bd654162` |
| `0003_model_requests.sql` | `6091318ba6c8f802d012070812d14610cf69ef22de6c9c3b9f79f02203055959` |
| `README.md` | `7d0df7d617c4f110eac7b63e4da7207012f671675cf4d92999e1b889df800c9e` |

Keys: `apps/tracker/keys/` contains only `README.md` + `hub-public.hex` (no
private key material in the tree; hub signing key expected in Vercel env —
the security audit confirms).

## Secrets spot-check (freeze-level)

- `git ls-files` scan for secret-like filenames (`*.env`, `*.pem`, `*.key`,
  `*secret*`, `*credential*`): **no matches**.
- Content-level secrets scan is delegated to the security audit
  (`docs/reviews/audit-security-findings.md`).

## Validation matrix (mirrors `.github/workflows/rust.yml`, windows-latest, 1.99.0)

Executed locally and sequentially; logs in `target/audit-logs/`, summary in
`target/audit-logs/summary.txt`. Results appended below when the suite
completes.

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings`
5. `cargo test --workspace --features modelswarm-node/libp2p-backend`
6. `cargo test -p modelswarm-desktop --features tauri-shell`
7. Tracker: `npm run typecheck`, `npm test` (vitest), `npm run build`,
   `npm run validate:vectors` (golden vectors)
8. Packaging dry run (`windows-package-dry-run.yml` equivalent) — deferred to
   the test/release audit (wave 2)
9. `cargo audit` / `cargo deny` — **NOT CONFIGURED** (no `deny.toml`,
   `cargo-audit` not installed); documented per §5 rather than skipped
   silently; supply-chain review delegated to the dependency audit.

### Results — local suite (completed 2026-10-09 07:13 +08, all at `730cebf`; logs in `target/audit-logs/`)

| # | Command | Result |
|---|---|---|
| 1 | `cargo fmt --all --check` | **PASS** |
| 2 | `cargo clippy --workspace --all-targets` (default) | **PASS** |
| 3 | `cargo test --workspace` (default) | **PASS** — 301 tests / 49 suites / 0 failed |
| 4 | clippy `--features modelswarm-node/libp2p-backend` | **PASS** |
| 5 | `cargo test --workspace` libp2p-backend | **PASS** — 313 tests / 0 failed |
| 6 | `cargo test -p modelswarm-desktop --features tauri-shell` | **PASS** — 7 tests / 0 failed |
| 7 | tracker `tsc --noEmit` | **PASS** |
| 8 | tracker `vitest run` | **PASS** — 110 passed / 2 skipped (by design, env-gated real-Pg) |
| 9 | tracker `next build` | **PASS** |
| 10 | tracker `validate:vectors` | **PASS** — all golden vectors |

### CI status at freeze (GitHub Actions, observed 2026-10-09)

- `rust` workflow on main: **RED on the last two runs** (37792918516 on
  `c4399a8`, 37796497342 on `730cebf`), failing step "Desktop tests
  (tauri-shell feature)" — never green since it was added. Root cause:
  `tauri_build` validates `bundle.resources` globs (`engine/*`), which
  match nothing on a clean runner because engines are gitignored and only
  the installer workflows stage (and hash-verify) them. The local run
  passed only because this dev machine has `engine/` + `engine-vulkan/`
  staged.
- `tracker` / `desktop-installer` / `windows-package-dry-run`: green.
- **Fixed during the audit** (sanctioned "CI green" maintenance per
  master prompt §9): commit **`63b98e1`** on main adds
  `TAURI_CONFIG='{"bundle":{"resources":[]}}'` to that one step. Proven
  locally by bare-runner simulation: engine dirs renamed away → exact CI
  failure reproduced (exit 101, same glob error); with the overlay → 7/7
  tests pass. **CI CONFIRMED GREEN: run `37858753428` on `63b98e1` =
  success, all steps including "Desktop tests (tauri-shell feature)"
  (observed 2026-10-09).** Follow-up hardening (test-release
  recommendation): a guards-job assertion freezing
  `bundle.resources` to the exact two-glob set restores push-time drift
  detection despite the test-only overlay — landed in `86450aa`, whose
  CI run `37860513036` also completed **success** (guards + check).
  CI on main is fully restored at the freeze baseline + the two
  CI-only fixes.

## Freeze-time observations (non-blocking, routed to audits)

- ADR-016 is absent from `docs/adr/` (001–015 and 017–026 exist) — protocol
  audit explains or flags.
- `apps/tracker/package.json` version is `0.1.0` while the workspace is
  `0.2.20` — dependency audit assesses whether intentional.
- `docs/architecture.md` §9 still says "No relay exists in v0.1 (ADR-003)"
  while `crates/modelswarm-relay` shipped and F2A relay was verified
  2026-10-08 (`docs/verification/f2a-relay-2026-10-08.md`) — doc drift for
  the architect audit.
- CI on main was red at freeze (`rust` workflow, tauri-shell step) —
  root-caused and fixed in `63b98e1`; see the Results appendix.
