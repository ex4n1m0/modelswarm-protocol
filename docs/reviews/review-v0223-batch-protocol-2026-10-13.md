# Review — v0.2.23 desktop batch, protocol seat (2026-10-13)

Reviewer: Protocol Architect (read-only cross-path review requested by the
Integrator). Scope: the protocol-adjacent surface of the v0.2.23 desktop
batch, feature commit `3a0bb609d0b4a0c141521e2cdbe770a924a1c193`
("feat(desktop+node) v0.2.23 batch"), integrated at HEAD
`f85347d31620dc94fd16db473931d6275342aa45` (verified via `.git/logs/HEAD`
and `.git/refs/heads/main`; chain `95cb6a4` → `3a0bb60` → `8a6f772` →
`f85347d`). Implementation handoff of record:
`docs/reviews/handoff-windows-product-engineer-2026-10-13-v0223.md`
(Integrator-executed gates: fmt/clippy clean, workspace 495/0, desktop
tauri-shell 50/0, ui-harness 14/14).

Reviewed surfaces only: (a) the chat auto-engage reason ladder and the
coordinator capability cache in `crates/modelswarm-desktop/src/app.rs`;
(b) the desktop-facing re-exports in
`crates/modelswarm-node/src/verifydrafts/mod.rs`; (c) the coordinator
self-check labels. Everything else in the batch (settings toggle, RAM
verdict, chat controls, updater, picker, staging) is out of this seat's
scope.

## Verdict

**APPROVE WITH REQUIRED CHANGES** — 0 blockers, 2 required changes
(documentation/telemetry-record level; no wire bytes, no schema, no code
behavior change demanded), 5 advisories. No frozen surface is touched:
`protocol/msp-v1.md`, `protocol/messages.proto`,
`protocol/msp-cooperative-v1.md`, and `crates/modelswarm-session` are
byte-identical to their pre-batch state; the checkpoint never dials, never
opens a session, and emits labels/counts only.

## (a) The chat auto-engage ladder and the capability cache

Code: `crates/modelswarm-desktop/src/app.rs:3495-3520` (boundary comment),
`3525-3560` (`family_engage_reason`), `3562-3620`
(`family_engage_checkpoint`), `3752-3759` (call site),
`4045-4056` (reply surface), `5873-5924` (stability test).

### The seam question — CONFIRMED CORRECT, no wire field needed today

`FamilyPeerCap` (`app.rs:217-232`, cache at `app.rs:120`) is filled from
the family `SessionAccept` engine disclosure (`app.rs:2838-2857`, reading
`client.engine_disclosure()`). This is the right seam, on the record:

- The ADR-032 §3 disclosure (`engine { backend, batch_verify,
  batch_window_max }`) is a REQUIRED field of the extended canonical
  `SessionAccept` (revision 3 / F-6) — it is the only capability surface
  the family wire has, and it is learned behind the ADR-026 lease gate
  from an admitted session, after which measured behavior
  (`verify_ms_lie`, acceptance collapse, de-ranking) polices the claim
  (ADR-032 §3 "What misreporting can and cannot do").
- The tracker roster genuinely carries nothing: `PeerEntry`
  (`crates/modelswarm-tracker-api/src/lib.rs:140-154`) has peerId,
  addresses, queueMs, freeSlots, leaseExpiresAt, lastSeenAt, capacityClass
  — and `capacityClass` is already a self-reported honest label (msp-v1 §5
  changelog note), the exact untrusted-self-report class a capability
  column would live in.
- ADR-032 §3 already rules the alternative out of scope: "A tracker-side
  backend column for cohort pre-filtering is a possible future ADR, not
  this one."

`capability_unknown` is therefore the honest default, not a gap. If a
wire/roster field ever becomes warranted (e.g., to avoid spending a
probe round per peer), the proper path is: an ADR extending the tracker
roster surface (ADR-028 successor) with a peer-asserted, explicitly
untrusted pre-filter column — never an admission authority, never
touching the frozen `EligibilityLease` signed shape — plus the
capability-token/lease interplay reviewed by Security. Nothing in today's
product needs it: engagement is protocol-blocked regardless, and the
probe rides the family-on-by-default tier.

### Labels — stable and sufficient; treat as frozen starting NOW

