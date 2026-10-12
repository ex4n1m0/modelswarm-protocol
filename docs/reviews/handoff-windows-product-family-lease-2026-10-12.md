# Handoff — Windows Product Engineer: the family lease PRESENTATION path (2026-10-12)

The last activation gap before the v0.2.21 release, closed: the light-up
(55d802e) shipped the composed daemon whose family sessions admit through
`RosterFamilyLeases`, but NOTHING fed the roster, so shipped desktops
refused every family session (`no_lease`). This change implements
ADR-032 §2's admission order — the coordinator PRESENTS its hub-signed
lease AT SESSION OPEN (the family-side equivalent of msp-v1's per-request
`capability_token`) — so two fresh desktops holding earned leases can now
open a family session with an EMPTY roster. The roster stays as the
tracker-fed future through the SAME validation engine.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-node/src/verifydrafts/mod.rs` | `WireSessionLease` — the minimal typed session-open carrier `{"lease": "<wire token>"}` (strict `deny_unknown_fields`, one field, the UNCHANGED `EligibilityLease::to_wire` string; discriminant collides with no other family frame — pinned by test); module docs + exports (`read_session_open`, `SessionOpenError`, `PrefacedWire`). |
| `crates/modelswarm-node/src/verifydrafts/serve.rs` | `read_session_open` + `SessionOpenError` — the venue-agnostic session-open reader: first frame = lease carrier (strict parse; then recv the offer) or the offer itself (`presented = None`, the roster/classic path stays). Offer value stays RAW (handed pre-read; never decoded twice). |
| `crates/modelswarm-node/src/verifydrafts/wire.rs` | `PrefacedWire<W>` — the pre-read wrapper generalized from composed's `PrefacedQuicWire` to any `VerifyDraftsWire` (staged tests use it too). |
| `crates/modelswarm-node/src/verifydrafts/composed.rs` | The seam: `FamilyLeaseSource::lease_for(peer, presented_lease: Option<&str>, profile)`; `RosterFamilyLeases` now consults the ROSTER entry first (tracker-fed future, authoritative when present) and falls back to the PRESENTED token through the SAME `validate_family_lease`; `validate_presented_lease` maps empty/`f0-unverified` to `NoLease` (the msp-v1 gate's first check, verbatim). The composed family branch reads the open exchange BEFORE any engine work (refusals never construct the engine), classifies a malformed carrier as `invalid_lease`, and hands the offer pre-read. Stub seam (`RefuseFamilyLeases`/`StaticLeaseExpiry`) keeps working under the widened signature. |
| `crates/modelswarm-node/src/verifydrafts/client.rs` | `VerifyDraftsClientConfig.lease: Option<String>`; `negotiate` sends the carrier FIRST when set. Refusals surface as `ClientOpenError::Io` — the same honest fallback-to-single signal as every refused open. |
| `crates/modelswarm-node/tests/verifydrafts_wiring.rs` | Staged loopback, both arms (cfg `verifydrafts+libp2p-backend` — the production source's feature pair): `presented_lease_lifecycle_opens_and_serves` (valid token, EMPTY roster, full round, exactly one engine pass) and `presented_broken_lease_refuses_before_engine` (wrong-profile presented token → `ProfileMismatch`, engine `calls() == 0`, honest `Io` on the client). |
| `crates/modelswarm-node/tests/verifydrafts_composed.rs` | The QUIC venue slot: the demux pin's family arm now PRESENTS (empty roster — the two-fresh-installations pairing); the RC-L1 wrong-profile pin presents its wrong token. |
| `crates/modelswarm-node/tests/verifydrafts_capi_composed_e2e.rs` | The E2E flows through the presented lease end-to-end: roster NEVER seeded; wrong-profile PRESENTED lease refuses pre-engine; the family arm presents and opens on the real engine. |
| `crates/modelswarm-desktop/src/app.rs` | The desktop REQUESTER: new `family_probe` IPC command — earns/reuses the cached consume lease (the same lease the msp-v1 swarm path presents), finds an eligible exact-profile QUIC peer, dials, and opens one real family session PRESENTING the lease (no rounds, no prompt content; clean close). Honest outcomes only: `opened` (+ peer, session id, real disclosure) or the exact reason (`hosting_off | no_presentable_lease | tracker_unavailable | lookup_failed | identity_unavailable | no_eligible_peer | peer_id_not_ed25519 | dial_failed | open_refused | peer_rejected | verifier_ineligible | invalid_accept | protocol | misconfigured | family_not_compiled`), all telemetered (`family.probe`). `compose_family_serving` docs/telemetry updated (roster empty is the NORMAL presentation configuration); `FamilyStatus.lease_roster_peers` doc updated; 2 new tests. |
| `crates/modelswarm-desktop/ui/index.html` | The family line no longer claims refusal at roster 0 ("coordinator leases: presented at session open (roster N)"); a `test family pairing` button + result line drives `family_probe`. |

No protocol-schema change: the carrier is a minimal typed wrapper in the
verifydrafts module (documented here for the architect's freeze-ADR
record — when the ADR-032 wire freeze lands, record `{"lease": …}` in
`protocol/` alongside the family's other bare frames). The token string
is exactly `modelswarm-eligibility`'s `to_wire` form; no invented fields,
no new signature, no new lease shape.

## 2. The admission order, end to end

1. Coordinator (`VerifyDraftsClient::negotiate` with `config.lease` set)
   sends `{"lease": "<wire token>"}` then its `SessionOffer`.
2. The composed loop's demux classifies the session by the first frame
   (raw JSON → family; the transport's VecDeque stash keeps carrier-then-
   offer ordering across the poll boundary).
3. `read_session_open` strict-parses the carrier and reads the offer
   (before ANY engine work).
4. `lease_for(peer, presented, served_profile)` — roster entry if fed,
   else the PRESENTED token — runs `validate_family_lease`: hub signature
   + msp1 form + expiry (±120 s skew) + lease cap, profile binding,
   `can_consume` + `slots >= 1`, peer binding vs the QUIC-authenticated
   PeerId; the token's `expires_at` (unix ms) becomes the session's
   stage-5 lease bound. Empty/`f0-unverified` → `NoLease`; a malformed
   carrier → `InvalidLease`; every refusal ends the session with the
   stable `verifydrafts.family_refused` telemetry label.
5. Accepted → engine constructed per session, disclosure = real geometry,
   the offer handed pre-read to the unchanged serve loop.

## 3. Exact commands and outcomes (final tree)

```text
cargo fmt --all --check                                                                    CLEAN
cargo clippy --workspace --all-targets -- -D warnings                                      CLEAN
cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings        CLEAN
cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend -- -D warnings   CLEAN
TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings   CLEAN
cargo test --workspace                                                                     469/0/6  (baseline EXACT)
cargo test --workspace --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend                     526/0/7  (520 + 6)
cargo test --workspace --features modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend                545/0/13 (539 + 6)
cargo test --workspace --features modelswarm-node/libp2p-backend                                                  492/0/7  (unchanged)
cargo test --workspace --features modelswarm-runtime/capi-adapter                                                 484/0/10 (unchanged)
TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --features tauri-shell                13/0/0   (11 + 2)
```

New tests: node lib +4 (`wire_session_lease_is_the_bare_strict_carrier`,
`presented_placeholder_forms_classify_as_no_lease`,
`presented_lease_admits_on_an_empty_roster`,
`roster_entry_takes_precedence_over_the_presented_token`; the
binding-by-binding pin became
`each_broken_binding_refuses_via_roster_and_via_presentation`, doubling
every arm through BOTH token sources), staged wiring +2, desktop +2
(`family_status_on_with_empty_roster_is_presentation`,
`family_open_error_labels_are_stable`).

## 4. E2E verdict (owner-local, real engine, this machine)

Pinned b11407 CPU set (`runtime-pins.json`, sha256-verified), SmolLM2-135M
-Instruct Q4_K_M, single-machine loopback, Windows x64, 24 logical CPUs —
NOT LAN, NOT a product claim. `--test verifydrafts_capi_composed_e2e`
(the swap path, now presented-lease end-to-end) — PASS:

- RC-L1: the wrong-profile PRESENTED lease refused with the honest
  session end, no accept frame, engine never touched;
- msp-v1 arm over the same listener: "Hello swarm" streamed;
- family arm (presented lease + real engine, EMPTY roster): session
  OPENED; disclosure from real geometry (cpu, batch_verify true, window
  32);
- round 1 (window 8): full accept + ground-truth bonus, verify_pass_ms
  13, round wall 23 ms;
- round 2 (corruption at index 4): exact divergence + ground-truth
  replacement, verify_pass_ms 13, round wall 21 ms;
- committed stream 14/14 tokens == HTTP ground truth; total wall 3250 ms
  including staging + the HTTP ground-truth arm (previous run on this
  machine: 3274 ms; presentation adds no measurable round overhead —
  12-14 ms verify_pass_ms both before and after).

`--test verifydrafts_capi_e2e` (plain-QUIC venue, regression re-run) —
PASS: verify_pass_ms 14/12, round wall 21/20 ms, same exactness pins.

## 5. The two-desktop confirmation

Two fresh v0.2.21 desktops hosting the same pinned profile, both with
leases earned through the tracker's hosting-challenge chain, CAN now open
a family session, with these pre-existing opt-in gates unchanged:
`MSP_LISTENER=1` on the serving side, the family prompt file pinned +
C-API bundle present (the engine bridge), and a reachable (LAN) QUIC
address. Evidence chain: (a) the composed QUIC integration pin opens a
session on an EMPTY roster with a presented lease (live QUIC, both
vocabularies); (b) the real-engine E2E repeats it end-to-end with
exactness pins; (c) the desktop's `family_probe` command drives exactly
that flow from the shipped UI with the cached/earned lease; (d) the
desktop validates under `LeasePolicy::production()` — the same pinned hub
key the msp-v1 path already exercises with tracker-issued leases in
production, so real earned leases satisfy the same bindings (the E2E's
test-hub key is the only substitution). What still limits a LAN product
claim: NAT (no relay — the recorded beta boundary), and the serving
side's family engine needs the out-of-band prompt file (G6, freeze-ADR
item) plus the RAM/VRAM-doubling coordination with the Resource Engineer
(light-up risk 1, unchanged).

## 6. Assumptions

1. ADR-032 §2's "session/lease (ADR-026 gate)" passed AT SESSION OPEN =
   the coordinator presents its lease as/with its first frames — the
   carrier `{"lease": …}` BEFORE the `SessionOffer`. The offer-first form
   stays valid (presented = None → roster-only), so the stub venues and
   the roster future are unaffected.
2. `RosterFamilyLeases` roster-entry precedence over the presented token:
   the tracker-fed record is authoritative when present (fail-closed
   feeder posture — a garbage entry refuses until `remove`; pinned by
   test).
3. The desktop's requester surface is a pairing DIAGNOSTIC (`family_probe`),
   not a chat flow: driving family VERIFICATION inside chat (local
   drafting + remote verify + commit) is the cooperative-plan Windows
   integration phase, deliberately not started here.
4. The lease presented is the SAME cached token the msp-v1 swarm path
   uses (`earn_lease`, F3(2)) — no second lease chain was invented.

## 7. Unresolved risks / notes for the release train

- **Carrier freeze record**: `WireSessionLease` is a typed wrapper in my
  module, not a protocol/ schema — the Architect should fold it into the
  ADR-032 freeze record (it is one field, no crypto, no envelope).
- **Lease material on the wire**: the carrier carries the wire lease
  token on the authenticated QUIC session — the same exposure as msp-v1's
  `capability_token`; tokens still never enter telemetry, logs, or the
  webview (the probe result carries labels/ids only).
- **Probe cadence**: `family_probe` triggers `earn_lease` (a hosting
  challenge) when no cached lease is valid — a UI button press is a
  reasonable cadence; if the probe ever becomes automatic, rate-limit it.
- The light-up's open items are unchanged: RAM/VRAM doubling vs the
  governor's 70% ceiling; the A-2/A-3 advisories; the ADR-032 §7
  completion line (owner/Protocol seat).
- Version NOT bumped (the release train owns it).

## 8. Suggested next task

Integrator: review + merge; then the release train cuts v0.2.21 (version
bump + `docs/verification` record) and the owner/Protocol seat appends
the light-up + activation completion lines to ADR-032 §7. After that, the
natural next product task is the family engine prompt pairing UX (G6)
with the Resource Engineer's governor coordination.
