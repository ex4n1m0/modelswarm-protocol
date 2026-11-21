// GET /api/hub/lookup/{peerId} — fetch a peer's latest signed registration
// (OnlyHumans hub port, ADR-013 D4; hub.rs `lookup`). Returns the stored record
// VERBATIM (opaque); the client re-verifies the signature and the key→peer-id
// binding locally. 404 when unknown/expired — hub.rs maps that to None.

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk } from "@/lib/errors";
import { getHubStore } from "@/lib/hub-store";
import { clientIpOf } from "@/lib/ratelimit";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(
  req: Request,
  { params }: { params: Promise<{ peerId: string }> },
): Promise<Response> {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  const { peerId } = await params;
  if (!peerId || peerId.length > 128) {
    return jsonErrorFor("invalid_body", "bad peer id");
  }
  const record = await getHubStore(ctx.now).getPeer(peerId);
  if (!record) {
    return jsonError(404, "unknown_request", "no registration for peer");
  }
  return jsonOk(record);
}
