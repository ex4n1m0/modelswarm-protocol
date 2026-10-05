import { getTrackerContext } from "@/lib/context";
import { jsonOk } from "@/lib/errors";
import { guardAdminBody, requireAdmin } from "@/lib/guard";
import { AuditSchema } from "@/lib/schemas";

// POST /api/v1/audit — Phase B admin endpoint (DESIGN.md): bumps the audit
// epoch for the listed peers (ADR-012). Leases pinned to the pre-sweep epoch
// become ineligible for eligibility-lease refresh without any per-token
// revocation lookup. Admin auth: X-MSP-Admin (env ADMIN_TOKEN), 403 otherwise.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const denied = requireAdmin(req, getTrackerContext());
  if (denied) return denied;

  const guarded = await guardAdminBody(req, AuditSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const results: Array<{ peer_id: string; audit_epoch: number }> = [];
  for (const peerId of body.peer_ids) {
    const epoch = await ctx.store.bumpPeerEpoch(peerId);
    results.push({ peer_id: peerId, audit_epoch: epoch });
  }
  return jsonOk({ updated: results.length, peers: results });
}
