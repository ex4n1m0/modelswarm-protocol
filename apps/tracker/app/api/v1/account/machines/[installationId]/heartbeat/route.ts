// POST /api/v1/account/machines/[installationId]/heartbeat — signed machine
// liveness + contact info for the account layer: touches last_seen_at
// (migration 0009), sets or clears the signaling mailbox mapping and
// auto-fills an unnamed label from the display name (migration 0011) on the
// installation's ACTIVE account_machines row. Auth is exactly the
// peer-route stack (MSP1 envelope + the 24h x-msp-session from
// auth/device/complete, guardPeerRequest); rate limiting is the house
// per-installation peer bucket — nothing new. The body is {} or the optional
// contact fields. Mismatched path id, no active binding, and revoked
// machines all answer the same 404 unknown_machine (no existence oracle).

import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { MachineHeartbeatSchema } from "@/lib/schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(
  req: Request,
  { params }: { params: Promise<{ installationId: string }> },
): Promise<Response> {
  const guarded = await guardPeerRequest(req, MachineHeartbeatSchema);
  if (!guarded.ok) return guarded.response;
  const { installationId } = await params;
  // The guard already proved session.installationId === envelope's; the path
  // must name the SAME installation. Equality with the schema-validated
  // envelope id (base58, 8-64 chars) also shape-gates the path param before
  // any store call — oversized garbage can never reach Pg's varchar(64).
  if (guarded.envelope.installationId !== installationId) {
    return jsonErrorFor("unknown_machine", "no active machine with that id");
  }
  const now = guarded.ctx.now();
  const touched = await guarded.ctx.store.updateMachineContact(installationId, guarded.body, now);
  if (!touched) {
    return jsonErrorFor("unknown_machine", "no active machine with that id");
  }
  return jsonOk({ seenAt: new Date(now).toISOString() });
}
