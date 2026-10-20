# Handoff — Protocol Architect, 2026-10-20 (ADR-045-routed contract design)

## 1. Changed files and why

Design returned read-only; the Integrator applied it:

- `protocol/msp-cooperative-v1.md` — three edits from the architect's
  design, plus one Integrator adjustment:
  - §2 field table: the `preset` type cell now cross-references the §6
    supersession (semantics superseded by ADR-045).
  - §4: the ADR-045 P2 receipt-population contract paragraph
    (SessionOffer.preset → ReceiptV2.preset, verbatim-copy rule,
    `unspecified` on unnegotiated paths, the requester countersign gate).
  - §6: full supersession replacement — ADR-045 semantics table, the
    implementer notes, and the fail-closed compatibility paragraph.
  - §7: the two P2 vector files with their pinning assertions.
  - **Integrator adjustment:** the architect's table and notes were
    drafted before the owner's S-7 resolution (commit 3c97649) and
    carried the old Maximum line plus a "S-7 pending re-approval" note;
    the committed §6 carries the owner-approved Option A wording ("the
    receipt records it — signed and tamper-evident") and no pending note.

## 2. Design decisions of record

1. **Reuse the existing `SessionOffer.preset` — zero new wire fields.**
   The field is required and frozen since ADR-029
   (`crates/modelswarm-session/src/negotiation.rs:196`); adding an
   `execution_preset` beside it would be exactly the T/R B1 collision.
   The B1/B2 traps are resolved by supersession + the existing slot.
2. **Receipt population (§4 contract):** serving peer writes the
   negotiated value verbatim; `unspecified` stays the honest literal on
   every unnegotiated path (including the whole msp-v1 §6 pull — receipts
   there can never carry a tier under this contract; extending that needs
   its own ADR — owner-visible scope fact).
3. **Countersign gate (INV-4, new contract text derived from ADR-045's
   "ONLY on paths where…" sentence):** the requester refuses to
   counter-sign any receipt whose preset mismatches the session's
   negotiation — a dishonest serving peer cannot monetize a false tier
   claim. Flagged for Security confirmation in the P2 review.
4. **Deploy order:** spec first (no wire-compat event — the five strings
   already parse everywhere), tracker intake enum BEFORE any
   preset-carrying receipt can be POSTed, peer behavior last behind the
   P2 gates; mixed-fleet windows are bounded fail-closed (offer schema
   failure, `preset_unavailable` reject, INV-4 refusal).

## 3. Commands and evidence

Design was produced read-only (Read/Grep over the protocol docs, the
session/negotiation/eligibility/node sources, the tracker TS parsers, and
the existing vectors; key anchors: `receipt_wire.rs:100` `unspecified`,
`negotiation.rs:196` existing required preset, `serve.rs:686-731/843`
fail-closed intake + echo, `schemas.ts:130` the `z.string()` gap). The
committed edits are documentation-only — no code compiled, no tests run
(spec text; the executable evidence lands with the P2 vectors and the
Tracker Engineer's intake amendment).

## 4. Assumptions

ADR-045 revision 2 is the operative authority; the cooperative-namespace
edits ride ADR-045's acceptance without a new ADR; msp-v1.md stays
byte-identical (verified).

## 5. Unresolved risks

- INV-4 awaits Security confirmation (folded into the next P2 review).
- Stale ADR-029 semantics still live in Rust doc comments
  (`negotiation.rs:71-79`: Deep "approximate — never lossless") and in
  `verifydrafts/mod.rs:424-426` (`default_supported_presets()` excludes
  Deep citing the retired justification) — the P2 implementation must
  refresh both, and whether Deep joins the default served set is a
  conscious P2 decision, not an inheritance.
- Legacy `"FAST"` (uppercase) in `apps/tracker/tests/granted-accounting.test.ts:34`
  must move to the frozen vocabulary with the intake amendment.

## 6. Suggested next tasks for the Integrator

1. Tracker Engineer: the S-5 intake amendment (zod enum + `.strict()` on
   the inner receipt object + the `granted-accounting` fixture fix + an
   out-of-vocabulary rejection test) — dispatched.
2. P2 implementation arc: the two vector files with dual-language tests,
   the ReceiptContext plumbing, the Rust doc-comment refresh, the
   `default_supported_presets` decision, then the engagement seam behind
   the pre-registered protocol
   (`experiments/reports/P2-BALANCED-FALLBACK-PROTOCOL.md`).
