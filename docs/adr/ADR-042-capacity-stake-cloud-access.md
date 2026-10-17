# ADR-042: Capacity-stake cloud access — hosts pull cloud at their measured tok/s

Status: Accepted (owner ruling 2026-10-17 night: "local models pull from
cloud keys with the token per second limits of their local machine …
the cost is not a consideration since users can choose … we should
leverage these much more advanced models to the swarm performance") ·
Amends ADR-040 (§cloud stake retired as a consumption gate), refines
ADR-041 (§capacity source), and extends the cooperative roster to
cloud profiles.

## Context

The 2026-10-17 live mixed-swarm test proved the one-way shape: A (a
deepseek key holder, cloud stake) pulls B's local Qwen, but B — an
active, verified local host — is refused cloud-profile leases with
`403 no_capability` because ADR-040's stake demands a provider key.
The owner ruled the stake's cost-protection purpose retired: a user
who chooses a cloud model accepts its costs, and a peer that stages a
key and advertises a cloud slot has consented to serve the swarm.
What must survive is capacity fairness: a machine may pull cloud at
the throughput its own machine can actually serve. The owner also
restated the founding direction — all models cross-check answers and
work as one — now explicitly including cloud models as first-class
swarm members, to be leveraged for swarm performance, not quarantined
behind a payment wall.

## Decision

1. **Eligibility (amends ADR-040).** Any peer with a verified, active
   serving slot for ANY profile (local or cloud) may consume leases for
   ANY profile in the swarm — cross-profile consumption granted by the
   capacity stake. This preserves ADR-040 path (b)'s proven live shape
   (a cloud host consuming a local family profile) AND opens the ruled
   direction (a local host pulling cloud keys with no key staged). The
   provider-key stake is retired as a consumption gate; serving is the
   stake. Exact-profile hosting (same P) remains the always-available
   base path.

2. **Capacity source (refines ADR-041 §1).** The allowance anchor is
   the machine's OWN measured serving throughput: trailing served
   tok/s computed from intake-validated receipts (the ledger both
   sides already write), with the verified CapacityClass table as the
   cold-start fallback until the peer's first served receipt. k and
   WINDOW unchanged (1.0 / 1 h; `MSP_SWARM_BALANCE_K`,
   `MSP_SWARM_BALANCE_WINDOW_SECS`).

3. **Enforcement is ON for cloud-profile leases.** The new local→cloud
   access path is born limited — a cloud lease request over the
   machine's window budget is refused `403 balance_exceeded` with its
   actual numbers. Local-family leases keep ADR-041's dark default
   until its own deliberate flip.

4. **Consent model.** Staging a provider key + advertising a cloud
   slot = consent to serve the swarm. Consuming cloud = the user's
   informed model choice. The tracker never sees keys; it continues
   to judge by advertised slots and receipts only. Cost protection is
   explicitly NOT a protocol goal (owner ruling, this ADR).

5. **Cooperative roster — "all as one".** Cloud profiles are
   first-class members of the cooperative track: cross-checking,
   verification, and later speculative/ensemble modes mix local and
   cloud models under the same honest-label and preset-disclosure
   rules (output-changing modes need their own ADR — unchanged). The
   v0.1 "no token-level distributed inference" line stands until the
   cooperative phases ship it properly; this ADR changes WHO is in
   the swarm, not HOW tokens are produced.

6. **AGENTS.md project rule** is amended accordingly: active hosting
   of any profile with an advertised slot grants consumption of the
   swarm's cloud profiles, bounded by ADR-041/042 capacity.

## Consequences

+ A local-only machine (the household's B) pulls deepseek through a
  serving peer's key with zero key staging — the two-way mixed swarm.
+ Fairness is capacity-native and receipt-verifiable; unpopular-
  profile hosts are never penalized (ADR-041's torrent lesson kept).
+ Cloud models join the cross-check/verified presets — the
  performance leverage the owner ordered, on the existing receipts
  and measurement base.
− A serving peer's key funds other machines' cloud consumption,
  bounded only by their measured capacity; accepted by owner ruling.
− Tracker work item: the ledger needs a per-peer trailing
  served-tok/s view for §2 before enforcement can use it (until then
  the CapacityClass fallback applies — coarser than the owner's
  measured-anchor intent; flag the gap honestly in the UI).
