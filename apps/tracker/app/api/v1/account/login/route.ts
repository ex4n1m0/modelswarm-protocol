// POST /api/v1/account/login — verify email + scrypt password and mint a
// 30-day HttpOnly msw_session cookie. No user enumeration: unknown email
// and wrong password produce the BYTE-IDENTICAL 401 invalid_credentials
// body, and timing stays similar by verifying against a dummy hash when
// the email is unknown. Rate class "account" applies (guardPublicBody).

import { jsonErrorFor, jsonOk, logEvent } from "@/lib/errors";
import { guardPublicBody, requireJsonContentType } from "@/lib/guard";
import { dummyPasswordHash, verifyPassword } from "@/lib/passwords";
import { sessionIssueCookie, withSessionCookie } from "@/lib/account-auth";
import { AccountLoginSchema } from "@/lib/schemas";
import { randomToken } from "@/lib/crypto";
import { USER_SESSION_TTL_MS } from "@/lib/constants";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request): Promise<Response> {
  const contentType = requireJsonContentType(req);
  if (contentType) return contentType;
  const guarded = await guardPublicBody(req, AccountLoginSchema, "account");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const user = await ctx.store.getAccountUserByEmail(body.email);
  const stored = user ? user.passwordHash : await dummyPasswordHash();
  const ok = await verifyPassword(body.password, stored);
  if (!user || !ok) {
    logEvent("warn", { event: "account_login_failed", userId: user?.userId ?? null });
    return jsonErrorFor("invalid_credentials", "invalid email or password");
  }

  const now = ctx.now();
  const token = randomToken();
  await ctx.store.createUserSession({
    token,
    userId: user.userId,
    createdAt: now,
    expiresAt: now + USER_SESSION_TTL_MS,
  });
  logEvent("info", { event: "account_login", userId: user.userId });
  return withSessionCookie(
    jsonOk({ userId: user.userId, displayName: user.displayName }),
    sessionIssueCookie(token),
  );
}
