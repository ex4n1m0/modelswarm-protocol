import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonOk } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";
import { MSP_PROTOCOL_VERSION, SERVICE_NAME } from "@/lib/version";

// Liveness + protocol version (msp-v1 §3.1). Exact body asserted by B1.
// Rate class: 120 req/min/IP shared with /catalog (F5).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  return jsonOk({
    status: "ok",
    service: SERVICE_NAME,
    protocol: MSP_PROTOCOL_VERSION,
  });
}
