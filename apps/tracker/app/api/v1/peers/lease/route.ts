import { issueEligibilityLease, queueWeightClass } from "@/lib/eligibility";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { PeerLeaseSchema } from "@/lib/schemas";
import type { LeaseRecord, TrackerStore } from "@/lib/store";
import { balanceDecision, balanceEnforced, balanceWindowSecs, cloudBalanceEnforced } from "@/lib/balance";
import { DEFAULT_FAIR_WEIGHT_T2, DEFAULT_FAIR_WEIGHT_T3, GRANTED_HALF_LIFE_SECS } from "@/lib/constants";

// POST /api/v1/peers/lease — consolidated EligibilityLease issuance/refresh
// (ADR-012; Phase B per DESIGN.md). Requires: active lease bound to the
// signing installation + a passed hosting challenge for the same lease+profile
// (else 403 no_capability) + not draining (403 ineligible) + a current audit
// epoch (stale epoch -> 403 ineligible). Token expiry is capped at
// leaseExpiresAt + 60 s. Serialized: base64url(json) "." base64url(sig).
//
// ADR-042 (owner ruling 2026-10-17 night) amends the capability rule: a
// passed hosting challenge for ANY profile this lease actively hosts (the
// capacity stake — local or cloud) grants cross-profile consumption of the
// whole swarm; ADR-040's provider-key stake is retired as a consumption
// gate. Cloud-family leases also enforce the swarm balance BY DEFAULT
// (born limited); local-family leases keep ADR-041's dark default.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, PeerLeaseSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;
  const now = ctx.now();

  const lease = await ctx.store.getLease(body.leaseId);
  const binding = leaseBindingError(lease, installationId, now);
  if (binding || !lease) return binding ?? jsonOk({});

  const profile = await ctx.store.getProfile(body.profileId);
  if (!profile || profile.status !== "active") {
    return jsonError(400, "unknown_profile", `profile ${body.profileId} is not in the active catalog`);
  }
  if (lease.draining) {
    return jsonError(403, "ineligible", "lease is draining; capability issuance suspended");
  }
  if (await ctx.store.isPeerBlocked(lease.peerId)) {
    return jsonError(403, "forbidden", "peer is blocked");
  }
  // ADR-012 suspension (row-aging: an aged-out window recovers right here).
  await ctx.store.recoverSuspensionIfDue(lease.peerId, now);
  const suspension = await ctx.store.getSuspension(lease.peerId);
  if (suspension && suspension.untilAt > now) {
    return jsonError(
      403,
      "ineligible",
      `peer is suspended until ${new Date(suspension.untilAt).toISOString()} (${suspension.reason})`,
    );
  }
  if (lease.maxSlots < 1) {
    return jsonError(403, "ineligible", "no serving slot available");
  }

  const challenge = await ctx.store.getPassedChallenge(body.leaseId, body.profileId);
  const targetIsCloud = (profile.manifest as { family?: string }).family === "cloud";
  const challengeRec = challenge ?? (await capacityStakeChallenge(ctx, lease)) ?? null;
  if (!challengeRec) {
    return jsonError(403, "no_capability", "no passed hosting challenge for this lease+profile");
  }

  const auditEpoch = await ctx.store.peerEpoch(lease.peerId);
  if (lease.auditEpoch < auditEpoch) {
    return jsonError(403, "ineligible", "audit epoch stale; re-run the hosting challenge");
  }

  // ADR-041 swarm balance, CAPACITY rule (owner revision 2026-10-17: the
  // torrent lesson — the gate keys on what your verified hardware+model
  // can push, never on whether anyone pulled from you). ADR-042 §2: the
  // anchor is the machine's OWN receipt-backed trailing served tok/s
  // (class table only as cold-start fallback). Dark for ordinary leases;
  // ON by default for cloud leases (§3 — the capacity-stake path is born
  // limited).
  const balance = balanceDecision(
    lease.capacityClass,
    await ctx.store.consumedInWindow(installationId, balanceWindowSecs()),
    await ctx.store.servedThroughputInWindow(lease.peerId, balanceWindowSecs()),
  );
  if (!balance.ok && (balanceEnforced() || (targetIsCloud && cloudBalanceEnforced()))) {
    return jsonError(
      403,
      "balance_exceeded",
      `swarm balance exceeded: consumed ${Math.round(balance.consumedInWindow)} of ${Math.round(balance.budget)} tokens in the last ${Math.round(balance.windowSecs / 60)} min (capacity ${balance.capacityTokensPerSec} tok/s, k=${balance.k}); the window refills continuously`,
    );
  }

  // ADR-043 fair queueing: the hub-signed queue weight class, bucketed from
  // the peer's OWN receipt-backed decayed served tok/s (granted accounting,
  // 7-day half-life). Serialized into the lease ONLY when > 1, so base
  // leases stay byte-identical to pre-ADR-043 tokens. Thresholds are
  // operator policy (env, defaults 1 / 5 decayed tok/s).
  const granted = await ctx.store.grantedCounters(lease.peerId, GRANTED_HALF_LIFE_SECS);
  const servedTokPerSec =
    granted.endpointSecondsDecayed > 0
      ? granted.verifiedTokensDecayed / granted.endpointSecondsDecayed
      : 0;
  const queueWeight = queueWeightClass(
    servedTokPerSec,
    fairThreshold(process.env.MSP_FAIR_WEIGHT_T2, DEFAULT_FAIR_WEIGHT_T2),
    fairThreshold(process.env.MSP_FAIR_WEIGHT_T3, DEFAULT_FAIR_WEIGHT_T3),
  );

  const issued = issueEligibilityLease(
    lease,
    challengeRec,
    body.profileId,
    auditEpoch,
    now,
    ctx.keys.secretKey,
    queueWeight,
  );

  await ctx.store.recordToken({
    nonce: issued.fields.nonce,
    leaseId: lease.leaseId,
    peerId: lease.peerId,
    installationId: lease.installationId,
    profileId: body.profileId,
    challengeId: challengeRec.challengeId,
    canHost: issued.fields.can_host,
    canConsume: issued.fields.can_consume,
    slots: issued.fields.slots,
    capacityClass: issued.fields.verified_capacity,
    auditEpoch: issued.fields.audit_epoch,
    issuedAt: now,
    expiresAt: lease.expiresAt + 60_000,
  });

  return jsonOk({ token: issued.token, lease: issued.fields, balance });
}

/// ADR-042 (owner ruling 2026-10-17 night) — the CAPACITY stake, which
/// retires ADR-040 §3 path (b)'s key stake: an installation ACTIVELY
/// hosting ANY profile (local or cloud — it registered the profile on
/// this lease and passed that profile's hosting challenge, the serving
/// proof) may consume ANY profile in the swarm, not only the ones it
/// hosts ("all as one": local hosts pull cloud keys; cloud hosts keep
/// pulling local models exactly as ADR-040 path (b) already did live).
/// The token itself stays bound to the requested profile + this peer
/// exactly as path (a) tokens are.
async function capacityStakeChallenge(
  ctx: { store: Pick<TrackerStore, "getPassedChallenge"> },
  lease: LeaseRecord,
) {
  for (const profileId of lease.profiles) {
    const challenge = await ctx.store.getPassedChallenge(lease.leaseId, profileId);
    if (challenge) return challenge;
  }
  return undefined;
}

/// ADR-043: env threshold parse — a missing or non-positive value falls back
/// to the frozen default (never 0, which would make every valid lease a
/// 2×-weighted contributor).
function fairThreshold(raw: string | undefined, fallback: number): number {
  const parsed = Number.parseFloat(raw ?? "");
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
