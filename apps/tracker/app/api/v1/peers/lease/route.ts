import { issueEligibilityLease } from "@/lib/eligibility";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { PeerLeaseSchema } from "@/lib/schemas";
import type { LeaseRecord, TrackerStore } from "@/lib/store";
import { balanceDecision, balanceEnforced, balanceWindowSecs } from "@/lib/balance";

// POST /api/v1/peers/lease — consolidated EligibilityLease issuance/refresh
// (ADR-012; Phase B per DESIGN.md). Requires: active lease bound to the
// signing installation + a passed hosting challenge for the same lease+profile
// (else 403 no_capability) + not draining (403 ineligible) + a current audit
// epoch (stale epoch -> 403 ineligible). Token expiry is capped at
// leaseExpiresAt + 60 s. Serialized: base64url(json) "." base64url(sig).

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
  const challengeRec = challenge ?? (await cloudStakeChallenge(ctx, lease)) ?? null;
  if (!challengeRec) {
    return jsonError(403, "no_capability", "no passed hosting challenge for this lease+profile");
  }

  const auditEpoch = await ctx.store.peerEpoch(lease.peerId);
  if (lease.auditEpoch < auditEpoch) {
    return jsonError(403, "ineligible", "audit epoch stale; re-run the hosting challenge");
  }

  // ADR-041 swarm balance, CAPACITY rule (owner revision 2026-10-17: the
  // torrent lesson — the gate keys on what your verified hardware+model
  // can push, never on whether anyone pulled from you). Ships DARK:
  // always computed and surfaced; blocks only under ENFORCE=on.
  const balance = balanceDecision(
    lease.capacityClass,
    await ctx.store.consumedInWindow(installationId, balanceWindowSecs()),
  );
  if (!balance.ok && balanceEnforced()) {
    return jsonError(
      403,
      "balance_exceeded",
      `swarm balance exceeded: consumed ${Math.round(balance.consumedInWindow)} of ${Math.round(balance.budget)} tokens in the last ${Math.round(balance.windowSecs / 60)} min (capacity ${balance.capacityTokensPerSec} tok/s, k=${balance.k}); the window refills continuously`,
    );
  }

  const issued = issueEligibilityLease(
    lease,
    challengeRec,
    body.profileId,
    auditEpoch,
    now,
    ctx.keys.secretKey,
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

/// ADR-040 §3 path (b) — the cloud stake: an installation ACTIVELY
/// serving a cloud profile (it registered that profile on this lease)
/// may consume ANY profile, not only the ones it hosts. The evidence it
/// must still show: a passed hosting challenge for one of ITS cloud
/// profiles (the serving proof). The token itself stays bound to the
/// requested profile + this peer exactly as path (a) tokens are.
async function cloudStakeChallenge(
  ctx: { store: Pick<TrackerStore, "getProfile" | "getPassedChallenge"> },
  lease: LeaseRecord,
) {
  for (const profileId of lease.profiles) {
    const profile = await ctx.store.getProfile(profileId);
    if (!profile || (profile.manifest as { family?: string }).family !== "cloud") continue;
    const challenge = await ctx.store.getPassedChallenge(lease.leaseId, profileId);
    if (challenge) return challenge;
  }
  return undefined;
}
