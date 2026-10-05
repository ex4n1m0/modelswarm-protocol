// C enrollment + signed-request hardening (C1–C5).

import { beforeEach, describe, expect, it } from "vitest";
import * as deviceStart from "@/app/api/v1/auth/device/start/route";
import * as deviceComplete from "@/app/api/v1/auth/device/complete/route";
import * as registerRoute from "@/app/api/v1/peers/register/route";
import * as heartbeatRoute from "@/app/api/v1/peers/heartbeat/route";
import {
  enroll,
  keyPairIds,
  makeKeypair,
  makeRig,
  register,
  seedActiveProfile,
  signedRequest,
  testManifest,
} from "./helpers";

let rig: ReturnType<typeof makeRig>;
let profileId: string;

beforeEach(async () => {
  rig = makeRig();
  profileId = await seedActiveProfile(rig, testManifest());
});

describe("C1 device enrollment", () => {
  it("start -> complete-without-approval -> 403 pending; approve -> session", async () => {
    const enrolledKeys = makeKeypair();
    const ids = keyPairIds(enrolledKeys);

    const startRes = await deviceStart.POST(
      new Request("http://tracker.local/api/v1/auth/device/start", {
        method: "POST",
        headers: { "content-type": "application/json", "x-forwarded-for": "198.51.100.1" },
        body: JSON.stringify({ installationId: ids.installationId, pubKey: ids.pubKeyB58 }),
      }),
    );
    expect(startRes.status).toBe(200);
    const start = (await startRes.json()) as {
      deviceCode: string;
      userCode: string;
      verifyUrl: string;
      expiresAt: string;
    };
    expect(start.deviceCode.length).toBeGreaterThanOrEqual(16);
    expect(start.userCode).toMatch(/^[A-Z2-9]{8}$/);
    expect(start.verifyUrl).toContain("https://");
    expect(start.expiresAt).toBeTruthy();

    const completeBody = JSON.stringify({ deviceCode: start.deviceCode });
    const pendingRes = await deviceComplete.POST(
      signedRequest(enrolledKeys, ids.installationId, "", "/api/v1/auth/device/complete", completeBody, rig.ctx.now, { ip: "198.51.100.1" }),
    );
    expect(pendingRes.status).toBe(403);
    expect(((await pendingRes.json()) as { error: { code: string } }).error.code).toBe("pending");

    // ops-runbook stand-in approval hook
    expect(await rig.store.approveDevice(start.deviceCode)).toBe(true);

    const okRes = await deviceComplete.POST(
      signedRequest(enrolledKeys, ids.installationId, "", "/api/v1/auth/device/complete", completeBody, rig.ctx.now, { ip: "198.51.100.1" }),
    );
    expect(okRes.status).toBe(200);
    const session = (await okRes.json()) as { token: string; expiresAt: string };
    expect(session.token.length).toBeGreaterThanOrEqual(32);
    // 24 h session
    expect(Date.parse(session.expiresAt) - rig.ctx.now()).toBe(24 * 60 * 60 * 1000);
  });

  it("approve-by-user-code (the /verify page hook) completes the same loop; unknown code -> 404", async () => {
    // Routes resolve the global context; give this whole loop one rig that
    // carries an admin token (the outer beforeEach rig has adminToken: null).
    const adminRig = makeRig({ adminToken: "test-admin-token" });
    const keys = makeKeypair();
    const ids = keyPairIds(keys);

    const startRes = await deviceStart.POST(
      new Request("http://tracker.local/api/v1/auth/device/start", {
        method: "POST",
        headers: { "content-type": "application/json", "x-forwarded-for": "198.51.100.10" },
        body: JSON.stringify({ installationId: ids.installationId, pubKey: ids.pubKeyB58 }),
      }),
    );
    expect(startRes.status).toBe(200);
    const start = (await startRes.json()) as { deviceCode: string; userCode: string };

    const approveRoute = await import("@/app/api/v1/admin/devices/approve/route");
    const approve = (userCode: string, admin?: string) =>
      approveRoute.POST(
        new Request("http://tracker.local/api/v1/admin/devices/approve", {
          method: "POST",
          headers: {
            "content-type": "application/json",
            ...(admin === undefined ? {} : { "x-msp-admin": admin }),
          },
          body: JSON.stringify({ userCode }),
        }),
      );

    // Gate stays owner-controlled: no token -> 403, wrong token -> 403.
    expect((await approve(start.userCode)).status).toBe(403);
    expect((await approve(start.userCode, "wrong")).status).toBe(403);

    const goodCode = await approve(start.userCode, "test-admin-token");
    expect(goodCode.status).toBe(200);

    const completeRes = await deviceComplete.POST(
      signedRequest(keys, ids.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: start.deviceCode }), adminRig.ctx.now, { ip: "198.51.100.10" }),
    );
    expect(completeRes.status).toBe(200);

    // Unknown / malformed codes are not approved silently.
    expect((await approve("ZZZZZZZZ", "test-admin-token")).status).toBe(404);
    expect((await approve("not-a-code", "test-admin-token")).status).toBe(400);
  });

  it("start rejects an installationId that does not derive from pubKey", async () => {
    const enrolledKeys = makeKeypair();
    const ids = keyPairIds(enrolledKeys);
    const res = await deviceStart.POST(
      new Request("http://tracker.local/api/v1/auth/device/start", {
        method: "POST",
        headers: { "content-type": "application/json", "x-forwarded-for": "198.51.100.2" },
        body: JSON.stringify({ installationId: ids.installationId + "x".repeat(3), pubKey: ids.pubKeyB58 }),
      }),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });
});

