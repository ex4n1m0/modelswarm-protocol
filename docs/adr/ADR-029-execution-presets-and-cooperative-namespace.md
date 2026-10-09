# ADR-029: Execution presets and the msp-cooperative-v1 namespace

Status: Accepted (2026-10-09) · Owner-approved gate D3/D18 (§C.1, §G
architecture-seat amendment 3). Extends the ADR-013 mode registry with a
preset layer; creates `protocol/msp-cooperative-v1.md` as the home for
`SessionOffer` + the preset enum. Sources: `zcode-master-prompt.md` §3 ·
`docs/reviews/gap-engineering-study-2026-10-09.md` §2 ·
`docs/reviews/handoff-protocol-architect-2026-10-09.md` §6.

## Context

ADR-013 registers five primitive modes but no composition rules — HYBRID is
named in the mission and ungoverned. No mode/preset field exists anywhere on
the wire; the only mode-ish surface is the tracker's
`session-authorize.mode` (unvalidated string, no Rust caller). Presets are
protocol design, not a UI preference (owner clarification, gap study §2);
two major AI vendors independently converged on named tiers in 2025–26, and
the documented failure mode is cost opacity — tiers must carry their
cost/loss cliff visibly. `SessionOffer` is proposed but has no namespace;
meanwhile the shipped `SpecMessage` cooperative vocabulary
(`modelswarm-session/src/spec.rs:206-339`) has no spec home — AGENTS.md
already forbids invented fields outside `protocol/msp-cooperative-v1.md`
while that file does not exist.

## Decision

### 1. The five presets (frozen table; extends the ADR-013 registry layer)

| Preset | Primitive modes allowed | Cohort cap | Correctness label | Extra disclosure |
|---|---|---|---|---|
| `fast` | `single` | 1 | lossless (the baseline) | none beyond today's |
| `balanced` | `single`, `hedged` (honesty-patched) | 3 | lossless | recipient set = raced peers |
| `deep` | `balanced` + `best_of_n` + deliberation roles | per budget | approximate — never lossless | recipient set + roles + compute budget |
| `verified` | `balanced` + `audited` chain | per budget | detection-oriented; carries audit proofs | auditors see canary-class tasks |
| `maximum` | HYBRID = composition of individually-gated modes only | per budget | union of component labels | full composition + budgets |

Rules (decision-shaped):

- `fast` is the default and live immediately; the others light up exactly
  when their component modes pass their existing gates (M10 build order
  item 11). A preset whose components are not yet gated SHALL refuse with
  an honest reason, not partially enable.
- HYBRID is **only** expressible as `maximum`, and `maximum` composes
  individually-gated modes only — this is the composition rule ADR-013
  lacked. Arbitrary mixing stays forbidden.
- No approximate mode is ever labeled lossless (`deep` carries "approximate
  — never lossless" as its contract); `maximum`'s label is the union of its
  components' labels.
- The planner still chooses the **smallest useful cohort inside the preset
  envelope** (master prompt §11 unchanged); presets constrain the search
  space, they do not override the optimizer.

### 2. Negotiation and recording discipline

- The **preset is the field negotiated at session open** — it rides
  `SessionOffer{preset, budget}` defined in
  `protocol/msp-cooperative-v1.md` (created by this ADR).
- **Primitive-mode resolution stays planner-internal**; no primitive-mode
  field is added to any msp-v1 message. The receipt records preset +
  resolved modes + budgets (ADR-030 fields).
- Each preset **declares its compute budget and correctness label on the
  wire** at negotiation — no hidden costs (the OpenAI cost-opacity failure).
  Budget numbers are **initial values, tunable, R0.5+
  measurement-informed** (see the spec file's initial caps); the tier set
  itself is frozen.
- The **wire enum lands only with `SessionOffer`** — additive, frozen like
  the rest of msp-v1 once shipped, cross-language golden vector required
  before any second implementation reads it.

### 3. Recipient-set disclosure (privacy touchpoint)

`docs/privacy.md` requires "the exact recipient set per mode" before any
cooperative request is served. Under presets the disclosure is stated **per
preset** (five human-readable tiers), not per primitive mode, per the Extra
disclosure column above: `fast` = today's single-peer disclosure;
`balanced` = the raced peer set; `deep` adds role assignment + budget;
`verified` discloses that auditors see canary-class tasks; `maximum`
discloses the full composition + budgets. `docs/privacy.md` wording updates
when each tier lights up — release-blocking per its own gate.

### 4. `protocol/msp-cooperative-v1.md` — the namespace home

Created by this ADR (before `SpecMessage` grows any further): defines
`SessionOffer{preset, budget}` + `SessionAccept`, the mode-resolution
recording rule, and the frozen-namespace constraints
(`protocol/msp-v1.md` and `protocol/messages.proto` stay byte-identical;
what would be breaking — preset fields on frozen `InferenceRequest`/
`Handshake`, in-place §7 receipt overwrites — is stated and forbidden
there). The shipped `SpecMessage` vocabulary remains Rust-implementation
detail until that namespace's own ADR freezes it; its known gaps
(`PrefixCommit` signed payload omits protocol version/nonce/deadline) are
recorded for that future ADR, not fixed here.

## Consequences

+ HYBRID gains its missing composition rules; consent granularity collapses
  from thirteen protocol modes to five readable tiers.
+ The dangling AGENTS.md reference to `msp-cooperative-v1.md` resolves.
+ FAST ships now with zero wire change — the enum is inert until
  `SessionOffer` exists.
− Budget numbers will need one measured retune after 9.6 pass 1 (by
  design — the tier set is frozen, the values are not).
