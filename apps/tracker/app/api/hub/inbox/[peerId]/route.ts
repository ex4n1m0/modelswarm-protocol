// GET /api/hub/inbox/{peerId}?ts_ms=&sig_b64= — drain that peer's mailbox
// (OnlyHumans hub port, ADR-013 D4; hub.rs `mail_drain`). Destructive by design
// (one delivery): returns every queued item and clears the box. Items are
// returned VERBATIM (opaque sealed ciphertext); the recipient verifies each
// item's signature + key→sender binding client-side (hub.rs verify_mail_item).
//
// OWNERSHIP GATING (reference inbox/[peerId].ts): the drain signature over
// OH1-drain-v1|peerId|ts_ms must verify against the peer's REGISTERED public key
// (the peer:<id> reg record). A stranger who knows a peerId — even with a valid
// signature from a DIFFERENT key — cannot read or delete its box (403). No reg
// record -> 403. The hub holds no private keys and never plaintexts env_json.
// Cache-Control: no-store on every response (reference drain sets no-store).

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk, noStore } from "@/lib/errors";
import { guardHubStore } from "@/lib/guard";
import { drainAuthorized } from "@/lib/hub-crypto";
import { HUB_DRAIN_FRESHNESS_MS } from "@/lib/hub-schemas";
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
  const url = new URL(req.url);
  // Absent ts_ms must read as NaN (Number(null) === 0 would slip past finite).
  const ts = Number(url.searchParams.get("ts_ms") ?? NaN);
  const sigB64 = url.searchParams.get("sig_b64");
  if (peerId.length < 8 || peerId.length > 128 || !Number.isFinite(ts) || !sigB64) {
    return noStore(jsonErrorFor("invalid_body", "bad drain request"));
  }
  if (Math.abs(ctx.now() - ts) > HUB_DRAIN_FRESHNESS_MS) {
    return noStore(jsonErrorFor("stale_timestamp", "drain ts outside the ±60 s window"));
  }

  // Ownership proof against the registered key (reference inbox/[peerId].ts).
  const store = hub.store;
  const reg = await store.getPeer(peerId);
  if (!drainAuthorized(peerId, ts, sigB64, reg?.public_key_b64)) {
    return noStore(jsonError(403, "forbidden", "drain not authorized"));
  }
  const items = await store.drainInbox(peerId);
  return noStore(jsonOk({ items }));
}
