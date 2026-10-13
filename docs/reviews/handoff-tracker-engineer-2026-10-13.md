# Tracker Engineer handoff — 2026-10-13

## Scope: review owed on the cross-path download-section retargets (commits d1c2dc0, f85347d)

The Integrator edited `apps/tracker/app/page.tsx` during the 0.2.22 and 0.2.23
release trains (Tracker-Engineer-owned path, flagged for review). This is that
review, of the CURRENT state at HEAD f85347d.

### Verdict: retarget APPROVED; one adjacent defect found and FIXED

**1. Links / label / stale references — correct.**
All four `/api/download/` links in `apps/tracker/app/page.tsx` (lines 180, 185,
187, 192) point at the four staged 0.2.23 artifacts with byte-exact names:
`ModelSwarm-Setup-0.2.23-windows-x64.exe`, `ModelSwarm-0.2.23-linux-amd64.deb`,
`ModelSwarm-0.2.23-linux-amd64.AppImage`, `ModelSwarm-0.2.23-macos-arm64.dmg`
(all present in `apps/tracker/public/downloads/`). Visible label is
`v0.2.23` (line 170). `grep -rInE "0\.2\.(2[012])" apps/tracker` (excluding
node_modules/.next) finds nothing — no stale 0.2.2x anywhere under the app.
The 0.2.22 retarget in d1c2dc0 had the same correct shape (0.2.21 -> 0.2.22,
same four links).

**2. Copy contract — holds.**
Honest tense only ("test builds, unsigned"), the engage/fallback claim matches
the shipped behavior ("works between two peers on a direct or LAN network; the
app reports honest status for anything not engaged"), no engine or model-file
format names in `app/` (grep llama|gguf|ggml: none; `tests/meta.test.ts`
source-scan green), no status pills in the download section. The AppImage link
alone carries "(self-updating)" — consistent with `latest.json`, whose only
Linux update target is the AppImage (.deb/.dmg/.exe make no self-update claim).
The self-update clause ("checks for updates on start and every six hours,
verifies each update against a key built into the app, and restarts to install
— this is the last manual download") matches the updater shipped in 0.2.22 and
the now-staged 0.2.23 manifest.

**3. NAME_PATTERN / checksums consistency — pattern correct; sums file FIXED.**
`apps/tracker/app/api/download/[file]/route.ts` NAME_PATTERN
(`/^[A-Za-z0-9][A-Za-z0-9._-]*\.(exe|deb|AppImage|dmg)$/`) accepts all four
filenames (verified mechanically) and still rejects traversal and
`SHA256SUMS.txt`; the sums file is served statically at `/downloads/SHA256SUMS.txt`,
exactly as the page references. All four staged binaries verify against
SHA256SUMS.txt (sha256sum -c: 4x OK). `latest.json` cross-checked: version
0.2.23, all three platform signature strings are verbatim-equal to their `.sig`
files (the "never re-encode the .sig" standing rule held), trusted comments
carry file:/version:0.2.23.

### The fix (defect found during review, inside my owned path)

The staged local `apps/tracker/public/downloads/SHA256SUMS.txt` had **CRLF line
terminators in the working tree** (repo blob was already LF; the CRLF came from
the Windows staging/autocrlf smudge). The Vercel CLI deploy uploads the local
working tree — so the deployed sums file would have carried CRLF, and
`sha256sum -c` / `shasum -a 256 -c` fail on Linux and macOS on every line
(trailing `\r` appended to each filename: "No such file or directory"), breaking
the page's checksum contract for exactly the .deb/.AppImage/.dmg audience.
The Integrator's "sha256sum -c over downloaded files == staged copies" claim
does not reproduce against this artifact as staged (verified: native
`sha256sum -c` failed 4/4 pre-fix).

- Normalized the working-tree `SHA256SUMS.txt` to LF (content byte-identical;
  native `sha256sum -c` now passes 4/4). Blob unchanged, so this is a
  deploy-artifact fix, not a repo diff.
- Added `apps/tracker/.gitattributes` pinning `public/downloads/SHA256SUMS.txt
  text eol=lf` so future checkouts on the `core.autocrlf=true` staging machine
  cannot re-smudge it (scoped inside apps/tracker/ to avoid a cross-path edit
  of the root .gitattributes).

### 1. Changed files and why

- `apps/tracker/public/downloads/SHA256SUMS.txt` — working-tree CRLF -> LF
  (deploy artifact; no repo diff results).
- `apps/tracker/.gitattributes` — new; pins the sums file to LF on checkout.
- `docs/reviews/handoff-tracker-engineer-2026-10-13.md` — this record.

### 2. Exact commands and outcomes

- `git show f85347d -- apps/tracker/app/page.tsx` / `git show d1c2dc0 -- ...`
  — retarget diffs reviewed (label + 4 links only; copy untouched).
- `grep -rInE "0\.2\.(2[012])" apps/tracker --exclude-dir=node_modules --exclude-dir=.next`
  — no matches.
- `grep -rniE "llama|gguf|ggml" apps/tracker/app/` — no matches.
- node NAME_PATTERN test — 4/4 accepted; `../evil.exe` and `SHA256SUMS.txt` rejected.
- node latest.json check — 3/3 signatures verbatim-equal to `.sig` files; version 0.2.23.
- `sha256sum -c SHA256SUMS.txt` — pre-fix: FAILED 4/4 (CRLF); post-fix: OK 4/4.
- `npm run typecheck` (apps/tracker) — clean.
- `npm run build` (apps/tracker) — green (all routes built).
- `npx vitest run tests/downloads.test.ts tests/meta.test.ts` — 11/11 passed.
- Cargo gates not re-run: no Rust path touched by this review/fix; workspace CI
  is green at f85347d per the release train.

### 3. Test evidence

See commands above; the load-bearing one for the fix is the native
`sha256sum -c` flip from 4x FAILED (pre) to 4x OK (post).

### 4. Assumptions

- The owner's Vercel CLI deploy uploads the local working tree (that is how the
  gitignored installers deploy at all), so the local LF fix is the one that
  reaches production — the deploy must run from this tree.
- The two-space text-mode separator in the sums file (vs 0.2.22's `*` binary
  marker) is fine: GNU coreutils and macOS shasum accept both with `-c`.
- `zcode-master-prompt.md` (untracked) and the in-flight
  `crates/modelswarm-transport` edits (Network's MSP_LISTENER retirement WIP,
  appeared in the tree during this review) are not mine and were left untouched
  and unstaged.

### 5. Unresolved risks

- The CRLF class of bug can recur for any other cross-platform-parsed artifact
  staged from Windows (latest.json is JSON, so CRLF would be harmless there;
  .sig files are single-line and clean).
- Deploy not performed (by design): the Integrator runs `vercel` from
  `apps/tracker/` next; if it runs from a DIFFERENT checkout than this tree,
  the CRLF sums file ships. Deploy from this tree, or re-check
  `file public/downloads/SHA256SUMS.txt` says "ASCII text" (no CRLF) first.

### 6. Suggested next task for the integrator

Run the production deploy from this working tree (Integrator/owner step), then
post-deploy probe: `curl -s https://modelswarm.deepflux.space/downloads/SHA256SUMS.txt | file -`
must NOT report CRLF, and `curl -s ... | sha256sum -c -` (on Linux/macOS or
Git Bash) must pass 4/4; spot-check one `/api/download/<file>` redirect and the
download counter bump. Then record the deploy in the release-train log.
