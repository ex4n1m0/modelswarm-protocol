// MSP1 signed-request envelope (msp-v1 §2.3): parsing, verification, replay
// protection, and the frozen validation precedence:
//
//   size cap -> auth/session -> timestamp -> nonce -> body digest -> schema/rate
//
// Rate limiting (msp-v1 §3.4) is wired in at the documented stages:
//   - per-IP public bucket (health/catalog) and enrollment bucket (device/*)
//     are checked before any body parsing beyond the size cap;
//   - per-IP unsigned bucket counts peer-endpoint calls that never reach a
//     verifiable envelope — unparseable headers AND well-formed envelopes
//     naming an unknown installation (both can never verify);
//   - the per-installation peer bucket is checked after full verification
//     (last, per the frozen precedence).

import { z } from "zod";
import {
  base64UrlDecode,
  base64UrlEncode,
  bodyDigestOf,
  bs58Decode,
  canonicalJson,
  signBase64,
  verifyBase64,
} from "@/lib/crypto";
import { jsonError, jsonErrorFor, logEvent } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";
import type { TrackerContext } from "@/lib/context";
import type { TrackerStore } from "@/lib/store";

export const MAX_BODY_BYTES = 64 * 1024; // 64 KiB (msp-v1 §3)
export const REPLAY_WINDOW_MS = 120_000; // ±120 s (msp-v1 §2.3)
export const EMPTY_BODY_DIGEST = bodyDigestOf("");

export const EnvelopeSchema = z
  .object({
    installationId: z.string().min(8).max(64).regex(/^[1-9A-HJ-NP-Za-km-z]{8,64}$/),
    method: z.string().min(3).max(8),
    path: z.string().min(1).max(256),
    ts: z.string().datetime({ offset: true }),
    nonce: z.string().regex(/^[0-9a-f]{16,128}$/),
    bodyDigest: z.string().regex(/^sha256:[0-9a-f]{64}$/),
    signature: z.string().min(86).max(120),
  })
  .strict();

export type SignedEnvelope = z.infer<typeof EnvelopeSchema>;

/** Fields covered by the detached signature (everything but the signature). */
export function envelopePayload(env: Omit<SignedEnvelope, "signature">): string {
  return canonicalJson({
    installationId: env.installationId,
    method: env.method,
    path: env.path,
    ts: env.ts,
    nonce: env.nonce,
    bodyDigest: env.bodyDigest,
  });
}

/** Build the Authorization header value for a signed envelope (client/test side). */
export function encodeAuthorization(
  env: Omit<SignedEnvelope, "signature">,
  secretKey: Uint8Array,
): { header: string; envelope: SignedEnvelope } {
  const signature = signBase64(envelopePayload(env), secretKey);
  const full: SignedEnvelope = { ...env, signature };
  return { header: `MSP1 ${base64UrlEncode(canonicalJson(full))}`, envelope: full };
}

/** Decode a "MSP1 <base64url(canonical json)>" Authorization header. */
export function decodeAuthorization(
  header: string | null,
): { envelope: SignedEnvelope } | null {
  if (!header || !header.startsWith("MSP1 ")) return null;
  const encoded = header.slice("MSP1 ".length).trim();
  if (encoded.length === 0 || encoded.length > 8192) return null;
  let json: string;
  try {
    json = Buffer.from(base64UrlDecode(encoded)).toString("utf8");
    JSON.parse(json); // must be valid JSON
  } catch {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return null;
  }
  const result = EnvelopeSchema.safeParse(parsed);
  if (!result.success) return null;
  return { envelope: result.data };
}

export type VerifyOutcome =
  | { ok: true; installationId: string; envelope: SignedEnvelope; sessionToken: string }
  | { ok: false; response: Response };

export interface VerifyOptions {
  /** Peer endpoints require X-MSP-Session; device/complete does not. */
  sessionRequired: boolean;
}

/**
 * Full verification pipeline for a peer/auth request.
 * `rawBody` must be the exact bytes received ("" for GET).
 */
