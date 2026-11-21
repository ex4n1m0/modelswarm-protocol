// POST /api/v1/admin/account/reset-password — ops password recovery until
// email infra exists: the owner (X-MSP-Admin) sets a new password hash for
// a known email. Unknown email → 404 unknown_machine.

import { getTrackerContext } from "@/lib/context";
import { jsonErrorFor, jsonOk, logEvent } from "@/lib/errors";
import { guardAdminBody, requireAdmin } from "@/lib/guard";
import { hashPassword } from "@/lib/passwords";
import { AdminAccountResetSchema } from "@/lib/schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request): Promise<Response> {
  const ctx = getTrackerContext();
  const denied = requireAdmin(req, ctx);
  if (denied) return denied;

  const guarded = await guardAdminBody(req, AdminAccountResetSchema);
  if (!guarded.ok) return guarded.response;
  const { body } = guarded;

  const user = await ctx.store.getAccountUserByEmail(body.email);
  if (!user) {
    return jsonErrorFor("unknown_machine", "no account with that email");
  }
  const passwordHash = await hashPassword(body.newPassword);
  await ctx.store.setAccountUserPassword(user.userId, passwordHash, ctx.now());
  logEvent("info", { event: "account_password_reset", userId: user.userId });
  return jsonOk({ ok: true });
}