The seven labels (`capability_unknown`, `capability_false`,
`backend_mismatch`, `gate_unmeasured`, `gate_no_win`,
`blocked_content_contract`, `gate_misconfigured`) are `&'static str`,
pinned by `family_engage_reason_ladder_is_stable`
(`app.rs:5875-5924`) including the case-insensitive backend compare. They
are sufficient to classify every reachable non-engagement. Important
consequence: v0.2.23 has shipped to testers via the self-updater, so
these strings are de-facto frozen telemetry vocabulary — renaming them
now would be a telemetry-compat break. They must be recorded (R1), not
re-bikeshedded.

Two naming observations, both resolve to "document, do not rename":

- `backend_mismatch` deliberately echoes the frozen `SessionReject`
  reason (ADR-032 §3) while `capability_false` does NOT echo that
  vocabulary's `batch_verify_unsupported`. The semantics coincide in the
  first case and the coordinator-side label set is its own namespace
  (telemetry/UI, never a wire frame), so this is not a collision — but
  the equivalence/intentional-difference should be stated where the
  vocabulary is recorded (R1).
- `blocked_content_contract` accurately names the ADR-034 rule-5 block.

### Priority order — SOUND

Capability facts (unknown → false) precede backend pairing (E0), which
precedes measurement presence, which precedes gate arithmetic, which
precedes the content block; `gate_misconfigured` last. This is correct:
hard, locally-known facts before estimates; the never-worse rule
(`gate_unmeasured` when either prediction is missing) is the honest
outer bound after the hard facts; and the rule-5 block is only evaluated
when the gate would actually engage — matching the test's framing
("even when the gate would engage", `app.rs:5910-5917`).

One decision-relevant consequence (advisory A4, feeds the rule-8 brief):
`family_engage_checkpoint` hardcodes `None` for the swarm-side prediction
(`app.rs:3589-3596`, honestly commented), so in production today every
capability-known turn reports `gate_unmeasured` and
`blocked_content_contract` is unreachable. Honest — the system never
claims a win it cannot predict — but it also means telemetry cannot yet
count "would engage but for the content contract." That number is one of
the inputs the owner would want for the rule-8 decision; the cheap fix is
a probe-sourced family-round EWMA (see the options review,
`docs/reviews/adr034-rule8-options-2026-10-13.md`, option 1).

### Smuggling check — CLEAN

The checkpoint "NEVER dials, NEVER opens a session, NEVER delays the
chat" (`app.rs:3562-3566`) — verified: it takes one brief inner-mutex
read, evaluates a pure function, emits one `family.engage` telemetry
event, and returns the label; the plain swarm path serves the turn
regardless, before the first output token (ADR-007 discipline). No wire
frame is constructed, no `protocol/` or `catalog/schema.json` field is
invented, and the `family_engage.{engaged,reason}` reply field
(`app.rs:4045-4056`) plus the UI note
(`crates/modelswarm-desktop/ui/index.html:719-720`) are IPC/UI surfaces,
not protocol surfaces. The gate arithmetic delegates to the scheduler's
frozen public rule (`should_engage_cooperative`,
`crates/modelswarm-scheduler/src/lib.rs:237-246`) rather than
re-implementing it. Nothing smuggles semantics.

## (b) The `verifydrafts` re-exports — APPROPRIATE (facade, keep)

Code: `crates/modelswarm-node/src/verifydrafts/mod.rs:79-94` —
`pub use modelswarm_runtime::{RuntimeError, SamplingParams};`,
`pub use modelswarm_session::negotiation::Backend;` (pre-existing),
`pub use modelswarm_session::negotiation::EngineDisclosure;`,
`pub use modelswarm_session::verifydrafts::BatchVerifyEngine;` — with the
four-line justification comment at `mod.rs:79-82`.

Assessment: this is the composition-crate facade pattern ADR-032 §4
already ruled in ("the composition crate (`modelswarm-node`, which already
depends on both) hosts the impl… moving the trait… into
`modelswarm-runtime` would relocate protocol-surface types out of the
protocol-owned crate"). The re-exports add no wrappers and no semantics —
they are the SAME public types the module already consumes, so there is
no drift surface, and `EngineDisclosure`/`BatchVerifyEngine` remain
protocol-owned in `modelswarm-session`. Consumers verified: non-test
desktop code names `Backend` (`app.rs:2744-2745`) and `SamplingParams`
(`app.rs:3054`); the desktop's scripted test engine names all four
(`app.rs:5752-5770`). The whole module is behind the `verifydrafts`
feature (`crates/modelswarm-node/src/lib.rs:60-61`), so the facade is
feature-gated transitively. No wire-compat impact (no schema change;
`messages.proto` untouched).

