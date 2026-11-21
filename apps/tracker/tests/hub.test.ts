// OnlyHumans hub port (ADR-013 D4, P3 slice 5). Drives the REAL route handlers
// with Request objects against an injected MemoryHubStore (the same DI mocking
// pattern account.test.ts uses for MemoryStore via makeRig).
//
// The hub verifies PUBLIC Ed25519 signatures + key→id binding server-side
// (lib/hub-crypto.ts) while keeping env_json opaque. These tests therefore use
// REAL Ed25519 identities and sign the canonical preimages INDEPENDENTLY (the
// same pipe-delimited strings the pinned client hub.rs signs — not by calling
// the server's canonicalizers), so a verify-path bug cannot hide behind a
// shared-helper tautology. Security regressions locked down: forged reg/room/mail
// signatures, reg/room key≠id, and non-owner vs owner inbox drain. A focused
// UpstashHubStore block mocks fetch to prove dm1: keyspace + dedicated-token
// isolation.

import { createPrivateKey, randomBytes, sign as nodeSign } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as regRoute from "@/app/api/hub/reg/route";
import * as lookupRoute from "@/app/api/hub/lookup/[peerId]/route";
import * as roomRoute from "@/app/api/hub/room/route";
import * as roomGetRoute from "@/app/api/hub/room/[roomId]/route";
import * as inboxRoute from "@/app/api/hub/inbox/route";
import * as inboxDrainRoute from "@/app/api/hub/inbox/[peerId]/route";
import * as presenceRoute from "@/app/api/hub/presence/route";
import { base64UrlEncode, derivePeerId, publicKeyOf } from "@/lib/crypto";
import { MemoryHubStore, UpstashHubStore, setHubStore } from "@/lib/hub-store";
import { HUB_INBOX_BATCH_MAX } from "@/lib/hub-schemas";
import type { HubMailItem, HubRegistration, HubRoomHost } from "@/lib/hub-schemas";
import { makeRig } from "./helpers";

let rig: ReturnType<typeof makeRig>;
let hub: MemoryHubStore;
let ipCounter = 1;

beforeEach(() => {
  rig = makeRig();
  hub = new MemoryHubStore(rig.ctx.now);
  setHubStore(hub);
  ipCounter = 1;
});
afterEach(() => {
  setHubStore(null);
  vi.restoreAllMocks();
});

const nextIp = () => `198.51.100.${(ipCounter++ % 250) + 1}`;
const ROOM_ID = "0123456789abcdef0123456789abcdef";
// Opaque sealed ciphertext the hub must store verbatim and never inspect —
// deliberately carries prompt/completion-like text to prove content-blindness.
const OPAQUE_ENV = '{"sealed":"opaque-ciphertext","note":"prompt/completion-like text the hub never parses"}';

// ---------------------------------------------------------------------------
// Real Ed25519 identities + independent canonical signing (mirrors hub.rs)

interface HubIdentity {
  seed: Uint8Array;
  peerId: string;
  pubB64: string; // URL-safe base64 (no padding) of the libp2p PublicKey protobuf
}

function makeIdentity(): HubIdentity {
  const seed = new Uint8Array(randomBytes(32));
  const pub = publicKeyOf(seed);
  // libp2p PublicKey protobuf: 0x08 0x01 (Type=Ed25519), 0x12 0x20 (Data, len 32).
  const protobuf = new Uint8Array(36);
  protobuf[0] = 0x08;
  protobuf[1] = 0x01;
  protobuf[2] = 0x12;
  protobuf[3] = 0x20;
  protobuf.set(pub, 4);
  return { seed, peerId: derivePeerId(pub), pubB64: base64UrlEncode(protobuf) };
}

const ID_A = makeIdentity();
const ID_B = makeIdentity();
const ID_C = makeIdentity();

const enc = (s: string): Uint8Array => new TextEncoder().encode(s);

/** Ed25519 sign via node:crypto (PKCS8 wrap of the 32-byte seed), URL-safe b64. */
function signHub(msg: Uint8Array, seed: Uint8Array): string {
  const pkcs8 = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.from(seed)]);
  const key = createPrivateKey({ key: pkcs8, format: "der", type: "pkcs8" });
  return base64UrlEncode(new Uint8Array(nodeSign(null, Buffer.from(msg), key)));
}

