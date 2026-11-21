// Shared request guards: body reading under the size cap, the full signed
// envelope pipeline, strict schema parsing, and the admin token check.

import type { z } from "zod";
import { getTrackerContext, type TrackerContext } from "@/lib/context";
import { parseJsonBody, readBody, verifySignedRequest } from "@/lib/envelope";
import { jsonError, jsonErrorFor } from "@/lib/errors";
import { safeEqual } from "@/lib/crypto";
import { clientIpOf } from "@/lib/ratelimit";
import type { SignedEnvelope } from "@/lib/envelope";

/**
 * Size cap -> auth/session -> ts -> nonce -> digest -> per-installation rate
 * limit -> strict schema. Returns the typed body on success.
 */
export async function guardPeerRequest<S extends z.ZodTypeAny>(
  req: Request,
  schema: S,
  opts: { sessionRequired?: boolean } = {},
): Promise<
  | { ok: true; ctx: TrackerContext; installationId: string; envelope: SignedEnvelope; body: z.infer<S> }
  | { ok: false; response: Response }
> {
  const ctx = getTrackerContext();
  const body = await readBody(req);
  if (!body.ok) return { ok: false, response: body.response };
  const guard = await verifySignedRequest(req, body.text, ctx, {
    sessionRequired: opts.sessionRequired ?? true,
  });
  if (!guard.ok) return { ok: false, response: guard.response };
  const parsed = parseJsonBody(body.text, schema, new URL(req.url).pathname);
  if (!parsed.ok) return { ok: false, response: parsed.response };
  return {
    ok: true,
    ctx,
    installationId: guard.installationId,
    envelope: guard.envelope,
    body: parsed.value,
  };
}

/** Guard a GET peer request (empty body, empty-string digest). */
export async function guardPeerGet(
  req: Request,
): Promise<{ ok: true; ctx: TrackerContext; installationId: string } | { ok: false; response: Response }> {
  const ctx = getTrackerContext();
  const guard = await verifySignedRequest(req, "", ctx, { sessionRequired: true });
  if (!guard.ok) return { ok: false, response: guard.response };
  return { ok: true, ctx, installationId: guard.installationId };
}

/** Read + size-cap + schema-parse an unsigned public/enrollment body, with the
 *  per-IP rate class checked before any storage write (F6). */
export async function guardPublicBody<S extends z.ZodTypeAny>(
  req: Request,
  schema: S,
  rateClass: "public" | "enroll" | "account",
): Promise<{ ok: true; ctx: TrackerContext; body: z.infer<S> } | { ok: false; response: Response }> {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  const body = await readBody(req);
  if (!body.ok) return { ok: false, response: body.response };
  const limit =
    rateClass === "public"
      ? ctx.limits.publicPerIp
      : rateClass === "enroll"
        ? ctx.limits.enrollPerIp
        : ctx.limits.accountPerIp;
  if (!ctx.limiter.hit(`${rateClass}:${ip}`, limit)) {
    return { ok: false, response: jsonError(429, "rate_limited", "rate limit exceeded", true) };
  }
  const parsed = parseJsonBody(body.text, schema, new URL(req.url).pathname);
  if (!parsed.ok) return { ok: false, response: parsed.response };
  return { ok: true, ctx, body: parsed.value };
}

/** Session-authed account routes (me/machines/logout/revoke/label): size
 *  cap + the per-IP "account" rate class, BEFORE any session store read —
 *  the session-authed twin of guardPublicBody (no public schema here;
 *  bodies, when a route has one, are parsed by the caller from `text`).
 *  devices/approve is NOT on this helper: it takes a public body and stays
 *  on guardPublicBody(..., "account"), which buckets it the same way. */
export async function guardAccountRequest(
  req: Request,
): Promise<{ ok: true; ctx: TrackerContext; text: string } | { ok: false; response: Response }> {
  const ctx = getTrackerContext();
  const body = await readBody(req);
  if (!body.ok) return { ok: false, response: body.response };
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`account:${ip}`, ctx.limits.accountPerIp)) {
    return { ok: false, response: jsonError(429, "rate_limited", "rate limit exceeded", true) };
  }
  return { ok: true, ctx, text: body.text };
}

/** Content-Type gate for the NEW account POST routes only: a cross-site
 *  form can POST text/plain bodies, and a JSON parse of those must not be
 *  reachable. Opt-in per call site — every pre-existing route keeps its
 *  frozen behavior (content-type was never enforced there). Null = ok. */
export function requireJsonContentType(req: Request): Response | null {
  const mime = (req.headers.get("content-type") ?? "").split(";")[0]!.trim().toLowerCase();
  if (mime !== "application/json") {
    return jsonErrorFor("invalid_body", "content-type must be application/json");
  }
  return null;
}

/** X-MSP-Admin check (ops runbook token from ADMIN_TOKEN). Null response = ok. */
export function requireAdmin(req: Request, ctx: TrackerContext): Response | null {
  const presented = req.headers.get("x-msp-admin") ?? "";
  if (!ctx.adminToken || presented === "" || !safeEqual(presented, ctx.adminToken)) {
    return jsonError(403, "forbidden", "admin token required");
  }
  return null;
}

/** Admin routes: size cap + strict schema (no session/envelope; admin token
 *  was already checked by requireAdmin). */
export async function guardAdminBody<S extends z.ZodTypeAny>(
  req: Request,
  schema: S,
): Promise<{ ok: true; ctx: TrackerContext; body: z.infer<S> } | { ok: false; response: Response }> {
  const ctx = getTrackerContext();
  const body = await readBody(req);
  if (!body.ok) return { ok: false, response: body.response };
  const parsed = parseJsonBody(body.text, schema, new URL(req.url).pathname);
  if (!parsed.ok) return { ok: false, response: parsed.response };
  return { ok: true, ctx, body: parsed.value };
}
