# Handoff — Protocol Architect — Protocol/Schema/ADR Consistency Review (2026-10-09)

> Filed by the Integrator on the Protocol Architect's behalf: the architect
> role holds read-only tools in this audit, so its report is delivered as the
> agent's final message and filed verbatim below. Audit branch
> `audit/master-prompt-2026-10-09`, baseline `730cebf`, product v0.2.20.

---

# Protocol/Schema/ADR Consistency Review — MSP v0.2.20 (audit/master-prompt-2026-10-09, baseline 730cebf)

## 1. Summary verdict

The frozen core is in genuinely good shape: lease wire (§5), envelope signing (§2.3), manifest identity (ADR-011/022), catalog v2, and the ADR-026 gate are byte-consistent end-to-end across spec, Rust, TS, and CI (including a real-tracker wire-compat job). The drift is concentrated at the edges the spec never re-adsorbed: (a) five live tracker endpoints and one response field exist on the wire with no msp-v1 §3 presence and no ADR (`session-authorize`, `/receipt`, `/audit`, `/stats`, `/download`, `capacityClass`); (b) §6.3's `accepted` event and §7 receipts are specified-but-unimplemented with no recorded deferral; (c) serving-peer P2P error codes diverge from the §6.5 registry (`duplicate_request` vs spec `replayed_request`); (d) msp-v1.md has no changelog, so post-freeze amendments are only discoverable by inline ADR labels — and several weren't labeled. Nothing here breaks the lease gate or content-blindness; all fixes are documentation-plus-retroactive-ADR work except the error-code rename, which is client-visible.

## 2. Mismatch tables (evidence: file:line)

### 2a. Hub REST surface (msp-v1 §3 vs TS routes vs Rust client)

| Field/Endpoint | Spec says | TS does | Rust does | Verdict |
|---|---|---|---|---|
| `POST /peers/lease` | §5:40-45 text defines response `{token, lease}`; **absent from §3.3 table** (msp-v1.md:109-123) | Implemented, full ADR-012 checks (`apps/tracker/app/api/v1/peers/lease/route.ts:17-77`) | `TrackerClient::request_lease` + `LeaseIssued` (`crates/modelswarm-tracker-api/src/lib.rs:94-102`) | **DOC-ONLY** (ADR-012-covered; §3.3 table stale) |
| `POST /api/v1/session-authorize` | Not in spec | Implemented; body `{peer_ids, profile_id, mode}`, `mode` = free regex string (`apps/tracker/lib/schemas.ts:126-132`, `app/api/v1/session-authorize/route.ts:16-58`) | **No caller in any crate** (grep: zero hits) | **ADR-REQUIRED** — post-freeze endpoint, DESIGN.md-only provenance; `mode` unvalidated against the ADR-013 five-mode registry |
| `POST /api/v1/receipt` | Not in spec (§3.3 has `/events/job-result` only) | Duplicate intake of `{receiptDigest, outcome}` (`app/api/v1/receipt/route.ts:12-24`) | `job_result()` targets only `/events/job-result` (`modelswarm-tracker-api/src/lib.rs:320-329`); **nothing posts from Rust** | **ADR-REQUIRED** (fold into receipts-v2 ADR) |
| `POST /api/v1/audit` | Not in §3.5 admin table | Admin epoch-bump (`app/api/v1/audit/route.ts:14-28`) | — | **ADR-REQUIRED** (semantics ADR-012-covered; route never folded into §3.5) |
| `GET /api/v1/stats`, `GET /api/v1/download/[file]` | Not in spec | Implemented, content-blind counters (`app/api/v1/stats/route.ts`, `download/[file]/route.ts`) | — | **DOC-ONLY** (website surface; declare non-protocol or add §3 appendix) |
| `capacityClass` in `GET /peers` response | §3.3:119 shape has 6 fields, no `capacityClass` | Returned (`app/api/v1/peers/route.ts:43`) | Parsed (`modelswarm-tracker-api/src/lib.rs:70-71`) | **ADR-REQUIRED** — frozen response shape changed; DESIGN.md:14 attributes to ADR-012 but ADR-012 defines the lease field, not this response field |
| `challenge/complete` result | §3.3:118 "capability token (§5)" | Returns `{passed, challengeId, capacityClass}`; issuance moved to `/peers/lease` (`challenge/complete/route.ts:7-9,49`) | Matches | **DOC-ONLY** (ADR-012-covered move; table stale) |
| §3.5:176 candidates validated "against `catalog/schema.json`" | v1 schema | Validates `ManifestSchema` (schema-v2 mirror, `schemas.ts:142-180`) | `ModelProfileManifest` | **DOC-ONLY** |
| §4 catalog profile shape "ModelProfile per catalog/schema.json" | v1 camelCase | Serves v2 `ProfileRecord` snake_case (`lib/catalog.ts:8-17`) | Parses v2 + re-derives id (`crates/modelswarm-node/src/catalog.rs:17-75`) | **DOC-ONLY** (ADR-011/022-covered; §4 text stale) |
| TS error codes | §3.4 set + `unknown_request` (§3.5, ADR-023) | Exact match incl. status map (`lib/errors.ts:4-41`) | — | Match |

