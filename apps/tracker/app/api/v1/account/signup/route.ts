// POST /api/v1/account/signup — create a website account (email + scrypt
// password) and log in immediately (30-day HttpOnly msw_session cookie).
// Gated by the ACCOUNT_SIGNUP_OPEN flag (default closed; the owner flips it
// at deploy). Rate class "account" (20/min/IP) is checked before any parse
// or storage write via guardPublicBody. The response body never contains
// the session token — only the Set-Cookie header carries it. If the user
// row lands but the session mint fails, the signup STILL succeeds (201,
// signedIn:false, no cookie) — the account exists and can log in normally.

import { jsonErrorFor, jsonOk, logEvent } from "@/lib/errors";
import { guardPublicBody, requireJsonContentType } from "@/lib/guard";
import { hashPassword } from "@/lib/passwords";
import { sessionIssueCookie, withSessionCookie } from "@/lib/account-auth";
import { AccountSignupSchema } from "@/lib/schemas";
import { randomId128, randomToken } from "@/lib/crypto";
import { USER_SESSION_TTL_MS } from "@/lib/constants";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** MemoryStore's marker error or Pg's unique_violation (23505). */
function isDuplicateAccount(err: unknown): boolean {
  if (err instanceof Error && err.message === "email_taken") return true;
  return typeof err === "object" && err !== null && (err as { code?: string }).code === "23505";
}

export async function POST(req: Request): Promise<Response> {
  const contentType = requireJsonContentType(req);
  if (contentType) return contentType;
  const guarded = await guardPublicBody(req, AccountSignupSchema, "account");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  if (!ctx.accountSignupOpen) {
    return jsonErrorFor("signup_closed", "account signup is currently closed");
  }
  if (await ctx.store.getAccountUserByEmail(body.email)) {
    return jsonErrorFor("email_taken", "an account with that email already exists");
  }

  const passwordHash = await hashPassword(body.password);
  const userId = randomId128();
  const now = ctx.now();
  try {
    await ctx.store.createAccountUser({
      userId,
      email: body.email,
      passwordHash,
      displayName: body.displayName,
      createdAt: now,
    });
  } catch (err) {
    // Lost the uniqueness race — same answer as the check above.
    if (isDuplicateAccount(err)) {
      return jsonErrorFor("email_taken", "an account with that email already exists");
    }
    throw err;
  }

  // Auto-login: mint the session the cookie will carry. A mint failure is
  // NOT a signup failure — report the account without a cookie.
  const token = randomToken();
  try {
    await ctx.store.createUserSession({
      token,
      userId,
      createdAt: now,
      expiresAt: now + USER_SESSION_TTL_MS,
    });
  } catch (err) {
    logEvent("error", { event: "account_signup_session_mint_failed", userId, reason: err instanceof Error ? err.message : "unknown" });
    return jsonOk({ userId, email: body.email, displayName: body.displayName, signedIn: false }, 201);
  }
  logEvent("info", { event: "account_signup", userId });
  return withSessionCookie(
    jsonOk({ userId, email: body.email, displayName: body.displayName, signedIn: true }, 201),
    sessionIssueCookie(token),
  );
}
