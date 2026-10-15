# Updater manifest rehearsal — v0.2.22 staged signing (2026-10-13)

Required by `docs/reviews/review-updater-security-2026-10-13.md`
(§10 assumption 2: "T&R's staged-manifest rehearsal is the remaining
proof and should run before the 0.2.22 artifacts go public") and the
RC-1 standing obligation (§6: sign with `--app-version`, key stays
owner-local, rotation = manual client ship). Executed by the Test and
Release Engineer at owner-local staging on 2026-10-13 (local 01:15–01:40
+0800 / 2026-10-12 17:15–17:40 UTC). The private key
`C:\Users\igorc\ModelSwarm-updater.key` never entered the repo, CI, or
any log; only its PATH appears in command lines below. `.sig` files and
the manifest signature fields are public by design (signatures).

## Artifacts and provenance

Desktop-installer run **38283004626** (workflow_dispatch on the bump
commit `aa90731`, matrix green: windows nsis / ubuntu deb+appimage /
macos arm64 dmg; engine `b11407` hash-verified per job and verified
INSIDE each artifact by the workflow's unpack checks). Hash chain
verified THREE ways before staging — CI job-log `sha256sum` output ==
per-job artifact `SHA256SUMS.txt` == local `gh run download` files ==
staged copies (`sha256sum -c` all OK):

| canonical staged name | sha256 | bytes |
|---|---|---|
| `ModelSwarm-Setup-0.2.22-windows-x64.exe` | `b38dd07de95adbc65d2958951bafe07cd11ee4fb24776d51120d4563a68ebc89` | 21,249,083 |
| `ModelSwarm-0.2.22-linux-amd64.deb` | `2848ede9822cd96aea4fd0866f8a41eebe5846b4e1911b61d884b95f5e2736ac` | 39,069,126 |
| `ModelSwarm-0.2.22-linux-amd64.AppImage` | `74a89987086504b65d05a257a57e537d1f18033730144e2304e4169d152c5c02` | 105,425,400 |
| `ModelSwarm-0.2.22-macos-arm64.dmg` | `012f0046c60f164135312b78e48e944e169b8f07e676321c4ebee6768e6646d0` | 37,163,558 |

Tauri's underscored outputs were renamed to the canonical dashed
filenames the `/api/download` surface and the guards expect (same
rename discipline as the 0.2.21 train). 0.2.21 binaries removed from
the (gitignored) staging dir per the all-OS-together rule.

## Signing (owner-local, at staging)

For EACH installer, in `apps/tracker/public/downloads/`:

```text
npx -y @tauri-apps/cli@2 signer sign -f "C:\Users\igorc\ModelSwarm-updater.key" --app-version 0.2.22 <file>
```

All four produced `<file>.sig` (non-interactive; the key carries no
password — a password-protected key would have failed closed here
rather than prompt). Decoding each `.sig` (public material) shows the
trusted comment the client's `requireSignedVersion` checks, e.g.:

```text
trusted comment: timestamp:1791826357	file:ModelSwarm-Setup-0.2.22-windows-x64.exe	version:0.2.22
```

All four carry `version:0.2.22` and the canonical filename. Note:
this CLI writes `.sig` as the single-line base64 of the whole classic
minisign signature text — i.e. exactly the string the plugin's
`base64_to_string` + `Signature::decode` chain expects — so the
manifest `signature` field is the `.sig` file's content verbatim.

Key identity (public material only): `C:\Users\igorc\
ModelSwarm-updater.key.pub` decodes to `untrusted comment: minisign
public key: 38FC98CB4B064F37` + key `RWQ3TwZLy5j8OCiXv6/
e7C5T1YnLt2Fq+veByS9favUE2Obnuy4o7PNa`, byte-identical to the key
embedded at build time in `crates/modelswarm-desktop/tauri.conf.json`
(`plugins.updater.pubkey`). Verification below therefore holds against
both spellings of the same key.

## (a) latest.json serves correctly from the staging dir

`apps/tracker/public/downloads/latest.json` (2,557 bytes):
`version "0.2.22"`, one honest notes paragraph (650 chars), RFC 3339
UTC `pub_date`, platforms `windows-x86_64` / `linux-x86_64` /
`darwin-aarch64`, each `{signature, url}` with `url` =
`https://modelswarm.deepflux.space/api/download/<file>` (exe, the
AppImage — the self-replacing rider; the deb stays site-only for
humans — and the dmg). Served over plain HTTP from the staging tree
with the plugin's full URL shape including query parameters:

```text
$ python -m http.server 8124 --directory apps/tracker/public
$ curl -o latest-fetched.json -w "%{http_code} %{size_download}\n" \
    "http://127.0.0.1:8124/downloads/latest.json?target=windows-x86_64&arch=x86_64&current_version=0.2.21"
HTTP 200 2557 bytes
fetched == staged manifest: True   (json.load equality, all fields)
$ curl -r 0-1023 .../downloads/ModelSwarm-Setup-0.2.22-windows-x64.exe   -> 200
```

Static serving ignores `target/arch/current_version` (they ride as
query parameters by design, ready for a dynamic server without a
client change — updater handoff §1).

## (b) every manifest signature verifies against the public key over the staged bytes

Tool: `tests/updater-manifest-verify` (new, T/R-owned, workspace-
excluded; pinned to `minisign-verify =0.3.0`, the exact locked version
`tauri-plugin-updater 2.13.3` verifies with). It replays the plugin
chain verbatim: embedded pubkey → `PublicKey::decode`; manifest
`signature` → `base64::STANDARD` decode → `Signature::decode`;
`public_key.verify(bytes, &signature, /* allow_legacy */ true)` over
the STAGED artifact bytes; then the `requireSignedVersion` check
(`version:` field of the signature-covered trusted comment must equal
the manifest version).

```text
$ cargo run --manifest-path tests/updater-manifest-verify/Cargo.toml
PASS [manifest] latest.json parses as JSON (...\apps\tracker\public\downloads\latest.json)
PASS [manifest] version=0.2.22 pub_date=2026-10-12T17:33:14Z notes=650 chars
PASS [pubkey] embedded tauri.conf.json pubkey decodes (minisign)
PASS [platform darwin-aarch64]  minisign verify OK over ModelSwarm-0.2.22-macos-arm64.dmg   (37163558 bytes); signed version == 0.2.22
PASS [platform linux-x86_64]    minisign verify OK over ModelSwarm-0.2.22-linux-amd64.AppImage (105425400 bytes); signed version == 0.2.22
PASS [platform windows-x86_64]  minisign verify OK over ModelSwarm-Setup-0.2.22-windows-x64.exe (21249083 bytes); signed version == 0.2.22
PASS [...] sha256 per artifact (matches SHA256SUMS.txt — independent third hash implementation)
ALL MANIFEST SIGNATURES VERIFY against the embedded public key over the staged artifact bytes.   (exit 0)
```

Negative controls (the verification bites; staging restored and
re-verified by `sha256sum -c` + a final PASS run afterwards):

```text
control 1 — one byte appended to the staged exe:
FAIL [platform windows-x86_64] MINISIGN VERIFY FAILED ... InvalidSignature
MANIFEST VERIFICATION FAILED — do not deploy this staging.

control 2 — manifest version field changed to 0.2.23:
FAIL [platform darwin-aarch64] signed version "0.2.22" != manifest version "0.2.23"
FAIL [platform linux-x86_64]   signed version "0.2.22" != manifest version "0.2.23"
FAIL [platform windows-x86_64] signed version "0.2.22" != manifest version "0.2.23"
```

## (c) the /api/download/<file> targets resolve

Real route, real staged bytes (Next dev server on the tracker,
MemoryStore, `apps/tracker` — the same route.ts that deploys):

```text
$ curl -I -L http://127.0.0.1:8125/api/download/<file>
exe       302 -> /downloads/<file> -> 200 Content-Length: 21249083
AppImage  302 -> /downloads/<file> -> 200 Content-Length: 105425400
dmg       302 -> /downloads/<file> -> 200 Content-Length: 37163558
$ curl -L -r 0-4095 .../api/download/ModelSwarm-Setup-0.2.22-windows-x64.exe
HTTP 206, 4096 bytes, final URL /downloads/<file>
```

Content-lengths equal the staged file sizes exactly; the 302→206
ranged behavior matches the live production probe recorded for 0.2.21
(updater handoff §2). The deb (site-only) resolves through the same
route. Live production resolution for 0.2.22 happens at the Integrator's
Vercel deploy — post-deploy check: `curl -sI
https://modelswarm.deepflux.space/api/download/ModelSwarm-Setup-0.2.22-windows-x64.exe`
must answer 302 with the `/downloads/` location.

## Standing obligations recorded (RC-1, every future train)

1. Sign every installer at owner staging with `--app-version <VERSION>`
   — mandatory; without it every client rejects the update
   (`MissingSignedVersion`).
2. The private key never enters the repo, CI, or logs (no
   `TAURI_SIGNING_PRIVATE_KEY` anywhere in `.github/`; not set in this
   train — signing happened locally against the key file path only).
3. Key rotation = a manual client ship with a new embedded pubkey;
   until adoption completes, the OLD public key stays valid on
   deployed clients — treat the old private key as
   compromised-and-live for one full adoption cycle.
4. Re-run this rehearsal (the tool is committed at
   `tests/updater-manifest-verify`) at every release staging before
   the manifest goes public.
5. The version-bump push MUST auto-trigger desktop-installer — its
   `paths` filter includes `Cargo.toml`/`Cargo.lock` since 2026-10-13
   (before that, bump-only commits matched no path and the correct
   artifacts existed only via manual dispatch — the v0.2.23 near-miss).
   On every train, the Integrator verifies that a desktop-installer run
   appeared on the bump commit WITHOUT manual dispatch and that its
   artifacts carry the NEW version in their filenames before staging
   anything. If the bump push did not auto-trigger, STOP the train: the
   filter has regressed — never paper over it with a dispatch. Manual
   `workflow_dispatch` on the bump head remains the documented fallback
   only for a cancelled or failed auto run.

## v0.2.23 staging rehearsal (same day, evening train)

Staged set (canonical dashed names, from desktop-installer run
38363557211 — workflow_dispatch on the bump commit 8a6f772; the
workflow's path filter means bump-only commits do not auto-trigger it,
so the run was dispatched explicitly after cancelling the wrong-version
auto run on 3a0bb60):

- ModelSwarm-Setup-0.2.23-windows-x64.exe
  sha256 cccbd8afd492c9a59684b7eea95b58baef68ff2651b226d45e04a1760e608ba0
- ModelSwarm-0.2.23-linux-amd64.deb
  sha256 adb263876c9c02a6861c80e687db3d8d83f19f812d6da746391e9a6227c9ba88
- ModelSwarm-0.2.23-linux-amd64.AppImage
  sha256 ce3da1134234b1473ac38d856528f8dfaedff45df4280217c2ee75d9ced1aedc
- ModelSwarm-0.2.23-macos-arm64.dmg
  sha256 f00ee120672e45e9e0dc6c0c9c838d6fb7475c7ac05df87117ac8f233dea85e4

Hash chain: per-job CI artifact SHA256SUMS.txt verified with
`sha256sum -c` (all OK) over the downloaded files; the staged copies are
byte-identical copies. 0.2.22 binaries removed (all-OS-together rule).

Signing: `npx -y @tauri-apps/cli@2 signer sign -f <key path>
--app-version 0.2.23` per installer (owner-local key file, passwordless,
non-interactive; key path only, never contents). Every .sig trusted
comment carries `file:<canonical name>` and `version:0.2.23` (decoded
and checked for all four).

**TRAIN FINDING (fixed before deploy — the rehearsal did its job):** the
first manifest build base64-encoded the .sig file CONTENT into
`platforms[...].signature`, and the rehearsal failed all three platforms
with `InvalidEncoding`: the v2 CLI's .sig files ALREADY contain the
base64 the plugin expects, so the manifest signature must be the .sig
content VERBATIM (exactly the 0.2.22 manifest's shape — the plugin's
`base64_to_string` + `Signature::decode` chain then yields the minisign
text). Regenerated verbatim; the standing rule for future trains: the
manifest signature == the .sig file's own text, never re-encoded.

Rehearsal verdict after the fix — `cargo run --manifest-path
tests/updater-manifest-verify/Cargo.toml`: ALL PLATFORM SIGNATURES
VERIFY against the embedded public key (key id 38FC98CB4B064F37) over
the staged artifact bytes, signed version == 0.2.23 on all three
platforms, sha256 values match the staged files; /api/download URL
shape validated against the counting route's NAME_PATTERN.

Tracker landing retarget: apps/tracker/app/page.tsx 0.2.22 → 0.2.23
(version label + 4 download links; copy rules kept — honest tense, no
engine names, no status pills); typecheck clean; `next build` green.
Tracker-Engineer-owned path — flagged for review with this commit.

## v0.2.24 staging rehearsal (2026-10-14, the true-swarm release)

Artifacts from desktop-installer run 38571419953 — AUTO-TRIGGERED on the
bump commit fe50f01 (the ba0ed1a path-filter fix's first real train;
zero production-code delta to the current head 549dd18, whose only
change is the cfg(test)-only env-lock race fix — the b1c2234 precedent).
Hash chain: CI per-job SHA256SUMS verified (4/4 OK) over the downloaded
files; staged copies byte-identical. 0.2.23 binaries removed
(all-OS-together rule). Signed at owner-local staging (v2 CLI,
--app-version 0.2.24, key file path only; trusted comments carry
file:<canonical name> version:0.2.24 — decoded+checked, 4/4). Manifest:
signature = the .sig file text VERBATIM (the 0.2.23 standing rule —
first-try verify this train). SHA256SUMS retargeted. Rehearsal verdict —
tests/updater-manifest-verify: ALL THREE PLATFORM SIGNATURES VERIFY
against the embedded key (38FC98CB4B064F37) over the staged bytes,
signed version == 0.2.24, sha256 match, URL shape OK. Staged hashes:
win exe 2e7669fd…f857e2; deb 5011daf5…1e622c; AppImage 737bc146…
bc2a8e5; dmg ea7eec77…c2b4690. CROSS-PATH: page.tsx 0.2.23→0.2.24
retarget (5 replacements) — Tracker-Engineer-owned, flagged for review;
typecheck clean, build green. Owner rulings in force: unsigned by
decision; key rotation deferred until he says so.

## v0.2.25 staging rehearsal (2026-10-15, the relay release)

Artifacts from desktop-installer run 38736456892 — AUTO-TRIGGERED on the bump
commit 330b96c (no manual dispatch; artifacts carry 0.2.25 in filenames,
verified). Hash chain: CI per-job SHA256SUMS verified (4/4 OK over the
downloaded files); staged copies byte-identical under canonical dashed names.
0.2.24 binaries removed (all-OS-together rule). Two-push window as on every
train: the bump commit's `guards` job was red on the SHA256SUMS version match
(expected; goes green at this staging commit — the 0.2.24 train's documented
pattern), while `check`, `check-nonwindows-capi`, `windows-package-dry-run`,
and the installer matrix were all green.

Staged hashes:

- ModelSwarm-Setup-0.2.25-windows-x64.exe
  sha256 2100b35d759090c683d69e30ba9ec01756f6ad0f305553e0c1c7d609b77afe23
- ModelSwarm-0.2.25-linux-amd64.deb
  sha256 5a78edbb96a1ecbea388bb8fc89f09bb88fea297c09de712abe589b581add3fe
- ModelSwarm-0.2.25-linux-amd64.AppImage
  sha256 6a9dcc9d88006fa81a45ca945fa2b59a728e623c9d3c44433854822962dd4f16
- ModelSwarm-0.2.25-macos-arm64.dmg
  sha256 97832672163a7a433f61cb8d9283d413f5cc89d986f8c28cedcb34e26330ae16

Signing: v2 CLI `--app-version 0.2.25`, owner-local key file path only,
passwordless, non-interactive; all four .sig trusted comments decoded and
checked to carry `file:<canonical name>` + `version:0.2.25`. Manifest
signature = the .sig text VERBATIM (the 0.2.23 standing rule — first-try
verify this train). Rehearsal verdict — tests/updater-manifest-verify:
ALL THREE PLATFORM SIGNATURES VERIFY against the embedded key
(38FC98CB4B064F37) over the staged bytes, signed version == 0.2.25, sha256
match, URL shape OK. Manifest notes state the relay posture honestly
(default-empty relay.addr, direct-first, slower relayed turns, tracker-minted
tickets). CROSS-PATH: page.tsx 0.2.24→0.2.25 retarget (5 replacements) —
Tracker-Engineer-owned, flagged for review; typecheck clean, next build
green. NOTE: an in-flight uncommitted visual redesign of page.tsx from a
parallel session was parked (byte-identical backup at owner-local temp,
restored to the working tree after this deploy with links already bumped) —
it is NOT part of this staging and ships only after its own rendered-
geometry verification per the standing rule. Owner rulings in force:
unsigned by decision; updater key rotation deferred (the DeepSeek advisory
key, separately, is removed by this release — different key, different fate).

## v0.2.26 staging rehearsal (2026-10-15, late night — the language-lock fix)

Artifacts from desktop-installer run 38756562713 — AUTO-TRIGGERED on the bump
commit (0.2.26; no dispatch). NOTE the bump's `check` job hit the known
~1-in-3 winjob flake (`close_all_retained_jobs_kills_every_supervised_child`)
on top of the expected guards window-red; local full-workspace at the
regeneration head = 0 failures, and the vector-regeneration commit ddab269
between bump and staging is TESTS-ONLY (zero production delta — the
b1c2234/549dd18 precedent), so the bump-commit artifacts are valid. Hash
chains verified per-artifact (the linux artifact needed the API-zip route —
gh run download stalled twice on the 144 MB archive; sizes byte-exact).
0.2.25 binaries removed. Staged: win exe 4327b5ec…dbc79; deb
f133b3f3…c99ad; AppImage 35801ba1…1379; dmg eb945088…206e. Signed
--app-version 0.2.26 (key path only; 4/4). Manifest sig = .sig text
VERBATIM; rehearsal verdict ALL THREE PLATFORM SIGNATURES VERIFY, signed
version == 0.2.26, sha256 match, first-try. page.tsx retarget: 5 functional
replacements (the redesign grew two historical comment mentions — left
as-is with an appended changelog line); typecheck clean, build green.
Unsigned by owner ruling.
