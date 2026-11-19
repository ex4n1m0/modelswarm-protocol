// POST /api/v1/account/devices/approve — the signed-in website user approves
// a device enrollment by its 8-char pairing code AND binds that installation
// to their account (the account-layer twin of /admin/devices/approve: the
// pending phase still lives in device_codes; approval here also creates the
// account_machines row directly as 'active'). Requires a live msw_session.
// Rate class "account" applies BEFORE any parse or store write.

import { jsonErrorFor, jsonOk, logEvent } from "@/lib/errors";
import { guardPublicBody, requireJsonContentType } from "@/lib/guard";
import { requireAccountUser } from "@/lib/account-auth";
import { AccountDeviceApproveSchema } from "@/lib/schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request): Promise<Response> {
  const contentType = requireJsonContentType(req);
  if (contentType) return contentType;
  const guarded = await guardPublicBody(req, AccountDeviceApproveSchema, "account");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const auth = await requireAccountUser(req, ctx);
  if (!auth.ok) return auth.response;

  // Unknown and expired codes answer identically (the admin approve route
  // likewise never distinguishes stale codes — an approved dead code yields
  // no enrollment): one 404, no code-liveness oracle.
  const device = await ctx.store.getDeviceAuthByUserCode(body.userCode);
  if (!device || device.expiresAt <= ctx.now()) {
    return jsonErrorFor("unknown_request", "no pending device with that code");
  }
  const active = await ctx.store.getActiveAccountForInstallation(device.installationId);
  if (active && active.userId !== auth.userId) {
    return jsonErrorFor("already_bound", "device is already bound to another account");
  }

  // Same update the admin route performs, then the binding.
  if (!(await ctx.store.approveDeviceByUserCode(body.userCode))) {
    return jsonErrorFor("unknown_request", "no pending device with that code");
  }
  const label = body.label ?? "Machine";
  const bound = await ctx.store.bindMachineToAccount({
    userId: auth.userId,
    installationId: device.installationId,
    label,
    now: ctx.now(),
  });
  if (bound === "already_bound") {
    // Lost the exclusivity race between the check and the write.
    return jsonErrorFor("already_bound", "device is already bound to another account");
  }
  logEvent("info", { event: "account_device_approve", userId: auth.userId, installationId: device.installationId });
  return jsonOk({ installationId: device.installationId, label, status: "active" }, 201);
}