### 2b. P2P wire (msp-v1 §6/§7 vs proto vs Rust)

| Item | Spec says | Proto says | Rust does | Verdict |
|---|---|---|---|---|
| `accepted` stream event | §6.3:287 `accepted` w/ queue position, etaMs | `Accepted{queue_position, eta_ms}` (messages.proto:110) | **No `Accepted` in `WireMessage`** (`modelswarm-transport/src/message.rs:204-215`); serving drops the executor event (`modelswarm-node/src/serving.rs:377`); `queue_position/eta_ms` live only in `HandshakeAck` — never sent on the libp2p path | **ADR-REQUIRED** to formally defer (or implement); gap-study G4b's fair-queueing plan depends on this vocabulary |
| `Completed.job_receipt` + §7 two-phase receipt | §6.3:291, §7:324-331 `{requestId, profileId, peerIds, startedAt, endedAt, outcome, usageDigest}` | `JobReceipt` + `ReceiptAck` (messages.proto:79-96) | Omitted with in-code deferral note ("receipts land with the gateway/session layer", message.rs:142-144); **no single-mode receipt exists anywhere**; only cooperative `Receipt{session_id, rounds, committed_tokens}` (`modelswarm-session/src/spec.rs:306-315`) | **BACKLOG** (receipts-v2 ADR pending; deferral recorded only in a code comment) |
| P2P error codes | §6.5:304-307 registry incl. `replayed_request`, `overloaded` | — | Serving emits `duplicate_request` (serving.rs:292-299), `over_limit` (:76), `bad_frame` (:264), `executor_error` (:370); `invalid_lease` is ADR-026-covered (ADR-026:32) but absent from §6.5 | **ADR-REQUIRED** — rename `duplicate_request`→`replayed_request`, `over_limit`→`overloaded`, or amend §6.5; code-matching clients break |
| §6.1 handshake frames | handshake → handshake_ack → inference_request | Handshake 9 fields | Libp2p backend authenticates via QUIC+Noise PeerId; **no application Handshake exchanged** (serving.rs reads `InferenceRequest` first; remote.rs sends it immediately after dial) — an implementer following §6.1 verbatim gets `expected InferenceRequest, got Handshake` | **ADR-REQUIRED** (ADR-018 covers the staging rationale; the libp2p-path skip was never recorded) |
| §6.4 sampling clamps at serving peer | temperature ∈[0,2], top_p∈(0,1], top_k∈[1,200] "clamped, not rejected" | — | `clamp_wire_request` clamps only deadline/maxTokens/prompt-bytes (serving.rs:103-114); sampling passes through (gateway does clamp fully at local HTTP entry, `modelswarm-gateway/src/http.rs:214-284`) | **BACKLOG** (spec-conformance gap on the wire side) |
| §6.4 `maxTokens ∈ [1, profile.maxOutputTokens]` | per-profile cap | — | Clamps to global `DEFAULT_MAX_OUTPUT_TOKENS`; v2 manifest has **no** `maxOutputTokens`/`contextTokens` (dropped with schema v1) | **DOC-ONLY** (spec references a dead v1 field) |
| `Control` ping/pong | Not in spec | Not in proto | Implemented, self-declared transport bookkeeping (message.rs:162-175) | **DOC-ONLY** |
| Handshake signed payload, requestId single-use (10 min), ±120 s skew | §6.6 | fields 1..8 | Matches (handshake.rs:64-77; Admission 600 s dedup serving.rs:222-238; `EXPIRY_CLOCK_SKEW_SECS=120` lease.rs:17) | Match |
| Cooperative `SpecMessage` vocabulary | No spec (msp-cooperative-v1.md does not exist; expanded-mission §4.2:96-99 confirms) | — | Full signed namespace shipped (`modelswarm-session/src/spec.rs:206-339`); PrefixCommit binds session/round/prev+new hash/profile/params/sender/signature — but **not** protocol version, nonce, or deadline in the signed payload | **ADR-REQUIRED** (documentation debt; noted under risks for the future ADR) |

