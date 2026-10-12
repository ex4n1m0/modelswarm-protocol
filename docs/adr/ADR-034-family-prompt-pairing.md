# ADR-034: The family prompt-pairing contract (deterministic serving-side pin)

Status: **Proposed (2026-10-12) — paste-ready for the Integrator; acceptance
per the ADR log process.** Decides the ADR-032 light-up's recorded G6 gap:
the family's session prompt is pinned out-of-band on the serving desktop as
a token file today, and no end-user path creates it, so two stock installs
cannot pair without manual files. Owner: Protocol Architect (contract);
implementation: Windows Product Engineer (tier 1), reviewed by Security and
Test and Release per the ownership map. This ADR changes NO frozen wire
bytes: `protocol/msp-v1.md`, `protocol/messages.proto`,
`protocol/msp-cooperative-v1.md`, and `crates/modelswarm-session` are
untouched beyond what exists.

## Context

ADR-032's family verifies continuations of a session prompt the wire never
carries: the coordinator chains draft windows on the prefix hash, and the
verifier must already hold the same prompt in engine KV. The light-up pinned
that prompt OUT OF BAND on the serving desktop —
`<data_dir>/family-prompt-tokens.txt`, a strict whitespace-separated
token-id list; absent/junk = unpinned = the family engine stays down
honestly (`family_prompt_unpinned`;
`crates/modelswarm-desktop/src/app.rs`, `compose_family_serving`). The
engine side is done: `FamilyEngineSource::Capi { prompt_tokens }` via
`capi_family_source` commits the pin as the PREFILLED root at engine
construction (`crates/modelswarm-node/src/verifydrafts/engine_source.rs`),
and offer-form v3 (ADR-032 §3/G3) already guarantees that a mismatched
profile or params pairing cannot open a session, so the prefilled KV is
never consumed by a mismatched pairing. The gap is product-side: nothing
creates the file, so the shipped opt-in gates include "create a token file
by hand" — the light-up record's honest disclosure and the recorded
freeze-ADR item (ADR-032 §7; Security `c1fb927` disposition condition 3;
T/R `e4eb851` unresolved risk 2).

## Decision

The family's PRODUCT pairing contract is a DETERMINISTIC PIN, computed by
the serving side, identical on every stock install hosting the same exact
profile.

1. **Who computes the pin: the SERVING desktop, at hosting start** — inside
   the family composition path, before the pin is read and before
   `capi_family_source` constructs the engine. The derivation is local,
   deterministic, and content-free: constants plus the profile's pinned
   tokenizer, nothing else — no user input, no network input, no session
   state.
2. **What it contains: a fixed, profile-pinned, version-recorded NEUTRAL
   prompt; NO user content.** The v1 text is frozen by this ADR and MUST be
   byte-identical in every implementation: `FAMILY_PROMPT_V1` = "You are
   serving a ModelSwarm cooperative verification session. Continue the given
   context exactly as this model normally would." The version tag
   `family-prompt-v1` travels with it in status and telemetry. The text is
   deliberately neutral (no content bias; aligned with the lossless
   contract — the verifier engine's own continuation), short, and safe to
   display.
3. **Tokenization (the determinism rule):** render the v1 text as a
   SYSTEM-ONLY turn through the profile's pinned chat template (ADR-025's
   application point: the catalog's ChatML-family template inputs pinned by
   the manifest; a non-ChatML profile is gated per ADR-025 rule 3 and
   tokenizes the raw text), then tokenize under the profile's pinned
   tokenizer — the C-API adapter's tokenizer or the sidecar's tokenization
   surface, any surface yielding the pinned engine's token ids. Same
   `ModelProfileId` ⇒ same template and tokenizer ⇒ same token ids on every
   install (ADR-011/022 profile immutability is the anchor). The pin
   identity for logging and the future commitment is the bare 64-hex
   `sha256` over the token ids rendered as little-endian u32 each (the
   ADR-032 §1 rule-2 id encoding, with no parent).
4. **Write discipline (the pin location stays the seam):** at hosting
   start, when `family-prompt-tokens.txt` is ABSENT, the serving desktop
   writes the derived ids automatically (atomic write) — this is the
   two-stock-installs case. A PRESENT, strictly parsable file is honored
   as-is and WINS (the documented expert override, including non-v1 pins);
   a present but unparsable file keeps the fail-closed
   `family_prompt_unpinned` behavior — the automatic pin never silently
   overwrites a file the user may own. Version advance: when the app's
   prompt version moves past v1, a present file whose ids EQUAL the
   previous version's derived ids (an unmodified automatic pin) is
   rewritten to the new version; any other present file is left alone and
   surfaced honestly (a cross-version pairing then reports mismatch
   through the rule-6 surfaces rather than silently verifying).
5. **The coordinator binds to the same pin.** A desktop acting as family
   coordinator prefills its drafting context with the same derivation (v1
   text, same template/tokenizer discipline). Today that is the
   `family_probe` pairing confirmation (rule 6); the future chat-integrated
   family flow (the cooperative-plan Windows integration phase,
   deliberately not started) inherits the rule. This contract rules the
   NEUTRAL-ROOT pairing; it does not smuggle user prompts onto the family
   wire — content-blindness and the family's no-prompt-on-wire design
   stand, and arbitrary-user-prompt family sessions require the rule-8
   commitment path or expert-pinned identical files on both ends.
