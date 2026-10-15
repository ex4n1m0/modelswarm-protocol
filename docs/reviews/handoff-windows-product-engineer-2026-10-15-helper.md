# Handoff — Windows Product Engineer, 2026-10-15 (the helper rewire: DeepSeek key removed, hosted-model backing)

Owner directive (verbatim): "how about we remove this deepseek key completely, and make
the helper use the hosted model when hosted". Adopted design: the assistant/helper is
backed by the machine's OWN hosted model behind the node's loopback gateway while
hosting, and by the existing frozen static-help corpus otherwise. NO cloud provider
anywhere: the DeepSeek key, `endpoint_url`, `model`, and `key_file` config surface are
REMOVED; the advisory plane's cloud legs (ReqwestTransport to a provider, https-endpoint
arming) are deleted, not just defaulted off.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-node/src/advisory/accum.rs` | DELETED — the digest plane's typed accumulators existed only to feed the metadata digest to the provider brain; with no provider there is no digest. |
| `crates/modelswarm-node/src/advisory/runner.rs` | DELETED — the timer-driven digest-cycle runner (the cloud call site) died with the provider surface. |
| `crates/modelswarm-node/src/advisory/state.rs` | Slimmed to the SHARED vocabulary types: the 12-value `Gap` enum (`no_key`/`interval`/`quota` removed as unemittable; `not_hosting` added), the suggestion schema, `ProviderRequest`/`TransportFailure`, `parse_one_suggestion` (now `pub` for the redaction suite's numeric-seam canaries). The digest state machine is gone. |
| `crates/modelswarm-node/src/advisory/config.rs` | Collapsed to the nine ADR-037 §3.4 `assistant.*` rows; `assistant.enabled` flips ON by default (its default-off existed because cloud mode spent money per call). Old provider/digest keys ignored + preserved, never fatal. Key resolution (`MSP_ADVISORY_KEY_FILE`, `key_file`, platform default, `read_key`) and `is_https_endpoint` DELETED. |
| `crates/modelswarm-node/src/advisory/transport.rs` | `ReqwestTransport` and `pinned_client`'s provider role DELETED. New production transport: `LocalGatewayTransport` — loopback-ONLY (refuses non-loopback), redirects never followed (RC-9's loopback form), posts the OpenAI-compatible shape to the node's own `/v1/chat/completions`, strips `choices[0].message.content`, byte-cap streaming. Trait drops the key parameter; `CountingTransport` loses `presented_keys` (no key exists). |
| `crates/modelswarm-node/src/advisory/egress.rs` | Digest half deleted (`TypedValue`/`DigestField`/`admit`/`choke_point_rescan`/`peer_ref16`/`profile_ref16`/`reduction_rules` parsing). The assistant context gate, choke-point re-scan, clamp table, and the R-1 tie tests stand. |
| `crates/modelswarm-node/src/advisory/assistant.rs` | The rewire's core: `HostedBacking` (gateway addr + hosted profile), `AssistantArm::{Hosted, Static, Disabled}` (`assistant_arm` reads only `advisory.json` + the caller's hosting facts — no key path), `AssistantPlane::spawn(data_dir, backing, transport_override)`, `run_turn` calls `send(&request)` key-free, `HttpStatus(402)`→quota mapping gone, system prompt restated for the hosted-model identity. Caps/conversation/scrub/parse mechanics unchanged. |
| `crates/modelswarm-node/src/advisory/mod.rs` | Module docs rewritten (assistant-only plane); `wire`/`Arm`/`arm` DELETED; unit tests retargeted: mock-runtime chat-wall success path (zero assistant calls + sensitivity control), honest-gap confinement, static-floor default. |
| `crates/modelswarm-node/src/lib.rs`, `selftest.rs` | The node daemon composition wires NO advisory machinery anymore: `NodeConfig::advisory_transport` and `NodeHandle::advisory` removed. The desktop constructs the plane against the running node's gateway. |
| `crates/modelswarm-node/tests/advisory_chat_wall.rs` | Layers 1–2 updated (names list: `ReqwestTransport`→`LocalGatewayTransport`+`HostedBacking`+`AssistantArm`; `.send(&request` from exactly ONE site; `advisory::wire` must appear NOWHERE); layer 3 arms the plane directly over a double instead of the deleted NodeConfig seam. |
| `crates/modelswarm-node/tests/advisory_assistant_suite.rs` | Boot helper: `advisory.json {"assistant.enabled":true}` + double (no key/endpoint/env). Key-material assertions replaced with user-text-never-in-telemetry canaries. |
| `crates/modelswarm-node/tests/advisory_redaction_suite.rs` | Retargeted to the surfaces that exist: the assistant context gate (over-bound/foreign/poisoned routes drop), the RC-3 assistant choke point, the TELEMETRY Redactor canaries (secret shapes neutralized — meaningful for local sinks), TR-AD-1 numeric seams, RC-5 renderer. |
| `crates/modelswarm-node/tests/advisory_a3_corpus_sweep.rs` | The two postures are now HOSTED-MODEL-BACKED vs STATIC-ONLY. Hosted-backed drives real turns over the echoing double DURING the node workload against a file sink on the same data dir — the `assistant.*` rows genuinely interleave into the compared stream (non-vacuous filter), decision corpus byte-identical. |
| `crates/modelswarm-desktop/src/app/assistant.rs` | `FIRST_USE_DISCLOSURE`/`PROVENANCE_CHIP` replaced by the pinned helper-identity texts (`FIRST_USE_IDENTITY`, `IDENTITY_CHIP`); `degraded_line` new wording; corpus: `no_key`/`quota`/`interval` entries removed, `not_hosting` added; `Plane::spawn`(+`spawn_hosted` composition seam so `app.rs` never names an advisory type); `TurnDeps` drops `acknowledged`; `PanelOutcome::{ModelReply, ModelGap, StaticHelp}` (no `NeedsAcknowledgement` — the gate folded away). |
| `crates/modelswarm-desktop/src/app.rs` | `set_hosting_inner` arms the plane at hosting start (`spawn_hosted(local_addr, profile)`) and returns it to the static floor (`not_hosting`) at stop; startup spawns without backing; `get_status` assistant block: `hosted` replaces `cloud`, `identity` replaces `disclosure`; `assistant_acknowledge` persists the (now cosmetic) banner dismissal; tests: chat-wall desktop leg + assistant_panel suite on the new postures. |
| `crates/modelswarm-desktop/ui/index.html` | Panel relabeled honestly: "Assistant" settings, hosted-vs-built-in mode line, dismissible identity banner (`assistant-ack` = "Got it", never blocking), new chip text, `renderAssistantPayload` without `needs_ack`. All ten mandated element ids and four IPC names survive with real behavior. |
| `crates/modelswarm-desktop/ui-harness/assistant.html` | Harness rewritten for static-vs-hosted labeling: no-gate first turn, verbatim identity texts, plain-text/cap/chip/facts/W4/namespace checks, `not_hosting` static floor, Settings opt-out. 25 checks. |
| `docs/adr/ADR-033-advisory-plane.md` | Append-only revision note (2026-10-15): the provider-key era ends (owner ruling verbatim); the digest plane, key resolution, and provider transport deleted; the gap vocabulary is the 12-value set; the artifact sync recorded (87/87/87); deterministic-local is the only mode. |
| `docs/adr/ADR-037-assistant-surface.md` | Append-only revision note (2026-10-15): backing = hosted model + static floor; the disclosure/ack gate folds away (identity line kept); caps/W1–W6/context builders unchanged; `assistant.enabled` default-on amendment; A3 postures. |
| `protocol/advisory-egress-v1.json` | Mechanical R-1 sync: the three `advisory.cycle.*`/`advisory.suggestion.recorded` emit-site rows removed; five `feeds_digest` classifications → `excluded`; `aggregation_event_allowlist` retired-empty; `plane_telemetry_events` = the two assistant rows; `plane_enum` = `assistant`; `reduction_rules` removed; `advisory_gap` = the 12-value set; `contract.history` records the change. |
| `tests/fixtures/advisory-egress/advisory-egress-inventory.json` | Regenerated via `scan_emit_sites.py --emit-inventory` (87 rows; the count legitimately changed from 90). |

## 2. Exact commands and outcomes

- `cargo fmt --all` then `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 (after deriving `Default` on `AdvisoryConfig` and allowing `result_unit_err` on the whole-discard `parse_one_suggestion`).
- clippy feature combos mirroring rust.yml — all 0: `modelswarm-node/libp2p-backend`; `modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend`; `modelswarm-runtime/capi-adapter`; `modelswarm-node/verifydrafts-capi,modelswarm-node/libp2p-backend`; `-p modelswarm-desktop --features tauri-shell`.
- `cargo test --workspace` (+ the same feature combos; `cargo test -p modelswarm-desktop --features tauri-shell` = 72/0/0) — all green, 0 failures anywhere.
- `python tests/fixtures/advisory-egress/scan_emit_sites.py` — **87/87/87, R-1 two-sided tie PASS**; regenerated with `--emit-inventory` first.
- Leak-grep (the guards pattern, run locally): `git ls-files -z | xargs -0 grep -rlE 'sk-[0-9a-f]{32}'` — clean.
- Harness family, headless Edge over CDP at 1152×648: **assistant 25/25, adr036-carriage 18/18, census 10/10, monitor 11/11, pairing-chat 15/15, updater 17/17 — six times ALL PASS**. jsdom behavioral leg (updated for the new flow): **30/30**.

## 3. Test evidence

Node: 60 lib tests pass incl. the 37 advisory tests (golden 12-value gap tie to the
artifact, cadence/cap property sweep, hosted-vs-static arming incl. the old-era
advisory.json file, transport round-trip/refusal/failure mapping against mock loopback
gateways); integration suites `advisory_chat_wall` (layers 1–3),
`advisory_assistant_suite` (canaries, scrub, post-gate corruption, honest gaps),
`advisory_redaction_suite` (retargeted), `advisory_a3_corpus_sweep`
(static-only vs hosted-model-backed, decision outputs byte-identical, filtered
telemetry byte-identical with REAL assistant rows interleaved and removed).
Desktop tauri-shell: 72 passed — chat-wall desktop leg (armed plane over a double;
tracker-unreachable, empty-roster, and QUIC dial-failure paths record ZERO calls) and
the assistant_panel suite (no-gate first turn, opt-out, static floor, display cap,
transcript/carrier canaries, A3 extension).

## 4. Assumptions

1. The digest plane dies with the provider (the directive removes the key "completely";
   the digest's only consumer was the cloud brain). The ADR-033 note records this and
   the artifact sync.
2. `assistant.enabled` flips ON by default (the cloud-cost rationale for default-off is
   gone). Operators can still pin the static floor with `assistant.enabled:false`.
3. The first-use banner stays (ids + `assistant_acknowledge` IPC) because the T/R-owned
   guards workflow asserts those surfaces exist; it is now a DISMISSIBLE identity
   banner that never blocks a turn — real behavior behind every mandated surface, no
   dead elements.
4. The `assistant_acknowledged` config.json field survives as the banner-dismissal
   persistence (read/write unchanged; old files simply carry a stale-but-harmless bit).
5. Local-model replies must emit the strict `advisory-assistant/v1` JSON (the system
   prompt instructs it); anything else fails closed with gap `schema` and the honest
   static-floor wording explains that small local models can miss the shape.

## 5. Unresolved risks

- Small hosted models may frequently fail the strict reply schema (gap `schema`) —
  honest, but the helper's usefulness on small models is bounded by their JSON
  discipline. A looser envelope would need an ADR-037 amendment.
- The A3 sweep's hosted-backed posture stands in for the hosted model with a transport
  double (the standing venue rule: never a real engine in tests). A real-engine
  assistant round-trip remains owner-local verification territory, like the challenge
  path.
- The egress artifact/inventory sync was done under the task's mechanical-sync
  authority; the Protocol seat should glance at the `reduction_rules` removal and the
  `plane_enum` collapse recorded in the ADR-033 note (both consumers were deleted).

## 6. Suggested next task for the integrator

Review + merge. Then the natural follow-up (owner-visible polish): a live owner-local
pass of the assistant against a real hosted model (the strict-schema hit rate), and —
if the owner wants it — an ADR-037 amendment sketch for a lenient reply envelope
(plain-text reply with hints optional) so small hosted models can serve the helper
without JSON-perfect outputs.
