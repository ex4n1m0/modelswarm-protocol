// GET /api/v1/account/directory — the account-scoped machine directory for
// the DESKTOP app (ADR-013 P2): the calling machine's own account's ACTIVE
// machines, each with its signaling mailbox id when the machine has shared
// one (a null signalingDeviceId = online-but-not-controllable). This is
// machine-authed (MSP1 envelope + the 24h x-msp-session, like the
// heartbeat) — the app holds no browser cookies; the cookie-authed
// dashboard keeps using GET /account/machines.

import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardPeerGet } from "@/lib/guard";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request): Promise<Response> {
  const guarded = await guardPeerGet(req);
  if (!guarded.ok) return guarded.response;

  // The signing machine must itself be bound (active) to an account — its
  // session alone is not account membership. Unbound/revoked answer the
  // same 404 (no existence oracle).
  const owner = await guarded.ctx.store.getActiveAccountForInstallation(guarded.installationId);
  if (!owner) {
    return jsonErrorFor("unknown_machine", "no active machine with that id");
  }
  const machines = await guarded.ctx.store.listAccountMachines(owner.userId);
  return jsonOk({
    machines: machines
      .filter((m) => m.status === "active")
      .map((m) => ({
        installationId: m.installationId,
        label: m.label,
        lastSeenAt: m.lastSeenAt === null ? null : new Date(m.lastSeenAt).toISOString(),
        signalingDeviceId: m.signalingDeviceId,
      })),
  });
}
