# Security review — the auto-update chain (v0.2.22 release gate)

Reviewer: Security Engineer (independent blocking seat). Reviewed commit:
`ea47032` (+ this review's own edits, committed together). Requested by
`docs/reviews/handoff-windows-product-updater-2026-10-13.md` §6.

Scope: the update path as it ships in v0.2.22 — `crates/modelswarm-desktop/
tauri.conf.json` (`plugins.updater`), `crates/modelswarm-desktop/src/app/
updater.rs`, the registration in `src/app.rs`, the `modelswarm-winjob`
`close_all_retained_jobs` seam, the UI status rendering, and the vendored
verifier itself (`tauri-plugin-updater` 2.13.3 / `minisign-verify` 0.3.0 /
`reqwest` 0.12.28, all lockfile-pinned; plugin source read from the local
cargo registry at the locked versions).

## Verdict

**APPROVE WITH REQUIRED CHANGES** for shipping the updater in v0.2.22.
Blockers: **0**. Required changes: **2** (RC-1 lands with the 0.2.22 train;
RC-2 is a v1 obligation recorded now). Advisories: **5**. One hardening was
applied inside this review (config-drift pin, see §7).

The core question — can anything the update origin serves cause an
unsigned-by-us package to install — is answered **no**, with the mechanics
enumerated in §2.

## 1. Trust root — ruling: SOUND

The only trust input is the minisign public key embedded at build time in
`tauri.conf.json` (`plugins.updater.pubkey`; decodes to
`untrusted comment: minisign public key: 38FC98CB4B064F37` +
`RWQ3TwZLy5j8OCiXv6/e7C5T1YnLt2Fq+veByS9favUE2Obnuy4o7PNa`, an Ed25519
minisign key). Verified against the locked plugin source:

- Verification is unconditional in `Update::download`
  (plugin `src/updater.rs:837-843`): `verify_signature(&buffer,
  &self.signature, &self.context.config.pubkey, &self.version,
  require_signed_version)` runs on the fully downloaded bytes before
  `install()` is possible, and `install(bytes)` in `src/app/updater.rs:375`
  only ever receives bytes that passed it. There is no code path that
  installs unverified bytes.
- No fallback to unsigned: `ReleaseManifestPlatform.signature` is a required
  field (plugin `src/updater.rs:83-89`); a manifest without it fails
  deserialization; an empty/garbage value fails base64/`Signature::decode`.
- No runtime override surface: `UpdaterBuilder::{endpoints,pubkey}` exist but
  are only callable from Rust, and `run_cycle`
  (`src/app/updater.rs:287-296`) calls only `on_before_exit`. The key and
  endpoint come from the plugin's `UpdaterState`, populated from the
  build-embedded `tauri.conf.json` (plugin `src/lib.rs:274-289`). No env var
  overrides them (the plugin reads only `SYSTEMROOT`, for the MSI path).
- No webview bypass: the plugin registers `check`/`download`/`install`/
  `download_and_install` invoke commands (plugin `src/lib.rs:291-296`), but
  no capability grants any `updater:*` permission — `capabilities/
  default.json` holds only `core:default` + a scoped `opener` grant, and the
  CSP (`default-src 'self'`) admits no remote script. Drift is now pinned by
  test (§7).
- No dev/allowlist flag reachable in release: the plugin's only debug-mode
  relaxation is http endpoints warning instead of erroring
  (`cfg(debug_assertions)`, plugin `src/config.rs:205-216`); our endpoint is
  https. `dangerousAcceptInvalidCerts`, `dangerousAcceptInvalidHostnames`,
  `dangerousInsecureTransportProtocol` are unset (default false) — now
  pinned by test (§7).

## 2. The manifest chain — ruling: SOUND, blast radius enumerated

The signature DOES travel inside the site-controlled manifest. That is safe
because validity is anchored client-side, not server-side:

- WHERE the signature comes from: the manifest's `platforms["windows-x86_64"]
  .signature` field — fully attacker-controlled *as a string*.
- WHAT makes it binding: `verify_signature` (plugin `src/updater.rs:1797-
  1821`) decodes it with `minisign_verify::PublicKey::verify(data, &sig,
  true)` where the key is the client-embedded key and `data` is the bytes
  actually downloaded from the (also site-controlled) `url`. `verify` checks
  the key_id matches the embedded key, the Ed25519 signature over the
  package bytes, AND the global signature over `sig || trusted_comment`
  (`minisign-verify` `verify_ed25519`) — so the trusted comment that carries
  `version:<V>` is itself cryptographically covered.
- Then `verify_signed_version` (plugin `src/updater.rs:1829-1863`) requires
  that comment's version to equal the announced manifest version
  (`requireSignedVersion: true`; missing → `MissingSignedVersion`, mismatch →
  `SignedVersionMismatch`).

Precise enumeration of a compromised-SITE attacker (controls `latest.json`
completely, and via `/api/download` redirects — reqwest default
`Policy::limited(10)`, verified — controls package serving too):

| Attacker serves | Client does | Why |
|---|---|---|
| Malicious package + forged signature | `InstallFailed`, running version keeps serving | minisign verify fails; forging needs the private key |
| Malicious package + genuine old `.sig` | `InstallFailed` | signature is over the old bytes, not these |
| Genuine vN package + genuine vN sig, manifest says vN, vN ≤ client version | treated as up-to-date, nothing installed | strictly-newer comparison (§3) |
| Genuine vN package/sig, manifest says vM ≠ vN | rejected | `SignedVersionMismatch` (requireSignedVersion) |
| Legacy sig without a version comment | rejected | `MissingSignedVersion` |
| Genuine vN package/sig, manifest says vN, vN > client version | installs vN | indistinguishable from our own release of vN — see §3 residual |
| Stale manifest forever, or manifest garbage/404 | stays on current version; `CheckFailed` line; app unaffected | update withholding / failed check |
| Endless package stream (huge `Content-Length`, slow drip) | buffers it all in RAM before verifying → OOM crash of the app | advisory A-1; no size cap/timeout; no code execution |
| Package URL redirected to `http://` or a third-party host | downloads over plain TLS-less transport, then verifies | signature still gates; reqwest follows https→http (strips headers) — advisory A-2 |

**Conclusion (two lines):** the minisign signature rides in the
site-controlled manifest but is verified against the build-embedded key over
the downloaded package bytes plus the signature-covered version comment, so
an unsigned-by-us package can never install — the manifest carries no trust,
it only transports a signature the attacker cannot mint. A compromised
origin's entire blast radius is: withhold updates, re-serve genuine signed
versions newer than the client's current one, and crash the client during
download (A-1).

## 3. Downgrade protection — ruling: ADEQUATE FOR v0.2.x; FENCE REQUIRED FOR v1

- No rollback below current: the app sets no comparator and
  `allowDowngrades` is unset, so the plugin default applies —
  `release.version > current_version` (plugin `src/updater.rs:669-672`,
  comparator wiring `src/lib.rs:99-104`). A client on 0.2.22 cannot be
  served 0.2.21. Pinning a tester on a vulnerable OLD version by serving it
  does not work; only withholding works against a current client.
- Version-confusion (old artifact labeled new) is blocked by
  `requireSignedVersion` (§2).
- **Residual, no fence exists:** (a) withholding — a compromised origin pins
  every client at whatever it runs today, indefinitely; (b) re-serving — any
  version we ever signed that is newer than a given client (e.g. a yanked
  0.2.24 to a 0.2.23 client) can be re-served forever. `pub_date`/`notes`
  are advisory only in this plugin version.
- **RC-2 (required for v1, recorded):** before any wider-than-invite
  distribution, add a rollback fence — either sign the manifest chain, or
  bake a signed minimum-version floor into each release (a floor shipped in
  release N protects clients at ≥ N, bounding pinning windows to one
  adoption cycle), and consider an update-nag when the check fails
  persistently. Recorded in `docs/threat-model.md` §4.5 residual risks and
  §5 non-guarantee 6.

## 4. Process hygiene — ruling: SOUND

- **Engine kill before install:** `close_all_retained_jobs`
  (`crates/modelswarm-winjob/src/lib.rs:162-180`) closes every retained
  kill-on-close job handle; it runs in `on_before_exit`
  (`src/app/updater.rs:294`, `:443-465`), which the plugin invokes AFTER
  signature verification + temp-file extraction and immediately BEFORE
  spawning the NSIS installer (plugin `src/updater.rs:944-949`) — the
  installer never races a live engine binary. Test-pinned in winjob (two
  children killed by one call, idempotent).
- **Install/exit order:** `Ready` is written before `install()`
  (`src/app/updater.rs:368`) and is terminal; on Windows `install()` ends in
  `std::process::exit(0)` (plugin `src/updater.rs:979`) after
  `ShellExecuteW` of the NSIS updater with `/P /UPDATE /R` (passive mode
  args, plugin `src/config.rs:45-58`) — per-user, no elevation
  (`perMachine = false`).
- **Crash-loop safety:** a failed check writes `CheckFailed` and the 6h
  ticker moves on; a failed download/install writes `InstallFailed` and the
  running version keeps serving — one attempt per 6h, no restart loop; the
  supervisor breaks after `Ready` (`src/app/updater.rs:259-265`); late
  progress/install events are ignored by the state machine (test-pinned).
  The NSIS `/R` restart is one-shot from the installer, and the restarted
  app's supervisor starts a fresh cycle — no loop. The only "chained
  installs" vector would require an unbounded supply of ever-newer genuine
  signed versions, which the attacker cannot mint.
- **Failure inside install:** `ShellExecuteW` failure (≤32) returns `Err`
  → `InstallFailed`, app keeps running. Note A-4: by then the engines are
  already dead (the hook ran first); the node's engine watcher / restart
  budget is the recovery path. Availability only.

## 5. Opt-out and dev surface — ruling: SOUND

`env_disables_autoupdate` (`src/app/updater.rs:237-239`) accepts exactly
`Some("1")` — test-pinned against `None`/`""`/`"0"`/`"true"`. It disables
CHECKING (supervisor writes `Disabled` and returns), never verification.
No other env var, config key, or command disables or weakens the chain (§1).
A typo'd opt-out keeps updates ON — correct direction (never silently strand
a client).

