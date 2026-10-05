import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardAdminBody, requireAdmin } from "@/lib/guard";
import { PromoteSchema } from "@/lib/schemas";

// POST /api/v1/admin/catalog/promote (msp-v1 §3.5): candidate -> active.
// Bumps the catalog version and broadcasts a catalog_update notice to every
// live lease (drained on next heartbeat).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const denied = requireAdmin(req, getTrackerContext());
  if (denied) return denied;

  const guarded = await guardAdminBody(req, PromoteSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const promoted = await ctx.store.promoteProfile(body.profileId);
  if (!promoted) {
    return jsonError(404, "unknown_profile", `no profile ${body.profileId}`);
  }
  await ctx.store.pushNotice(
    { all: true },
    { type: "catalog_update", catalogVersion: await ctx.store.catalogVersion() },
  );
  return jsonOk({ profileId: body.profileId, status: "active" });
}
