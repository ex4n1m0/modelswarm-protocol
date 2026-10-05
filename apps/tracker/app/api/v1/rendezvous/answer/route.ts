import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { RendezvousAnswerSchema } from "@/lib/schemas";

// POST /api/v1/rendezvous/answer (msp-v1 §3.3): mirror of /offer for answers.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, RendezvousAnswerSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;

  const senderLease = await ctx.store.findActiveLeaseByInstallation(installationId);
  if (!senderLease) {
    return jsonError(403, "ineligible", "no active lease for the sending installation");
  }
  await ctx.store.pushRendezvous({
    toPeerId: body.toPeerId,
    fromPeerId: senderLease.peerId,
    kind: "answer",
    payload: body.answer,
    receivedAt: ctx.now(),
  });
  return jsonOk({ accepted: true });
}
