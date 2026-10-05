import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerGet } from "@/lib/guard";

// GET /api/v1/rendezvous/pending (msp-v1 §3.3): drains the caller's mailbox
// (items are deleted on read). Items carry the frozen triple
// {fromPeerId, offer, receivedAt} for offers; answers additionally carry an
// `answer` field (additive Phase B field — see handoff).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const guarded = await guardPeerGet(req);
  if (!guarded.ok) return guarded.response;
  const { ctx, installationId } = guarded;

  const lease = await ctx.store.findActiveLeaseByInstallation(installationId);
  if (!lease) {
    return jsonError(403, "ineligible", "no active lease for the calling installation");
  }
  const items = await ctx.store.drainRendezvous(lease.peerId);
  return jsonOk({
    items: items.map((item) => {
      const base: Record<string, unknown> = {
        fromPeerId: item.fromPeerId,
        receivedAt: new Date(item.receivedAt).toISOString(),
      };
      if (item.kind === "offer") base.offer = item.payload;
      else base.answer = item.payload;
      return base;
    }),
  });
}
