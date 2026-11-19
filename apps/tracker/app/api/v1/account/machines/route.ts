// GET /api/v1/account/machines — the signed-in account's bound machines
// ("your machines"): newest first, both active and revoked rows.

import { jsonOk } from "@/lib/errors";
import { guardAccountRequest } from "@/lib/guard";
import { requireAccountUser } from "@/lib/account-auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request): Promise<Response> {
  const guarded = await guardAccountRequest(req);
  if (!guarded.ok) return guarded.response;
  const auth = await requireAccountUser(req, guarded.ctx);
  if (!auth.ok) return auth.response;

  const machines = await guarded.ctx.store.listAccountMachines(auth.userId);
  return jsonOk({
    machines: machines.map((m) => ({
      installationId: m.installationId,
      label: m.label,
      status: m.status,
      addedAt: new Date(m.addedAt).toISOString(),
      approvedAt: m.approvedAt === null ? null : new Date(m.approvedAt).toISOString(),
      lastSeenAt: m.lastSeenAt === null ? null : new Date(m.lastSeenAt).toISOString(),
    })),
  });
}
