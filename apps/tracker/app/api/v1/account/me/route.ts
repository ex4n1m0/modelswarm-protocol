// GET /api/v1/account/me — the signed-in account's own profile. Requires a
// live msw_session (cookie or Bearer); 401 otherwise.

import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardAccountRequest } from "@/lib/guard";
import { requireAccountUser } from "@/lib/account-auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request): Promise<Response> {
  const guarded = await guardAccountRequest(req);
  if (!guarded.ok) return guarded.response;
  const auth = await requireAccountUser(req, guarded.ctx);
  if (!auth.ok) return auth.response;

  const user = await guarded.ctx.store.getAccountUserById(auth.userId);
  if (!user) {
    return jsonErrorFor("unauthorized", "account session required");
  }
  return jsonOk({
    userId: user.userId,
    email: user.email,
    displayName: user.displayName,
    createdAt: new Date(user.createdAt).toISOString(),
  });
}
