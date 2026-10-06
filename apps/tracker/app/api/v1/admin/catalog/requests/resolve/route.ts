import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardAdminBody, requireAdmin } from "@/lib/guard";
import { ModelRequestResolveSchema } from "@/lib/schemas";

// POST /api/v1/admin/catalog/requests/resolve — mark a model request
// resolved ("promoted" after the candidate is live, "rejected" otherwise).
// Closing the loop keeps the queue honest: the desktop can later show a
// request's outcome once a status read is added.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const denied = requireAdmin(req, getTrackerContext());
  if (denied) return denied;

  const guarded = await guardAdminBody(req, ModelRequestResolveSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const ok = await ctx.store.resolveModelRequest(body.id, body.resolution);
  if (!ok) {
    return jsonError(404, "unknown_request", `no open request ${body.id}`);
  }
  return jsonOk({ id: body.id, resolution: body.resolution });
}
