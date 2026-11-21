// POST /api/v1/account/logout — delete the server session row and clear
// the msw_session cookie (Max-Age=0). Requires a live session; an expired
// or missing one is just 401 (nothing left to log out).

import { jsonOk, logEvent } from "@/lib/errors";
import { guardAccountRequest, requireJsonContentType } from "@/lib/guard";
import { requireAccountUser, sessionCookieHeader, withSessionCookie } from "@/lib/account-auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request): Promise<Response> {
  const contentType = requireJsonContentType(req);
  if (contentType) return contentType;
  const guarded = await guardAccountRequest(req);
  if (!guarded.ok) return guarded.response;
  const auth = await requireAccountUser(req, guarded.ctx);
  if (!auth.ok) return auth.response;

  await guarded.ctx.store.deleteUserSession(auth.token);
  logEvent("info", { event: "account_logout", userId: auth.userId });
  return withSessionCookie(jsonOk({ ok: true }), sessionCookieHeader("", 0));
}
