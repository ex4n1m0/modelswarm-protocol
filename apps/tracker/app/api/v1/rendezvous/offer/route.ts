import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { RendezvousOfferSchema } from "@/lib/schemas";

// POST /api/v1/rendezvous/offer (msp-v1 §3.3): store-and-forward signaling.
// The sender is the active lease of the signing installation; `offer` is an
// opaque <= 4 KiB string (never interpreted by the hub).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, RendezvousOfferSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;

  const senderLease = await ctx.store.findActiveLeaseByInstallation(installationId);
  if (!senderLease) {
    return jsonError(403, "ineligible", "no active lease for the sending installation");
  }
  await ctx.store.pushRendezvous({
    toPeerId: body.toPeerId,
    fromPeerId: senderLease.peerId,
    kind: "offer",
    payload: body.offer,
    receivedAt: ctx.now(),
  });
  return jsonOk({ accepted: true });
}