### 2c. Doc drift

| Where | Says | Reality | Verdict |
|---|---|---|---|
| `docs/architecture.md:148` (§9) | "No relay exists in v0.1 (ADR-003)" | `crates/modelswarm-relay` shipped; F2A relay verified 2026-10-08 (`docs/verification/f2a-relay-2026-10-08.md:41-60`) | **DOC-ONLY** |
| `docs/architecture.md:219-224` (§15) | Decisions index stops at ADR-008 | 25 ADRs exist (001–015, 017–026) | **DOC-ONLY** |
| `docs/architecture.md:124-128` (§7) | Serving peer verifies "…and nonce" per request | Nonce is issuance-dedup only; per-request replay = `requestId` (msp-v1 §5:243-245) | **DOC-ONLY** |
| msp-v1.md freeze discipline | "changes require an ADR" (line 3-4); **no changelog section exists** | Amendments folded inline with labels (§3.1/3.5 ADR-023; §5 ADR-012 + vector; §3.2 auto-approval dated 2026-10-06 with threat-model pointer but no ADR number); the §2b/2a gaps above were never folded | **DOC-ONLY** (process gap) |
| messages.proto:5-6, msp-v1 §6:252-254 | Wire encoding "fixed in the Phase 3 opening ADR" | JSON framing chosen by ADR-018 staging (amended Phase C); libp2p path still JSON; proto header never updated | **DOC-ONLY** |
| schema-v2.json gating | — | Created by ADR-011; amended twice by ADR-022 with parity-lock digests (`ADR-022:139-178`) | **Properly ADR-gated** — not a parallel evolution; v1 `schema.json` is dead-but-referenced (see risks) |

## 3. ADR coverage status

