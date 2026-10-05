import { bs58Decode, derivePeerId, randomId128 } from "@/lib/crypto";
import { jsonError, jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { PeerRegisterSchema } from "@/lib/schemas";
import { LEASE_TTL_MS } from "@/lib/constants";
import type { LeaseRecord } from "@/lib/store";

// POST /api/v1/peers/register (msp-v1 §3.3). Signed envelope + session.
// Validation: peerId is the ADR-020 identity-multihash derivation of the
// enrolled pubKey (base58(0x12 0x20 ‖ sha256(pubKey)) — the "12D3Koo…" form;
// the Phase B–E placeholder equality with installationId is retired);
// addresses parse as (light) multiaddrs; every profile exists and is active;
// maxSlots ∈ [1,8]. Issues a 128-bit random leaseId bound to the signing
// installation.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, PeerRegisterSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;

  const installation = await ctx.store.getInstallation(installationId);
  if (!installation) {
    return jsonError(401, "unknown_installation", "installation is not enrolled");
  }
  const pubBytes = bs58Decode(installation.pubKeyB58);
  if (!pubBytes) {
    return jsonError(401, "unauthorized", "installation public key unusable");
  }
  if (derivePeerId(pubBytes) !== body.peerId) {
    return jsonErrorFor("invalid_body", "peerId must be the multihash derivation of the installation pubKey (ADR-020)");
  }
  if (await ctx.store.isPeerBlocked(body.peerId)) {
    return jsonError(403, "forbidden", "peer is blocked");
  }
  for (const profileId of body.profiles) {
    const profile = await ctx.store.getProfile(profileId);
    if (!profile || profile.status !== "active") {
      return jsonError(400, "unknown_profile", `profile ${profileId} is not in the active catalog`);
    }
  }

  const now = ctx.now();
  const leaseId = randomId128();
  const lease: LeaseRecord = {
    leaseId,
    installationId,
    peerId: body.peerId,
    createdAt: now,
    expiresAt: now + LEASE_TTL_MS,
    profiles: body.profiles,
    addresses: body.addresses,
    maxSlots: body.maxSlots,
    freeSlots: body.maxSlots,
    queueMs: 0,
    draining: false,
    capacityClass: "cpu",
    auditEpoch: await ctx.store.peerEpoch(body.peerId),
    lastSeenAt: now,
  };
  await ctx.store.createLease(lease);
  return jsonOk({ leaseId, leaseExpiresAt: new Date(lease.expiresAt).toISOString() });
}
