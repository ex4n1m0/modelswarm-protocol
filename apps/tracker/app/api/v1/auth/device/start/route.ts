import { getTrackerContext } from "@/lib/context";
import { bs58Decode, deriveKeyId, randomToken, randomUserCode } from "@/lib/crypto";
import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardPublicBody } from "@/lib/guard";
import { DeviceStartSchema } from "@/lib/schemas";
import { DEVICE_CODE_TTL_MS } from "@/lib/constants";

// POST /api/v1/auth/device/start (msp-v1 §3.2). Enrollment rate class:
// 10 req/min/IP, enforced BEFORE any storage write (F6). The installationId
// must be base58(sha256(pubKey)) (msp-v1 §2.1); the derived peer id uses the
// same rule.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPublicBody(req, DeviceStartSchema, "enroll");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const pubBytes = bs58Decode(body.pubKey);
  if (!pubBytes || pubBytes.length !== 32) {
    return jsonErrorFor("invalid_body", "pubKey must be a base58 32-byte ed25519 public key");
  }
  if (deriveKeyId(pubBytes) !== body.installationId) {
    return jsonErrorFor("invalid_body", "installationId must equal base58(sha256(pubKey))");
  }

  const now = ctx.now();
  await ctx.store.upsertInstallation({
    installationId: body.installationId,
    pubKeyB58: body.pubKey,
    createdAt: now,
  });

  const deviceCode = randomToken();
  const userCode = randomUserCode();
  // Auto-approval (owner decision 2026-10-06, msp-v1 §3.2): when enabled and
  // under the enrollment cap the device code is born approved — enrollment
  // completes on the client's first /complete poll with no owner action.
  // The enroll rate limit above bounds throughput; the cap bounds the total.
  // Beyond the cap (or when disabled) the code stays pending and /verify +
  // the admin token approve it exactly as before.
  const autoApprove =
    ctx.autoApproveDevices &&
    (await ctx.store.countEnrolledInstallations()) < ctx.deviceApprovalCap;
  await ctx.store.createDeviceAuth({
    deviceCode,
    installationId: body.installationId,
    userCode,
    verifyUrl: ctx.verifyUrl,
    expiresAt: now + DEVICE_CODE_TTL_MS,
    approved: autoApprove,
  });

  return jsonOk({
    deviceCode,
    userCode,
    verifyUrl: ctx.verifyUrl,
    expiresAt: new Date(now + DEVICE_CODE_TTL_MS).toISOString(),
  });
}
