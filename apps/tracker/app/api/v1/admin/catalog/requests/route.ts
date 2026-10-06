import { getTrackerContext } from "@/lib/context";
import { jsonOk } from "@/lib/errors";
import { requireAdmin } from "@/lib/guard";

// GET /api/v1/admin/catalog/requests — the owner's model-request queue
// (ADR-023): open requests first (oldest first), then resolved ones.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const denied = requireAdmin(req, getTrackerContext());
  if (denied) return denied;
  const requests = await getTrackerContext().store.listModelRequests();
  return jsonOk({ requests });
}
