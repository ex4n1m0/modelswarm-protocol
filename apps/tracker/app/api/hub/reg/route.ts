// PUT /api/hub/reg — peers publish their current addresses (OnlyHumans hub
// port, ADR-013 D4 / P3 slice 5; wire contract = onlyhumans-core hub.rs
// `Registration`). Account-ORTHOGONAL: no session/envelope gate.
//
// The hub is untrusted storage in the D4 sense (no private keys; the sealed
// payload stays opaque), but it DOES verify the PUBLIC Ed25519 signature over
// the canonical registration bytes AND that the included key derives the
// claimed peer_id before storing (reference reg.ts) — forged/mismatched records
// are rejected 403. Clients re-verify every fetched record again (hub.rs). The
// accepted record is stored VERBATIM. TTL 300 s. Mini-STUN: echo the observed
// public IP (advisory response data, never stored) as the reference does.

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardPublicBody } from "@/lib/guard";
import { registrationVerified } from "@/lib/hub-crypto";
import { getHubStore } from "@/lib/hub-store";
import { HubRegistrationSchema, HUB_REG_FRESHNESS_MS } from "@/lib/hub-schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// In-memory approximation of the reference's `rl:<peer>` 1-per-30s NX throttle
// (the app re-registers every ~120 s, so this only stops accidental hammering).
const REG_PER_MIN = 2;

export async function PUT(req: Request): Promise<Response> {
  const guarded = await guardPublicBody(req, HubRegistrationSchema, "public");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  if (Math.abs(ctx.now() - body.ts_ms) > HUB_REG_FRESHNESS_MS) {
    return jsonErrorFor("stale_timestamp", "registration ts outside the ±10 min window");
  }
  // Bind the registration to the signer: without this anyone could publish
  // addresses under another peer's id and redirect dials (reference reg.ts).
  if (!registrationVerified(body)) {
    return jsonError(403, "forbidden", "signature verification failed (key does not derive peer id)");
  }
  if (!ctx.limiter.hit(`hub:reg:${body.peer_id}`, REG_PER_MIN)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }

  await getHubStore(ctx.now).setPeer(body.peer_id, body);
  return jsonOk({ ok: true, observedIp: observedIp(req) });
}

/** First IPv4 in X-Forwarded-For, advisory only (mirrors reference reg.ts). */
function observedIp(req: Request): string | undefined {
  const first = (req.headers.get("x-forwarded-for") ?? "").split(",")[0]!.trim();
  return /^\d{1,3}(\.\d{1,3}){3}$/.test(first) ? first : undefined;
}
