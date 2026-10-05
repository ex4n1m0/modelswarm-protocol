// D peer lifecycle: register/lease TTL, heartbeat extension + expiry,
// row-aging (injectable clock), drain, cross-installation lease binding,
// profile filter + limit clamp.

import { beforeEach, describe, expect, it } from "vitest";
import * as heartbeatRoute from "@/app/api/v1/peers/heartbeat/route";
import * as drainRoute from "@/app/api/v1/peers/drain/route";
import * as challengeStartRoute from "@/app/api/v1/peers/challenge/start/route";
import * as peersLeaseRoute from "@/app/api/v1/peers/lease/route";
import {
  drain,
  enroll,
  heartbeat,
  listPeers,
  makeRig,
  register,
  seedActiveProfile,
  signedRequest,
  testManifest,
  passChallenge,
  requestEligibilityLease,
  type Enrollment,
  type TestRig,
} from "./helpers";

let rig: TestRig;
let profileA: string;
let profileB: string;

beforeEach(async () => {
  rig = makeRig();
  profileA = await seedActiveProfile(rig, testManifest());
  profileB = await seedActiveProfile(
    rig,
    testManifest({ hf_revision: "b".repeat(40), runtime: { name: "rt-test", version: "b1", build_hash: "1".repeat(64) } }),
  );
});

describe("D1 register issues a 60–90 s lease", () => {
  it("lease TTL within the frozen band and bound to the installation", async () => {
    const who = await enroll(rig, "198.51.100.10");
    const { leaseId, leaseExpiresAt } = await register(rig, who, [profileA]);
    expect(leaseId).toMatch(/^[0-9a-f]{32}$/); // unguessable 128-bit id
    const ttl = Date.parse(leaseExpiresAt) - rig.ctx.now();
    expect(ttl).toBeGreaterThanOrEqual(60_000);
    expect(ttl).toBeLessThanOrEqual(90_000);
    const lease = await rig.store.getLease(leaseId);
    expect(lease?.installationId).toBe(who.installationId);
    expect(lease?.peerId).toBe(who.peerId);
    // installation + peer key recorded
    const installation = await rig.store.getInstallation(who.installationId);
    expect(installation?.pubKeyB58).toBe(who.pubKeyB58);
  });
});

describe("D2 heartbeat extends; expired lease -> 410 lease_expired", () => {
  it("extends then expires by row-aging", async () => {
    const who = await enroll(rig, "198.51.100.11");
    const { leaseId } = await register(rig, who, [profileA]);

    rig.advance(20_000);
    const hb1 = await heartbeat(rig, who, leaseId, { profiles: [profileA] });
    expect(hb1.status).toBe(200);
    const hb1Body = (await hb1.json()) as { leaseExpiresAt: string; notices: unknown[] };
    expect(Date.parse(hb1Body.leaseExpiresAt) - rig.ctx.now()).toBeGreaterThanOrEqual(60_000);
    expect(hb1Body.notices).toEqual([]);

    // no heartbeat until past TTL
    rig.advance(80_000);
    const hb2 = await heartbeat(rig, who, leaseId, { profiles: [profileA] });
    expect(hb2.status).toBe(410);
    expect(((await hb2.json()) as { error: { code: string } }).error.code).toBe("lease_expired");
  });
});

describe("D3 lease expiry is row-aging with no background jobs", () => {
  it("peer disappears from GET /peers after TTL passes (clock injection only)", async () => {
    const who = await enroll(rig, "198.51.100.12");
    await register(rig, who, [profileA]);

    const before = await listPeers(rig, who, `?profile_id=${profileA}`);
    expect(((await before.json()) as { peers: unknown[] }).peers).toHaveLength(1);

    rig.advance(76_000); // TTL is 75 s; pure clock move, no timers

    const after = await listPeers(rig, who, `?profile_id=${profileA}`);
    expect(((await after.json()) as { peers: unknown[] }).peers).toHaveLength(0);
  });
});

describe("D4 drain excludes the peer and blocks issuance", () => {
  it("drain -> excluded from GET /peers and 403 ineligible on new issuance", async () => {
    const who = await enroll(rig, "198.51.100.13");
    const { leaseId } = await register(rig, who, [profileA]);
    await passChallenge(rig, who, leaseId, profileA);

    const drainRes = await drain(rig, who, leaseId);
    expect(drainRes.status).toBe(200);
    expect(((await drainRes.json()) as { draining: boolean }).draining).toBe(true);

    const listing = await listPeers(rig, who, `?profile_id=${profileA}`);
    expect(((await listing.json()) as { peers: unknown[] }).peers).toHaveLength(0);

    const leaseRes = await requestEligibilityLease(rig, who, leaseId, profileA);
    expect(leaseRes.status).toBe(403);
    expect(((await leaseRes.json()) as { error: { code: string } }).error.code).toBe("ineligible");
  });
});

describe("D5 lease ids are installation-bound", () => {
  it("heartbeat/drain/challenge/lease from installation B on A's lease -> 401 unauthorized", async () => {
    const a = await enroll(rig, "198.51.100.14");
    const b = await enroll(rig, "198.51.100.15");
    const { leaseId } = await register(rig, a, [profileA]);

    const call = (route: { POST: (req: Request) => Promise<Response> }, path: string, body: unknown) =>
      route.POST(
        signedRequest(b.keys, b.installationId, b.session, path, JSON.stringify(body), rig.ctx.now),
      );

    const hb = await call(heartbeatRoute, "/api/v1/peers/heartbeat", {
      leaseId,
      activeProfiles: [],
      freeSlots: 1,
      queueMs: 0,
      draining: false,
    });
    expect(hb.status).toBe(401);

    const drainRes = await call(drainRoute, "/api/v1/peers/drain", { leaseId });
    expect(drainRes.status).toBe(401);

    const challenge = await call(challengeStartRoute, "/api/v1/peers/challenge/start", {
      leaseId,
      profileId: profileA,
    });
    expect(challenge.status).toBe(401);

    const lease = await call(peersLeaseRoute, "/api/v1/peers/lease", { leaseId, profileId: profileA });
    expect(lease.status).toBe(401);

    // sanity: the rightful owner can still heartbeat
    const own = await heartbeat(rig, a, leaseId, { profiles: [profileA] });
    expect(own.status).toBe(200);
  });
});

describe("D6 profile filter + limit clamp", () => {
  it("filters by profile and clamps limit to 50", async () => {
    // 55 installations, distinct IPs to stay under per-IP enrollment limits
    for (let i = 0; i < 55; i += 1) {
      const who = await enroll(rig, `198.51.200.${i}`);
      const profiles = i % 3 === 0 ? [profileA, profileB] : [profileB];
      await register(rig, who, profiles, { maxSlots: 1 });
    }

    const observer = await enroll(rig, "198.51.100.16");

    const onlyA = await listPeers(rig, observer, `?profile_id=${profileA}`);
    const onlyABody = (await onlyA.json()) as { peers: Array<{ peerId: string }> };
    expect(onlyABody.peers).toHaveLength(19); // i % 3 === 0 -> 19 of 55

    const all = await listPeers(rig, observer, "?limit=100");
    const allBody = (await all.json()) as { peers: unknown[] };
    expect(allBody.peers).toHaveLength(50); // clamped from 100 to 50

    const one = await listPeers(rig, observer, "?limit=1");
    expect(((await one.json()) as { peers: unknown[] }).peers).toHaveLength(1);
  });
});
