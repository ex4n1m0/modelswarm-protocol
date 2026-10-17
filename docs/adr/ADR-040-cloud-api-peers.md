# ADR-040: Cloud API peers — provider keys as network-wide stake

Status: Accepted (owner ruling 2026-10-17, option B) · Supersedes the
2026-10-15 no-cloud ruling and the serving half of the 2026-10-13
advisory wall (the advisory plane itself stays advice-only); amends the
AGENTS.md project rule (exact-profile reciprocity gains a cloud-stake
clause).

## Context

The owner wants growth beyond GPU owners: a user who cannot host a local
model can join by contributing a cloud inference API key (DeepSeek or
OpenRouter — any OpenAI-compatible provider). The owner explicitly chose
**option B, network-wide stake**: a live cloud stake grants consume
eligibility across ALL profile families, not only cloud ones. Fairness
rules for that asymmetry (cloud-key consumers drawing on local GPU peers)
are DELIBERATELY DEFERRED (see §7) — the owner directed torrent-tracker-
style ratio management as the future mechanism, and accepted the interim
asymmetry.

This overturns, on the record:
- the 2026-10-15 "no cloud / never build the key-proxy" ruling,
- the 2026-10-13 "cloud never SERVES" wall **for the swarm serving
  plane** (the local advisory/assistant plane remains advice-only and
  never proxies chat — that wall stands),
- the strict form of the AGENTS.md project rule (amended below).

## Decision

1. **Cloud profiles are their own immutable profile family.** A cloud
   profile pins `provider ∈ {deepseek, openrouter}`, the provider's
   exact model id, and the context/sampling policy — no artifact digest
   exists, so provider+model+policy IS the exactness. A provider-side
   model revision change is a NEW profile (same discipline as local
   revision bumps). Cloud profiles appear in the catalog and the model
   picker with a cloud marker and NO download entry.

2. **A configured key makes the installation a serving peer.** The
   desktop app (only place a key ever lives — a local store file, never
   the hub, never telemetry, never logs) builds a cloud executor for the
   chosen cloud profile and the node advertises a serving slot for it.
   The peer counts as exactly 1 in the census, identical to local peers.

3. **Network-wide stake (option B).** The project rule becomes:

   > An installation may consume swarm inference while it is (a)
   > actively and verifiably hosting the same exact profile P with at
   > least one advertised serving slot, OR (b) actively serving at least
   > one cloud profile through a configured provider key with an
   > advertised slot.

   Path (a) is unchanged in every detail. Path (b) is the cloud stake.

4. **Honesty, always.** Every cloud-served reply carries the mode label
   `cloud — <provider> <model> (API)`; the UI discloses that prompts
   leave to that provider before the first cloud turn on a machine; the
   census counts cloud peers without distinction; receipts record cloud
   work with the same signed shape (the serving installation signs).

5. **Money never flows through ModelSwarm** (AGENTS.md constraint 8
   unchanged): each key owner pays their provider directly; ModelSwarm
   never intermediates, meters, or marks up. The owner accepts the API
   terms of the two default providers (DeepSeek, OpenRouter) for the
   product; ADDING further providers needs an owner gate.

6. **The existing discipline polices cloud peers unchanged.** A dead or
   exhausted key produces challenge timeouts/failures → the ADR-012
   suspension machinery applies verbatim; receipts/suspension/future
   reputation treat cloud and local peers identically.

7. **Fairness is deferred and ratio-shaped (future ADR-041).** The
   granted-accounting ledger (ADR-030) is the ratio substrate: granted
   verified tokens/seconds = the upload side; consumed tokens = the
   download side. Until ADR-041 lands rules (ratios, seeding grace,
   throttle tiers), cloud-stake consumption is unlimited but fully
   metered — both directions already produce signed ledger rows.

## Consequences

+ The network grows beyond GPU owners; every swarm gains a cloud on-ramp;
  the census reflects real contributing members.
+ Nothing new needs inventing for integrity: leases, challenges,
  suspension, receipts apply as-is.
− Local GPU peers carry cloud-key consumers' work with no ratio limit
  until ADR-041 (the owner accepted this interim asymmetry explicitly).
− Provider outages/ToS drift become swarm-visible failure modes; they
  surface through the same honest labels and suspension paths.
− The hub gains a new blind spot it must keep: provider keys. The
  tracker never sees one (registration carries only the cloud PROFILE
  id and the same peer/address data as local hosting).
