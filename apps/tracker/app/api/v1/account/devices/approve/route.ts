// POST /api/v1/account/devices/approve — the signed-in website user claims
// a device enrollment by its 8-char pairing code (gate (a), migration 0010).
// This no longer binds: the claim is PENDING until the DEVICE confirms it
// (its next /auth/device/complete poll reports the claim; the device answers
// via /auth/device/claim-confirm, and the bind happens there). A leaked
// userCode can then at worst dangle a pending claim the device declines.
// Requires a live msw_session. Rate class "account" applies BEFORE any
// parse or store write.

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

  const claimed = await ctx.store.claimDeviceByUserCode(
    body.userCode,
    auth.userId,
    body.label?.trim() || null,
    ctx.now(),
  );
  if (claimed === "unknown") {
    // Lost a race with code expiry between the read and the write.
    return jsonErrorFor("unknown_request", "no pending device with that code");
  }
  if (claimed === "already_claimed") {
    return jsonErrorFor("already_claimed", "another account has a pending claim on this code");
  }
  logEvent("info", { event: "account_device_claim", userId: auth.userId, installationId: device.installationId });
  // 202: the claim is accepted but the binding awaits the device-side
  // confirmation. The dashboard tells the user to look at the machine.
  return jsonOk(
    {
      status: "claim_pending",
      installationId: device.installationId,
      label: body.label?.trim() || null,
      confirmBy: new Date(device.expiresAt).toISOString(),
    },
    202,
  );
}
