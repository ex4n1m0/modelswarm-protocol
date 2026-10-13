# Handoff — Test and Release Engineer: desktop-installer trigger honesty (2026-10-13)

Task: make the v0.2.23 near-miss impossible to repeat — the release-train
version-bump commit matched no `push.paths` entry in
`.github/workflows/desktop-installer.yml`, so the installer never
auto-built on the bump; the auto run on the preceding feature commit
(3a0bb60) was building 0.2.22-NAMED artifacts and was cancelled, with the
correct 0.2.23 set existing only via manual `workflow_dispatch` (run
38363557211). Incident record:
`docs/verification/updater-manifest-rehearsal-2026-10-13.md` (v0.2.23
section).

## 1. Choice made: option (a) — widen the paths filter

`Cargo.toml` and `Cargo.lock` added to `push.paths` (commit `ba0ed1a`).

Why this is a complete structural fix: the product version is
single-sourced in the workspace `Cargo.toml` (`[workspace.package]
version`; 16 member crates inherit via `version.workspace = true`;
`rust.yml` guards *enforce* that `tauri.conf.json` carries no version).
Every release-train bump commit therefore edits root `Cargo.toml` by
construction — the bump push now auto-triggers the 4-OS build **on the
exact bump commit**, which is where shippable artifacts must come from.
`Cargo.lock` rides along for the same reason it does in `rust.yml`:
dependency edges are compiled into the shipped binary, so a lock change
is an installer input that previously never exercised the 4-OS matrix.

Consistency: `rust.yml` already filters on `Cargo.toml` + `Cargo.lock`
and `windows-package-dry-run.yml` on `Cargo.toml` — desktop-installer
was the outlier among the three Rust-building workflows.

### Rejected alternative: option (b) — keep the filter, add an enforced guard

A guard that fails when a bump lands without a green installer run on
that exact sha (gh-api comparison of workspace version vs the latest
green run) was rejected on four grounds:

1. **Placement is a trap.** Inside `rust.yml`, the `concurrency:
   cancel-in-progress` group cancels a long-polling guard on the next
   push — the exact b1c2234 lesson (runs cancelled before the gate
   concludes = silently unenforced). A separate no-cancel workflow
   avoids that but adds a fifth workflow to maintain.
2. **New failure modes exactly at the incident's location.** The guard
   needs polling (~20-40 min installer duration), retry-wrapped gh api
   calls, `workflow_dispatch`-from-workflow token permissions, and
   non-bump skip logic — every element is a way to spuriously fail or
   silently not enforce.
3. **It does not remove the manual step.** A post-hoc red check still
   requires someone to dispatch the installer and re-run the guard;
   option (a) makes the correct run exist at push time with no human in
   the loop.
4. **The proposed version check can't see versions.** Artifact names are
   `modelswarm-installer-<platform>` (no version); versioned filenames
   live *inside* the artifacts, so the check needs ~20-100 MB downloads
   or log scraping — both fragile surfaces (retention, expiry).

### Cost of (a), measured (not estimated)

Over `main` history 2026-07-01 → 2026-10-13 (f85347d): 37 commits
touched `Cargo.toml`/`Cargo.lock`; **18 of those matched nothing in the
old filter** (9 of the 18 in the final 6 days of active ADR-032 work).
Accepted cost: roughly 1–2 additional ~20–40-min 4-OS runs per active
development day (~80 runner-minutes each). Mitigating: those runs
validate real installer inputs rather than wasting them; the repo is
public (no artifact-quota pressure; ~3.3 GB of unexpired artifacts
active at the time of this change); and pushes that also touch
desktop/node crates already triggered the workflow today.

## 2. Changed files and why

| file | change | why |
|---|---|---|
| `.github/workflows/desktop-installer.yml` | `Cargo.toml`, `Cargo.lock` added to `push.paths` + rationale comment | the fix itself |
| `docs/verification/updater-manifest-rehearsal-2026-10-13.md` | standing obligation **5** appended to the RC-1 list | mechanically checked train step: the bump push MUST auto-trigger desktop-installer with NEW-version artifact names before staging; a bump that did not auto-trigger STOPS the train (never paper over with a dispatch); manual dispatch remains the fallback only for a cancelled/failed auto run |
| `Cargo.toml` | 3-line comment addition above `[workspace.package]` | (i) durable doc at the version source ("editing this version auto-triggers the 4-OS build"); (ii) the **positive-control push** — see §4. **Cross-path note:** root `Cargo.toml` is Integrator-by-default territory; comment-only, zero build behavior (`cargo metadata` parses; the `^version = ` guards line and `0.2.23` value byte-identical); landed under the same release-gate authority as b1c2234 — flagged for Integrator review here. |

## 3. Exact commands and outcomes

