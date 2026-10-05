import { getTrackerContext } from "@/lib/context";
import { profileToWire, signCatalog } from "@/lib/catalog";
import { jsonError, jsonOk } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";

// GET /api/v1/catalog/{profileId} — single-profile signed envelope (§3.1).
// Unknown or non-active profile -> 404 unknown_profile.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(
  req: Request,
  { params }: { params: Promise<{ profileId: string }> },
) {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  const { profileId } = await params;
  const record = await ctx.store.getProfile(profileId);
  if (!record || record.status !== "active") {
    return jsonError(404, "unknown_profile", `no active profile ${profileId}`);
  }
  return jsonOk(
    signCatalog(
      {
        catalogVersion: await ctx.store.catalogVersion(),
        generatedAt: new Date(ctx.now()).toISOString(),
        profiles: [profileToWire(record)],
      },
      ctx.keys.secretKey,
    ),
  );
}
