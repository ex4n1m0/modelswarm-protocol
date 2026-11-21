// GET /api/hub/lookup/{peerId} — fetch a peer's latest signed registration
// (OnlyHumans hub port, ADR-013 D4; hub.rs `lookup`). Returns the stored record
// VERBATIM (opaque); the client re-verifies the signature and the key→peer-id
// binding locally. 404 when unknown/expired — hub.rs maps that to None.
// Cache-Control: no-store on every response (reference lookup sets no-store).

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk, noStore } from "@/lib/errors";
import { guardHubStore } from "@/lib/guard";
import { clientIpOf } from "@/lib/ratelimit";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(
  req: Request,
  { params }: { params: Promise<{ peerId: string }> },
): Promise<Response> {
  const hub = guardHubStore();
  if (!hub.ok) return noStore(hub.response);
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return noStore(jsonError(429, "rate_limited", "rate limit exceeded", true));
  }
  const { peerId } = await params;
  if (!peerId || peerId.length > 128) {
    return noStore(jsonErrorFor("invalid_body", "bad peer id"));
  }
  const record = await hub.store.getPeer(peerId);
  if (!record) {
    return noStore(jsonError(404, "unknown_request", "no registration for peer"));
  }
  return noStore(jsonOk(record));
}