describe("C2 unsigned/unauthenticated peer calls", () => {
  it("missing envelope -> 400 unsigned_request; garbage header -> 401; bad session -> 401", async () => {
    const body = JSON.stringify({
      peerId: "x",
      addresses: ["/ip4/10.0.0.1/tcp/1"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "b" },
    });
    const noAuth = await registerRoute.POST(
      new Request("http://tracker.local/api/v1/peers/register", { method: "POST", body }),
    );
    expect([400, 401]).toContain(noAuth.status);
    expect(((await noAuth.json()) as { error: { code: string } }).error.code).toBe("unsigned_request");

    const garbageAuth = await registerRoute.POST(
      new Request("http://tracker.local/api/v1/peers/register", {
        method: "POST",
        headers: { authorization: "MSP1 !!!not-base64!!!" },
        body,
      }),
    );
    expect([400, 401]).toContain(garbageAuth.status);

    const who = await enroll(rig, "198.51.100.3");
    const noSession = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, "", "/api/v1/peers/register", body, rig.ctx.now, { ip: "198.51.100.3" }),
    );
    expect(noSession.status).toBe(401);

    const wrongSession = await heartbeatRoute.POST(
      signedRequest(who.keys, who.installationId, "deadbeef", "/api/v1/peers/heartbeat", JSON.stringify({ leaseId: "0".repeat(32), activeProfiles: [], freeSlots: 1, queueMs: 0, draining: false }), rig.ctx.now),
    );
    expect(wrongSession.status).toBe(401);
  });
});

describe("C3 timestamp skew", () => {
  it("ts older than 120 s -> 400 stale_timestamp", async () => {
    const who = await enroll(rig, "198.51.100.4");
    const body = JSON.stringify({
      peerId: who.peerId,
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "b" },
    });
    const res = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", body, rig.ctx.now, {
        ts: rig.ctx.now() - 121_000,
      }),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("stale_timestamp");
  });
});

describe("C4 replay protection", () => {
  it("replaying a captured request -> 400 replayed_nonce", async () => {
    const who = await enroll(rig, "198.51.100.5");
    const body = JSON.stringify({
      peerId: who.peerId,
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "b" },
    });
    const nonce = "a".repeat(32);
    const first = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", body, rig.ctx.now, { nonce }),
    );
    expect(first.status).toBe(200);
    const replay = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", body, rig.ctx.now, { nonce }),
    );
    expect(replay.status).toBe(400);
    expect(((await replay.json()) as { error: { code: string } }).error.code).toBe("replayed_nonce");
  });
});

describe("C5 tampered body", () => {
  it("body changed after signing (digest mismatch) -> 401", async () => {
    const who = await enroll(rig, "198.51.100.6");
    const goodBody = JSON.stringify({
      peerId: who.peerId,
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "b" },
    });
    // sanity: untampered works
    const ok = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", goodBody, rig.ctx.now),
    );
    expect(ok.status).toBe(200);

    const tampered = JSON.stringify({
      peerId: who.peerId,
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 8, // escalated after signing
      runtime: { name: "r", build: "b" },
    });
    const res = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", tampered, rig.ctx.now, {
        tamperBodyDigest: true,
      }),
    );
    expect(res.status).toBe(401);
  });
});

describe("register validation", () => {
  it("peerId not deriving from the pubKey -> 400 invalid_body", async () => {
    const who = await enroll(rig, "198.51.100.7");
    const body = JSON.stringify({
      peerId: "WrongPeerId123",
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "b" },
    });
    const res = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", body, rig.ctx.now),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });

  it("ADR-020 hard switch: the placeholder peerId==installationId is rejected; only the multihash derivation registers", async () => {
    const who = await enroll(rig, "198.51.100.9");
    // Shape checks on the helper derivation (pure TS mirrors the Rust
    // peer_id_for: base58(identity-multihash of the protobuf-encoded key)).
    expect(who.peerId.startsWith("12D3Koo")).toBe(true);
    expect(who.peerId.length).toBe(52);
    expect(who.peerId).not.toBe(who.installationId);

    const placeholderBody = JSON.stringify({
      peerId: who.installationId, // the retired Phase B–E placeholder equality
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "b" },
    });
    const rejected = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", placeholderBody, rig.ctx.now),
    );
    expect(rejected.status).toBe(400);
    expect(((await rejected.json()) as { error: { code: string } }).error.code).toBe("invalid_body");

    // The multihash derivation of the same key registers cleanly.
    const ok = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", JSON.stringify({
        peerId: who.peerId,
        addresses: ["/ip4/10.0.0.1/tcp/4001"],
        profiles: [profileId],
        maxSlots: 2,
        runtime: { name: "r", build: "b" },
      }), rig.ctx.now),
    );
    expect(ok.status).toBe(200);
  });

  it("unknown profile -> 400 unknown_profile; bad multiaddr -> 400; maxSlots out of range -> 400", async () => {
    const who = await enroll(rig, "198.51.100.8");
    const base = (overrides: Record<string, unknown>) =>
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/peers/register",
        JSON.stringify({
          peerId: who.peerId,
          addresses: ["/ip4/10.0.0.1/tcp/4001"],
          profiles: [profileId],
          maxSlots: 2,
          runtime: { name: "r", build: "b" },
          ...overrides,
        }),
        rig.ctx.now,
      );

    const unknownProfile = await registerRoute.POST(base({ profiles: ["msp1:" + "9".repeat(64)] }));
    expect(unknownProfile.status).toBe(400);
    expect(((await unknownProfile.json()) as { error: { code: string } }).error.code).toBe("unknown_profile");

    const badAddr = await registerRoute.POST(base({ addresses: ["ftp://not-a-multiaddr"] }));
    expect(badAddr.status).toBe(400);

    const badSlots = await registerRoute.POST(base({ maxSlots: 9 }));
    expect(badSlots.status).toBe(400);
  });
});
