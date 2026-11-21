// POST /api/v1/auth/device/claim-confirm (ADR-013 P2 gate (a), migration
// 0010) — the DEVICE-side answer to a pending account claim. Signed exactly
// like /auth/device/complete (MSP1 envelope over {deviceCode, accept}; no
// session yet — enrollment has not finished). accept=true confirms the
// claiming account: the claim is stamped confirmed, the code flips
// approved, and the account_machines bind happens HERE (the device said
// yes — no window remains where the userCode alone attached a machine).
// accept=false clears the claim; the still-live code is re-claimable.

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk, logEvent } from "@/lib/errors";
import { parseJsonBody, readBody, verifySignedRequest } from "@/lib/envelope";
import { clientIpOf } from "@/lib/ratelimit";
import { DeviceClaimConfirmSchema } from "@/lib/schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const ctx = getTrackerContext();
  const body = await readBody(req);
  if (!body.ok) return body.response;

  // Enrollment rate class (10 req/min/IP), same as complete.
  if (!ctx.limiter.hit(`enroll:${clientIpOf(req)}`, ctx.limits.enrollPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }

  const guard = await verifySignedRequest(req, body.text, ctx, { sessionRequired: false });
  if (!guard.ok) return guard.response;

  const parsed = parseJsonBody(body.text, DeviceClaimConfirmSchema, "/api/v1/auth/device/claim-confirm");
  if (!parsed.ok) return parsed.response;

  const device = await ctx.store.getDeviceAuth(parsed.value.deviceCode);
  if (!device || device.installationId !== guard.installationId) {
    return jsonError(401, "unauthorized", "unknown device code for this installation");
  }
  if (device.expiresAt <= ctx.now()) {
    return jsonError(401, "unauthorized", "device code expired; restart enrollment");
  }

  if (!parsed.value.accept) {
    // Deny: clear the pending claim; the owner can re-approve this code.
    await ctx.store.resolveDeviceClaim(parsed.value.deviceCode, false, ctx.now());
    logEvent("info", { event: "device_claim_denied", installationId: guard.installationId });
    return jsonOk({ status: "claim_denied" });
  }

  if (device.claimedByUserId === null) {
    // Nothing to confirm — a stale prompt replaying against a fresh code.
    return jsonErrorFor("unknown_request", "no pending claim for this device");
  }

  // Bind BEFORE flipping the code approved (security review finding 5): a
  // lost exclusivity race must not leave an approved+confirmed code whose
  // session then lands under a different account than the one this
  // device's user just allowed.
  const bound = await ctx.store.bindMachineToAccount({
    userId: device.claimedByUserId,
    installationId: device.installationId,
    label: device.claimedLabel,
    now: ctx.now(),
  });
  if (bound === "already_bound") {
    // Lost the race to another account — clear the dangling claim so the
    // code cannot be confirmed into a binding it does not have.
    await ctx.store.resolveDeviceClaim(parsed.value.deviceCode, false, ctx.now());
    return jsonErrorFor("already_bound", "device is already bound to another account");
  }
  const resolved = await ctx.store.resolveDeviceClaim(parsed.value.deviceCode, true, ctx.now());
  if (!resolved) {
    return jsonError(401, "unauthorized", "unknown device code for this installation");
  }
  logEvent("info", {
    event: "device_claim_confirmed",
    userId: resolved.claimedByUserId,
    installationId: resolved.installationId,
  });
  // The device's next /complete poll now finds an approved+confirmed code
  // and mints the session normally.
  return jsonOk({ status: "claim_confirmed" });
}