6. **How mismatches surface (honest labels, never silent):**
   (a) UI status — the family line reports the pin provenance
   (`auto family-prompt-v1` | `file`), token count, and pin hash, alongside
   the existing honest off-reasons (`family_prompt_unpinned` unchanged).
   (b) The `test family pairing` probe gains a pairing confirmation: after
   the session opens (presented lease; every existing label unchanged), it
   runs ONE deterministic round — window = the coordinator's greedy
   continuation of the v1 pin under the session's pinned sampling params,
   w = min(8, the accepted `batch_window_max`), drafted by the
   coordinator's own pinned engine (its hosting sidecar; no second engine
   on the coordinator) — and reports `pairing_confirmed` on full
   acceptance, `pin_mismatch` on any rejection (divergence point 0 is the
   canonical signature). Counts only in the result and telemetry — token
   ids never appear (the family's redaction discipline). Honest caveat on
   record: on CROSS-MACHINE venues a divergence can also be E0's open
   cross-machine determinism question, so the label means
   context-or-backend divergence class; the RC-1 audit path independently
   telemeters `verifier_divergence`. The probe stays user-triggered
   (button cadence; rate-limit it if it becomes automatic — the
   lease-earn cadence note stands). (c) In-session mismatches (a
   coordinator that ignored the contract) surface through the existing
   honest mechanisms — acceptance collapse, RC-1 audit divergence with
   fallback to single, round-level rejects — never silent verification
   against a mismatched prompt (the fail-closed posture ADR-032 already
   pins).
7. **Security and privacy note: the pin is a PROTOCOL PARAMETER, not
   content.** The v1 text, version tag, token count, and pin hash MAY be
   logged, versioned, telemetered, and shown in the UI; the
   content-redaction rules (privacy rule 5) bind session
   prompts/completions, not ADR-defined constants derivable from public
   text. The pin introduces no new oracle (confirming a public constant
   discloses nothing — the same analysis as the G3 offer-field oracle) and
   no new trust input (verifier authority is unchanged; a wrong pin
   degrades acceptance economics, never exactness — I1/I2 untouched).
8. **Reserved wire vocabulary (owner-gated; NOT authorized to land by this
   ADR):** a negotiation-time pin commitment — a session-open carrier
   following the `WireSessionLease` precedent, plus a `pin_mismatch`
   `SessionReject` reason (negotiation-level vocabulary, the G3 class) —
   is the recorded future path for making mismatches a session-open reject
   instead of a first-round observation. It edits the negotiation
   vocabulary and therefore rides its own ADR and gate when product
   evidence or a second consumer demands it; nothing in tier 1 (rules
   1–7) depends on it.

## Resource note (recorded consequence)

Removing the manual file removes the de-facto opt-in the light-up record
relied on ("ordinary hosting users pay nothing until they opt into family
verification"): with the gates held (`MSP_LISTENER=1` + C-API bundle +
hosting on), the family engine now inits on stock installs, and the
RAM/VRAM-doubling posture (sidecar + in-process C-API copy vs the
governor's 70% ceiling, gate D11) becomes the default posture for exactly
those installs. The Resource Engineer coordination stays the recorded open
item; Windows Product MAY keep an explicit family-hosting toggle as the
visible opt-in (permitted, not required); the UI must keep surfacing the
family engine state honestly either way.

## Consequences

+ Two stock installs hosting the same exact profile pair for probe-class
  family sessions with zero manual files; the expert seam (the file)
  survives with precedence; nothing on the frozen wire changes; the
  session crate is untouched (tier 1 lives in `modelswarm-desktop` plus
  the existing node seams).
+ Pairing becomes diagnosable from the shipped UI (provenance + one-round
  confirmation) instead of discoverable only through acceptance collapse.
− One more versioned constant to maintain: a prompt-version bump is a
  pairing-visible event and must ride the rule-4 rewrite discipline.
− The probe's confirmation round consumes one admitted round on the
  serving peer (bounded: w ≤ 8, one round, user-triggered), and the
  resource posture in the note above makes the governor coordination
  binding rather than optional.

## Review and gate requirements (tier 1, Windows Product)

Tests owed with the implementation: determinism (same text + pinned
tokenizer ⇒ identical ids and pin hash across runs); auto-write-on-absent;
junk-file stays fail-closed unpinned; present-file precedence; the
version-advance rewrite rule (unmodified auto pin rewritten; expert file
untouched); probe label stability including NO token ids in results or
telemetry; status provenance fields. Gates per AGENTS.md: `cargo fmt
--all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, plus the tauri-shell feature runs; Security and
Test and Release review the handoff. No CI-schema or wire-compat impact
(no `protocol/` or session-crate bytes change).

Evidence base (all committed): ADR-032 revisions 4–5 (§2 carriers and
one-vocabulary discipline, §3 G3, §7 light-up ledger incl. the G6 record)
· the serving seam `crates/modelswarm-node/src/verifydrafts/engine_source.rs`
(module docs: the recorded gap; `FamilyEngineSource::Capi { prompt_tokens
}`; `capi_family_source`) · `crates/modelswarm-desktop/src/app.rs`
(`family_prompt_tokens` strict parser, `compose_family_serving`,
`FamilyStatus`, `family_probe`) · the light-up handoffs
`docs/reviews/handoff-windows-product-lightup-2026-10-12.md` (assumption
2, G6) and
`docs/reviews/handoff-windows-product-family-lease-2026-10-12.md` (§5: the
out-of-band prompt file as the product boundary) · the disposition reviews
`c1fb927` (condition 3; G5/G6 hold) and `e4eb851` (§1 G6 fold; unresolved
risk 2) · ADR-025 (template application point), ADR-011/022 (profile
immutability), ADR-026 (the lease gate the carrier rides), ADR-031 (the
C-API adapter), E0 determinism.
