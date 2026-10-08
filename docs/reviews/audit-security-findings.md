# Security Audit Findings — Master Prompt §12 Floor Verification

- Date: 2026-10-09
- Branch: `audit/master-prompt-2026-10-09` (frozen baseline `730cebf`, v0.2.20)
- Auditor: Security Engineer
- Method: static analysis only (Read/Grep/git ls-files). The validation suite
  holds the `target/` lock; no `cargo`/`npm` commands were run. Findings are
  evidence-cited file:line against this exact baseline.
- Scope: master prompt §12 non-negotiable floor + §2 resource policy + owner
  calibration ("fast working system over hardened; the floor is non-negotiable;
  answer perf-vs-crypto with measured numbers").

## Floor verification table

| Floor clause (§12) | Verdict | Evidence |
|---|---|---|
| Tracker never receives prompts or completions | PASS | Strict `.strict()` zod schemas reject unknown keys incl. `messages`/`prompt`/`input` — `apps/tracker/lib/schemas.ts:1-4, 209-222`; F4 abuse test `apps/tracker/tests/abuse.test.ts:118-166`; migration `0003_model_requests.sql:6-21` stores only artifact pointers + bounded labels (`note` ≤280, `display_name` ≤80 are user-supplied labels, not inference content); no prompt-shaped DB columns anywhere; logging is structured and value-free — `apps/tracker/lib/errors.ts:80-86`, `lib/envelope.ts:232-253` (issue counts only, never field values). |
| Local API binds to loopback by default | PASS | Structural: `crates/modelswarm-gateway/src/http.rs:58-79` (`assert_loopback` + node-constructed socket), re-asserted in `crates/modelswarm-node/src/lib.rs:261-263`; node refuses non-loopback sidecar URLs `lib.rs:549-573`; llama.cpp sidecar spawned `--host 127.0.0.1`, random port, random bearer — `crates/modelswarm-node/src/engine.rs:240-247, 386-409`. The only non-loopback listener is the opted-in P2P serving transport (`MSP_LISTENER=1`, `crates/modelswarm-desktop/src/app.rs:248-294`), which is not the local API. |
| Secrets never enter source control | PASS | `git ls-files` scan: no key/pem/env/credential files; `apps/tracker/keys/` holds only `hub-public.hex` (public key; private seed injected via `MSP_HUB_SEED` env — `apps/tracker/lib/context.ts:36-56`, `keys/README.md:3-5`); dev seed is published but fail-closed in production (`context.ts:31-50`, tested in `tests/hardening.test.ts:25-38`); CI fixture values (`07…07`, `ci-admin-token`) are clearly test-only (`github/workflows/tracker.yml:93-94`). Residual: identity seed written plaintext — see M-3. |
| Logs redact prompt/generated content by default | PARTIAL | Redaction is convention-based, not structural: forbidden field names + 256-char truncation + secret-shape scrub (`crates/modelswarm-telemetry/src/redact.rs:12-36, 60-71`); the bypass is documented in its own test — prompt text under an allowed field name passes through (`redact.rs:189-196`). Unredacted `eprintln!` paths exist outside the telemetry sink (see L-1). The default JSONL file log and tracker logs are redacted; stderr is not. |
| Model and client updates are signed | PARTIAL | Model side is strong: signed catalog verified against pinned hub key + id re-derivation (`crates/modelswarm-node/src/catalog.rs:32-79`), artifact SHA-256 + identity hashes (`artifact.rs:96-193`), engine binaries hash-verified at CI fetch, staging, and every launch (`engine.rs:177-237`). Client side: installers are NOT code-signed — no signing certificate exists (`docs/verification/phase-g-notes.md:166-172`), SmartScreen warnings expected (`phase-h.md:39`); `SHA256SUMS.txt` is published but nothing verifies it automatically; no auto-update channel exists (updates = re-download over HTTPS). |
| Untrusted peer data has strict size, time, and rate limits | PARTIAL | Transport: 256 KiB frames validated before allocation + deadlines (`crates/modelswarm-transport/src/frame.rs:24-95`, `libp2p_backend.rs:145-167`); serving clamps wire limits (`crates/modelswarm-node/src/serving.rs:103-114`); tracker: 64 KiB body cap + ±120 s ts + single-use nonces + rate limits (`apps/tracker/lib/envelope.ts:29-30, 168-178`, `lib/ratelimit.ts:16-21`). Gaps: `/catalog/requests` reads its body uncapped (M-5); serving admission runs BEFORE the lease gate (M-2); the relay has no per-circuit byte cap and no access control (M-1). |
| Every network parser is fuzzed | FAIL | No coverage-guided fuzzing anywhere (no cargo-fuzz/AFL/libFuzzer targets; no fuzz crates in `Cargo.toml`). Hand-rolled seeded mutation tests exist: session machine 10k iterations (`crates/modelswarm-session/tests/session_fuzz.rs:74-181`), malformed-frame fuzz 10k seeded mutations (`tests/adversarial.rs:1512-1560`), tracker abuse suite (`apps/tracker/tests/abuse.test.ts`). The network-facing `WireMessage` serde parser, the tracker zod surface, and the GGUF metadata reader have no dedicated fuzz target. Reachability of the un-fuzzed parsers is bounded by the 256 KiB frame cap / 64 KiB body cap / post-hash-verify GGUF parsing — see L-3. |
| Dangerous operations require authenticated capability checks | PASS | Admin ops behind `ADMIN_TOKEN` constant-time compare (`apps/tracker/lib/guard.ts:73-79`, `lib/crypto.ts:229-233`); every peer mutation requires the signed envelope + session (`lib/envelope.ts:107-192`); the ADR-026 lease gate — hub signature against pinned key, freshness, profile match, `can_consume` rights, QUIC-authenticated PeerId binding — is enforced in the production serving path per request, before the executor (`crates/modelswarm-node/src/serving.rs:302-312, 941-1001`), wired with `LeasePolicy::production()` in the shipping desktop (`crates/modelswarm-desktop/src/app.rs:922-929`). |

**Floor verdict: 3 PASS / 4 PARTIAL / 1 FAIL.** No CRITICAL floor violation was found.
The FAIL (fuzzing) and the PARTIAL items are gap findings below; none of them
changes the content-blindness or loopback guarantees.

---

## Findings by severity

### HIGH-1 — Hosting challenge is a self-attested timing form; a free-rider earns consume leases with two integers (result fabrication)

- Floor clause: project rule (actively and verifiably hosting), §12 "result fabrication".
- Evidence:
  - `apps/tracker/app/api/v1/peers/challenge/complete/route.ts:37-47` — `passChallenge` stores client-supplied `firstTokenMs`/`totalMs`; nothing verifies any token was generated, by which runtime, or that the artifact exists.
  - `apps/tracker/lib/eligibility.ts:41-46` — `capacityFromTimings(totalMs)` derives the `verified_capacity` class purely from the self-reported number; a claimed `totalMs ≤ 2000` labels the peer `gpu_high`.
  - `apps/tracker/lib/constants.ts:26` — the challenge prompt is a fixed 31-character public constant, so a peer can precompute it once (or on any machine) and reuse timings.
  - The result: `apps/tracker/app/api/v1/peers/lease/route.ts:41-76` issues a hub-signed consume lease on the strength of that fabricated challenge; the serving gate then accepts it because the *lease* is genuinely signed — `crates/modelswarm-node/src/serving.rs:970-1000` checks signature/freshness/profile/rights/binding, but never whether the requester ever hosted anything.
- Attack: register an installation (enrollment is auto-approved up to 250 devices), claim an active catalog profile, POST `challenge_complete` with `firstTokenMs: 0, totalMs: 1`, POST `request_lease`. Total: three signed HTTP calls, zero inference, zero model. The peer can now consume swarm inference from honest hosts of profile P while hosting nothing — the exact core rule the protocol exists to enforce.
- Reachability: remote; any enrolled installation (Sybil exposure bounded only by `DEVICE_APPROVAL_CAP=250` + rate limit).
- Exploitability: trivial (two integer fields; no cryptography, no compute).
- Severity: HIGH, not CRITICAL — AGENTS.md frames the rule as "prototype-grade deterrence, not remote attestation", and the honest host still enforces a real signature chain; but the docs overstate the defense: threat-model §4.1 "hosting challenge before capability issuance" and ADR-026 "earned by demonstrably hosting the profile … enforced cryptographically" describe verification that does not exist. The challenge currently adds ~zero deterrence value while producing a trust-named `verified_capacity` label.
- Time-boxed fix: (a) 30 min — rename/label honestly (the label feeds roster ranking; see M-6); (b) 2–4 h tracker + node — make `challenge_complete` carry a digest of the generated tokens for the fixed prompt (`sha256(prompt || tokens)`) and have the tracker spot-verify a sample of claims by asking one *other* currently-passing peer to reproduce the canary locally; (c) beyond that, external timing probes are ADR-gated audit-chain work, not v0.x. No new cryptography anywhere; zero hot-path latency cost (challenge is an off-path ceremony). This is the single highest-value remediation in this audit.

### MEDIUM-1 — Relay has no access control and no per-circuit byte cap

- Evidence: `crates/modelswarm-relay/src/main.rs:10-12` ("Access control is NOT here"), `:27-42` (`max_circuit_bytes: u64::MAX`, `max_circuit_duration` ≈ 136 years, kept deliberately so token streams are never reset — ADR-014).
- Attack: any internet host can reserve (30/2 min/peer creation rate applies) and pump unbounded bytes through circuits to any destination PeerId; the serving peer's lease gate protects *inference*, but the relay host pays for every byte (bandwidth exhaustion, relay-as-amplifier, relayed DoS against serving listeners).
- Reachability: remote (public relay host; not yet deployed — owner decision pending). Exploitability: easy for bandwidth abuse; harder to turn into inference theft (lease gate holds).
- Time-boxed fix: 1–2 h — cap `max_circuit_bytes` at a value far above any legitimate session (e.g., 4 GiB per circuit) and/or add a per-peer byte allowance; document the trade-off. Zero latency impact (the cap is never hit by honest streams; connection reuse means pooled sessions amortize it).

### MEDIUM-2 — Serving admission runs before the lease gate; 8 un-leased sessions can starve remote serving

- Evidence: `crates/modelswarm-node/src/serving.rs:70-79` (`admit_session` on accept) vs `:304-312` (lease check after the first frame). A peer that opens a session and sends nothing holds a slot for the full `BETWEEN_REQUESTS = 300 s` (`:254`). Eight such sessions (2 per PeerId × 4 peers, PeerIds free to generate) exhaust `Admission::defaults()` (8 total, `:175-179`) → all honest remote serving refused; the local gateway is unaffected.
- Reachability: remote, un-leased (only a QUIC handshake, no enrollment). Exploitability: trivial script; DoS only, no data access.
- Time-boxed fix: 1 h — perform the lease check before `admit_session` (or use a short, e.g., 10 s, first-frame deadline for not-yet-leased sessions). Zero latency cost for honest peers.

### MEDIUM-3 — Installation Ed25519 seed stored in plaintext; threat model claims DPAPI/Credential Manager

- Evidence: `crates/modelswarm-node/src/lib.rs:608-622` (plain `fs::write`; 0600 only on Unix; Windows relies on the profile-directory ACL — honestly documented in the same file, `:14-20`). Claimed mitigation in `docs/threat-model.md:85` ("Windows Credential Manager/DPAPI") and `docs/privacy.md:44` is not implemented; it is a recorded deferred hardening.
- Attack: any same-user process (or malware running as the user) reads `%LOCALAPPDATA%\ModelSwarm\Data\identity.seed` and impersonates the installation — signs hub requests, steals the victim's earned lease cycle, poisons roster state.
- Reachability: local, same-user. Exploitability: trivial file read. Also undermines the value of every signed-envelope defense for that installation.
- Time-boxed fix: 2–4 h — DPAPI `CryptProtectData` on the seed at rest (Windows) with the existing file as fallback for other platforms; add a migration that protects existing seeds. Zero performance impact (startup-only).

### MEDIUM-4 — Loopback API has no authentication and no Origin/Host/content-type checks (DNS rebinding)

- Evidence: `crates/modelswarm-gateway/src/http.rs:113-118` (router with no layers; no CORS/origin/host validation anywhere in the crate), `:174` (`body: Bytes` extractor — `content-type` is never checked, so a `text/plain` "simple" POST carrying JSON is accepted without a CORS preflight).
- Attack: a page the user visits (or any DNS-rebinding name) issues blind `POST http://127.0.0.1:11435/v1/chat/completions` with a JSON body; the request executes (compute/resource theft, prompt side-effects); the response cannot be read cross-origin (default CORS deny), so no output exfiltration by a browser page. Documented posture: threat-model §4.3 "v0.1 accepts any local client (documented limitation; local auth token is a candidate hardening)" — but that documents *local* clients; rebinding extends it to remote web pages.
- Reachability: requires the user to visit a malicious site while the node runs. Exploitability: blind-use only.
- Time-boxed fix: 1–2 h — reject requests whose `Host` is not `127.0.0.1`/`localhost` and whose `Origin` is present and non-local. Zero latency cost; no cryptography.

### MEDIUM-5 — `/api/v1/catalog/requests` reads the request body without the 64 KiB cap

- Evidence: `apps/tracker/app/api/v1/catalog/requests/route.ts:23` (`await req.json()` directly) vs the shared cap used everywhere else (`apps/tracker/lib/guard.ts:60-63`, `lib/envelope.ts:217-230`). Every persisted field remains bounded (zod: `note` ≤280, `display_name` ≤80, `artifact_path` ≤200, `hf_repo` ≤120 — `lib/schemas.ts:209-222`), so content-blindness holds; the unbounded part is the JSON parse itself, limited only by Vercel's platform body cap (~4.5 MB).
- Reachability: remote, anonymous. Exploitability: parse-cost amplification at 10 req/min/IP (self-imposed rate limit helps; per-IP impact bounded).
- Time-boxed fix: 30 min — route through `guardPublicBody`/`readBody`. Zero perf impact.

### LOW-1 — Redaction is bypassable by convention and some logging paths skip the redactor entirely

- Evidence: documented bypass test `crates/modelswarm-telemetry/src/redact.rs:189-196`; unredacted peer-controlled text printed via `eprintln!` in `crates/modelswarm-node/src/remote.rs:170-172` (`wire_error.message`, attacker-controlled up to the frame cap), plus `:143, 212, 228, 375, 381` and node `main.rs` prints. The JSONL file log (the diagnostic-export surface) is protected; stderr is not part of any export bundle (no log-bundle feature exists in v0.2.20 — verified in `crates/modelswarm-desktop/src/app.rs`).
- Time-boxed fix: 1 h — route the `remote.rs` diagnostics through the telemetry redactor (stable codes only, matching `serving.rs:119-131`); add a CI lint asserting no direct `eprintln!` of peer-controlled strings in node/transport crates.

### LOW-2 — Revocation list is not checked at the serving gate (docs claim it is)

- Evidence: threat-model §4.1 "revocation list checked by serving peers via hub lookups" and ADR-006 "checks hub revocation notices … at heartbeat cadence" vs the actual gate `crates/modelswarm-node/src/serving.rs:970-1000` — fully offline (signature + freshness + profile + rights + binding). Revocation only reaches peers via heartbeat notices; a revoked token stays usable until its own ≤135 s expiry (`LEASE_TTL_MS` 75 s + 60 s grace, `apps/tracker/lib/constants.ts:5-8`).
- Impact: bounded by token TTL; deterrence-grade. Fix: optional periodic revocation probe for long-lived sessions (defer; ADR-006's heartbeat-cadence design is acceptable for v0.x). Zero perf impact if batched.

### LOW-3 — No coverage-guided fuzzing (floor FAIL, bounded reachability)

- See floor table. The un-fuzzed parsers: `WireMessage` serde (≤256 KiB frames, validated prefix first — `crates/modelswarm-transport/src/libp2p_backend.rs:145-167`), tracker zod/JSON (64 KiB cap on all but one route), GGUF metadata (only ever reads hash-verified bytes — `crates/modelswarm-node/src/artifact.rs:96-125`, so tampering requires a hash collision or a compromised hub key). libp2p internals are an upstream dependency.
- Time-boxed fix: 1 day for a cargo-fuzz target on frame decode + `WireMessage` round-trip invariants, run in CI on a schedule; tracker parser fuzzing can be a vitest-based mutation loop. No perf impact.

### LOW-4 — No `cargo audit` / `cargo deny` in CI

- Evidence: `.github/workflows/rust.yml` runs fmt/clippy/test only (lines 31–41); no dependency-advisory gate. Flagged for the dependency auditor (their depth). Cheap: add `cargo audit` + `cargo deny` (or `npm audit` for the tracker) as a scheduled job.

### LOW-5 — Installers unsigned; checksums published but never verified automatically

- Evidence: `docs/verification/phase-g-notes.md:166-172` (no signing certificate), `phase-h.md:39` (SmartScreen expected), `apps/tracker/public/downloads/SHA256SUMS.txt` (manual verification only), `apps/tracker/app/api/download/[file]/route.ts:15-32` (count + redirect; the filename pattern blocks traversal). There is no auto-update channel, so the attack surface is the HTTPS download itself plus supply chain (covered elsewhere). Remediation is a product/certificate decision (stop-and-ask), not code.

### LOW-6 — Published dev seed + fail-closed guard: residual misconfiguration window

- Evidence: `apps/tracker/lib/context.ts:34` (dev seed in repo) with the fail-closed guard at `:46-50`. The guard keys on `VERCEL_ENV === "production" || NODE_ENV === "production"`; any production-shaped deployment that fails to set both would sign with the public dev key. Acceptable; note in the ops runbook (already covered by `tests/hardening.test.ts`).

### INFO-1 — Self-reported roster metrics (freeSlots/queueMs) are unverified

- `apps/tracker/app/api/v1/peers/heartbeat/route.ts:23-37` accepts client-supplied `freeSlots`/`queueMs` (bounded 0–8 / 0–3 600 000). The master prompt §11 already mandates "advertised values are untrusted" with measurement plumbing; no action beyond keeping that commitment.

### INFO-2 — Session token persisted plaintext

- `crates/modelswarm-desktop/src/app.rs:979` writes `session.token` to the data dir. A same-user reader can enumerate the roster (24 h window) but cannot sign envelopes without the identity seed (M-3 subsumes this). Fold into the M-3 remediation.

### Trade-off posture (owner calibration)

Every remediation above is structural or a short fixed check: none adds cryptography to any hot path, and none adds measurable latency. The one item with a real cost trade-off is HIGH-1(c) (external timing probes / audit chain) — deferred, ADR-gated, and priced against measured deterrence benefit rather than asserted. Connection reuse and prefill remain the sanctioned latency levers; nothing in this report disturbs them.
