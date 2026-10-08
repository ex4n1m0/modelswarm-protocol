# Audit — Production Risk Register — 2026-10-09

Production-facing risks from the 2026-10-09 master-prompt audit. Updates
`docs/risk-register.md` (R1–R23) where statuses changed — that register
remains the planning-risk record; this one governs production decisions
until the owner-approved gate replaces both with a merged register.

Severity = impact × likelihood, owner-calibrated (speed over hardening
above the non-negotiable floor; measured numbers over crypto additions).

## A. New / sharpened production risks (audit-ranked)

| # | Risk | Sev | Evidence | Mitigation (time-boxed) | Owner |
|---|---|---|---|---|---|
| P1 | Fake streaming: TTFT ≈ completion on every chat (local + LAN); all latency baselines built on it are wrong-shaped | CRITICAL | runtime handoff (measured 9.11→24.73 ms/tok; native SSE = TTFT 4 ms, 7.3× at long prompts) | true SSE adapter + incremental executor + per-delta UI (~3–4 d total), BEFORE M9 baseline freeze | Runtime |
| P2 | Earn chain integrity: two self-attested integers earn a lease; `verified_capacity` inflatable | HIGH | security + tracker T1 + architect (triangulated) | relabel honestly now (30 min); nonce-bound prompt + greedy-canary possession digest riding E0 (2–4 h + tracker work) | Tracker + Security |
| P3 | Suspension impossible: policy fn has no caller; slots-to-zero blocked by DDL CHECK; de-facto revocation = token TTL ≤ ~135 s | HIGH | architect + tracker T2 | migration-0004 ADR + refusal-event feeder; prerequisite for M10 reputation chassis | Tracker |
| P4 | Zero parser fuzzing — §12 floor FAIL; hostile exposure grows with relay/internet use | HIGH | security + test-gaps G3 | cargo-fuzz targets: transport frames, GGUF, canonical JSON, lease wire | Security + Test/Release |
| P5 | Pre-first-token failover broken in practice (`retryable:false` on stale-pool/early transport failures) — ADR-007's core promise | HIGH | network HIGH-2 | map failure classes → retryable pre-first-token; pooled-session health check | Network |
| P6 | No RTT/loss telemetry on the production QUIC path — planner, M9 baselines, and hedging all blocked | HIGH | network HIGH-1 + scheduler S2 | measurement plumbing first (QUIC RTT EWMA, completion distributions, measured queue depth) | Network + Scheduler |
| P7 | Test-gate holes: real-model-e2e NEVER ran; GPU-variant test never CI-executed; tauri-shell never clippy-linted (2,301-line app.rs) — the class that hid the H1 lease break | HIGH | test-gaps G2/G4 + windows F3 | dispatch + record e2e; wire GPU ignored test; add tauri-shell clippy pass; guards resource-freeze assertion | Test/Release |
| P8 | Wire drift: 5 live tracker endpoints + `capacityClass` unlabeled; P2P error codes off-registry; §6.1 handshake skip unrecorded; no msp-v1 changelog | HIGH | architect | one tracker-surface reconciliation ADR + error-code fix + changelog | Protocol Architect |
| P9 | Tracker bloat/abuse surface: write-only unbounded tables; `DEVICE_APPROVAL_CAP=250` counts installs EVER (zero-click enrollment ends permanently once hit); uncapped `/catalog/requests` body; unthrottled download-counter writes | MEDIUM | tracker T4/T5 | hygiene batch + prune policy + migration; extend wire-compat job | Tracker |
| P10 | Relay is open to anyone with uncapped per-circuit bytes — currently LAN-staged on the owner's desktop B; any port-forward exposes it | MEDIUM (HIGH if exposed) | security M-1 + relay/main.rs:26-42 | auth + per-circuit caps BEFORE any internet exposure (gate item with F2b) | Network + Security |
| P11 | Local API DNS-rebinding: no Origin/Host checks on 127.0.0.1:11435 — a malicious page can drive inference blind | MEDIUM | security M-4 | Origin/Host + content-type checks (loopback, no user friction) | Gateway (Integrator-held) |
| P12 | identity.seed plaintext on disk vs documented DPAPI claim | MEDIUM | security M-3 | DPAPI/credential-manager storage; document honestly meanwhile | Security |
| P13 | Uninstaller may delete identity/models (data-dir split unverified; stale NSIS notes) | MEDIUM (unverified) | windows F4 | clean-VM uninstall drill before next release | Windows |
| P14 | Unsigned installers/updates (SmartScreen; update-delivery trust) | MEDIUM (owner decision pending: cert spend) | dependency + windows | code-signing certificate decision at the gate | Windows + Owner |
| P15 | Resource ceiling deviation: recommender gates at 90% RAM/VRAM vs §2's 70%; engine runs all cores, normal priority, fixed ctx | MEDIUM | windows + runtime | M2 governor ADR reconciling recommendation-vs-runtime ceilings | Windows (gate input) |
| P16 | Engine-adapter dependency: every lossless cooperative mode is structurally ≤ 0 speedup until an in-process C-API adapter exists | HIGH (schedule risk) | scheduler S3 + runtime | adapter spike lands inside R0/R3; 9.6 pass 1 published honestly (expected negative) | Runtime + Scheduler |

## B. Existing register (R1–R23) — status updates from this audit

| Risk | Status change | Evidence |
|---|---|---|
| R5 free-riders | **PARTIALLY MITIGATED, weakened**: lease coupling live and gate-proven, but the challenge is self-attested (P2) and suspension is non-functional (P3) | wire-compat CI + security/tracker audits |
| R9 self-reported metrics | **OPEN, larger than assumed**: not "advertised vs measured" — NO measurements exist on the production path at all (P6); scheduler unwired (alphabetical selection) | network + scheduler audits |
| R4 NAT / R3 relay boundary | **EVOLVED**: explicit-failure states shipped (good); relay now EXISTS standalone (F2A proven) but nodes cannot use it (F2b) and it is unauthenticated (P10) | network audit |
| R22 PgStore never executed | **MITIGATED**: tracker CI integration-postgres job green | freeze CI evidence |
| R23 peerId placeholder | **CLOSED**: ADR-020 multihash shipped with cross-language goldens | architect §3 |
| R20 llama.cpp hooks | **SHARPENED**: now P16 — the HTTP adapter (not just hooks) is the blocking dependency, measured | runtime + scheduler |
| R11 unsigned installers | **OPEN** — still internal-test-unsigned; becomes P14 (owner cert decision) | dependency audit |
| R16/R21 honesty gates | **HELD**: honest-results culture verified passing (negative results visible, TEST-ONLY labels, no average-baseline claims) | scheduler audit |
| R1/R7 artifact integrity | **HELD, strengthened**: pin chain exemplary (per-file hashes verified engine→CI→launch; ADR-022 parity locks) | dependency + runtime audits |

## C. Owner-stop items riding on this register

- Code-signing certificate (P14) — spend decision.
- k=4 harness VM spend (expanded-mission §15) — folds into the gate.
- Relay internet exposure (P10) — only after auth + caps.
- Migration-0004 (P3) touches the production DB → nonproduction deploy +
  gradual promotion per change control, owner notified.