| ADR | Status | Evidence / divergence |
|---|---|---|
| 001–010 | Implemented | Content-blindness, sidecar, discovery, identity, immutability, tokens, retry, packaging, positioning, restructure — all reflected in code/tests; no divergence found |
| 011 manifest-derived profile id | **Implemented** (chunk challenge missing) | Rust derivation + golden test (`modelswarm-types/src/manifest.rs:549-583`); schema-v2; resolver. Possession #2 (randomized chunk challenge, ADR-011:46-48) **not implemented** anywhere (grep: zero hits in tracker + crates); #1 full-sha and #3 live-inference challenge are implemented (`artifact.rs:103-110`, challenge routes) |
| 012 eligibility lease + suspension | **Implemented with two partials** | Lease fields/wire/cap/enforcement match end-to-end (§4 below). (a) Suspension policy table exists as pure Rust fn (`modelswarm-eligibility/src/policy.rs:82-116`) with **no feeder** — tracker never reduces slots on challenge refusal (grep `suspend` in apps/tracker: zero). (b) `verified_capacity` derives from **self-reported** timings only (`challenge/complete/route.ts:44-47`); ADR-012's "+ hardware evidence" half unimplemented. Bootstrap allowance OFF — correctly absent |
| 013 execution modes + cost model v2 | **Implemented as planned-phased** | Only `single` ships, per ADR; cost model v2 frozen in scheduler/bench; telemetry vocabulary in experiments. Registry has **no wire presence** (correct today) but tracker `session-authorize` accepts any mode string — no registry validation (divergence from registry discipline, ADR-REQUIRED) |
| 014 NAT traversal roadmap | **Implemented (Phase F delivered early)** | Relay + DCUtR stack shipped and verified F2A; tracker signaling-only boundary held |
| 015 supersession map | Implemented | A–G (+H, I, F0-F3) docs exist under docs/verification/ |
| 018 transport staging | **Implemented, ahead of text** | SignedFrame staging honored; libp2p backend shipped (Phase F); non-loopback denial tested. The libp2p-path handshake skip (§2b) exceeds the ADR's recorded scope |
| 020 identity derivations | Implemented | Corrected multihash derivation, hard switch, cross-language golden (TS `crypto-vectors.test.ts:60-71`, enrollment rejection test `enrollment.test.ts:262-283`) |
| 022 GGUF-anchored hashes | Implemented | GGUF reader + re-derivation tests + amendments 1/2 parity locks; 15 candidate profiles present |
| 023 community model requests | Implemented | Routes + migration 0003 + strict GGUF-pointer schema match ADR exactly |
| 024 GPU engine variant | Implemented | Backend-invariant wire runtime descriptor held (`runtime {name, build_hash}` unchanged); forward constraint (P0+ verifier-class eligibility) correctly not yet exercised |
| 025 chat-template application point | Implemented | Serving executor applies ChatML once; desktop client-side render removed |
| 026 lease gate at session open | **Implemented, fully** | See §4 |

## 4. ADR-026 lease gate + earn chain — end-to-end wire parity

**Wire shape identical.** The signed field set is byte-pinned by `protocol/vectors/lease-hubkey-1.json` (token, `canonical_json_sha256`, signing seed/pubkey, `verify_at`, `fields`): TS issuer (`apps/tracker/lib/eligibility.ts:56-86`, canonical JSON, `base64url(json).base64url(sig)`, `expires_at = lease+60 s` exactly at cap) ↔ Rust verifier (`modelswarm-eligibility/src/lease.rs:240-274` `from_wire`/`to_wire`, serde form = signed payload via `skip_serializing` issuer_signature). Gate checks match the ADR's five conditions in order (`modelswarm-node/src/serving.rs:970-1000`): pinned hub key (`protocol/keys/hub-public.hex`, include_str at :939), freshness+lease-cap (lease.rs:197-233), profile match, `can_consume && slots>=1`, QUIC-authenticated PeerId binding. Refusal is `invalid_lease` before the executor, tested by the ADR's named tests (`gate_accepts_own_and_rejects_foreign_binding`, `serving_bridge_refuses_leaseless_requests`, `serving_bridge_refuses_foreign_peer_lease`, serving.rs:441-451, 699-863).

**Earn chain:** desktop `earn_lease` completes register→real-engine-timed challenge→`request_lease`→cache→present (`crates/modelswarm-desktop/src/app.rs:1281-1393`) — ahead of ADR-026's "harness-tested up to the challenge" phrasing (fine; phased).

**CI home:** `.github/workflows/tracker.yml` job `wire-compat` (lines 72-146): real Next.js tracker + Postgres container, `MSP_HUB_SEED` = the vector's fixture seed, seeds a real candidate profile via admin routes, then runs `cargo test -p modelswarm-node --features libp2p-backend --test live_tracker -- --ignored` which asserts enroll→register→heartbeat→lookup→challenge_start→challenge_complete→`request_lease` parses→`LeasePolicy::check` passes on the TS-issued token (`crates/modelswarm-node/tests/live_tracker.rs:119-145`). Plus static byte-parity on both sides: Rust `golden_vector_token_from_ts_issuer_verifies` (lease.rs:591-627, incl. tamper) and TS `crypto-vectors.test.ts:196-267` (signature, byte-exact canonical JSON + sha256 pin, re-issuance parity minus nonce).

## 5. ADR-016 absence — explanation