## 6. Key lifecycle — ruling: GAP → RC-1 (partially applied in this review)

The handoff states rotation in prose (§6: "key rotation = ship a new client
with a new pubkey (manual install) — documented, not solved") but NO
standing release-checklist obligation carries it, and the v0.2.21 train
record predates the updater. **RC-1:** the release-train obligations (the
`docs/reviews/handoff-test-release-*` execution records, fed by the updater
handoff §5) must permanently carry: (1) sign every installer at owner
staging with `--app-version <VERSION>` — mandatory, or every client rejects
the update; (2) the private key never enters the repo, CI, or logs (no
`TAURI_SIGNING_PRIVATE_KEY` in CI — verified absent; `createUpdaterArtifacts`
correctly NOT set); (3) rotation = a manual client ship with a new embedded
pubkey; until adoption of that release completes, the OLD public key remains
valid on deployed clients — treat the old private key as
compromised-and-live for one full adoption cycle. Applied in this review:
the substance is recorded in `docs/threat-model.md` §4.5 (Security-owned
path); the checklist line itself is routed to Windows Product + Test and
Release and must appear in the 0.2.22 train record.

## 7. Applied hardening (this review; test-covered, desktop crate only)

`crates/modelswarm-desktop/src/app/updater.rs`, test
`tauri_conf_pins_the_update_channel` — extended to pin the ABSENCE of chain
weakening, so a future config regression fails the suite instead of shipping:
`allowDowngrades` must be absent/false; the three `dangerous*` flags must not
exist; every endpoint must be https; and NO capability file under
`capabilities/` may contain an `updater:` permission (webview must never
drive installs). No production code was changed.

## 8. Findings summary

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| RC-1 | required-change | Key-lifecycle + mandatory `--app-version` obligations absent from the standing release checklist | Substance recorded in threat-model §4.5 (this review); checklist line routed to Windows Product + T&R, must land in the 0.2.22 train record |
| RC-2 | required-change (v1) | No rollback/minimum-version fence: withholding and re-serving of genuine newer signed versions are unbounded | Recorded (threat-model §4.5 residual, §5.6); not a v0.2.22 blocker — invite-only tester population |
| A-1 | advisory | Unbounded in-memory package buffering + no download timeout: a compromised origin can OOM-crash the client (local DoS; no code execution) | Routed to Windows Product: wrap `update.download(...)` in `tokio::time::timeout` (e.g. 30 min) and add a byte cap in `download_with_progress`; `on_chunk` cannot cancel, so wall-clock is the enforceable bound |
| A-2 | advisory | Plugin does not scheme-check manifest platform URLs or redirect targets (https→http redirect IS followed by reqwest default policy, headers stripped) | Signature gate covers integrity; transport-privacy-only loss; no action for v0.2.x; revisit if packages ever carry private metadata (they do not) |
| A-3 | advisory | Our `on_before_exit` REPLACES the plugin's default `cleanup_before_exit` hook (plugin `src/lib.rs:120-123` + `replace` in `on_before_exit`) — Tauri exit cleanup never runs on the update path | Impact nil today: `.run(generate_context!())` registers no `RunEvent` handlers, config writes are synchronous, telemetry `FileSink` appends synchronously. If exit-time persistence is ever added, call `handle.cleanup_before_exit()` inside the hook or state will be skipped on update restarts |
| A-4 | advisory | Engines are killed before the installer spawns; a `ShellExecuteW` failure then leaves the app running with dead engines until the node's engine watcher/restart budget recovers | Availability only; rare (temp file was just written); accept for v0.2.x |
| A-5 | advisory | Known items correctly disclosed by the owner: deb-managed Linux installs self-replace outside dpkg; installers remain unsigned (code signing = G6) so package signatures do not address initial-install trust or SmartScreen warnings; plugin `zip` feature (default on) would also accept signed `.zip` updater bundles — harmless while the train ships raw `.exe` | Already in handoff §7/nsis-notes; no change requested |

## 9. Commands and evidence (this review)

- `cargo fmt --check` — clean.
- `cargo clippy --workspace -- -D warnings` — clean.
- `cargo test --workspace` — **495 passed / 0 failed / 6 ignored** (matches
  the 495/0/6 handoff baseline; this review changed tests only).
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop
  --features tauri-shell` — **40 passed / 0 failed**; the strengthened
  `tauri_conf_pins_the_update_channel` passes (verified by filter run).
- Static verification against locked sources: plugin `verify_signature` /
  `verify_signed_version` / default comparator (`release.version > current`)
  / `install_inner` exit order / `on_before_exit` replace semantics /
  `validate_endpoints` release-mode https enforcement; `minisign-verify`
  `verify_ed25519` covers the trusted comment; reqwest 0.12.28 default
  redirect policy (limited(10), http/https only, https→http followed);
  `Cargo.lock` pins `tauri-plugin-updater 2.13.3` + `minisign-verify 0.3.0`.
- Repo scan: no minisign secret-key material anywhere (only the public key,
  docs, and lock entries); no `TAURI_SIGNING*` in `.github/`; no
  `createUpdaterArtifacts`.

## 10. Assumptions

1. The embedded pubkey is the owner's real channel key (decodes to a valid
   minisign Ed25519 public key; the owner brief supplied it). If the private
   half was generated with a pre-version-comment CLI, the first train sign
   must use tauri-cli ^2 (handoff §5.1 already says so).
2. v0.2.22's first updater-bearing cycle cannot be E2E-verified before ship
   (no signed manifest exists yet); T&R's staged-manifest rehearsal
   (handoff "suggested next task") is the remaining proof and should run
   before the 0.2.22 artifacts go public.
3. The 302 `/api/download` route stays rate-limit-compatible with a 6h
   check cadence (live probe in the handoff; per-IP limits).
