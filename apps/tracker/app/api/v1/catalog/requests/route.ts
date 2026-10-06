import { getTrackerContext } from "@/lib/context";
import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";
import { ModelRequestSchema } from "@/lib/schemas";

// POST /api/v1/catalog/requests — community model request (ADR-023).
// Public write at enrollment-grade rate limiting (10/min/IP): strictly
// bounded artifact-pointer fields the desktop fills from the public HF Hub API.
// Content-blind by schema — no field can carry prompts or completions.
// Dedupes on artifact_sha256, so repeat submits are idempotent.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`modelreq:${ip}`, ctx.limits.enrollPerIp)) {
    return jsonErrorFor("rate_limited", "rate limit exceeded", true);
  }
  const parsed = ModelRequestSchema.safeParse(await req.json().catch(() => null));
  if (!parsed.success) {
    return jsonErrorFor("invalid_body", parsed.error.issues.map((i) => `${i.path}: ${i.message}`).join("; "));
  }
  const b = parsed.data;
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