ADR-016 was a **conditionally reserved slot never issued**. Phase A's first response reserved it for "p2ptokens reuse decisions, if any concrete module is a clear net win" (`docs/reviews/phase-a-first-response.md:94,116`). The Phase A verification closed it: "p2ptokens decision: differentiate + selective study, no fork/interop … decided; ADR-016 not needed (no code reuse adopted)" and "ADR-016 (conditional p2ptokens code reuse) closed as not-needed" (`docs/verification/phase-a.md:23,59`). Numbering then continued at 017 (ADR-017:1-3 dated Phase B, 2026-10-05). Not a lost document — a deliberately skipped number; recommend a one-line note in the ADR index so future audits don't re-investigate.

## 6. Preset-layer readiness (master prompt §3)

**msp-v1.md has no session-offer/negotiation surface today.** No `mode`/`preset` field exists in `messages.proto`, `InferenceRequest`, or `Handshake`; `SessionOffer` exists only as a proposal (expanded-mission §4.2:98; gap study §2 evidence, lines 127-131). The nearest live surface is the tracker's `session-authorize` (`{peer_ids, profile_id, mode}`) — tracker-side, metadata-only, **unconsumed by any Rust code**, with `mode` an unvalidated free string. Receipts: v1 spec fields (§7) unimplemented on the single-mode wire; the cooperative receipt carries only `{session_id, rounds, committed_tokens}`; tracker intake stores `{receiptDigest, outcome}` with the digest's canonical preimage **undefined anywhere** — receipts-v2 (modeId, cohort digest, roles, accepted/verified tokens, endpoint-seconds; expanded-mission §4.3:103-106) is the vehicle the master prompt assumes.

**Additive-and-freezable:** preset enum + budgets on a *new* `SessionOffer` inside the still-unwritten `msp-cooperative-v1.md` namespace (msp-v1/messages.proto stay byte-identical); tightening `session-authorize.mode` to a registered enum (existing values preserved); receipt scalar additions via receipts-v2 (new signed object ⇒ new version, old verifiers unaffected); lease additive fields per expanded-mission §4.4 with a new golden vector. **Breaking if done wrong:** adding preset to frozen `InferenceRequest`/`Handshake`; overwriting the §7 receipt shape in place; changing `JobResultSchema.intake` semantics.

**Exact spec touchpoints an ADR must touch:** (1) ADR-013 extension (preset table, per-tier budget + correctness label + cohort cap + recipient-set disclosure, HYBRID/MAXIMUM composition rules); (2) creation of `protocol/msp-cooperative-v1.md` defining `SessionOffer{preset, budget}` + mode-resolution recording (currently dangling: AGENTS.md already forbids invented fields outside it while `SpecMessage` ships); (3) receipts-v2 ADR amending msp-v1 §7 + messages.proto `JobReceipt` + defining the canonical receipt digest that `/events/job-result`'s `receiptDigest` hashes; (4) tracker `SessionAuthorizeSchema.mode` → enum + msp-v1 §3 absorption of the route; (5) `docs/privacy.md` per-preset recipient-set disclosure wording; (6) golden vectors (preset enum serialization, receipts-v2 canonical form); (7) state diagrams for preset negotiation/fallback (none exist in msp-v1 §8, which is sequences only).

## 7. Golden-vector inventory + gaps

| Vector | Pins | CI consumer |
|---|---|---|
| `protocol/vectors/manifest-basic.json`, `manifest-no-spec.json` | synthetic manifests + expected_profile_id | TS `npm run validate:vectors` (tracker.yml `check`) + Rust `golden_vectors_derive_expected_profile_ids` (types; rust.yml `cargo test --workspace`) |
| `manifest-qwen25-05b-q4km-real.json`, `manifest-smollm2-135m-q4km-real.json`, `manifest-smollm2-360m-q8-real.json` | real-artifact manifests + ids (ADR-022 §8) | same |
| `protocol/vectors/lease-hubkey-1.json` | full lease token bytes, canonical sha256, fixture key, verify_at, fields | Rust lease.rs golden + TS crypto-vectors.test.ts + wire-compat live chain (§4) |
| `protocol/keys/hub-public.hex` | pinned hub key | include_str in serving.rs:939; catalog verify |
| ADR-020 peer-id derivation golden | seed→12D3Koo form | lives in test suites (crypto-vectors.test.ts:60-71), **not** protocol/vectors |

