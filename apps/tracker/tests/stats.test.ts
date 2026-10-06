// I1: GET /api/v1/stats — public online counter (counts only, content-blind).

import { beforeEach, describe, expect, it } from "vitest";
import * as statsRoute from "@/app/api/v1/stats/route";
import { enroll, heartbeat, makeRig, register, seedActiveProfile, testManifest } from "./helpers";

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig();
});

const stats = async () =>
  (await (await statsRoute.GET(new Request("http://tracker.local/api/v1/stats"))).json()) as {
    peersOnline: number;
    models: { profileId: string; peers: number }[];
  };
const count = () => stats().then((s) => s.peersOnline);

describe("I1 GET /api/v1/stats", () => {
  it("counts zero with no leases", async () => {
    expect(await stats()).toEqual({ peersOnline: 0, models: [], downloads: {} });
  });

  it("counts distinct live peers and excludes draining ones", async () => {
    const a = await enroll(rig);
    const b = await enroll(rig);
    const profile = await seedActiveProfile(rig, testManifest());
    const leaseB = await register(rig, b, [profile]);
    await register(rig, a, [profile]);

    expect(await count()).toBe(2);

    // Draining peers are not "online".
    await heartbeat(rig, b, leaseB.leaseId, { draining: true });
    expect(await count()).toBe(1);

    // Draining is one-way (msp-v1): B stays out of the counter.
    // Lapsed leases (no heartbeat past TTL) drop out entirely.
    rig.advance(10 * 60 * 1000);
    expect(await count()).toBe(0);
    expect((await stats()).models).toEqual([]);
  });

  it("counts online peers per hosted profile", async () => {
    const a = await enroll(rig);
    const b = await enroll(rig);
    const c = await enroll(rig);
    const one = await seedActiveProfile(rig, testManifest());
    const two = await seedActiveProfile(rig, { ...testManifest(), decoding_abi_version: 2 });
    // A hosts both; B and C host only `one`.
    await register(rig, a, [one, two]);
    await register(rig, b, [one]);
    await register(rig, c, [one]);

    const body = await stats();
    expect(body.peersOnline).toBe(3);
    const models = Object.fromEntries(body.models.map((m) => [m.profileId, m.peers]));
    expect(models[one]).toBe(3);
    expect(models[two]).toBe(1);
  });

  it("reports a single registered host as its model's sole online peer", async () => {
    // The one-machine swarm: a single host registers its one profile and the
    // census must show that model online — the landing page renders
    // "1 online" from exactly this shape.
    const a = await enroll(rig);
    const one = await seedActiveProfile(rig, testManifest());
    await register(rig, a, [one]);

    const body = await stats();
    expect(body.peersOnline).toBe(1);
    expect(body.models).toEqual([{ profileId: one, peers: 1 }]);
  });
});
