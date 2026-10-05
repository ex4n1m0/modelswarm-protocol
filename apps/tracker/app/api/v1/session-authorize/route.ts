import { randomId128 } from "@/lib/crypto";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { SessionAuthorizeSchema } from "@/lib/schemas";
import { SESSION_AUTH_TTL_MS } from "@/lib/constants";

// POST /api/v1/session-authorize — Phase B cooperative session authorization
// (DESIGN.md). Metadata ONLY: {session_id, peer_ids[], profile_id, mode}. The
// schema structurally cannot carry prompt-shaped fields (F4 invariant).
// Every roster member must hold an active, non-draining lease covering the
// profile; otherwise 403 ineligible.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, SessionAuthorizeSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;
  const now = ctx.now();

  const profile = await ctx.store.getProfile(body.profile_id);
  if (!profile || profile.status !== "active") {
    return jsonError(400, "unknown_profile", `profile ${body.profile_id} is not in the active catalog`);
  }

  for (const peerId of body.peer_ids) {
    const lease = await ctx.store.findActiveLeaseByPeer(peerId);
    const eligible =
      lease !== null &&
      !lease.draining &&
      lease.profiles.includes(body.profile_id) &&
      !(await ctx.store.isPeerBlocked(peerId));
    if (!eligible) {
      return jsonError(403, "ineligible", `peer ${peerId} has no active lease for this profile`);
    }
  }

  const sessionId = randomId128();
  await ctx.store.recordSessionAuthorization({
    sessionId,
    peerIds: body.peer_ids,
    profileId: body.profile_id,
    mode: body.mode,
    createdAt: now,
    expiresAt: now + SESSION_AUTH_TTL_MS,
  });

  return jsonOk(
    {
      session_id: sessionId,
      peer_ids: body.peer_ids,
      profile_id: body.profile_id,
      mode: body.mode,
    },
    201,
  );
}