function regRecord(id: HubIdentity = ID_A, fields: Partial<HubRegistration> = {}): HubRegistration {
  const peer_id = fields.peer_id ?? id.peerId;
  const public_key_b64 = fields.public_key_b64 ?? id.pubB64;
  const addrs = fields.addrs ?? ["/ip4/1.2.3.4/udp/4001/quic-v1"];
  const ts_ms = fields.ts_ms ?? rig.ctx.now();
  const sig_b64 = fields.sig_b64 ?? signHub(enc(`OH1-reg|${peer_id}|${public_key_b64}|${addrs.join(",")}|${ts_ms}`), id.seed);
  return { peer_id, public_key_b64, addrs, ts_ms, sig_b64 };
}

function roomRecord(id: HubIdentity = ID_A, fields: Partial<HubRoomHost> = {}): HubRoomHost {
  const room_id = fields.room_id ?? ROOM_ID;
  const host_peer_id = fields.host_peer_id ?? id.peerId;
  const host_public_key_b64 = fields.host_public_key_b64 ?? id.pubB64;
  const ts_ms = fields.ts_ms ?? rig.ctx.now();
  const sig_b64 = fields.sig_b64 ?? signHub(enc(`OH1-room|${room_id}|${host_peer_id}|${host_public_key_b64}|${ts_ms}`), id.seed);
  return { room_id, host_peer_id, host_public_key_b64, ts_ms, sig_b64 };
}

function mailRecord(from: HubIdentity, to: string, fields: Partial<HubMailItem> = {}): HubMailItem {
  const fromId = fields.from ?? from.peerId;
  const toId = fields.to ?? to;
  const public_key_b64 = fields.public_key_b64 ?? from.pubB64;
  const env_json = fields.env_json ?? OPAQUE_ENV;
  const ts_ms = fields.ts_ms ?? rig.ctx.now();
  const sig_b64 = fields.sig_b64 ?? signHub(enc(`OH1-mail-v1|${fromId}|${toId}|${ts_ms}|${env_json}`), from.seed);
  return { to: toId, from: fromId, public_key_b64, env_json, ts_ms, sig_b64 };
}

const drainSig = (peerId: string, ts: number, id: HubIdentity): string =>
  signHub(enc(`OH1-drain-v1|${peerId}|${ts}`), id.seed);

