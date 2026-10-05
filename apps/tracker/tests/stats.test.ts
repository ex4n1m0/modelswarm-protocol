// I1: GET /api/v1/stats — public online counter (counts only, content-blind).

import { beforeEach, describe, expect, it } from "vitest";
import * as statsRoute from "@/app/api/v1/stats/route";
import { enroll, heartbeat, makeRig, register, seedActiveProfile, testManifest } from "./helpers";

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig();
});

const count = async () =>
  (await (await statsRoute.GET(new Request("http://tracker.local/api/v1/stats"))).json()) as {
    peersOnline: number;
  };

describe("I1 GET /api/v1/stats", () => {
  it("counts zero with no leases", async () => {
    expect(await count()).toEqual({ peersOnline: 0 });
  });

  it("counts distinct live peers and excludes draining ones", async () => {
    const a = await enroll(rig);
    const b = await enroll(rig);
    const profile = await seedActiveProfile(rig, testManifest());
    const leaseB = await register(rig, b, [profile]);
    await register(rig, a, [profile]);

    expect((await count()).peersOnline).toBe(2);

    // Draining peers are not "online".
    await heartbeat(rig, b, leaseB.leaseId, { draining: true });
    expect((await count()).peersOnline).toBe(1);

    // Draining is one-way (msp-v1): B stays out of the counter.
    // Lapsed leases (no heartbeat past TTL) drop out entirely.
    rig.advance(10 * 60 * 1000);
    expect((await count()).peersOnline).toBe(0);
  });
});