Advisory A5: keep the facade a closed set. If a second consumer appears
or the set grows, revisit (direct crate deps or a dedicated
`verifydrafts::facade` doc note) so `modelswarm-node` does not
accidentally become the de-facto public path for session-crate types.

## (c) The coordinator self-check — CONSISTENT with ADR-034 rule 6b

Code: `crates/modelswarm-desktop/src/app.rs:3003-3067`
(`FamilySelfcheck` + `family_selfcheck_window`), `3069-3154`
(`family_probe_round` integration), `2919-2940`
(`pairing_round_failure_label`), `2950-3001`
(`pairing_result_fields`/`log_family_pairing`), tests `5744-5866`.

ADR-034 rule 6b pins: "Counts only in the result and telemetry — token
ids never appear." Compliance verified:

- `local_selfcheck` emits `window` (length), `accepted`, `divergence`
  (mirrored to `accepted`, matching the ADR-032 §1 invariant
  `divergence_point == accepted_prefix_len`), no detail — counts only.
- `selfcheck_unavailable` emits the window length and a detail string.
- The fail-closed posture is correct protocol behavior and strengthens,
  not changes, the wire contract: a locally divergent or uncheckable
  window NEVER reaches `client.round` (`app.rs:3110-3130`), so no
  unverified draft crosses the wire — a coordinator-side extension of the
  RC-1/§3 cache-cold discipline (fresh engine per check, cited at
  `app.rs:3012-3016`), operating entirely on the coordinator's own
  output. No wire semantics are altered.
- The standing guard is real: the telemetry test asserts no arrays can
  ride the pairing event (`app.rs:5851-5852`).

One soft spot (advisory A3): the `Unavailable(why)` detail embeds
lower-layer `Display` strings ("engine load: …", "engine verify: …",
`app.rs:3043-3049, 3065`). These are bounded error texts, not content,
and the same precedent already exists on the probe's error paths
(`app.rs:2765, 2893`); the no-arrays assertion plus the redaction
framework bound the risk. Before any wider-than-tester distribution,
map engine errors to stable subclass labels so the detail field is
label-vocabulary, not free text.

The 2026-10-13 additions (`local_selfcheck`, `selfcheck_unavailable`,
`pairing_detail` + the ten failure labels at `app.rs:2928-2939`, and
`pin_unpinned`/`draft_unavailable` at `app.rs:3102-3106`) extend the
ADR-034 rule-6 label set after that ADR was drafted. ADR-034 is still
status **Proposed** — so the fold is cheap and pre-acceptance, which is
R1 below.

## Required changes

- **R1 — record the label vocabularies in ADR-034 before/at acceptance**
  (protocol-seat-owned text; the authority for the Windows-Product-owned
  code paths per the ownership map). Add to ADR-034 rule 6 (or a revision
  note in the same ADR) the complete stable label inventory the shipped
  tier now emits: (i) the probe/pairing vocabulary —
  `pairing_confirmed | pin_mismatch | pin_unpinned | draft_unavailable |
  local_selfcheck | selfcheck_unavailable` plus the `pairing_detail`
  labels `rejected | cancelled | violation | torn_down | dropped |
  timed_out | window_too_large | session_torn_down | unparsable_result |
  io`; (ii) the chat engage ladder — `capability_unknown |
  capability_false | backend_mismatch | gate_unmeasured | gate_no_win |
  blocked_content_contract | gate_misconfigured` — with its pinned
  priority order, the event names (`family.probe.pairing`,
  `family.engage`), the note that `backend_mismatch` deliberately echoes
  the frozen `SessionReject` reason while coordinator-side labels are
  their own telemetry namespace, and the counts-only rule binding all of
  them. Without this, the code comments' "the pinned vocabulary" claim
  (`app.rs:3520, 3523`) is pinned only in code and a unit test — below
  this repo's own discipline for stable label sets (cf. ADR-032's
  decided reject strings; ADR-034's off-reason list).
