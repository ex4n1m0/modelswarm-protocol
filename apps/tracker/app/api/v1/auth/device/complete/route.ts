import { getTrackerContext } from "@/lib/context";
import { randomToken } from "@/lib/crypto";
import { jsonError, jsonOk } from "@/lib/errors";
import { parseJsonBody, readBody, verifySignedRequest } from "@/lib/envelope";
import { clientIpOf } from "@/lib/ratelimit";
import { DeviceCompleteSchema } from "@/lib/schemas";
import { SESSION_TTL_MS } from "@/lib/constants";

// POST /api/v1/auth/device/complete (msp-v1 §3.2): signed envelope over
// {deviceCode} (no session yet). Approved -> {token, expiresAt} (24 h);
// not approved -> 403 pending. Since the claim handshake (gate (a),
// migration 0010) a PENDING account claim gates the session too: the
// device must first see who claimed it and answer at
// /auth/device/claim-confirm — 200 {status:"claim_pending", claimedBy}
// (masked email) instead of a token. The admin /verify approve and the
// auto-approve path (no claim rows touched) still complete directly.

/** Masked claimer identity for the device-side confirm prompt: the local
 *  part keeps its first character only. The email (not the display name —
 *  which is choosable and confusable) is what lets the person at the
 *  machine spot a wrong-account claim. */
function maskEmail(email: string): string {
  const at = email.indexOf("@");
  if (at <= 0) return "…";
  return `${email.slice(0, 1)}***${email.slice(at)}`;
}

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const ctx = getTrackerContext();
  const body = await readBody(req);
  if (!body.ok) return body.response;

  // Enrollment rate class also applies to complete (10 req/min/IP).
  if (!ctx.limiter.hit(`enroll:${clientIpOf(req)}`, ctx.limits.enrollPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }

  const guard = await verifySignedRequest(req, body.text, ctx, { sessionRequired: false });
  if (!guard.ok) return guard.response;

  const parsed = parseJsonBody(body.text, DeviceCompleteSchema, "/api/v1/auth/device/complete");
  if (!parsed.ok) return parsed.response;

  const device = await ctx.store.getDeviceAuth(parsed.value.deviceCode);
  if (!device || device.installationId !== guard.installationId) {
    return jsonError(401, "unauthorized", "unknown device code for this installation");
  }
  if (device.expiresAt <= ctx.now()) {
    return jsonError(401, "unauthorized", "device code expired; restart enrollment");
  }
  // An unconfirmed claim gates the session even when approved is already
  // true (admin/auto-approve): the device-side confirm is the anti-hijack
  // step, so it takes precedence over every approval path.
  if (device.claimedByUserId !== null && device.claimConfirmedAt === null) {
    const claimer = await ctx.store.getAccountUserById(device.claimedByUserId);
    const claimedBy = claimer ? maskEmail(claimer.email) : "unknown account";
    return jsonOk({ status: "claim_pending", claimedBy });
  }
  if (!device.approved) {
    return jsonError(403, "pending", "device enrollment awaiting user approval");
  }

  const now = ctx.now();
  const expiresAt = now + SESSION_TTL_MS;
  const token = randomToken();
  await ctx.store.createSession({
    token,
    installationId: guard.installationId,
    createdAt: now,
    expiresAt,
  });
  return jsonOk({ token, expiresAt: new Date(expiresAt).toISOString() });
}
