// RelayAdmissionTicket issuance (ADR-038 §3.1/§3.3 — tracker half of the D12
// close). Pins: ticket on every successful heartbeat, the exact signed field
// set + canonical-JSON signature against the hub key, the frozen 600 s TTL,
// fresh nonces, ≤ 1024 B serialized form, and NO ticket on any failure path
// (unauthenticated, wrong installation, stale lease).
//
// Golden-fixture discipline: protocol/vectors/relay-ticket-1.json does not
// exist yet (it lands with the Protocol seat's crate/vectors half of the
// ADR-038 gate). Until then the fixture lives INLINE here, recorded exactly
// per the lease-hubkey-1.json discipline (fixture-only signing key, wire
// form + canonical_json_sha256 byte-pinned, consumer-neutral) — the PENDING
// CROSS-TIE is noted in docs/reviews/handoff-tracker-engineer-2026-10-15-relay-tickets.md.

import { beforeEach, describe, expect, it } from "vitest";
import * as heartbeatRoute from "@/app/api/v1/peers/heartbeat/route";
import {
  base64UrlEncode,
  canonicalJson,
  publicKeyOf,
  sha256Hex,
  signBase64,
  verifyBase64Url,
} from "@/lib/crypto";
import { issueRelayAdmissionTicket, splitRelayAdmissionTicket } from "@/lib/relay-ticket";
import { RELAY_TICKET_TTL_MS } from "@/lib/constants";
import {
  enroll,
  heartbeat,
  makeRig,
  register,
  seedActiveProfile,
  signedRequest,
  testManifest,
  HUB_SEED,
  type Enrollment,
  type TestRig,
} from "./helpers";

let rig: TestRig;
let profileA: string;

beforeEach(async () => {
  rig = makeRig();
  profileA = await seedActiveProfile(rig, testManifest());
});

/** Decode + structurally validate one served ticket. */
function decodeTicket(token: string): {
  fields: Record<string, unknown>;
  json: string;
  signature: string;
} {
  const parts = splitRelayAdmissionTicket(token);
  expect(parts).not.toBeNull();
  const fields = JSON.parse(parts!.json) as Record<string, unknown>;
  return { fields, json: parts!.json, signature: parts!.signature };
}

describe("heartbeat mints a relay admission ticket (ADR-038 §3.3)", () => {
  let who: Enrollment;
  let leaseId: string;

  beforeEach(async () => {
    who = await enroll(rig, "198.51.100.41");
    ({ leaseId } = await register(rig, who, [profileA]));
  });

  it("carries relayTicket on a successful heartbeat, bound to server-side rows", async () => {
    rig.advance(5_000);
    const res = await heartbeat(rig, who, leaseId, { profiles: [profileA] });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { relayTicket: string; leaseExpiresAt: string };
    expect(typeof body.relayTicket).toBe("string");
    expect(body.relayTicket.length).toBeGreaterThan(0);

    const { fields, json, signature } = decodeTicket(body.relayTicket);
    // Exact signed field set (T2 content-blind): nothing but the five fields.
    expect(Object.keys(fields).sort()).toEqual(
      ["expires_at", "installation_id", "issued_at", "nonce", "peer_id"],
    );
    // installation_id from the AUTHENTICATED identity, peer_id from the
    // tracker-side registered lease row — never client input.
    expect(fields.installation_id).toBe(who.installationId);
    expect(fields.peer_id).toBe(who.peerId);
    // RFC-3339 millisecond timestamps.
    expect(fields.issued_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    expect(fields.expires_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    // 32 lowercase hex nonce.
    expect(fields.nonce).toMatch(/^[0-9a-f]{32}$/);

    // Canonical JSON (sorted keys) and hub-key Ed25519 signature over it.
    expect(json).toBe(canonicalJson(fields));
    expect(verifyBase64Url(signature, json, publicKeyOf(HUB_SEED))).toBe(true);

    // Frozen serialized-length bound (ADR-038 §3.1: ≤ 1024 bytes).
    expect(Buffer.byteLength(body.relayTicket, "utf8")).toBeLessThanOrEqual(1024);
  });

  it("TTL delta is EXACTLY 600 s and issued_at is the heartbeat clock", async () => {
    rig.advance(7_000);
    const before = rig.ctx.now();
    const res = await heartbeat(rig, who, leaseId);
    expect(res.status).toBe(200);
    const { fields } = decodeTicket(((await res.json()) as { relayTicket: string }).relayTicket);
    expect(Date.parse(fields.issued_at as string)).toBe(before);
    expect(RELAY_TICKET_TTL_MS).toBe(600_000); // the frozen constant itself
    expect(Date.parse(fields.expires_at as string) - Date.parse(fields.issued_at as string))
      .toBe(600_000);
  });

  it("mints a fresh nonce per issuance (one per heartbeat)", async () => {
    const first = decodeTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket);
    rig.advance(15_000); // past envelope timestamp staleness
    const second = decodeTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket);
    expect(first.fields.nonce).not.toBe(second.fields.nonce);
    // Distinct full tickets (fresh signature over the new canonical form).
    const t1 = splitRelayAdmissionTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket)!;
    const t2 = splitRelayAdmissionTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket)!;
    expect(t1.signature).not.toBe(t2.signature);
  });
});