- **R2 — record the margin actually used, and fix the stale margin
  comment.** The scheduler's frozen rule says "the margin actually used
  must be recorded in every run record"
  (`crates/modelswarm-scheduler/src/lib.rs:235-236`). The ladder passes
  `DEFAULT_CONFIDENCE_MARGIN` = **0.15** (`app.rs:3551`; constant at
  `crates/modelswarm-scheduler/src/lib.rs:26`), but (i) the
  `family.engage` event fields (`app.rs:3597-3616`) omit it, and (ii) the
  boundary comment says "margin ≥ 0.05" (`app.rs:3506-3508`), which is
  the reject floor (`MIN_CONFIDENCE_MARGIN`), not the value used — the
  handoff repeats the same wording. Required: add a `margin` field to the
  `family.engage` telemetry rendered from
  `modelswarm_scheduler::DEFAULT_CONFIDENCE_MARGIN` (not a string
  literal, so it cannot drift), and correct the `app.rs:3506-3508`
  comment to "DEFAULT_CONFIDENCE_MARGIN (0.15; 0.05 is the reject
  floor)". Windows-Product executes; R1's ADR-034 fold records this
  review as the cross-boundary authority.

## Advisories (non-blocking)

- **A1** `app.rs:3538` — `own_backend.unwrap_or("cpu")` silently defaults
  an unknown local backend to cpu; on a degraded hosting state against a
  vulkan cap this mislabels the reason as `backend_mismatch` (fail-closed
  either way — engagement never fires on it). Consider an
  `own_backend_unknown` label or an early honest return.
- **A2** `app.rs:229-231` — `FamilyPeerCap` records `learned_unix_ms`
  and telemeters `cap_age_ms` (`app.rs:3610-3615`) but defines no
  staleness rule; the doc comment defers to "stale caps re-learn through
  the next probe." Harmless while engagement is blocked; define a TTL (or
  re-probe trigger) before any rule-8 lift.
- **A3** `app.rs:3043-3049, 3065` — free-form engine-error text in the
  `selfcheck_unavailable` detail (see (c) above).
- **A4** `app.rs:3589-3596` — with the swarm-side prediction hardcoded
  `None`, `blocked_content_contract` is unreachable in production today;
  every capability-known turn reads `gate_unmeasured`. Honest, but the
  would-engage-but-blocked count the owner needs for the rule-8 decision
  is unobservable until a family-round EWMA exists (probe-sourced EWMA is
  the no-ADR path; see the options review).
- **A5** `crates/modelswarm-node/src/verifydrafts/mod.rs:79-94` — keep
  the re-export facade a closed, documented set (see (b) above).
- **A6** `app.rs:2848-2856` — the capability cache is keyed by peer id
  only, implicitly assuming engine geometry is profile-independent per
  peer. True for the single-profile desktop of today; key by
  `(peer_id, profile_id)` if multi-profile hosting ever lands, before a
  rule-8 lift makes the cache decision-bearing.

## Evidence

- Inspected (read-only): `AGENTS.md`; `docs/adr/ADR-032` (revisions
  1-5), `docs/adr/ADR-034`; `protocol/msp-v1.md` (§5/§6/changelog),
  `protocol/msp-cooperative-v1.md`; `docs/threat-model.md`;
  `docs/privacy.md`; `crates/modelswarm-desktop/src/app.rs` (ladder,
  cache, probe, self-check, reply surface, tests);
  `crates/modelswarm-node/src/verifydrafts/mod.rs` and
  `engine_source.rs`; `crates/modelswarm-node/src/lib.rs`;
  `crates/modelswarm-scheduler/src/lib.rs`;
  `crates/modelswarm-tracker-api/src/lib.rs`;
  `crates/modelswarm-desktop/ui/index.html`;
  `docs/reviews/handoff-windows-product-engineer-2026-10-13-v0223.md`.
- Git state verified by direct reads: HEAD
  `f85347d31620dc94fd16db473931d6275342aa45`; batch commit
  `3a0bb609d0b4a0c141521e2cdbe770a924a1c193` present in the reflog with
  the described content.
- No code was changed and no gates were run by this seat (read-only
  session). Executable evidence of record is the Integrator's re-run in
  the batch handoff (§2 there): fmt clean; clippy workspace +
  desktop-tauri-shell `-D warnings` clean; workspace 495/0; desktop
  tauri-shell 50/0; ui-harness 14/14.

## Sign-off

Protocol seat: **approve with required changes** (R1, R2). R1 is
authorable by this seat once write access is granted; R2 is a two-line
Windows-Product change riding the next batch with this review as
authority. Neither blocks the v0.2.23 train or the shipped release.
