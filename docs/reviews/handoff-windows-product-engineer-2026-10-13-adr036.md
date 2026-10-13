# Handoff — Windows Product Engineer, ADR-036 §3 implementation (2026-10-13)

The ADR-036 five-line order, executed in the Windows Product crates
(`modelswarm-node` + `modelswarm-desktop` + `ui`). Everything below ran
against HEAD `1e371ed` (the Protocol seat's carrier landing) with no
rebase. **No version bump** (the release train owns bumps). No carrier
traffic before the Security + T/R review gate — nothing here is enabled
in production beyond what the ADR directs (serving carriage default-on;
requester election default-off).

## 1. Changed files and why

### Line 1 — SERVING SIDE (node)

- `crates/modelswarm-node/src/verifydrafts/serve.rs`:
  - `ContentCarriage` (`On`/`Off`, `as_str`) — the ADR-036 §3.1
    serving-side posture enum; `VerifyDraftsServeConfig` gains
    `content_carriage` (DEFAULT `On` in `honest()` — the owner
    directive) and `prefill_context_tokens: Option<u64>` (the
    session-level context bound; `None` = the ADR-035 governor verdict
    at hosting start remains the machine-level authority).
  - `read_session_open` rewritten to the §3.2 FIXED order
    `[lease]? → [pin]? → offer`, returning the new `SessionOpen { lease,
    pin: Option<WirePinCarrier>, offer }`. `SessionOpenError` extends by
    `BadPinCarrier`, `PayloadTooLarge` (checked BEFORE parse — the
    transport class), and `OutOfOrder` (pin→lease, repeated lease, a
    carrier after the pin carrier). A pin carrier without a lease is
    valid; an offer without a carrier is a NEUTRAL session — the
    existing path byte-unchanged.
  - `admit_pin_carrier(...)` — the §3.2 admission TABLE,
    first-failure-wins in the ADR's row order: schema (every
    `validate()` fault except the commitment BIND → `BadFrame(label))`
    → signature/sender/remote-PeerId binding (sender-binding-first
    `verify_pin_carrier_signature`; the coordinator key pinned from the
    transport-authenticated PeerId) → carriage OFF → reject
    `content_carriage_refused` → profile → `profile_mismatch` (the G3
    early close at the carrier) → params → `params_mismatch` → the
    commitment BIND → `pin_mismatch` → prefill cap →
    `budget_infeasible` → the clamped `[1000,120000]` deadline →
    `DeadlineExceeded`. Returns `AdmittedPinCarrier { session_id,
    prompt_tokens, seq, nonce, sender, deadline_expires_at_ms }`
    (content — never logged; only its COUNT may reach telemetry).
  - `serve_verifydrafts_session_with_pin(...)`: the carrier-session
    serve loop — the offer's `session_id` MUST equal the carrier's
    (mismatch `bad_frame`), and the gate is SEEDED via the existing
    public `VerifyDraftsGate::seed_pre_state` with the carrier's seq
    baseline (`last_seq[sender] = seq` — every later `VerifyDrafts`
    strictly greater) and its nonce (`consumed_nonces` — reuse is
    `replayed_request`). Rounds ≥ 1 already hold by construction
    (round 0 is `round_mismatch` at stage 8). The old 4-arg
    `serve_verifydrafts_session` stays as the neutral delegate —
    existing callers/tests untouched in behavior.
- `crates/modelswarm-node/src/verifydrafts/composed.rs`: the family arm
  now reads the full open exchange, runs the lease gate (still FIRST —
  the wire order's disclosure-minimizing rationale), then the carrier
  admission BEFORE any engine work; `PinAdmission::Rejected` sends the
  honest `SessionReject` (codes only — the carrier's session id +
  protocol version echoed, never content) and ends the session;
  admitted carriers get the deadline RE-CHECK immediately before
  prefill dispatch, then `FamilyEngineSource::engine_for_root(Some(carrier
  prompt_tokens))` — the per-session prefilled engine. Telemetry:
  `verifydrafts.pin_carrier_admitted` (peer, token COUNT, seq — counts
  only) / `verifydrafts.pin_carrier_refused` (peer, reason label). The
  pin commitment NEVER appears in any event.
- `crates/modelswarm-node/src/verifydrafts/engine_source.rs`:
  `engine_for_root(Option<&[u32]>)` — the F2 seam's ADR-036 form.
  `None` keeps the ADR-034 neutral pin; `Some(tokens)` constructs a
  FRESH per-session engine with the conversation as the prefilled root
  (Capi — KV residency bounded by the session: the returned handle's
  drop tears it down). `engine_for()` delegates with `None`.
- `crates/modelswarm-node/src/verifydrafts/mod.rs`: re-exports
  (`WirePinCarrier`, `pin_commitment_of`, `sign_pin_carrier`,
  `admit_pin_carrier`, `ContentCarriage`, `PinAdmission`,
  `AdmittedPinCarrier`, `SessionOpen`, `serve_verifydrafts_session_with_pin`).
- `crates/modelswarm-node/src/lib.rs`: `runtime_detokenize` (the
  elected path's decode seam — the committed stream is the
  coordinator's OWN data) + `render_chatml` re-export (the ADR-025
  application point shared with the serving executor — same rendering,
  same tokenizer, so the carrier root is the single-serving path's
  tokenization).

### Line 2 — COORDINATOR SIDE (node client)

- `crates/modelswarm-node/src/verifydrafts/client.rs`:
  `VerifyDraftsClientConfig` gains `carrier_prompt_tokens:
  Option<Vec<u32>>`. When set, `negotiate` sends lease → the SIGNED pin
  carrier (seq 1 baseline, round 0, nonce `new_nonce()`,
  `pin_commitment_of` auto-computed, `sender` = the identity's
  installation id, `sign_pin_carrier`) → offer, and opens the client's
  `next_seq` at 2 (every later `VerifyDrafts` strictly greater). A
  carrier form over the 256 KiB frame limit fails negotiate honestly
  (`Misconfigured` with "truncating to fit is forbidden") — the caller
  surfaces its `blocked_content_contract` label. The new reject reasons
  ride the EXISTING `ClientOpenError::Rejected(reason.as_str())` —
  the fallback-to-single contract, unchanged.

### Line 3 — SERVING-DESKTOP

- `crates/modelswarm-desktop/src/app.rs`:
  - Persisted keys: `allow_content_carriage` (serde default TRUE —
    fresh installs and pre-ADR-036 configs read default-on) and the
    IPC `set_content_carriage` (refused while hosting — applies at the
    next hosting start, the pairing-toggle discipline). Flows into
    `compose_family_serving` → `VerifyDraftsServeConfig.content_carriage`.
  - `FamilyStatus.carriage: Option<FamilyCarriageStatus { enabled,
    disclosure }>` — present whenever the family is ON; `disclosure`
    is the §3.1 text VERBATIM (`CARRIAGE_DISCLOSURE_SERVING`) while
    enabled, or the honest opt-out statement (`CARRIAGE_DISCLOSURE_OFF`)
    when not. `get_status` exposes `content_carriage_allowed` and the
    election block; the UI renders the daemon's text verbatim.
- `crates/modelswarm-desktop/ui/index.html`: the Setup "carry others'
  conversations" toggle + note; the always-visible `#carriage-line`
  disclosure under the family line (visible iff family on; verbatim
  from `family.carriage.disclosure`); the Privacy section gains the
  honest serving-peer/family-verified line.

### Line 4 — REQUESTER-DESKTOP

- `crates/modelswarm-desktop/src/app.rs`:
  - Persisted key `family_verified_chat` (serde default FALSE —
    election is the user's choice; only the SERVING side is
    default-on) + IPC `set_family_verified_chat` (flippable any time —
    it routes the next turn). `Inner.last_family_cohort` records the
    ACTUAL cohort N of the most recent elected turn (1 in the shipped
    configuration); `get_status.family_verified_chat` carries
    `{ elected, cohort, disclosure }` — the disclosure rendered by
    `requester_disclosure(N)` (the §3.1 text with N substituted).
  - `try_family_verified_chat(...)` — the elected path, invoked from
    `try_swarm_chat` INSTEAD of the automatic ladder when elected (the
    probe's sibling: user-triggered, does not ride the engage gate).
    Renders the conversation via `render_chatml` + the pinned tokenizer
    (the same tokenization the single path sends), refuses honestly
    (`blocked_content_contract`, detail `carrier_exceeds_frame_limit` /
    unrenderable) when the carrier form exceeds the 256 KiB frame
    bound — TRUNCATION IS FORBIDDEN and never attempted; opens the
    carrier session (open-phase deadline 20 s — the serving side
    prefills the whole conversation at engine construction before the
    accept; the 2.5 s round default would refuse real conversations);
    drives the lossless `speculative_exact` loop (this machine's engine
    drafts greedy under the pinned params → the cache-cold LOCAL
    self-check gates every window (one self-check engine prefilled
    with the conversation root via `engine_for_root`) → the peer
    verifies → commit; EOS ends the turn); detokenizes the committed
    stream; streams the reply; drops the self-check engine
    (teardown-bounded). The mode label states "slower by design";
    `family_engage { engaged: true, reason: "elected", cohort }` rides
    the reply; `family.engage` telemetry is counts/labels only. EVERY
    failure (no local engine, unrenderable/oversized, refused open —
    including `content_carriage_refused`/`pin_mismatch`/
    `profile_mismatch`/`params_mismatch`/`budget_infeasible`, dial,
    local divergence, detokenize) falls back to the plain swarm path —
    the mandatory fallback, labeled, never silent.
  - `family_engage_reason`: behavior for the automatic path unchanged
    (the frozen ladder); the contract comment + terminal-arm comment
    now state the §3.3 NARROWED meaning (fires when the carrier path is
    not elected or not locally available — never a blanket content
    block).
- `crates/modelswarm-desktop/ui/index.html`: the chat-surface election
  toggle (`#verified-chat-switch`, default OFF) + the always-visible
  `#verified-chat-line` §3.1 requester disclosure while elected (N from
  the daemon); the reply meta names the cohort for engaged replies
  ("family: verified by cohort N (slower by design)").

### Line 5 — HYGIENE + TESTS + DOCS

- `crates/modelswarm-node/tests/verifydrafts_pin_carrier.rs` (NEW, 6
  tests, feature `verifydrafts`): golden-vector REPLAY of all three
  `protocol/vectors/pin-carrier-*.json` through `admit_pin_carrier`
  (accept incl. the strict-parse canonical round-trip; mismatch →
  `pin_mismatch`; every malformed row's pinned label incl. the two
  admission-path arms carriage-off → `content_carriage_refused` and
  context-overflow → `budget_infeasible`); the first-failure order
  swept pairwise (schema < signature < carriage < profile < params <
  pin < prefill < deadline); the fixed open-sequence order (all four
  valid shapes + the three deviation classes + the oversize
  payload_too_large-before-parse); a full carrier session over the
  REAL staged transport (first round seq 2 / round 1; a later frame
  re-using the carrier's NONCE → `replayed_request`, engine untouched);
  the codes-only reject shape pinned byte-exact + no content-derived
  hash or token array in telemetry; and the teardown bound at the seam
  (the per-session engine handle the serve loop holds is DROPPED at
  session end — a drop-counting wrapper proves it).
- `crates/modelswarm-desktop/src/app.rs` tests (+6): fresh-install
  postures (carriage ON, election OFF, pre-ADR-036 config reads the
  same via serde defaults, round-trips); the §3.1 texts VERBATIM (both
  ends, N substitution, the honesty clauses present);
  `FamilyStatus.carriage` shape (verbatim text while on, honest off
  statement when opted out, null while family off); the opt-out
  refusal end-to-end at unit level (`honest()` defaults On → the
  config flows Off → the node's admission table refuses a
  well-formed SIGNED carrier with `content_carriage_refused`); the
  narrowed `blocked_content_contract` (ladder frozen, gate-would-engage
  arm still labeled); and the carrier-fields redaction pin (the status
  surface NEVER carries `prompt_tokens`/`pin_commitment`/arrays; the
  carriage object is exactly `{enabled, disclosure}`; the neutral
  `pin.pin_hash` stays allowed per ADR-034 rule 7 — the §4.4
  non-transfer to content-derived commitments respected).
- `crates/modelswarm-desktop/ui-harness/adr036-carriage.html` (NEW):
  the rendered-UI harness for both disclosures + both toggles at
  1152×648 (the standing rule), five scenarios (default on / opted out
  / hosting off / elected N=1 / elected N=3), self-checking (title
  "ALL PASS").
- `crates/modelswarm-node/tests/verifydrafts_wiring.rs`: the two
  `read_session_open` call sites adapted to the `SessionOpen` return
  (behavior identical; the presented-lease arms additionally assert
  `open.pin.is_none()`).
- `docs/privacy.md` §2 (product docs are my owned path; ADR-036 §6
  discharges the paragraph for this mode): the carrier recipient set
  recorded (every cohort member of the elected session, bounded by the
  preset cohort cap and `budget.max_endpoints` — 1 shipped — plus the
  requester's own machine; token-id form; slower by design; counts-only
  telemetry), and the gated paragraph narrowed to the REMAINING
  cooperative modes.

## 2. Exact commands and outcomes

- `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean;
  ditto for `--features modelswarm-node/libp2p-backend`,
  `modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`,
  `modelswarm-runtime/capi-adapter`,
  `modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend`,
  and `-p modelswarm-desktop --all-targets --features tauri-shell`.
- `cargo test --workspace` — **526 / 0 / 6** (default baseline EXACT:
  the new tests are feature-gated).
- `cargo test --workspace --features
  modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend` —
  **589 / 0 / 7** (includes the 6 new pin-carrier tests).
- `cargo test --workspace --features modelswarm-node/libp2p-backend` —
  **549 / 0 / 7** (one transient `modelswarm-winjob`
  `close_all_retained_jobs_kills_every_supervised_child` flake on the
  first run — pre-existing, machine-state-dependent (left 4 vs right 2
  process counts), winjob untouched by this diff; it passed on the
  immediate re-run, 3/3 standalone, and in every other combo).
- `cargo test --workspace --features modelswarm-runtime/capi-adapter` —
  **541 / 0 / 10**.
- `cargo test --workspace --features
  modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend` —
  **608 / 0 / 13**.
- `cargo test -p modelswarm-desktop --features tauri-shell` — **56 /
  0 / 0** (baseline 50 + the 6 new ADR-036 tests).
- UI harness: no browser tool exists in this agent session, so the
  harness logic ran HEADLESSLY (linkedom, real `ui/index.html`, stubbed
  Tauri bridge, 1152×648 stub window): the ADR-036 harness assertions
  **16/16 PASS** (both disclosures verbatim + visibility, both toggles'
  defaults + IPC writes, opt-out honest line, cohort N=1/N=3), and the
  v0.2.23 pairing-chat regression assertions **10/10 PASS** against the
  edited UI. The RENDERED-GEOMETRY leg at 1152×648 needs one manual
  browser pass: serve the crate root (`python -m http.server 4817`) and
  open `ui-harness/adr036-carriage.html` (self-checking; expects ALL
  PASS including the disclosure-line geometry check).

## 3. Test evidence

See §2 — every gate green on this machine; the only failure observed
all day was the pre-existing winjob flake, re-run clean.

## 4. Assumptions

- The lease gate stays FIRST in the composed loop's processing order
  (matching the wire order's rationale); the pin admission runs after
  it. A refused lease therefore never reaches carrier admission.
- `prefill_context_tokens` defaults `None` (no session-level bound is
  known locally today); the ADR-035 governor verdict at hosting start
  (already the desktop's `compose_family_serving` authority) remains
  the machine-level ceiling. Setting the field (e.g. from a future
  runtime context-window API) turns on the `budget_infeasible`
  per-session refusal; the golden vector's context-overflow arm is
  pinned with the cap explicitly supplied.
- The elected path's self-check uses ONE engine per TURN prefilled with
  the conversation root (its KV advances round by round exactly as the
  serving engine's does); it is cache-cold at open by construction —
  the discipline's target is the DRAFTING sidecar's warm cache, which
  is separate.
- The requester disclosure's N is the ACTUAL cohort of the most recent
  elected turn (`last_family_cohort`, 1 shipped); before any elected
  turn it renders the shipped bound (1). It updates after each turn.
- The carrier's open-phase deadline rides `round_deadline_ms` (the
  desktop's elected path sets 20 s for the prefill; the serving side
  clamps [1000, 120000] regardless).
- ADR-034's neutral path, the probe, the lease carrier, the frozen
  msp-v1 surfaces, and the engage gate are untouched; the neutral
  family continues to serve carrier-less sessions through the shared
  neutral engine.

## 5. Unresolved risks / open items for the review gate

1. **Security + T/R review seats are the implementation gate** (ADR-036
   §6): NO carrier traffic before both sign off. Requested focus: the
   refusal path's honesty (labels vs mismatches), the
   commitment-oracle analysis (the serving peer already holds the
   payload — the check is self-consistency), replay/reorder at open,
   the disclosure contract (both ends), the default-on fresh-install
   posture test, and the redaction suite extension.
2. `docs/threat-model.md` §4.2/§5.2 rows (adversary B's scope under
   default-on carriage, the KV-residency row, the consent-UX downgrade)
   are Security-Engineer-owned and NOT edited here — the ADR's §6 texts
   are ready for that seat.
3. The rendered-geometry harness leg (1152×648, the standing rule)
   needs one manual browser pass — no browser tool in this agent
   session; `ui-harness/adr036-carriage.html` is self-checking and
   ready (headless behavioral run: 16/16).
4. The `FamilyEngineSource::Capi` per-session construction means an
   elected turn's serving side builds a SECOND resident engine during
   the session (alongside the neutral family engine and the sidecar).
   The ADR records this as Windows Product + Resource coordination
   (§7): the governor owns admission; a future swap/eviction policy
   between the neutral and carrier engines is the follow-up if memory
   pressure shows up in practice.
5. The first cross-machine carrier chat needs both live machines on a
   build with this change; the probe remains the pairing diagnostic.
   No live-machine rehearsal was run from this session (the review
   gate precedes carrier traffic).

## 6. Suggested next task for the integrator

Route this handoff to the Security Engineer and T/R Engineer for the
ADR-036 §6 implementation review (the required reviewers per the
ownership map), then land the threat-model §4.2/§5.2 rows (their texts
are in ADR-036 §6), and run the one-pass rendered-UI check
(`ui-harness/adr036-carriage.html`). After both seats sign off, carrier
traffic is unblocked; the v0.2.24 train (when opened) carries the
disclosures into release notes verbatim — §3.4's performance-honesty
discipline applies to any copy that mentions the mode.
