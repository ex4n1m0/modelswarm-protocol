// PUT /api/hub/room — publish who currently hosts a room (OnlyHumans hub port,
// ADR-013 D4; hub.rs `register_room`). Account-ORTHOGONAL: D4 rooms are keyed
// by word+GK (the 32-hex room_id), NOT by account — no session gate.
//
// The hub verifies the PUBLIC host-record signature AND that the host key
// derives host_peer_id before the election (reference room.ts) — a forged or
// mismatched claim is rejected 403. Without the binding anyone could "refresh"
// a live record they do not own by naming its host and signing with their own
// key, pinning a dead host's pointer forever. Accepted records store VERBATIM.
//
// First-writer-wins election (NX): a NEW host claims with SET NX and loses
// (409) if a live record exists — hub.rs maps 409 to Ok(false) ("join them").
// The CURRENT host may refresh with a plain SET (extend the 300 s TTL) but
// ONLY when both host_peer_id AND host_public_key_b64 match the live record,
// so the identity key is the refresh credential (mirrors reference room.ts;
// hosts retry every ~120 s while records live 300 s).

import { jsonError, jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardHubStore, guardPublicBody } from "@/lib/guard";
import { roomHostVerified } from "@/lib/hub-crypto";
import { HubRoomHostSchema, HUB_ROOM_FRESHNESS_MS } from "@/lib/hub-schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function PUT(req: Request): Promise<Response> {
  const hub = guardHubStore();
  if (!hub.ok) return hub.response;
  const guarded = await guardPublicBody(req, HubRoomHostSchema, "public");
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  if (Math.abs(ctx.now() - body.ts_ms) > HUB_ROOM_FRESHNESS_MS) {
    return jsonErrorFor("stale_timestamp", "room ts outside the ±10 min window");
  }
  if (!roomHostVerified(body)) {
    return jsonError(403, "forbidden", "signature verification failed (key does not derive host peer id)");
  }

  const store = hub.store;
  const existing = await store.getRoom(body.room_id);
  const mine =
    existing !== null &&
    existing.host_peer_id === body.host_peer_id &&
    existing.host_public_key_b64 === body.host_public_key_b64;
  const won = await store.setRoom(body.room_id, body, !mine);
  if (!won) {
    // 409 = election lost. hub.rs reads only the status (Ok(false)); the
    // reference's `record` body field is omitted — lookup_room fetches it.
    return jsonError(409, "already_claimed", "room already hosted");
  }
  return jsonOk({ ok: true });
}
