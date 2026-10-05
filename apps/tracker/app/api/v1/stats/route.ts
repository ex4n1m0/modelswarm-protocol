// GET /api/v1/stats — public online counter for the landing page (Phase I1).
// Counts only: distinct peers holding a live, non-draining lease. Content-
// blind by construction — no peer ids, addresses, or profiles are exposed.
// Rate class: 120 req/min/IP shared with /health + /catalog (F5).

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonOk } from "@/lib/errors";
import { clientIpOf } from "@/lib/ratelimit";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  const peersOnline = await ctx.store.countOnlinePeers();
  return jsonOk({ peersOnline });
}
