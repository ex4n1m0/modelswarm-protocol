import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardPeerGet } from "@/lib/guard";

// GET /api/v1/peers?profile_id=…&limit=… (msp-v1 §3.3 + DESIGN capacity_class).
// Signed envelope + session over the empty body. Only live leases are listed:
// expired and draining peers are excluded by row-aging comparison, never by a
// background job. limit clamps to 50.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const MAX_LIMIT = 50;

export async function GET(req: Request) {
  const guarded = await guardPeerGet(req);
  if (!guarded.ok) return guarded.response;
  const { ctx } = guarded;

  const url = new URL(req.url);
  const profileId = url.searchParams.get("profile_id");
  if (profileId !== null && !/^msp1:[0-9a-f]{64}$/.test(profileId)) {
    return jsonErrorFor("invalid_body", "profile_id must be an msp1:<hex> profile id");
  }
  let limit = MAX_LIMIT;
  const rawLimit = url.searchParams.get("limit");
  if (rawLimit !== null) {
    const parsed = Number.parseInt(rawLimit, 10);
    if (!Number.isFinite(parsed) || parsed < 1) {
      return jsonErrorFor("invalid_body", "limit must be a positive integer");
    }
    limit = Math.min(parsed, MAX_LIMIT);
  }

  const peers = await ctx.store.listPeers({ profileId: profileId ?? undefined, limit });
  return jsonOk({
    peers: peers.map((p) => ({
      peerId: p.peerId,
      addresses: p.addresses,
      queueMs: p.queueMs,
      freeSlots: p.freeSlots,
      leaseExpiresAt: new Date(p.leaseExpiresAt).toISOString(),
      lastSeenAt: new Date(p.lastSeenAt).toISOString(),
      capacityClass: p.capacityClass,
    })),
  });
}
