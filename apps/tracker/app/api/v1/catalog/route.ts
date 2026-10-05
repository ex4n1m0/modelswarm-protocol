import { getTrackerContext } from "@/lib/context";
import { profileToWire, signCatalog } from "@/lib/catalog";
import { jsonError, jsonOk } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";

// GET /api/v1/catalog — signed CatalogEnvelope over schema-v2 ProfileRecords
// (msp-v1 §4, ADR-011). Signature is base64 Ed25519 over the canonical JSON of
// {catalogVersion, generatedAt, profiles} with the hub signing key. Profiles
// are sorted by profile_id (deterministic signature input per catalog version).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  const [records, version] = await Promise.all([
    ctx.store.listProfiles("active"),
    ctx.store.catalogVersion(),
  ]);
  return jsonOk(
    signCatalog(
      {
        catalogVersion: version,
        generatedAt: new Date(ctx.now()).toISOString(),
        profiles: records.map(profileToWire),
      },
      ctx.keys.secretKey,
    ),
  );
}