function req(method: string, path: string, body?: unknown, ip = nextIp()): Request {
  const headers: Record<string, string> = { "x-forwarded-for": ip };
  if (body !== undefined) headers["content-type"] = "application/json";
  return new Request(`http://tracker.local${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

const lookupReq = (peerId: string) => lookupRoute.GET(req("GET", `/api/hub/lookup/${peerId}`), { params: Promise.resolve({ peerId }) });
const roomGetReq = (roomId: string) => roomGetRoute.GET(req("GET", `/api/hub/room/${roomId}`), { params: Promise.resolve({ roomId }) });
const drainReq = (peerId: string, ts: number, sig: string) =>
  inboxDrainRoute.GET(req("GET", `/api/hub/inbox/${peerId}?ts_ms=${ts}&sig_b64=${sig}`), { params: Promise.resolve({ peerId }) });

async function register(id: HubIdentity): Promise<void> {
  const res = await regRoute.PUT(req("PUT", "/api/hub/reg", regRecord(id)));
  expect(res.status).toBe(200);
}

const errCode = async (res: Response) => ((await res.json()) as { error: { code: string } }).error.code;

// ---------------------------------------------------------------------------
// reg + lookup
describe("hub reg / lookup", () => {
  it("reg round-trips the opaque record verbatim and echoes the observed IP", async () => {
    const reg = regRecord(ID_A);
    const put = await regRoute.PUT(req("PUT", "/api/hub/reg", reg, "203.0.113.7"));
    expect(put.status).toBe(200);
    expect(await put.json()).toEqual({ ok: true, observedIp: "203.0.113.7" });
    const got = await lookupReq(ID_A.peerId);
    expect(got.status).toBe(200);
    expect(await got.json()).toEqual(reg); // every field unchanged
  });

  it("lookup of an unknown peer -> 404 (hub.rs maps to None)", async () => {
    expect((await lookupReq(ID_B.peerId)).status).toBe(404);
  });

  it("lookup rejects an over-long peer id -> 400", async () => {
    const long = "x".repeat(129);
    const res = await lookupReq(long);
    expect(res.status).toBe(400);
    expect(await errCode(res)).toBe("invalid_body");
  });

  it("reg rejects a stale timestamp -> 400 stale_timestamp", async () => {
    const res = await regRoute.PUT(req("PUT", "/api/hub/reg", regRecord(ID_A, { ts_ms: rig.ctx.now() - 11 * 60_000 })));
    expect(res.status).toBe(400);
    expect(await errCode(res)).toBe("stale_timestamp");
  });

  it("reg rejects unknown keys (strict / content-blind) -> 400", async () => {
    const res = await regRoute.PUT(req("PUT", "/api/hub/reg", { ...regRecord(ID_A), messages: [] }));
    expect(res.status).toBe(400);
    expect(await errCode(res)).toBe("invalid_body");
  });

  it("reg record expires after the 300 s TTL", async () => {
    await regRoute.PUT(req("PUT", "/api/hub/reg", regRecord(ID_A)));
    rig.advance(301_000);
    expect((await lookupReq(ID_A.peerId)).status).toBe(404);
  });

  it("SECURITY: rejects a forged registration signature -> 403", async () => {
    const forged = regRecord(ID_A, { sig_b64: signHub(enc("not-the-registration-preimage"), ID_A.seed) });
    const res = await regRoute.PUT(req("PUT", "/api/hub/reg", forged));
    expect(res.status).toBe(403);
    expect(await errCode(res)).toBe("forbidden");
    expect((await lookupReq(ID_A.peerId)).status).toBe(404); // never stored
  });

  it("SECURITY: rejects a reg whose key does not derive the claimed peer_id -> 403", async () => {
    // Signed by A's key (valid signature) but claims B's peer id.
    const mismatch = regRecord(ID_A, { peer_id: ID_B.peerId });
    const res = await regRoute.PUT(req("PUT", "/api/hub/reg", mismatch));
    expect(res.status).toBe(403);
    expect(await errCode(res)).toBe("forbidden");
  });
});

// ---------------------------------------------------------------------------
// room election
describe("hub room NX election", () => {
  it("first writer wins; a different host gets 409; lookup returns the winner", async () => {
    const a = roomRecord(ID_A);
    expect((await roomRoute.PUT(req("PUT", "/api/hub/room", a))).status).toBe(200);
    expect((await roomRoute.PUT(req("PUT", "/api/hub/room", roomRecord(ID_B)))).status).toBe(409);
    const got = await roomGetReq(ROOM_ID);
    expect(got.status).toBe(200);
    expect(await got.json()).toEqual(a);
  });

  it("the CURRENT host (same id + key) may refresh its own record", async () => {
    await roomRoute.PUT(req("PUT", "/api/hub/room", roomRecord(ID_A)));
    rig.advance(120_000);
    const refreshed = roomRecord(ID_A); // new ts, same host id + key
    expect((await roomRoute.PUT(req("PUT", "/api/hub/room", refreshed))).status).toBe(200);
    expect(await (await roomGetReq(ROOM_ID)).json()).toEqual(refreshed);
  });

  it("a new host may claim after the record's TTL lapses", async () => {
    await roomRoute.PUT(req("PUT", "/api/hub/room", roomRecord(ID_A)));
    rig.advance(301_000);
    expect((await roomRoute.PUT(req("PUT", "/api/hub/room", roomRecord(ID_B)))).status).toBe(200);
  });

  it("room lookup of an unhosted room -> 404; bad room id -> 400", async () => {
    expect((await roomGetReq(ROOM_ID)).status).toBe(404);
    expect((await roomGetReq("nothex")).status).toBe(400);
  });

  it("SECURITY: rejects a forged room host signature -> 403", async () => {
    const forged = roomRecord(ID_A, { sig_b64: signHub(enc("not-the-room-preimage"), ID_A.seed) });
    const res = await roomRoute.PUT(req("PUT", "/api/hub/room", forged));
    expect(res.status).toBe(403);
    expect(await errCode(res)).toBe("forbidden");
    expect((await roomGetReq(ROOM_ID)).status).toBe(404); // never stored
  });

  it("SECURITY: rejects a room claim whose key does not derive the host id -> 403", async () => {
    // A's key signing a claim that names B as host (a hijack/refresh attempt).
    const mismatch = roomRecord(ID_A, { host_peer_id: ID_B.peerId });
    const res = await roomRoute.PUT(req("PUT", "/api/hub/room", mismatch));
    expect(res.status).toBe(403);
    expect(await errCode(res)).toBe("forbidden");
  });
});

// ---------------------------------------------------------------------------
// inbox batch + drain
describe("hub inbox batch / drain", () => {
  it("rejects a batch over 16 items -> 400", async () => {
    const items = Array.from({ length: HUB_INBOX_BATCH_MAX + 1 }, () => mailRecord(ID_A, ID_B.peerId));
    const res = await inboxRoute.POST(req("POST", "/api/hub/inbox", { items }));
    expect(res.status).toBe(400);
    expect(await errCode(res)).toBe("invalid_body");
  });

  it("rejects an empty batch -> 400", async () => {
    expect((await inboxRoute.POST(req("POST", "/api/hub/inbox", { items: [] }))).status).toBe(400);
  });

  it("stores a mixed-recipient batch and drains it verbatim (destructive)", async () => {
    await register(ID_A);
    await register(ID_B);
    const toB = mailRecord(ID_A, ID_B.peerId);
    const toA = mailRecord(ID_A, ID_A.peerId, { env_json: '{"sealed":"second"}' });
    const push = await inboxRoute.POST(req("POST", "/api/hub/inbox", { items: [toB, toA] }));
    expect(push.status).toBe(200);
    expect(await push.json()).toEqual({ ok: true, stored: 2 });

    const ts = rig.ctx.now();
    const drainB = await drainReq(ID_B.peerId, ts, drainSig(ID_B.peerId, ts, ID_B));
    expect(drainB.status).toBe(200);
    expect(await drainB.json()).toEqual({ items: [toB] }); // opaque env_json verbatim
    expect(await (await drainReq(ID_B.peerId, ts, drainSig(ID_B.peerId, ts, ID_B))).json()).toEqual({ items: [] });
    expect(await (await drainReq(ID_A.peerId, ts, drainSig(ID_A.peerId, ts, ID_A))).json()).toEqual({ items: [toA] });
  });

  it("drain requires ts_ms + sig_b64 and a fresh timestamp", async () => {
    const noSig = await inboxDrainRoute.GET(req("GET", `/api/hub/inbox/${ID_B.peerId}?ts_ms=${rig.ctx.now()}`), {
      params: Promise.resolve({ peerId: ID_B.peerId }),
    });
    expect(noSig.status).toBe(400);
    const stale = await drainReq(ID_B.peerId, rig.ctx.now() - 61_000, drainSig(ID_B.peerId, rig.ctx.now() - 61_000, ID_B));
    expect(stale.status).toBe(400);
    expect(await errCode(stale)).toBe("stale_timestamp");
  });

  it("rejects a stale inbox item -> 400", async () => {
    const items = [mailRecord(ID_A, ID_B.peerId, { ts_ms: rig.ctx.now() - 26 * 3600_000 })];
    const res = await inboxRoute.POST(req("POST", "/api/hub/inbox", { items }));
    expect(res.status).toBe(400);
    expect(await errCode(res)).toBe("stale_timestamp");
  });

  it("each box keeps only the last 32 items (store-level trim)", async () => {
    const items = Array.from({ length: 40 }, (_, i) => mailRecord(ID_A, ID_B.peerId, { env_json: `{"n":${i}}` }));
    await hub.appendInbox(items);
    const drained = await hub.drainInbox(ID_B.peerId);
    expect(drained).toHaveLength(32);
    expect(drained[0]!.env_json).toBe('{"n":8}');
    expect(drained[31]!.env_json).toBe('{"n":39}');
  });

  it("SECURITY: rejects a batch with a forged item signature -> 403", async () => {
    const forged = mailRecord(ID_A, ID_B.peerId, { sig_b64: signHub(enc("not-the-mail-preimage"), ID_A.seed) });
    const res = await inboxRoute.POST(req("POST", "/api/hub/inbox", { items: [forged] }));
    expect(res.status).toBe(403);
    expect(await errCode(res)).toBe("forbidden");
  });

  it("SECURITY: rejects a non-owner drain (valid sig from a DIFFERENT key) -> 403, box intact", async () => {
    await register(ID_B);
    await inboxRoute.POST(req("POST", "/api/hub/inbox", { items: [mailRecord(ID_A, ID_B.peerId)] }));
    const ts = rig.ctx.now();
    const stranger = await drainReq(ID_B.peerId, ts, drainSig(ID_B.peerId, ts, ID_C));
    expect(stranger.status).toBe(403);
    expect(await errCode(stranger)).toBe("forbidden");
    // The 403 attempt did NOT delete the box: the owner can still drain it.
    const owner = await drainReq(ID_B.peerId, ts, drainSig(ID_B.peerId, ts, ID_B));
    expect(owner.status).toBe(200);
    expect(((await owner.json()) as { items: unknown[] }).items).toHaveLength(1);
  });

  it("SECURITY: accepts a drain signed by the registered owner -> 200", async () => {
    await register(ID_B);
    const item = mailRecord(ID_A, ID_B.peerId);
    await inboxRoute.POST(req("POST", "/api/hub/inbox", { items: [item] }));
    const ts = rig.ctx.now();
    const res = await drainReq(ID_B.peerId, ts, drainSig(ID_B.peerId, ts, ID_B));
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ items: [item] });
  });

  it("SECURITY: rejects draining an unregistered peer's box -> 403", async () => {
    const ts = rig.ctx.now();
    const res = await drainReq(ID_C.peerId, ts, drainSig(ID_C.peerId, ts, ID_C));
    expect(res.status).toBe(403);
  });
});

// ---------------------------------------------------------------------------
// presence (identity-free; no signatures)
describe("hub presence", () => {
  const TOKEN_A = "a".repeat(32);
  const TOKEN_B = "b".repeat(32);

  it("counts live tokens and expires them after the 300 s TTL", async () => {
    const a = await presenceRoute.POST(req("POST", "/api/hub/presence", { token: TOKEN_A, leave: false }));
    expect(a.status).toBe(200);
    expect(await a.json()).toEqual({ ok: true, online: 1 });
    expect(await (await presenceRoute.POST(req("POST", "/api/hub/presence", { token: TOKEN_B, leave: false }))).json()).toEqual({ ok: true, online: 2 });
    rig.advance(301_000);
    expect(await (await presenceRoute.POST(req("POST", "/api/hub/presence", { token: TOKEN_A, leave: false }))).json()).toEqual({ ok: true, online: 1 });
  });

  it("leave removes the token (graceful exit)", async () => {
    await presenceRoute.POST(req("POST", "/api/hub/presence", { token: TOKEN_A, leave: false }));
    const leave = await presenceRoute.POST(req("POST", "/api/hub/presence", { token: TOKEN_A, leave: true }));
    expect(leave.status).toBe(200);
    expect(await leave.json()).toEqual({ ok: true });
    expect(await (await presenceRoute.POST(req("POST", "/api/hub/presence", { token: TOKEN_B, leave: false }))).json()).toEqual({ ok: true, online: 1 });
  });

  it("rejects a non-hex token -> 400", async () => {
    const res = await presenceRoute.POST(req("POST", "/api/hub/presence", { token: "not-a-hex-token!", leave: false }));
    expect(res.status).toBe(400);
    expect(await errCode(res)).toBe("invalid_body");
  });
});

// ---------------------------------------------------------------------------
// UpstashHubStore isolation (dm1: keyspace + dedicated token), fetch mocked.
describe("UpstashHubStore dm1: isolation", () => {
  const TOKEN_HEX = "c".repeat(32);
  it("prefixes every key with dm1: and uses the dedicated bearer token", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(async (url: unknown, init?: RequestInit) => {
      const u = String(url);
      calls.push({ url: u, init });
      const body = u.includes("/pipeline") ? [{ result: 0 }, { result: 2 }] : { result: "OK" };
      return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
    });
    const store = new UpstashHubStore("https://upstash.example/", "dm1-token", () => rig.ctx.now());

    await store.setPeer(ID_A.peerId, regRecord(ID_A));
    expect(calls[0]!.url.startsWith("https://upstash.example/set/dm1%3Apeer%3A")).toBe(true);
    expect(calls[0]!.url).toContain("?EX=300");
    expect((calls[0]!.init!.headers as Record<string, string>).Authorization).toBe("Bearer dm1-token");

    await store.setRoom(ROOM_ID, roomRecord(ID_A), true);
    expect(calls[1]!.url.startsWith("https://upstash.example/set/dm1%3Aroom%3A")).toBe(true);
    expect(calls[1]!.url).toContain("&NX");

    await store.appendInbox([mailRecord(ID_A, ID_B.peerId)]);
    const pipeBody = JSON.parse(String(calls[2]!.init!.body)) as unknown[][];
    expect(pipeBody.some((c) => c[0] === "rpush" && String(c[1]).startsWith("dm1:inbox:"))).toBe(true);

    await store.presenceBeat(TOKEN_HEX);
    const zaddBody = JSON.parse(String(calls[3]!.init!.body)) as unknown[][];
    expect(zaddBody.some((c) => c[0] === "zadd" && c[1] === "dm1:presence")).toBe(true);
  });
});
