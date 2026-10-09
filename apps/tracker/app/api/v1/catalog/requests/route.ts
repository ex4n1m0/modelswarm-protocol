import { getTrackerContext } from "@/lib/context";
import { parseJsonBody, readBody } from "@/lib/envelope";
import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";
import { ModelRequestSchema } from "@/lib/schemas";

// POST /api/v1/catalog/requests — community model request (ADR-023).
// Public write at enrollment-grade rate limiting (10/min/IP): strictly
// bounded artifact-pointer fields the desktop fills from the public HF Hub API.
// Content-blind by schema — no field can carry prompts or completions.
// Dedupes on artifact_sha256, so repeat submits are idempotent.
//
// T4 hygiene: the body is read through the shared capped reader (64 KiB,
// content-length pre-check) — never a raw req.json() that would make the
// parse cost bounded only by the platform body cap.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`modelreq:${ip}`, ctx.limits.enrollPerIp)) {
    return jsonErrorFor("rate_limited", "rate limit exceeded", true);
  }
  const body = await readBody(req);
  if (!body.ok) return body.response;
  const parsed = parseJsonBody(body.text, ModelRequestSchema, new URL(req.url).pathname);
  if (!parsed.ok) return parsed.response;
  const b = parsed.value;
  const result = await ctx.store.insertModelRequest({
    artifactSha256: b.artifact_sha256,
    hfRepo: b.hf_repo,
    hfRevision: b.hf_revision,
    artifactPath: b.artifact_path,
    artifactBytes: b.artifact_bytes,
    quantMethod: b.quant_method,
    quantBits: b.quant_bits,
    displayName: b.display_name,
    note: b.note ?? "",
    requestedFrom: ip,
  });
  return jsonOk({ status: result === "inserted" ? "queued" : "already-requested" }, 201);
}