**Gaps:** no vector for (a) §7 job receipt (unimplemented), (b) handshake signed payload, (c) catalog-envelope signature form, (d) §2.3 envelope canonical bytes (covered only by the live wire-compat harness — acceptable but not static), (e) SpecMessage signed commit/receipt (Rust-only today; becomes mandatory the moment TS or a second implementation touches the cooperative namespace), (f) relay/DCUtR signaling metadata, (g) `revoked_tokens`/notice shapes.

## 8. Top 5 recommended actions (severity-ordered)

1. **Retroactive ADR for the unlabeled wire additions** (one "tracker surface reconciliation" ADR): fold `/peers/lease`, `session-authorize`, `/receipt`, `/audit`, `capacityClass` into msp-v1 §3 (or explicitly mark `/stats`,`/download` non-protocol website surface), and constrain `session-authorize.mode` to the ADR-013 registry. These are live production routes a spec implementer cannot discover. (Files: `docs/adr/`, `protocol/msp-v1.md`, `apps/tracker/lib/schemas.ts`.)
2. **Fix the P2P error-code divergence** (`serving.rs` `duplicate_request`→`replayed_request`, `over_limit`→`overloaded`, decide `bad_frame`/`executor_error`/`invalid_lease` registry entries via the same ADR or a §6.5 amendment). Code-matching clients break today; it is the only client-visible wire mismatch found.
3. **Record the two §6 deferrals explicitly**: (a) `accepted`/queue vocabulary — either emit it or ADR-defer it (fair-queueing M10 depends on it); (b) §7 receipts + the undefined `receiptDigest` preimage — fold into the receipts-v2 ADR with the digest definition. Add a **changelog section to msp-v1.md** listing every post-freeze amendment with its ADR.
4. **Doc-sync pass**: architecture.md §9 relay line, §15 index (001–026 incl. "016 reserved-not-issued"), §7 nonce wording; msp-v1 §3.3 table (`/peers/lease`, challenge/complete result), §3.5/§4 schema.json→schema-v2 references, §6.4 `profile.maxOutputTokens` dead-field reference; ADR-026 "up to the challenge" phrasing.
5. **Backlog hardening, non-blocking**: wire ADR-012's suspension table into the tracker (challenge-refusal events → slots-to-zero) since the policy fn currently has no caller; implement ADR-011 chunk challenges or descope them in an amendment; retire/rename `modelswarm-types::ModelProfileId` (validates only the dead v1 `msp:` shape — a type-level trap; real ids are untyped strings, lease.rs has its own `msp1:` validator); add sampling clamps to `clamp_wire_request` per §6.4.

---

**Status:** complete; read-only audit, no files modified by the architect.

**Risks:** (1) `verified_capacity` inflatable via self-reported challenge timings (gpu_high at ≤2 s claim) — scheduling-hint-only impact today, must not gate anything until hardware evidence lands; (2) cooperative `PrefixCommit` signature payload omits protocol version/nonce/deadline — bind them in the future cooperative ADR per the state-changing-message binding rule; (3) `msp-cooperative-v1.md` dangling reference while `SpecMessage` is shipped code; (4) rust CI was red at freeze on the tauri-shell step (fixed in `63b98e1`, watcher pending per freeze record).

**Assumptions:** git-history claims (silent-mutation check, ADR-016 history) rely on in-repo documents, not `git log` — the architect session exposed read-only tools only; the inline ADR labels in msp-v1.md were treated as the amendment record. Freeze-record migration hashes accepted as given.

**Handoff:** Integrator — sequence actions 1–2 (single reconciliation ADR + error-code fix) before any M-phase protocol work; route action 5's suspension wiring to Tracker Engineer with Security review, and the msp-v1 changelog addition to the Protocol Architect once write access is granted.
