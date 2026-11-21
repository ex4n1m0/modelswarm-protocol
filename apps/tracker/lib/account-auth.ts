// Website-account session auth: the opaque msw_session token travels in an
// HttpOnly cookie (Set-Cookie) or an `authorization: Bearer <token>`
// header. Candidate tokens are tried cookie-first (cookie takes
// precedence), then Bearer — a stale/garbage cookie does not shadow a
// valid Bearer. Tokens are shape-checked (exactly 64 lowercase hex, the
// randomToken() form) BEFORE any store read so garbage never reaches the
// database. Expiry is row-aging against the context clock (mirroring how
// peer getSession usage judges X-MSP-Session), never a timer.

import type { TrackerContext } from "@/lib/context";
import { jsonErrorFor } from "@/lib/errors";
import { USER_SESSION_TTL_MS } from "@/lib/constants";

export const SESSION_COOKIE = "msw_session";

/** randomToken() wire form: 256-bit hex. The cheap constant-shape gate in
 *  requireAccountUser — malformed tokens are never worth a store read. */
export const SESSION_TOKEN_PATTERN = /^[0-9a-f]{64}$/;

/** Parse a Cookie header into name/value pairs (minimal, no dependencies).
 *  Duplicate names: the LAST occurrence wins (Map overwrite), matching the
 *  common browser/set-cookie behavior of later assignments replacing
 *  earlier ones. */
export function parseCookieHeader(header: string | null): Map<string, string> {
  const out = new Map<string, string>();
  if (!header) return out;
  for (const part of header.split(";")) {
    const eq = part.indexOf("=");
    if (eq <= 0) continue;
    const name = part.slice(0, eq).trim();
    const value = part.slice(eq + 1).trim();
    if (name.length > 0) out.set(name, value);
  }
  return out;
}

/** Candidate session tokens in precedence order: cookie first, then the
 *  Bearer header. Empty values are skipped. */
export function accountTokensOf(req: Request): string[] {
  const tokens: string[] = [];
  const cookie = parseCookieHeader(req.headers.get("cookie")).get(SESSION_COOKIE);
  if (cookie && cookie.length > 0) tokens.push(cookie);
  const auth = req.headers.get("authorization");
  if (auth?.startsWith("Bearer ")) {
    const token = auth.slice("Bearer ".length).trim();
    if (token.length > 0) tokens.push(token);
  }
  return tokens;
}

/** Set-Cookie value for issuing (ttl seconds) or clearing (maxAge 0). */
export function sessionCookieHeader(token: string, maxAgeSeconds: number): string {
  return `${SESSION_COOKIE}=${token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${maxAgeSeconds}`;
}

/** Standard issue header for a fresh 30-day session. */
export function sessionIssueCookie(token: string): string {
  return sessionCookieHeader(token, Math.floor(USER_SESSION_TTL_MS / 1000));
}

/** Attach a Set-Cookie to a Response (headers are pre-lock mutable). */
export function withSessionCookie(res: Response, cookie: string): Response {
  res.headers.set("set-cookie", cookie);
  return res;
}

/** Require a live account session. Candidates run cookie-first; a token
 *  that is shape-invalid, unknown, or row-aged expired falls through to
 *  the next candidate (stale cookie + valid Bearer authenticates via the
 *  Bearer). 401 unauthorized (the existing code) when none does. The
 *  success shape carries the winning token so callers can delete exactly
 *  that session (logout). */
export async function requireAccountUser(
  req: Request,
  ctx: TrackerContext,
): Promise<{ ok: true; userId: string; token: string } | { ok: false; response: Response }> {
  for (const token of accountTokensOf(req)) {
    if (!SESSION_TOKEN_PATTERN.test(token)) continue;
    const session = await ctx.store.getUserSession(token);
    if (session && session.expiresAt > ctx.now()) {
      return { ok: true, userId: session.userId, token };
    }
  }
  return { ok: false, response: jsonErrorFor("unauthorized", "account session required") };
}
