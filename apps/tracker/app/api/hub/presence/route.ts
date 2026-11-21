// POST /api/hub/presence — anonymous "someone is online" beacon (OnlyHumans hub
// port, ADR-013 D4; hub.rs `presence` / `presence_leave`). Account-ORTHOGONAL
// and identity-free: the token is random per app start (NOT a peer id), so the
// hub can count running apps but never tie a count to an identity.
//   { token }            -> heartbeat, returns { ok, online }
//   { token, leave:true } -> remove ourselves (graceful exit), returns { ok }
// Entries live 300 s (apps beat every ~120 s); the whole zset self-destructs at
// 900 s. hub.rs maps a 429 to Ok(0) ("a beat landed <30 s ago; still counted").

import { jsonError, jsonOk } from "@/lib/errors";
import { guardHubStore, guardPublicBody } from "@/lib/guard";
import { HubPresenceSchema } from "@/lib/hub-schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// In-memory approximation of the reference's `rl:presence:<token>` 1-per-30s NX.
const PRESENCE_PER_MIN = 2;

export async function POST(req: Request): Promise<Response> {
  const hub = guardHubStore();
  if (!hub.ok) return hub.response;
  const guarded = await guardPublicBody(req, HubPresenceSchema, "public");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;
  const store = hub.store;

  if (body.leave === true) {
    await store.presenceLeave(body.token);
    return jsonOk({ ok: true });
  }
  if (!ctx.limiter.hit(`hub:presence:${body.token}`, PRESENCE_PER_MIN)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }
  const online = await store.presenceBeat(body.token);
  return jsonOk({ ok: true, online });
}