export async function verifySignedRequest(
  req: Request,
  rawBody: string,
  ctx: TrackerContext,
  opts: VerifyOptions,
): Promise<VerifyOutcome> {
  const store = ctx.store;
  const ip = clientIpOf(req);

  // -- 1. size cap (first, always) -------------------------------------------
  if (Buffer.byteLength(rawBody, "utf8") > MAX_BODY_BYTES) {
    return fail(413, "payload_too_large", "request body exceeds 64 KiB");
  }

  // -- 2. auth/session --------------------------------------------------------
  const decoded = decodeAuthorization(req.headers.get("authorization"));
  if (!decoded) {
    // No parseable envelope: count against the per-IP unsigned bucket.
    if (!ctx.limiter.hit(`unsigned:${ip}`, ctx.limits.unsignedPerIp)) {
      return failRateLimited();
    }
    return fail(401, "unsigned_request", "missing or malformed MSP1 authorization envelope");
  }
  const env = decoded.envelope;

  const installation = await store.getInstallation(env.installationId);
  if (!installation) {
    // A well-formed envelope naming an unenrolled installation would
    // otherwise burn a DB read per request with no limiter bucket ever
    // filling (audit T4). A call that can never reach a verifiable
    // envelope is unsigned in effect — count it against the same per-IP
    // bucket (msp-v1 §3.4), THEN fail.
    if (!ctx.limiter.hit(`unsigned:${ip}`, ctx.limits.unsignedPerIp)) {
      return failRateLimited();
    }
    return fail(401, "unknown_installation", "installation is not enrolled");
  }

  const sessionToken = req.headers.get("x-msp-session") ?? "";
  if (opts.sessionRequired || sessionToken !== "") {
    if (sessionToken === "") {
      return fail(401, "unauthorized", "X-MSP-Session header required");
    }
    const session = await store.getSession(sessionToken);
    if (
      !session ||
      session.expiresAt <= ctx.now() ||
      session.installationId !== env.installationId
    ) {
      return fail(401, "unauthorized", "session token invalid, expired, or mismatched");
    }
  }

  // Signature over the canonical sub-envelope, verified against the
  // installation's registered public key; method and path are signed, so a
  // captured envelope cannot be replayed against a different endpoint.
  const pubBytes = bs58Decode(installation.pubKeyB58);
  if (!pubBytes || pubBytes.length !== 32) {
    return fail(401, "unauthorized", "installation public key unusable");
  }
  const url = new URL(req.url);
  if (
    env.method !== req.method ||
    env.path !== url.pathname ||
    !verifyBase64(env.signature, envelopePayload(env), pubBytes)
  ) {
    return fail(401, "unauthorized", "envelope signature verification failed");
  }

  // -- 3. timestamp (±120 s) ---------------------------------------------------
  const tsMs = Date.parse(env.ts);
  if (Math.abs(ctx.now() - tsMs) > REPLAY_WINDOW_MS) {
    return fail(400, "stale_timestamp", "envelope ts outside the ±120 s replay window");
  }

  // -- 4. nonce (single-use per installation) -----------------------------------
  const fresh = await store.consumeNonce(env.installationId, env.nonce, REPLAY_WINDOW_MS * 2);
  if (!fresh) {
    return fail(400, "replayed_nonce", "nonce already used within the replay window");
  }

  // -- 5. body digest -------------------------------------------------------------
  const expected = req.method === "GET" ? EMPTY_BODY_DIGEST : bodyDigestOf(rawBody);
  if (env.bodyDigest !== expected) {
    return fail(401, "unauthorized", "bodyDigest does not match the request body");
  }

  // -- 6. rate limit (per installation; last per frozen precedence) ----------------
  if (!ctx.limiter.hit(`inst:${env.installationId}`, ctx.limits.peerPerInstallation)) {
    return failRateLimited();
  }

  return { ok: true, installationId: env.installationId, envelope: env, sessionToken };
}

function fail(status: number, code: Parameters<typeof jsonError>[1], message: string): {
  ok: false;
  response: Response;
} {
  return { ok: false, response: jsonError(status, code, message) };
}

function failRateLimited(): { ok: false; response: Response } {
  return {
    ok: false,
    response: jsonError(429, "rate_limited", "rate limit exceeded; retry after the current window", true),
  };
}

// ---------------------------------------------------------------------------
// Helpers shared by routes

export interface BodyRead {
  ok: true;
  text: string;
}

/** Read the body under the 64 KiB cap. Returns a 413 Response on overflow. */
export async function readBody(req: Request): Promise<BodyRead | { ok: false; response: Response }> {
  const declared = req.headers.get("content-length");
  if (declared !== null) {
    const value = Number.parseInt(declared, 10);
    if (Number.isFinite(value) && value > MAX_BODY_BYTES) {
      return { ok: false, response: jsonError(413, "payload_too_large", "request body exceeds 64 KiB") };
    }
  }
  const text = await req.text();
  if (Buffer.byteLength(text, "utf8") > MAX_BODY_BYTES) {
    return { ok: false, response: jsonError(413, "payload_too_large", "request body exceeds 64 KiB") };
  }
  return { ok: true, text };
}

/** Parse and validate a JSON body against a strict schema. Bodies are never
 *  logged (F3); only the validation outcome is (redacted, structured). */
export function parseJsonBody<T extends z.ZodTypeAny>(
  text: string,
  schema: T,
  path: string,
): { ok: true; value: z.infer<T> } | { ok: false; response: Response } {
  let parsedJson: unknown;
  try {
    parsedJson = JSON.parse(text);
  } catch {
    logEvent("warn", { code: "invalid_body", path, reason: "not_json" });
    return invalid(path, "body is not valid JSON");
  }
  const result = schema.safeParse(parsedJson);
  if (!result.success) {
    // Structured, redacted: issue count only — never field values.
    logEvent("warn", { code: "invalid_body", path, reason: "schema", issues: result.error.issues.length });
    return invalid(path, `body failed schema validation (${result.error.issues.length} issue(s))`);
  }
  return { ok: true, value: result.data };
}

function invalid(path: string, message: string): { ok: false; response: Response } {
  return { ok: false, response: jsonErrorFor("invalid_body", `${path}: ${message}`) };
}
