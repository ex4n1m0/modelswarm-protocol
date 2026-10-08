# Handoff — Security Engineer — 2026-10-09

Audit of the master prompt §12 security floor against frozen baseline
`730cebf` (v0.2.20) on branch `audit/master-prompt-2026-10-09`. Static
analysis only: the validation suite holds the `target/` lock, so no
`cargo`/`npm` commands were run (constraint honored). Full findings:
`docs/reviews/audit-security-findings.md`.

## 1. Changed files (paths) and why

- `docs/reviews/audit-security-findings.md` — NEW. Floor verification table
  (clause → verdict → file:line evidence) + severity-ranked findings with
  reachability, exploitability, time-boxed fixes, and trade-off posture.
- `docs/reviews/handoff-security-2026-10-09.md` — NEW. This file.
- Nothing else was modified. No commits were made. `zcode-master-prompt.md`
  and `docs/reviews/gap-engineering-study-2026-10-09.md` were read as
  context and left untouched.

## 2. Exact commands run and their outcomes

- `git branch --show-current` → `audit/master-prompt-2026-10-09`; `git log
  --oneline -3` → HEAD `730cebf` (expected baseline).
- `git ls-files | grep -iE "\.(pem|key|crt|p12|pfx|env|secret|token)$|id_rsa|…"`
  → no key material tracked; only `apps/tracker/keys/hub-public.hex` exists
  (public). CI fixture values confirmed test-only.
- Grep passes over `apps/tracker/**` (routes, `lib/`, migrations), all
  `crates/**`, `installer/`, `.github/workflows/`, `docs/adr/`, `docs/`
  (threat model, privacy, verification) — outcomes recorded in the findings
  file; every finding carries file:line evidence.
- Production tracker was not contacted at all (no GETs were needed; live
  site untouched).

## 3. Test evidence (command + summary)

Not runnable in this session by design (static-analysis mandate; the
validation suite holds `target/`). Evidence reviewed instead:
- `apps/tracker/tests/abuse.test.ts` (F1–F7: rate limits, 64 KiB cap,
  unlogged bodies, inference-shaped rejection, enroll flooding) — present
  and asserted against the exact code paths audited.
- `apps/tracker/tests/hardening.test.ts` — production fail-closed on missing
  `MSP_HUB_SEED`.
- `crates/modelswarm-session/tests/session_fuzz.rs` (10 000-iteration
  mutation fuzz) and `tests/adversarial.rs` F9 malformed-frame fuzz
  (10 000 seeded mutations) — hand-rolled fuzz exists; no coverage-guided
  fuzzing anywhere (recorded as floor FAIL).
- `crates/modelswarm-node/src/serving.rs` gate tests (own/foreign lease,
  leaseless refusal, profile mismatch) and `protocol/vectors/lease-hubkey-1.json`
  golden vector — the ADR-026 gate is verified in production wiring
  (`crates/modelswarm-desktop/src/app.rs:922-929`), not only in tests.

## 4. Assumptions made

- Vercel applies its platform request-body cap (~4.5 MB) to the one uncapped
  route (M-5), bounding its severity.
- The relay is not yet production-hosted (owner decision pending), so its
  resource-bound gap is MEDIUM rather than a live production issue.
- The published dev seed (`context.ts`) is safe because the fail-closed
  production guard is tested; residual risk recorded as LOW-6.

## 5. Unresolved risks

- HIGH-1: hosting challenge verifies nothing — self-reported timings earn
  consume leases; the core contribute-to-consume rule is currently an
  honesty form (deterrence-grade by documented design, but docs overstate
  it). Highest-value remediation in the report.
- M-2: serving admission precedes the lease gate → 8 un-leased 300 s
  sessions starve remote serving.
- M-3: plaintext identity seed vs DPAPI claim in threat model.
- M-4: DNS-rebinding blind use of the loopback API (no Origin/Host checks).
- M-1: relay open access + uncapped per-circuit bytes.
- Floor FAIL: no coverage-guided fuzzing (L-3); floor PARTIALs: redaction
  bypassable by convention (L-1), installers unsigned (L-5).

## 6. Suggested next task for the integrator

1. Fold the findings into the consolidated audit (`audit-current-system.md`,
   `audit-production-risk-register.md` from the parallel tracks).
2. Assign HIGH-1 to the Security/Tracker engineers (30-min honest labeling
   first, then the 2–4 h receipt-digest + spot-verification fix) and the
   four MEDIUM items as time-boxed hardening above the floor.
3. No production behavior changes were made and none are requested here;
   nothing here disturbs the loopback, content-blindness, or signed-catalog
   guarantees.
