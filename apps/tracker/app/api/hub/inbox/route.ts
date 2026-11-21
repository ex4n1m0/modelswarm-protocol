// POST /api/hub/inbox — queue sealed envelopes for unreachable peers as ONE
// mixed-recipient batch (OnlyHumans hub port, ADR-013 D4; hub.rs
// `mail_push_many` sends `{ items: [...] }`, ≤16). Account-ORTHOGONAL: no
// session gate. Each item is opaque end-to-end sealed ciphertext (`env_json`)
// plus the sender's signature. The hub verifies every item's PUBLIC signature
// before storing (reference inbox.ts) and stores it VERBATIM — it never reads
// or plaintexts env_json. Recipients verify AGAIN and check the key→sender
// binding client-side (hub.rs verify_mail_item); the reference does not check
// that binding server-side, so neither does this port.
//
// Batch cap 1..16 (the client asserts ≤16). Per box: TTL 86400 s, keep the last
// 32 items. The body cap is the hub inbox's own (2 MiB), NOT the tracker's
// 64 KiB peer cap — a 16-item batch of 64 KiB envelopes exceeds 64 KiB, and the
// reference rides Vercel's ~4.5 MB limit. Rate class "public" per IP, plus a
// per-sender throttle mirroring the reference's `rl:mail:<from>` 1-per-4s NX.

import { getTrackerContext } from "@/lib/context";
import { jsonError, jsonErrorFor, jsonOk } from "@/lib/errors";
import { parseJsonBody } from "@/lib/envelope";
import { mailItemVerified } from "@/lib/hub-crypto";
import { getHubStore } from "@/lib/hub-store";
import { HubInboxBatchSchema, HUB_INBOX_FRESHNESS_MS, HUB_INBOX_MAX_BODY_BYTES } from "@/lib/hub-schemas";
import { clientIpOf } from "@/lib/ratelimit";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// In-memory approximation of the reference's `rl:mail:<from>` 1-per-4s NX.
const MAIL_PER_MIN = 15;

export async function POST(req: Request): Promise<Response> {
  const ctx = getTrackerContext();
  const ip = clientIpOf(req);
  if (!ctx.limiter.hit(`public:${ip}`, ctx.limits.publicPerIp)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }

  // Size cap first (hub inbox cap; see header). Mirrors envelope.readBody.
  const declared = req.headers.get("content-length");
  if (declared !== null) {
    const value = Number.parseInt(declared, 10);
    if (Number.isFinite(value) && value > HUB_INBOX_MAX_BODY_BYTES) {
      return jsonError(413, "payload_too_large", "inbox batch exceeds the 2 MiB cap");
    }
  }
  const text = await req.text();
  if (Buffer.byteLength(text, "utf8") > HUB_INBOX_MAX_BODY_BYTES) {
    return jsonError(413, "payload_too_large", "inbox batch exceeds the 2 MiB cap");
  }

  const parsed = parseJsonBody(text, HubInboxBatchSchema, new URL(req.url).pathname);
  if (!parsed.ok) return parsed.response;
  const items = parsed.value.items;

  if (items.some((it) => Math.abs(ctx.now() - it.ts_ms) > HUB_INBOX_FRESHNESS_MS)) {
    return jsonErrorFor("stale_timestamp", "inbox item ts outside the ±25 h window");
  }
  // Verify every signature against the sender's included libp2p key (reference
  // inbox.ts). env_json is covered by the signature but never inspected.
  for (const it of items) {
    if (!mailItemVerified(it)) {
      return jsonError(403, "forbidden", "signature verification failed");
    }
  }
  // All items in a push share one sender (hub.rs signs each with the same id).
  if (!ctx.limiter.hit(`hub:mail:${items[0]!.from}`, MAIL_PER_MIN)) {
    return jsonError(429, "rate_limited", "rate limit exceeded", true);
  }

  await getHubStore(ctx.now).appendInbox(items);
  return jsonOk({ ok: true, stored: items.length });
}
