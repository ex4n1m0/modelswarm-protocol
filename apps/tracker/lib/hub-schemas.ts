// Strict zod schemas for the OnlyHumans hub API ported onto the tracker
// (ADR-013 D4, P3 slice 5). The wire contract is the pinned onlyhumans-core
// client (crates/onlyhumans-core/src/hub.rs): snake_case fields, exact shapes.
// `.strict()` everywhere — unknown keys are rejected with invalid_body, the
// same content-blindness discipline as lib/schemas.ts.
//
// OPAQUE-RECORD INVARIANT (D4): the hub is untrusted storage — it holds NO
// private keys and never reads/plaintexts the sealed payload. These schemas
// validate SHAPE only; the routes additionally verify the PUBLIC Ed25519
// signature + key→id binding server-side (lib/hub-crypto.ts, the reference's
// proven posture) and store accepted records VERBATIM. Clients STILL re-verify
// every fetched record and the key→id binding (hub.rs). Verifying public
// signatures is fully compatible with "untrusted storage": it rejects forgeries
// before storage without ever touching plaintext.
//
// `env_json` is opaque sealed ciphertext (a stringified Envelope). It is the
// hub inbox's only free-form payload and is compatible with lib/store.ts's
// "nothing that could resemble prompt/completion text" invariant: it is covered
// by the sender's signature but never parsed, logged, or inspected — bytes in,
// bytes out.

import { z } from "zod";

/** libp2p peer ids are base58 strings; the reference bounds them by length
 *  (no alphabet check), so the port does the same. */
const PeerId = z.string().min(1).max(128);
/** Room ids are 32 lowercase hex chars (word+GK derived; D4 rooms are keyed
 *  by word+GK, NOT by account — see the route headers). */
export const ROOM_ID_PATTERN = /^[0-9a-f]{32}$/;
const RoomId = z.string().regex(ROOM_ID_PATTERN, "32 lowercase hex chars");
const PubKeyB64 = z.string().min(1).max(256);
const SigB64 = z.string().min(1).max(128);
const TsMs = z.number().int().min(0);

/** Per-item opaque envelope cap (reference inbox.ts: env_json ≤ 65536). */
export const HUB_INBOX_ENV_JSON_MAX = 65536;

/** PUT /api/hub/reg — hub.rs `Registration`. */
export const HubRegistrationSchema = z
  .object({
    peer_id: PeerId,
    public_key_b64: PubKeyB64,
    addrs: z.array(z.string().max(256)).max(16),
    ts_ms: TsMs,
    sig_b64: SigB64,
  })
  .strict();
export type HubRegistration = z.infer<typeof HubRegistrationSchema>;

/** PUT /api/hub/room — hub.rs `RoomHostRecord`. */
export const HubRoomHostSchema = z
  .object({
    room_id: RoomId,
    host_peer_id: PeerId,
    host_public_key_b64: PubKeyB64,
    ts_ms: TsMs,
    sig_b64: SigB64,
  })
  .strict();
export type HubRoomHost = z.infer<typeof HubRoomHostSchema>;

/** One queued mailbox envelope — hub.rs `MailItem`. `env_json` is opaque. */
export const HubMailItemSchema = z
  .object({
    to: z.string().min(8).max(128),
    from: z.string().min(8).max(128),
    public_key_b64: PubKeyB64,
    env_json: z.string().max(HUB_INBOX_ENV_JSON_MAX),
    ts_ms: TsMs,
    sig_b64: SigB64,
  })
  .strict();
export type HubMailItem = z.infer<typeof HubMailItemSchema>;

/** POST /api/hub/inbox — hub.rs `mail_push_many` sends `{ items: [...] }`.
 *  The reference caps a push at 16 items (1..16); the client asserts ≤16. */
export const HUB_INBOX_BATCH_MAX = 16;
export const HubInboxBatchSchema = z
  .object({
    items: z.array(HubMailItemSchema).min(1).max(HUB_INBOX_BATCH_MAX),
  })
  .strict();
export type HubInboxBatch = z.infer<typeof HubInboxBatchSchema>;

/** POST /api/hub/presence — hub.rs `PresencePing { token, leave }`. The token
 *  is random per app start (deliberately NOT a peer id); the reference bounds
 *  it to 16..64 lowercase hex chars. `leave` defaults to absent (heartbeat). */
export const HubPresenceSchema = z
  .object({
    token: z.string().regex(/^[0-9a-f]{16,64}$/, "16..64 lowercase hex chars"),
    leave: z.boolean().optional(),
  })
  .strict();
export type HubPresence = z.infer<typeof HubPresenceSchema>;

// ---------------------------------------------------------------------------
// Operational bounds (mirrored from OnlyHumans-main/api/*.ts)

/** A 16-item batch of max-size envelopes plus field overhead exceeds the
 *  tracker's default 64 KiB peer-body cap; the inbox is opaque ciphertext
 *  storage (like the reference, which rides Vercel's ~4.5 MB limit), so it
 *  gets its own documented cap that comfortably holds the schema maximum. */
export const HUB_INBOX_MAX_BODY_BYTES = 2 * 1024 * 1024;

/** Freshness windows are anti-replay, NOT identity (the signature binds the
 *  payload; verification is client-side). Values mirror the reference:
 *  reg/room ±10 min (drifted desktop clocks), inbox ±25 h (keeps join working
 *  across the two flows), drain ±60 s. */
export const HUB_REG_FRESHNESS_MS = 10 * 60_000;
export const HUB_ROOM_FRESHNESS_MS = 10 * 60_000;
export const HUB_INBOX_FRESHNESS_MS = 25 * 3600_000;
export const HUB_DRAIN_FRESHNESS_MS = 60_000;
