import { issueEligibilityLease } from "@/lib/eligibility";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { PeerLeaseSchema } from "@/lib/schemas";

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
  if (lease.maxSlots < 1) {
    return jsonError(403, "ineligible", "no serving slot available");
  }

  const challenge = await ctx.store.getPassedChallenge(body.leaseId, body.profileId);
  if (!challenge) {
    return jsonError(403, "no_capability", "no passed hosting challenge for this lease+profile");
  }

  const auditEpoch = await ctx.store.peerEpoch(lease.peerId);
  if (lease.auditEpoch < auditEpoch) {
    return jsonError(403, "ineligible", "audit epoch stale; re-run the hosting challenge");
  }

  const issued = issueEligibilityLease(
    lease,
    challenge,
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
    challengeId: challenge.challengeId,
    canHost: issued.fields.can_host,
    canConsume: issued.fields.can_consume,
    slots: issued.fields.slots,
    capacityClass: issued.fields.verified_capacity,
    auditEpoch: issued.fields.audit_epoch,
    issuedAt: now,
    expiresAt: lease.expiresAt + 60_000,
  });

  return jsonOk({ token: issued.token, lease: issued.fields });
}
