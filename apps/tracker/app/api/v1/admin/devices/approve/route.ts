// POST /api/v1/admin/devices/approve — the /verify page hook: approve a
// device enrollment by its user-visible pairing code (msp-v1 §3.2 approval
// gate). Admin-token protected (X-MSP-Admin, same ops token as the other
// admin routes) so only the owner can let an installation join the roster.
import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonOk } from "@/lib/errors";
import { requireAdmin, guardAdminBody } from "@/lib/guard";
import { DeviceApproveSchema } from "@/lib/schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request): Promise<Response> {
  const denied = requireAdmin(req, getTrackerContext());
  if (denied) return denied;

  const guarded = await guardAdminBody(req, DeviceApproveSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const approved = await ctx.store.approveDeviceByUserCode(body.userCode);
  if (!approved) {
    return jsonError(404, "unknown_profile", "no pending device with that code", false);
  }
  return jsonOk({ userCode: body.userCode, approved: true });
}
