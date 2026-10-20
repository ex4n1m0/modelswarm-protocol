# Handoff — Tracker Engineer, 2026-10-20 (ADR-045 S-5 intake amendment)

Authority: `docs/adr/ADR-045-execution-presets.md` revision 2 (S-5:
intake-side enum/length validation routed to this seat),
`docs/reviews/handoff-protocol-architect-2026-10-20.md` §2 item 4 + §6
item 1, and the compatibility paragraph of
`protocol/msp-cooperative-v1.md` §6 ("trust/accounting surfaces
additionally REJECT out-of-vocabulary values rather than relabel them").

## 1. Changed files and why

- `apps/tracker/lib/schemas.ts` — `JobResultSchema`'s inner `receipt`
  object:
  - `preset: z.string()` → `z.enum(["fast","balanced","deep","verified",
    "maximum","unspecified"])`. The five tiers are the frozen ADR-029/045
    vocabulary; `unspecified` is the honest literal for unnegotiated
    paths (msp-cooperative-v1 §4), so it MUST stay intake-legal.
  - The inner receipt object is now `.strict()` — unknown keys fail the
    schema with `invalid_body` instead of being silently stripped before
    the digest preimage check (the T/R B2 zod-strip trap).
  - Peer entries (`peers[]` items) are also `.strict()` — a deliberate
    one-step extension, same rationale: peers sit inside the digest
    preimage, and zod strips unknown keys at every non-strict nesting
    level. Verified safe (below).
- `apps/tracker/tests/granted-accounting.test.ts` —
  - fixture `preset: "FAST"` → `preset: "fast"` (pre-freeze legacy
    uppercase is out-of-vocabulary as of this amendment);
  - new `it.each(["turbo", "FAST"])` rejection test: a fully signed,
    digest-consistent receipt carrying an out-of-vocabulary preset is
    rejected with HTTP 400 `error.code === "invalid_body"` (the honest
    schema failure — never a relabel), and no state moves (no digest row
    via `store.getReceipt`, zero granted rows).

Untouched, by design: `protocol/vectors/receipt-v2-1.json` (byte-frozen;
it carries legacy `"FAST"` and pins the digest MIRROR, which never
transits intake), `apps/tracker/lib/receipt-v2.ts` (`preset: string`
stays — the enum belongs to intake only), all Rust code.

## 2. The .strict() compatibility verification (the "verify that claim" step)

Cross-checked the full signed ReceiptV2 that transits
`/api/v1/events/job-result` against the Rust wire struct
`crates/modelswarm-eligibility/src/receipt_v2.rs` (`ReceiptV2`, 16
fields; `ReceiptPeer`, 3 fields — all plain `pub` with no
`skip_serializing_if`, no `flatten`, no `deny_unknown_fields` divergence),
the TS mirror `apps/tracker/lib/receipt-v2.ts` (`ReceiptV2` interface —
the route casts the parsed body to it and recomputes the digest from
exactly these fields), and the route
`apps/tracker/app/api/v1/events/job-result/route.ts`.

Result: **field-for-field exact match — no field had to be added.** The
zod schema now rejects nothing a Rust-built legitimate receipt carries.
`budgets` remains an open `z.record` (its inner keys are per-session
budget values, intentionally unvalidated content-wise; the signed
canonical form still pins them via the digest).

## 3. Commands and outcomes (all run from the stated dirs)

| Command | Outcome |
|---|---|
| `npm run typecheck` (apps/tracker) | pass (tsc --noEmit, no output) |
| `npm run build` (apps/tracker) | pass (Next.js production build, all routes emitted) |
| `npx vitest run tests/granted-accounting.test.ts tests/receipt-v2.test.ts` | 2 files, 15 tests passed |
| `npm test` (apps/tracker, full suite) | 25 files passed / 1 skipped, 216 tests passed / 2 skipped (skips are the live-PG suite) |
| `cargo fmt --check` (workspace root) | pass |
| `cargo clippy --workspace -- -D warnings` | pass (0 warnings) |
| `cargo test --workspace` | pass (exit 0; 64 test-result-ok suites, 0 failed) |

## 4. Test evidence highlights

- New rejections observed in the run log exactly as designed:
  `{"code":"invalid_body","path":"/api/v1/events/job-result","reason":"schema","issues":1}`
  for both `"turbo"` and legacy `"FAST"` — the redacted, issue-count-only
  failure shape (no field values logged).
- `receipt-v2.test.ts` (7 tests) unchanged and green — the byte-frozen
  golden-vector parity is untouched; the vector's `"FAST"` never passes
  through intake.

## 5. Assumptions

- `unspecified` must remain intake-legal: msp-cooperative-v1 §4 makes it
  the required value on every unnegotiated path (including the whole
  msp-v1 §6 pull), so rejecting it would break legitimate traffic today.
- Length bound on `preset` is unnecessary — the six-value enum is
  terminal (max length 11); ADR-045 S-5's "enum/length validation" is
  satisfied by the enum for this field.
- Deploy order per the architect handoff (§2.4): this intake enum lands
  BEFORE any preset-carrying receipt can be POSTed; mixed-fleet windows
  are bounded and fail-closed. Production deploy is the owner's Vercel
  CLI step from apps/tracker — NOT performed from this seat.

## 6. Unresolved risks

- Any hypothetical client that today POSTs receipts with junk keys under
  `receipt.*` or `receipt.peers[*].*` (previously stripped, digest still
  had to match so such traffic could only ever earn the honest
  `receipt_not_valid_evidence` reason) now gets a 400 instead. No such
  producer exists in the repo (Rust emits exactly the wire fields), but
  an out-of-tree client would see the stricter, honest failure.
- The skipped live-PG test file (`pg-enrollment`) was not executed here;
  unchanged by this diff.

## 7. Suggested next task for the Integrator

Proceed with the architect handoff §6 item 2: the P2 implementation arc
(two vector files incl. `session-offer-4-unknown-preset.json` and the
receipt-preset vector, ReceiptContext plumbing, the Rust doc-comment
refresh at `negotiation.rs:71-79` and `verifydrafts/mod.rs:424-426`, and
the `default_supported_presets` Deep decision) — the tracker intake side
is now ready to receive preset-carrying receipts.
