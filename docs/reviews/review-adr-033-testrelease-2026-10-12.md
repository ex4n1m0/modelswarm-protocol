# Review: ADR-033 advisory plane — Test and Release Engineer §9.2 (implementation gate)

Reviewer: Test and Release Engineer (release seat, independent validation).
Routed per ADR-033 §9.2. Date: 2026-10-12 · Verdict: **APPROVE WITH
REQUIRED CHANGES**. Reviewed object: `docs/adr/ADR-033-advisory-plane.md`
(status **Accepted — option B**, accepted at `be4199a`; reviewed at main @
`c865877`). Complements, does not duplicate, the Security §9.1 review
(`86c6c77`, 0 blockers / 11 required changes / 5 advisories): that seat owns
the redaction/injection surface; this seat owns testability, execution
venues, the release surface, and the promotion gates. Where their findings
and mine touch (RC-2 vocabulary artifact, RC-6 outcome measurement, AD-2
log forging), I cross-reference rather than re-rule.

Scope note: DESIGN review of a docs-only state — no implementation exists
(the ADR's own framing: §9.1/§9.2 "remain the implementation gate").
Nothing here reopens the owner's option-B decision. The verdict gates the
START of implementation: the required changes below are contract-text
deltas the Protocol Architect must record as an ADR-033 amendment (its own
amendment-ready style) before the first advisory-client commit lands,
alongside Security's RC-1…RC-11.

Grounding read: ADR-033 in full; the Security §9.1 review
(`docs/reviews/review-adr-033-security-2026-10-12.md`); the shipped shadow
join (`crates/modelswarm-bench/src/shadow_join.rs` — LEFT-join discipline,
sentinel mapping, stats contract) and its four in-crate tests; the
shadow/scheduler tunables (`crates/modelswarm-scheduler/src/shadow.rs:79-85`
— the env-var knobs with no declared ranges); the bench harness set
(`corpus.rs`, `engage_gate.rs`, `divergence.rs`, `stats.rs`); the shipped
config surface (`crates/modelswarm-desktop/src/app.rs:222-245` reader,
`:617-625` and `:905-913` writers; `crates/modelswarm-node/src/lib.rs:146-201`
— the ADR's cited range, which is `default_data_dir` + the migration list,
NOT a config parser); the telemetry emitter API
(`crates/modelswarm-telemetry/src/lib.rs:163-189`); the CI workflows
(`rust.yml` check + guards jobs, `real-model-e2e.yml` manual dispatch,
`desktop-installer.yml`); the wire harness precedent
(`crates/modelswarm-node/tests/live_tracker.rs`, `MSP_LIVE=1`); the frozen
protocol surfaces (`protocol/msp-v1.md`, `protocol/messages.proto`,
`catalog/schema.json` — grep-clean of any advisory reference); ADR-032 §7
(the B4 execution-venue matrix and its standing statements — the style this
review's §3 follows); `apps/modelswarm-sim/src/mixed_swarm.rs` (pinned
verdict scenarios, the item-4b per-push precedent); `tests/README.md`.

Verdict counts: **0 blockers · 10 required changes · 5 advisories.**

No blocker: the §9.2 enumeration is the RIGHT enumeration — every item
maps to a test class this repo already runs somewhere (property suite,
golden fixture, CI guard step, ignored env-gated test), the four invariants
are testable as stated once the deltas below are recorded, the venue split
(mock-transport per-push vs owner-local provider runs) follows established
precedent, and the wire-compat claim verified clean on spot-check. Every
finding is fixable by recorded contract text without architecture change.
But three of them (TR-RC-1, TR-RC-6, TR-RC-8) are exactly the class that
becomes an untestable or silently-breaking surface at implementation time
when an implementer "makes it work": a dependency check nobody specified
concretely, a promotion bar with no artifact to compute it from, and a
config table that collides with the shipped config writer.

---

## 1. The §9.2 test enumeration (items 1–8) — executability, item by item

**Ruling: 6 of 8 items are executable as specified once injectable
clock/transport and the frozen artifacts they reference exist; item (4) is
not executable at all until Security RC-7's clamp table lands; item (7)
is unsatisfiable as literally written (the byte-identical claim needs an
`advisory.*` telemetry carve-out that §6 itself forbids omitting).**

**(1) Redaction property suite — executable; the enumeration source must
be a frozen committed fixture inventory, not live enumeration (TR-RC-3).**
"Every telemetry event type the node emits" needs an enumeration, and Rust
has no runtime reflection over emit sites — live enumeration is impossible
in the test binary. My call on the TEST shape (the value-typed allowlist
question itself is Security RC-2's; I rule only how the suite is built):
a frozen, committed, versioned fixture set plus a CI completeness check
that ties it to the source. Concretely:

- `tests/fixtures/advisory-egress/` (T/R-owned): one fixture row set per
  emitted event type, every field carrying an adversarial value; plus a
  committed inventory manifest (`advisory-egress-inventory.json`) of
  `{event, field, type}` triples the unit tests walk — the tests apply the
  allowlist + Redactor to every inventory row and assert the serialized
  digest never contains the raw adversarial bytes.
- Adversarial value set (enumerated, frozen): prompt-shaped text; control
  characters incl. `\n`/`\r`/`\t` (the AD-2 interplay); base58 PeerIds
  (`12D3Koo…` — Security RC-1's form); serde parse-error fragments via a
  crafted `capability_token` (RC-2's `serving.rs` path); composite
  `"X->Y"` clamp strings; float-formatted timings; `sk-`/JWT/Bearer/hex
  blobs; overlong strings; unicode/RTL. NaN/Inf where numeric parsing
  occurs (TR-AD-1).
- The completeness tie (CI, guards job): extract the emitted event
  vocabulary from source and diff it against the manifest. The emitter
  API is `.info/.warn/.error/.log(event, …)`
  (`modelswarm-telemetry/src/lib.rs:163-189`), so the extraction is
  mechanical today:
  `grep -rhoE '\.(info|warn|error|log)\("[a-z0-9_.]+"' crates/ apps/ --include='*.rs' | sed -E 's/.*\("//' | sort -u`
  (16 distinct event names today). A new event name without a manifest
  row is a red build. Note the inventory walks the ACCUMULATOR→digest
  seam per Security RC-4 (typed accumulators, never parsed JSONL as
  input); the JSONL fixtures still matter for the join side — see
  TR-RC-10.

**(2) State-machine property — executable only with a pure core, and one
edge is ambiguously specified (TR-RC-4).** "Every failure edge lands in
DORMANT with exactly one gap event" is writable as a golden
edge→gap-value mapping plus a property sweep, but only if (a) the machine
is a pure `step(state, input) -> (state, effects)` over an injected clock
and transport double (§4's timeout/late/quota edges are otherwise
untestable without real time and a real provider), and (b) the ADR
resolves a contradiction I found between the malformed-handling table and
the frozen gap vocabulary: the timeout row says a late response is a
"no-op discard (gap already recorded)" while the vocabulary contains a
distinct `late` value — so a timeout-then-late cycle emits either one gap
(`timeout`) or two (`timeout` + `late`), and "exactly one gap event"
cannot be asserted until the ADR says which. Delta: pin whether `late` is
emitted at discard time (then item (2) must read "exactly one gap per
edge, `late` counted separately") or never emitted as a new event (then
define its firing condition or drop it from the vocabulary). Also pin
that a FAILED call still consumes daily-cap budget (caps count calls, not
successes — otherwise a flapping endpoint retries toward the cap for
free).

**(3) Cadence/cap property — executable only on a virtual clock; pin the
UTC-day boundary (TR-RC-4).** Defaults mean one full day of coverage is
6+ hours of wall time (24 calls × 900 s spacing) — untestable in CI on a
real clock. The property (for any adversarial event-arrival schedule,
calls ≤ `max_calls_per_day` per UTC day and inter-call spacing ≥
`min_interval_ms`) needs the injected clock from item (2)'s pure core —
record that as a design precondition, not an implementation detail. Also
pin the rollover rule: the cap window is the UTC calendar day, so a flood
straddling midnight legally earns a fresh 24 — intended, and the test
should pin it rather than discover it. Security's added test 9 (forged
JSONL lines never move cadence past caps) folds into this same suite.

**(4) Suggestion-clamp property — NOT executable as specified (TR-RC-6
depends on Security RC-7).** "Clamped to the existing config range" has
no oracle: msp-v1 §6.4 ranges cover requester-facing parameters, but the
natural hint targets are the env-tunable scheduler knobs
(`MSP_SHADOW_QUEUE_{ADVERTISED,MEASURED}_WEIGHT`, `_CAP_MS`,
shadow.rs:79-85) with no declared min/max. This item must be written
against Security RC-7's frozen clamp table and sequenced after it:
∀ target t in the table, ∀ numeric v (negative, zero, huge, NaN, Inf):
recorded value ∈ [min_t, max_t] with `clamped=true` when v is out of
range; t absent from the table ⇒ `requires_adr=true` and never an
improvised bound. NaN/Inf cannot ride JSON at all — the classification
(discard-as-malformed vs gap `schema`) must be pinned (TR-AD-1).

**(5) Shadow-join golden rows — executable exactly in the
`shadow_join.rs` test style (deterministic in-crate string fixtures);
the gaps are in §5's spec, not the test form — see §2 below (TR-RC-5).**
The shipped file's four tests are the template: full-semantics input,
order independence, missing-correlation-key counting, duplicate-trace
survival. The extension's golden rows must add: two hints live on one
trace; a hint live but not applicable (`advisory_pick=none` → null);
an advisory row whose trace never realized (counted and visibly
dropped).

**(6) Dependency-graph test for A2 — executable; the mechanism, specified
concretely (TR-RC-1).** See §5 of this review for the full mechanism
(three layers: cargo-metadata link check, source-scope grep allowlist,
shared-schema exclusion). One structural precondition belongs in the
amendment, not in review notes: §9.4 routes the advisory client into
`crates/modelswarm-node`/desktop — the node crate therefore CANNOT be in
the forbidden-dependency set, and the check's meaningfulness for node
lives entirely in the grep allowlist unless the client lands as a
dedicated module (`modelswarm-node::advisory`) or crate. Record that
seam in the amendment.

**(7) The A3 proof — executable, but unsatisfiable as literally written;
the corpus is the shipped one, not a new one (TR-RC-2).** Two rulings:

- *The carve-out (spec bug in the obligation).* "Disabled/absent/errored
  plane leaves every production decision byte-identical" — decisions,
  yes; telemetry, no: §6 REQUIRES `advisory.cycle.gap` events when the
  plane is enabled-but-erroring, so an unscoped byte-identity assertion
  over the telemetry stream can never pass. Delta: identical outputs
  defined as (i) decision outputs byte-identical and (ii) the telemetry
  stream byte-identical after filtering out events whose name starts
  with `advisory.`.
- *The corpus.* Reuse the shipped deterministic-decision corpora — a new
  invented corpus tests the corpus author, not the system: (a) the
  scheduler/shadow decision property inputs; (b) the seeded session
  admission + coordinator suites (`crates/modelswarm-session/tests/`,
  the `verifydrafts_admission_properties.rs` fixture discipline); (c)
  `apps/modelswarm-sim` mixed-swarm pinned-verdict scenarios (the
  ADR-032 §7 item-4b per-push precedent — decision-level, no sockets,
  fully deterministic); (d) the bench corpora (`corpus.rs`,
  `engage_gate.rs`, `divergence.rs`); (e) the item-(1) fixture corpus as
  the accumulation layer's input distribution. Mechanics: run each
  corpus twice — plane absent vs plane enabled with an
  always-erroring transport double — and assert both (i) and (ii);
  seeds recorded per the ADR-032 fixture discipline.

**(8) The promotion-gate test — executable only as a LAYERED mechanism;
"no code path applies a tuning_hint" rots if left as prose (TR-RC-9).**
See §5.

## 2. The shadow-extension design (§5) vs the shipped `shadow_join.rs`

**Ruling: the LEFT-join-on-realized discipline is correctly restated and
the shipped join is the right skeleton, but the ADR's "correlation is by
`trace`/`cycle_id`" does not specify a join the shipped code can perform,
and the promotion criteria — while correctly bar-shaped — are not
computable from any artifact the ADR defines. Four join-shape/sentinel
gaps (TR-RC-5) and one artifact/computability gap (TR-RC-6).**

What I verified against `shadow_join.rs`: every realized row survives
(the LEFT side); decision-only rows are counted and visibly reported
(`stats.decision_only_dropped`, shadow_join.rs:137-140); correlation is
by `trace` alone (a `BTreeMap<String, ParsedDecision>`, line 99 — one
decision per trace, last write wins); the sentinels
`none`/`unmeasured`/`unknown` map to null (`optional_field`, lines
230-236); parsing is tolerant with visible counters. §5's sentence
matches this discipline in spirit. The gaps:

**TR-RC-5 (required change) — pin the third-stream join shape.**
Deltas for §5, in order of severity:

1. *Join key.* The shipped join has exactly one key: `trace`. A cycle is
   a window, not a row; `cycle_id` cannot join a cycle-scoped fact to
   trace-scoped realized rows unless realized/decision rows grow a new
   correlation field — an unstated row-shape change. But §5's own row
   definition carries `trace` (emitted "for every realized decision
   point in a cycle's window where a hint was live"), so the correct
   shape is available: **join by `trace`, exactly as `shadow_join.rs`;
   `cycle_id` is grouping metadata that buckets joined rows into cycles
   for the promotion artifact — never a join key.** As written, an
   implementer can lawfully build a `cycle_id`-keyed map and produce a
   different, wrong join.
2. *Cardinality.* Decisions are one-per-trace (map insert); advisory
   rows are naturally per-(decision point, live hint): two hints can be
   live in one cycle, and one hint spans many traces. The extension must
   attach a Vec of advisory contexts per trace, and the promotion
   denominator needs a pinned event definition — is "100 differing
   events" 100 traces or 100 (trace, hint) pairs? Unpinned, the count is
   inflatable by hint multiplicity. Delta: pin "one event = one (trace,
   hint) pair" (recommended) or "one trace", and the golden rows must
   cover the two-hints-one-trace case.
3. *Visible drops.* The discipline requires decision-only rows to be
   counted and visibly dropped — extend it: advisory rows whose trace
   never realized are counted and visibly dropped (new stats field,
   e.g. `advisory_only_dropped`); hiding them would break the
   honest-negative discipline the extension exists to serve.
4. *Sentinels and nulls.* `advisory_pick=none` (hint not applicable to
   this decision point) maps to null per the shipped sentinel rule — and
   null-pick rows must be EXCLUDED from the promotion denominator
   ("differ" requires both arms non-null); `agree=unknown` → null.
   State both.

**TR-RC-6 (required change) — make the promotion criteria mechanically
checkable: pin the promise metric, the artifact, and the provenance.**
Security RC-6 ruled the OUTCOME measurement (dual execution, never
attribution of realized deterministic outcomes). What remains unowned —
and is this seat's to rule — is everything the checker computes FROM:

- *The "pinned promise metric" is pinned nowhere.* No rule says where a
  hint's promised metric is declared. Delta: Security RC-7's clamp-table
  companion artifact gains, per target, `{promise_metric, direction}`
  (e.g. target `MSP_SHADOW_QUEUE_MEASURED_WEIGHT` → metric `ttft_p50_ms`,
  direction down). Without it, "strict-majority win" has no referent and
  item (8) has nothing to gate.
- *The artifact layout.* The criteria are computable from: (i) the
  joined shadow rows (join output over per-cycle telemetry), and (ii)
  the dual-execution outcome records from the experiment harness. Ruling:
  the promotion checker is a `modelswarm-bench` component (Scheduler's
  §9.4 routing) that consumes both and emits a machine-readable verdict
  JSON — `{windows[], differing_events, per_metric{wins,losses},
  majority, worst_cells[{cell, n, median_delta, dispersion}],
  provenance{…}}` — COMMITTED under `docs/verification/` with the
  promotion entry (metadata-only, already-redacted; the ADR-032 9.6
  pass-2 committed-negatives precedent), plus a per-push CI test that
  re-runs the checker over the committed artifact and asserts the
  recorded verdict (reproducible, tamper-evident — the same
  fixture-replay discipline as every other frozen artifact in this
  repo).
- *Provenance and variance.* The standing release rule for performance
  claims (hardware, model profile, runtime build, prompt corpus,
  network conditions, baseline-selection method, repetitions,
  variance) applies to promotion evidence; §5 currently has none of it.
  The verdict artifact must record the full provenance block and
  per-cell n, median, and a dispersion measure (MAD or p10/p90); the
  ADR already holds "medians alone never suffice" — add "single-run
  windows never suffice". The fastest-single baseline must be recorded
  per window: which host was the fastest eligible single and its
  measured value (baseline-selection method named, never an average —
  the competitive-revision invariant the ADR already states).
- *Worst-cell definition.* "Non-regressing worst-cell median" is
  subjective until "cell" is pinned. Recommend (profile × capacity
  class), matching the scheduler's decision inputs; record it in the
  amendment so the checker computes it mechanically.

**TR-RC-10 (required change) — promotion evidence integrity: the join's
input is forgeable today (Security AD-2 dependency).** The join parses
telemetry JSONL, and the emitter escapes only `\` and `"`
(`modelswarm-telemetry/src/lib.rs:203-205`), so attacker-chosen bytes can
forge entire lines — including fabricated `sched.shadow.advisory` rows
and fabricated realized timings. Egress stays fail-closed (Security's
domain), but the PROMOTION artifact is built from these files, so log
forging is a direct path to inflating the advisory arm's win count — a
release-gate integrity issue, not just a log-hygiene one. I do not re-rule
the emitter fix (Integrator-owned, Security AD-2 routes it); my deltas:
(a) the AD-2 escaping fix is a PRECONDITION for any promotion decision
(recorded in §5's promotion criteria); (b) the verdict artifact must
disclose the join stats of every contributing window — an
`unparsable_lines > 0` window is disqualifying or at minimum visibly
flagged; (c) the promotion entry pins the SHA-256 of each source
telemetry file it was computed from.

## 3. Venue honesty — the execution-venue matrix (ADR-032 §7 B4 style)

**Ruling: record the matrix below (or its equivalent) in the amendment,
with the standing statement in bold: the provider round-trip NEVER runs
in CI.** Per the B4 discipline ("standing statements so nobody blocks a
release on a job that does not exist"):

| §9.2 item | Venue | Mechanism | Gated on |
|---|---|---|---|
| (1) redaction property suite | per-push, `rust.yml` check job | frozen fixture inventory + inventory-walking unit tests; no network | ADR-033 amendment (this review + Security RC-1/2/3) |
| (2) state-machine property | per-push | pure `step()` core, injected failure doubles, virtual clock | amendment (TR-RC-4) |
| (3) cadence/cap property | per-push | virtual clock, adversarial arrival schedules | amendment (TR-RC-4) |
| (4) clamp property | per-push | property sweep over the frozen clamp table | Security RC-7 artifact |
| (5) join golden rows | per-push, `modelswarm-bench` | in-crate string fixtures, `shadow_join.rs` test style | amendment (TR-RC-5) |
| (6) dependency-graph (A2) | per-push, `guards` job (new step) | `cargo metadata` + jq + grep allowlist (§5 here) | amendment (TR-RC-1) |
| (7) A3 proof | per-push | double-run of shipped corpora + `advisory.*`-filtered telemetry diff | amendment (TR-RC-2) |
| (8) promotion-gate | per-push | golden-pinned defaults/clamp table + opaque-payload compile discipline + (6) | amendment (TR-RC-9) |
| promotion verdict replay | per-push, `modelswarm-bench` (once any verdict artifact exists) | checker re-derives the committed verdict | first promotion decision |
| provider round-trip (real DeepSeek HTTPS call: build → send → parse → record, incl. RC-9 https-only/no-redirect behavior against the live endpoint) | **OWNER-LOCAL ONLY — never CI, never manual-dispatch-with-key** | `#[ignore]`d tests gated on `MSP_ADVISORY_E2E=1` AND the key file present (`MSP_ADVISORY_KEY_FILE` / platform default); evidence = `docs/verification/` entry | §3.5 rotation evidence (Security RC-10) |

Standing statements:

- **The provider round-trip never runs in CI — say so in the amendment.**
  Three reasons, each alone sufficient: (a) the key's custody scope is
  local client config ONLY (§3.4) — a GitHub Actions secret is a second
  custody location the owner authorization does not cover (TR-AD-4);
  (b) a per-push external-SaaS call puts a third party's latency and
  uptime inside the release gate; (c) it spends owner money on every
  push. A green CI badge therefore implies NOTHING about live provider
  interop; that evidence lives exclusively in `docs/verification/`
  owner-local run records (the ADR-032 §7 item-2 precedent: manual
  dispatch or owner-local run + verification record; the
  `MSP_REAL_GGUF`/`MSP_LLAMA_SERVER` and `MSP_LIVE=1` ignored-test
  patterns are the established forms).
- Mock-transport coverage of Security RC-9 (https-only, no-redirect,
  no Authorization on redirect) IS per-push CI-able and MUST be: a local
  in-process transport double returning non-https and 3xx shapes — the
  same injected-transport precondition as item (2). The live-endpoint
  confirmation of that behavior rides the owner-local venue above.
- An unauthenticated manual-dispatch reachability probe (e.g. the 401
  shape) is permissible CI-side without the key but proves little;
  it is optional, not a gate.

## 4. Release-surface check — the 11-key table vs the shipped config layout; wire-compat spot-check

**TR-RC-8 (required change) — the advisory config collides with the
shipped `config.json` writer: silent round-trip loss.** Verified chain:
`config.json` is owned by the desktop app, which parses it into a
TWO-field struct (`PersistedConfig { tracker, profile_id }`,
`crates/modelswarm-desktop/src/app.rs:222-227`) and rewrites the file
wholesale on every tracker/profile change (`persist_config`,
app.rs:617-625; the set-tracker write, app.rs:905-913 —
`std::fs::write` of `serde_json::to_string_pretty(&PersistedConfig…)`).
`read_config` tolerates unknown keys (no `deny_unknown_fields`), so
advisory keys survive READS and are DELETED on the next WRITE. Sequence
the ADR as written produces: owner records `advisory.endpoint_url`,
`advisory.model`, `advisory.key_file` at setup → later changes the
tracker URL or hosting profile in the UI → every advisory key is
silently erased → the plane goes inert with `no_config`/`no_key` and no
error. It fails safe (inert, per invariant (c)) but it is a silent
config-loss defect at exactly the release surface, and it WILL fire in
ordinary operation. Deltas:

1. **Preferred: advisory config lives in its own file**
   (`<data_dir>/advisory.json`), not in `config.json` — zero round-trip
   coupling with the desktop writer, and it dissolves the dotted-key
   form question. Alternative: `PersistedConfig` gains a
   round-trip-preserving advisory section (a `serde_json::Value`/map
   passthrough, not a re-typed struct). Record which.
2. **Fix the citation**: §4's "local `config.json` alongside the
   existing per-install config, `crates/modelswarm-node/src/lib.rs:146-186`
   layout" points at `default_data_dir` + the migration filename list —
   there is no config parser in that range. The reader/writer is
   `crates/modelswarm-desktop/src/app.rs:222-245/617-625`.
3. **Pin the JSON form** if the keys stay in `config.json`: nested
   `{"advisory": {…}}` object vs literal dotted key names
   (`"advisory.enabled"`) — the shipped precedent is flat snake_case
   (`tracker`, `profile_id`), and the ADR table's dotted notation is
   ambiguous as JSON.
4. **Pin where promoted tunables land**: §5 says promotion is "an
   owner-edited config change (runtime-tunable parameters)" — but the
   runtime-tunable scheduler knobs are ENV VARS today
   (`MSP_SHADOW_QUEUE_*`). A promotion applied by env edit is invisible
   to every in-repo audit and silently reverts per-process. Delta:
   promoted values land in the (round-trip-safe per delta 1) config
   file, recorded in the `docs/verification/` promotion entry — never
   in unrecorded environment state.

Also verified, no delta needed: no env-name collision
(`MSP_ADVISORY_KEY_FILE` is unused today, consistent with the
`MSP_LLAMACPP_*` precedent); `advisory.enabled=false` +
key-absent-inert means community installs, the 3-OS installer matrix,
`SHA256SUMS.txt`, and the downloads staging are untouched — zero
packaging surface from this ADR (grep-clean: no advisory reference
anywhere in `crates/`, `apps/`, `installer/` today).

**Wire-compat claim (zero protocol fields) — spot-check PASSED.** Grep
over `protocol/msp-v1.md`, `protocol/messages.proto`, `protocol/vectors/`,
and `catalog/schema.json`: zero advisory/suggestion references; the ADR's
§2/§8 pin all four frozen surfaces byte-identical; §4's "no protocol
binding set applies" (no peer id, session, seq, nonce, signature
surface) means no golden vectors and no cross-implementation parity
surface is owed — correct. The claim holds for the docs-only state.
Residual noted as TR-AD-5: nothing in CI byte-pins those frozen
surfaces today, so during implementation a stray `protocol/` edit is
caught only by review.

## 5. The promotion-gate test (item 8) — the mechanism so it cannot rot

**TR-RC-9 (required change) — three enforcement layers, recorded in the
amendment as the structural form of §5's "no automatic promotion path
may exist in code":**

1. *Compiler layer (strongest — make the illegal act unrepresentable).*
   The suggestion payload's `hint` map is not publicly readable outside
   its sanctioned consumers: the advisory module exposes only (i) a
   Display-rendered summary type (operator UI; the RC-5 plain-text
   rendering contract) and (ii) a ledger-record type whose fields are
   the already-clamped, already-typed values (bench shadow ledger).
   Production code then has no API through which to read a hint value
   to apply — "no code path applies a `tuning_hint`" becomes
   compiler-enforced rather than asserted. This is what §5's sentence
   should have specified.
2. *Link layer — TR-RC-1, the A2 dependency-graph mechanism, concretely:*
   - **Layer 1 (graph):** a new step in the `guards` job (add the pinned
     toolchain — `cargo metadata --no-deps` compiles nothing and takes
     seconds; jq is already a guards dependency):
     ```bash
     cargo metadata --format-version 1 --no-deps | jq -e '
       [ .packages[]
         | select(.name | test("^(modelswarm-(scheduler|session|eligibility|gateway|runtime|transport|tracker-api|store|types))$"))
         | select([.dependencies[] | select(.kind == "normal") | .name]
                  | index("modelswarm-advisory"))
       ] | length == 0'
     ```
     The frozen decision-crate set is exactly
     `modelswarm-{scheduler, session, eligibility, gateway, runtime,
     transport, tracker-api, store, types}` — `types` is in the set
     deliberately (the shared-schema crate is the sneaky path to
     universal visibility); `bench` is exempt (the shadow ledger is a
     sanctioned consumer); node/desktop host the client (§9.4) and are
     covered by layer 2. If the client lands as a node module instead
     of a crate, layer 1 still guards the nine crates above and layer
     2 carries node/desktop. Dev-dependencies: tolerated for test
     fixture reuse only, and recommended against in scheduler/session
     (a dev-dep today is one refactor away from a normal dep, and the
     A3 suites stay honestest with none).
   - **Layer 2 (source scope):** the suggestion type name may appear
     only in the advisory module, `modelswarm-bench`, the desktop
     display renderer, and `*/tests/`:
     ```bash
     HITS=$(grep -rln "AdvisorySuggestion" crates/ apps/ --include='*.rs' \
       | grep -Ev 'modelswarm-bench/|/advisory/|advisory_display|/tests/' || true)
     [ -z "$HITS" ] || { echo "::error::suggestion type escaped its sanctioned consumers: $HITS"; exit 1; }
     ```
     (This also catches a re-declared lookalike type inside a decision
     crate — which the dep graph cannot see.)
   - **Layer 3 (schema):** covered by layer 1 including `types`.
3. *Drift layer (the anti-rot for "promotion is a human config edit"):*
   a per-push golden test pins (i) the eleven §4 config defaults to the
   ADR-table values and (ii) the frozen clamp-table contents. Any value
   change without an ADR-033 amendment is a red build — the only green
   path to a changed tunable is the recorded amendment plus the
   `docs/verification/` promotion entry with its TR-RC-6 verdict
   artifact. This closes the quiet channel the ADR leaves open:
   someone "just tuning" a scheduler default in code is precisely the
   unmeasured promotion §5 forbids, and nothing else in the enumeration
   would catch it.

Together with TR-RC-6's committed-verdict replay test, every element of
item (8) is machine-checked: no apply-path (compiler + link), no
unrecorded value change (drift), no unverifiable promotion claim
(replay).

## 6. Findings summary

| ID | Class | One line |
|---|---|---|
| TR-RC-1 | required | Pin the A2 mechanism: cargo-metadata+jq graph check over the frozen nine-crate decision set + grep allowlist for the suggestion type + the node-module seam (§5.2 here, ready to implement verbatim). |
| TR-RC-2 | required | A3 needs the `advisory.*` telemetry carve-out (else unsatisfiable by §6's own gap events) and must reuse the shipped corpora (scheduler/session/sim/bench), not an invented one. |
| TR-RC-3 | required | Redaction suite = frozen committed fixture inventory + CI completeness tie to the emit-site vocabulary (16 event names extractable today), adversarial value set enumerated. |
| TR-RC-4 | required | Items (2)/(3) need a pure injectable-core precondition; pin the `late`-vs-`timeout` double-gap contradiction, failed-call cap accounting, and the UTC-day rollover rule. |
| TR-RC-5 | required | Join shape: trace is the only join key (cycle_id groups), per-(trace,hint) cardinality with a pinned event definition, advisory-only drops visible, null-pick denominator exclusion. |
| TR-RC-6 | required | Promotion computability: promise metric pinned per clamp-table target, committed machine-readable verdict artifact + CI replay, full provenance/dispersion/repetitions, worst-cell definition, fastest-single baseline recorded. |
| TR-RC-7 | required | Record the execution-venue matrix with the standing statement: the provider round-trip NEVER runs in CI (key custody, third-party uptime in the gate, owner spend); evidence is owner-local `docs/verification/` records only. |
| TR-RC-8 | required | Config surface: separate `advisory.json` (or round-trip-preserving section) — the desktop writer deletes unknown keys on every tracker/profile change today; fix the wrong lib.rs:146-186 citation; pin the JSON form; promoted tunables land in config, never env-only. |
| TR-RC-9 | required | Promotion-gate anti-rot: opaque (non-applicable) hint payload API + golden-pinned defaults and clamp table so value changes require the amendment path. |
| TR-RC-10 | required | Promotion evidence integrity: AD-2 log-forging fix is a precondition for any promotion decision; verdict artifact discloses join stats (unparsable windows flagged/disqualified) and pins source-file digests. |
| TR-AD-1 | advisory | Pin NaN/Inf numeric-hint classification (JSON cannot carry them) and add the edge vectors to item (4). |
| TR-AD-2 | advisory | Record the injectable clock/transport as a §9.4 routing note — items (2)/(3) and the RC-9 mock tests are unrunnable without them. |
| TR-AD-3 | advisory | Golden-pin the 12-value gap vocabulary as an enum test; a new gap value without an amendment note is a red build (cheap drift alarm). |
| TR-AD-4 | advisory | Never add the provider key as a CI secret — second custody location outside the §3.4 authorization, even "just for one smoke test". |
| TR-AD-5 | advisory | Release checklist gains: the next tagged release carrying advisory wiring requires Security RC-10's rotation-evidence entry; optionally hash-pin the four frozen protocol surfaces in guards to mechanically enforce the zero-wire-fields claim during implementation. |

## 7. What I could not verify (and why it does not block)

- Anything about the advisory client's implementation — none exists;
  this review is the gate it must pass first.
- The live provider's behavior (redirects, error shapes, key form) —
  owner-local venue by design; the round-trip evidence will arrive as
  `docs/verification/` records, not CI artifacts.
- The key file's existence/rotation state — outside the repo by design;
  Security RC-10 governs.
- That the promotion bar is ACHIEVABLE — only that it is computable and
  honestly reportable once TR-RC-5/6/10 land; the first promotion
  decision may well be an honest negative, which the discipline requires
  publishing either way.

## 8. Gate statement

Implementation of the advisory plane (any commit under §9.4's routing)
MUST NOT land until TR-RC-1…TR-RC-10 are recorded as an ADR-033
amendment by the Protocol Architect — jointly with Security's
RC-1…RC-11 from `86c6c77`; the two sets are disjoint and both bind. The
§3.5 rotation evidence (Security RC-10) remains a precondition for the
first production digest cycle and for any tagged release carrying
advisory wiring (TR-AD-5 puts it on the release checklist). With the
amendment recorded, this review's approval stands; no re-review is
required for the amendment itself unless it deviates from the deltas
above, in which case return it to this seat.

Gate evidence (run at main @ `c865877` before writing this file,
docs-only change): `cargo fmt --all --check` PASS; `cargo clippy
--workspace --all-targets -- -D warnings` PASS; `cargo test --workspace`
PASS — verified under `set -o pipefail` (cargo exit 0), 69 suite
summaries ok, 0 failures, incl. the 56 s mixed-swarm scenario run.

— Test and Release Engineer, 2026-10-12
