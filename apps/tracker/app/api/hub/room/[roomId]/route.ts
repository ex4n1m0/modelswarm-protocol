// GET /api/hub/room/{roomId} — fetch the current host record of a room
// (OnlyHumans hub port, ADR-013 D4; hub.rs `lookup_room`). Returns the stored
// record VERBATIM (opaque); the client re-verifies signature + key→host binding.
// 404 when nobody hosts / TTL expired — hub.rs maps that to None.

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardHubStore } from "@/lib/guard";
import { ROOM_ID_PATTERN } from "@/lib/hub-schemas";
import { clientIpOf } from "@/lib/ratelimit";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(
  req: Request,
  { params }: { params: Promise<{ roomId: string }> },
): Promise<Response> {
  const hub = guardHubStore();
  if (!hub.ok) return hub.response;
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  const { roomId } = await params;
  if (!ROOM_ID_PATTERN.test(roomId)) {
    return jsonErrorFor("invalid_body", "bad room id");
  }
  const record = await hub.store.getRoom(roomId);
  if (!record) {
    return jsonError(404, "unknown_request", "no host record");
  }
  return jsonOk(record);
}