describe("no ticket on failure paths (ADR-038 §3.3: success-only issuance)", () => {
  it("unauthenticated request -> 401, no relayTicket anywhere in the body", async () => {
    const res = await heartbeatRoute.POST(
      new Request("http://tracker.local/api/v1/peers/heartbeat", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          leaseId: "0".repeat(32),
          activeProfiles: [],
          freeSlots: 1,
          queueMs: 0,
          draining: false,
        }),
      }),
    );
    expect(res.status).toBe(401);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.relayTicket).toBeUndefined();
  });

  it("another installation's lease -> 401 unauthorized, no ticket", async () => {
    const a = await enroll(rig, "198.51.100.42");
    const b = await enroll(rig, "198.51.100.43");
    const { leaseId } = await register(rig, a, [profileA]);
    const res = await heartbeatRoute.POST(
      signedRequest(
        b.keys,
        b.installationId,
        b.session,
        "/api/v1/peers/heartbeat",
        JSON.stringify({
          leaseId,
          activeProfiles: [],
          freeSlots: 1,
          queueMs: 0,
          draining: false,
        }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(401);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.relayTicket).toBeUndefined();
    expect(body).toEqual({ error: expect.any(Object) });
  });

  it("stale (expired) lease -> 410 lease_expired, no ticket", async () => {
    const who = await enroll(rig, "198.51.100.44");
    const { leaseId } = await register(rig, who, [profileA]);
    rig.advance(80_000); // past the 75 s row-aging TTL
    const res = await heartbeat(rig, who, leaseId);
    expect(res.status).toBe(410);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.relayTicket).toBeUndefined();
    // Unknown lease is the same refusal family.
    const ghost = await heartbeat(rig, who, "f".repeat(32));
    expect(ghost.status).toBe(401);
    expect(((await ghost.json()) as Record<string, unknown>).relayTicket).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Inline golden fixture — the lease-hubkey-1.json discipline, recorded while
// protocol/vectors/relay-ticket-1.json does not exist. The fixture key is
// 0707…0707 (the same fixture-only key as the lease vector — NOT the
// production hub key). installation_id/peer_id are the coherent identity of
// seed [9u8;32] (base58(sha256(pubkey)) / ADR-020 multihash derivation), so
// the relay's §3.4 check-8 re-derivation will hold for this fixture too.
// PENDING CROSS-TIE: when the Protocol seat lands the real vector, this
// fixture is replaced by (or asserted against) that file.

const FIXTURE = {
  signing_seed_hex: "0707070707070707070707070707070707070707070707070707070707070707",
  signing_public_key_hex: "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c",
  verify_at: "2026-10-15T12:05:00Z",
  fields: {
    installation_id: "FnrPPY5qYvRuY4baWjb11vSzekQ5y1dZd2xHP8vgkvi9",
    peer_id: "12D3KooWSrKnMZUcSxK8G7wmBbXdU8nFEfWGhLu6H8xjn8LmCSJb",
    issued_at: "2026-10-15T12:00:00.000Z",
    expires_at: "2026-10-15T12:10:00.000Z",
    nonce: "4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c",
  } as Record<string, unknown>,
  token:
    "eyJleHBpcmVzX2F0IjoiMjAyNi0xMC0xNVQxMjoxMDowMC4wMDBaIiwiaW5zdGFsbGF0aW9uX2lkIjoiRm5yUFBZNXFZdlJ1WTRiYVdqYjExdlN6ZWtRNXkxZFpkMnhIUDh2Z2t2aTkiLCJpc3N1ZWRfYXQiOiIyMDI2LTEwLTE1VDEyOjAwOjAwLjAwMFoiLCJub25jZSI6IjRmM2EyYjFjMGQ5ZThmN2E2YjVjNGQzZTJmMWEwYjljIiwicGVlcl9pZCI6IjEyRDNLb29XU3JLbk1aVWNTeEs4Rzd3bUJiWGRVOG5GRWZXR2hMdTZIOHhqbjhMbUNTSmIifQ.KZDVvVuiOJFvEY5BymSgqK-I2Py4StSqh16e9ydxzjZ6Bt06TtcS6m_wsy3B6nc-rtLfIe57e-A6V5Geyj8UAg",
  canonical_json_sha256:
    "a078e98fbd852a325cd69f5a459aae45f4c43f7d4c20035c3bf801ca6ab0d3eb",
};

describe("relay ticket golden fixture (ADR-038 §3.1 form)", () => {
  const publicKey = Buffer.from(FIXTURE.signing_public_key_hex, "hex");

  it("payload is the byte-exact canonical JSON of the recorded fields", () => {
    const parts = splitRelayAdmissionTicket(FIXTURE.token)!;
    expect(parts).not.toBeNull();
    expect(parts.json).toBe(canonicalJson(FIXTURE.fields));
    expect(sha256Hex(parts.json)).toBe(FIXTURE.canonical_json_sha256);
    expect(JSON.parse(parts.json)).toEqual(FIXTURE.fields);
  });

  it("signature verifies against the recorded fixture key; flipped byte fails", () => {
    const parts = splitRelayAdmissionTicket(FIXTURE.token)!;
    expect(verifyBase64Url(parts.signature, parts.json, publicKey)).toBe(true);
    const flipped = (Number.parseInt(parts.signature[0]!, 36) ^ 1).toString(36) + parts.signature.slice(1);
    expect(verifyBase64Url(flipped, parts.json, publicKey)).toBe(false);
    // Wrong key (the rig hub key is a different key) must also fail.
    expect(verifyBase64Url(parts.signature, parts.json, publicKeyOf(HUB_SEED))).toBe(false);
  });

  it("pins the frozen shape: exact field set, 600 s delta, 32-hex nonce, ≤ 1024 B", () => {
    const fields = FIXTURE.fields;
    expect(Object.keys(fields).sort()).toEqual(
      ["expires_at", "installation_id", "issued_at", "nonce", "peer_id"],
    );
    expect(Date.parse(fields.expires_at as string) - Date.parse(fields.issued_at as string))
      .toBe(600_000);
    expect((fields.nonce as string)).toMatch(/^[0-9a-f]{32}$/);
    expect(Buffer.byteLength(FIXTURE.token, "utf8")).toBeLessThanOrEqual(1024);
    // The admission deadline the relay computes: expires_at + 120 s skew grace.
    expect(Date.parse(fields.expires_at as string) + 120_000).toBe(
      Date.parse("2026-10-15T12:12:00.000Z"),
    );
  });

  it("the issuer reproduces the fixture's deterministic fields (nonce excepted)", () => {
    const issued = issueRelayAdmissionTicket(
      FIXTURE.fields.installation_id as string,
      FIXTURE.fields.peer_id as string,
      Date.parse(FIXTURE.fields.issued_at as string),
      Buffer.from(FIXTURE.signing_seed_hex, "hex"),
    );
    const parts = splitRelayAdmissionTicket(issued.token)!;
    const got = JSON.parse(parts.json) as Record<string, unknown>;
    const expected = { ...FIXTURE.fields };
    delete expected.nonce;
    delete got.nonce;
    expect(got).toEqual(expected);
    expect(verifyBase64Url(parts.signature, parts.json, publicKey)).toBe(true);
  });

  it("the wire form reconstructs byte-exactly from the frozen primitives", () => {
    // Rebuilding the serialized ticket from canonicalJson + the base64url
    // Ed25519 discipline (exactly what lib/relay-ticket.ts does) must
    // reproduce the recorded fixture token — pinning the whole
    // serialization discipline, not just its parse.
    const json = canonicalJson(FIXTURE.fields);
    const signature = Buffer.from(signBase64(json, Buffer.from(FIXTURE.signing_seed_hex, "hex")), "base64")
      .toString("base64url");
    expect(`${base64UrlEncode(json)}.${signature}`).toBe(FIXTURE.token);
  });
});