```text
git log/rev-list measurement (cost basis)     -> 37 toml/lock commits, 18 filter-orphaned
actionlint 1.7.12 desktop-installer.yml        -> clean
python yaml.safe_load                          -> paths list verified incl. both new entries
cargo metadata --no-deps                       -> parse OK after the comment edit
cargo fmt --all --check                        -> clean
git push (ba0ed1a)                             -> carried by the concurrent tracker push (see §6)
git push (7db6caa)                             -> solo push; range = exactly 1 commit, 1 file (Cargo.toml)
```

## 4. Test evidence (CI, machine-readable)

- **Run 38374656179** — head `ba0ed1a` (workflow-fix commit), `push`
  event, **success**; windows nsis + ubuntu deb/appimage + macos dmg all
  green. Trigger matched via the pre-existing workflow-file entry; proves
  the edited workflow still parses and the full matrix still builds.
- **Run 38374817537** — head `7db6caa` (positive control), `push` event,
  **success**; all three matrix jobs green. The push range
  `origin/main..main` at push time contained exactly one commit touching
  exactly one file, `Cargo.toml` — the run can only have matched through
  the **new** entry. This is the executable proof that a version-source
  edit (what every bump commit is) now auto-triggers the installer.
- Version plumbing on 38374817537: windows job log shows
  `ModelSwarm_0.2.23_x64-setup.exe` built (21,281,145 bytes,
  sha256 `c75bedee…ab9158` in the job's SHA256SUMS.txt), engine-inside
  unpack checks green, three platform artifacts uploaded (windows-x64
  21,268,694 B; macos-arm64 37,059,437 B; linux-x64 143,683,000 B).
  These are **evidence artifacts, not release artifacts** — staging stays
  a manual owner step keyed to the train's bump commit; no action needed.
- Side-workflow confirmation on `7db6caa`: `rust.yml` and
  `windows-package-dry-run.yml` both triggered via their own
  `Cargo.toml` entries (consistent path semantics).
- **Negative control:** this handoff commit is docs-only; it must NOT
  trigger desktop-installer (verified below in §7 before final push of
  this file's commit).
- **Unrelated main failure observed:** `rust.yml` run 38376300562 on
  `f344ae1` (Integrator commit, contains none of my code — my deltas are
  yml/md/toml-comment) failed in
  `winjob::tests::close_all_retained_jobs_kills_every_supervised_child`
  (panicked `crates/modelswarm-winjob/src/lib.rs:356`) — fmt and clippy
  steps had passed, so this is a test-runtime failure in
  Windows-Product-owned code. Rerun of the failed job dispatched
  (verdict in §7). Not caused by and not fixable from this change;
  routed to Windows Product Engineer.

## 5. Assumptions

1. GitHub `paths` filter semantics are stable documented behavior
   (union match over the push's changed files) — the structural
   guarantee rests on it plus the version single-source guard staying
   in place (`rust.yml` "Version is single-sourced" step).
2. The standing two-push release-train window (bump → staging) is
   unchanged; obligation 5 only replaces "dispatch manually" with
   "verify the auto run appeared".
3. No production behavior changed anywhere in this handoff (workflow
   trigger + docs + a TOML comment).

## 6. Unresolved risks / session notes

- **First real exercise is the next train.** No live version bump has
  flowed through the new entry yet (today's bump predates it);
  obligation 5 makes that verification a mandatory train step.
- **Concurrent multi-agent session:** a Tracker-Engineer file
  (`apps/tracker/.gitattributes`) was staged in the shared index and
  swept into my first commit attempt; I reset before pushing and
  recommitted with only my two files — their change then landed properly
  under their own commit `95c3fb5`. My `ba0ed1a` reached origin inside
  the tracker agent's push of the shared local main ("Everything
  up-to-date" on my push); `7db6caa` I pushed solo with the isolated
  range proven above. Agents pushing from this tree should `git log
  origin/main..main` before pushing to keep trigger-evidence claims
  clean.
- `Cargo.toml` comment cross-path edit awaits Integrator ack (§2).
- The winjob test failure (§4) needs a Windows Product verdict
  (flake vs regression) regardless of the rerun outcome.

## 7. Suggested next task for the Integrator

1. On the next release train, execute obligation 5: confirm the bump
   push auto-triggered desktop-installer on the bump sha with
   new-version artifact names **before** any staging (this is the first
   live exercise of the new filter entry).
2. Ack the `Cargo.toml` comment cross-path edit (or have me revert it —
   the filter guarantee does not depend on it).
3. Route the winjob CI failure to Windows Product with the rerun
   verdict below.

## 8. Post-write evidence updates

- Negative control (docs-only push of this file): see run list below.
- rust.yml 38376300562 rerun verdict: recorded after completion by the
  T/R agent in the task reply (this file is committed before the rerun
  concludes).
