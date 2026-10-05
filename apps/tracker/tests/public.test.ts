// B public endpoints + F5 public per-IP flooding.

import { beforeEach, describe, expect, it } from "vitest";
import { canonicalJson, verifyBase64 } from "@/lib/crypto";
import * as catalogRoute from "@/app/api/v1/catalog/route";
import * as catalogOneRoute from "@/app/api/v1/catalog/[profileId]/route";
import { makeRig, seedActiveProfile, testManifest, healthRoute } from "./helpers";

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig();
});

describe("B1 GET /api/v1/health", () => {
  it("returns the exact frozen body", async () => {
    const res = await healthRoute.GET(new Request("http://tracker.local/api/v1/health"));
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({
      status: "ok",
      service: "modelswarm-tracker",
      protocol: "1",
    });
  });
});

describe("B2 GET /api/v1/catalog", () => {
  it("returns a signed envelope whose signature verifies and breaks on any byte flip", async () => {
    const manifest = testManifest();
    await seedActiveProfile(rig, manifest);

    const res = await catalogRoute.GET(new Request("http://tracker.local/api/v1/catalog"));
    expect(res.status).toBe(200);
    const envelope = (await res.json()) as {
      catalogVersion: number;
      generatedAt: string;
      profiles: unknown[];
      signature: string;
    };
    expect(envelope.catalogVersion).toBeGreaterThan(0);
    expect(envelope.profiles).toHaveLength(1);
    expect(envelope.profiles[0]).toMatchObject({ status: "active" });

    const payload = {
      catalogVersion: envelope.catalogVersion,
      generatedAt: envelope.generatedAt,
      profiles: envelope.profiles,
    };
    // verifies with the hub public key over the canonical payload
    expect(verifyBase64(envelope.signature, canonicalJson(payload), rig.ctx.keys.publicKey)).toBe(true);

    // flipping any byte of profiles must break verification
    const flipped = JSON.parse(JSON.stringify(payload)) as typeof payload;
    (flipped.profiles[0] as Record<string, unknown>).display_name = "tampered";
    expect(verifyBase64(envelope.signature, canonicalJson(flipped), rig.ctx.keys.publicKey)).toBe(false);

    // signature is invalid with the wrong key
    expect(verifyBase64(envelope.signature, canonicalJson(payload), rig.ctx.keys.secretKey)).toBe(false);
  });
});

describe("B3 GET /api/v1/catalog/{profileId}", () => {
  it("unknown profile -> 404 unknown_profile", async () => {
    const res = await catalogOneRoute.GET(
      new Request("http://tracker.local/api/v1/catalog/msp1:" + "0".repeat(64)),
      { params: Promise.resolve({ profileId: "msp1:" + "0".repeat(64) }) },
    );
    expect(res.status).toBe(404);
    const body = (await res.json()) as { error: { code: string } };
    expect(body.error.code).toBe("unknown_profile");
  });

  it("known active profile -> single-profile signed envelope", async () => {
    const manifest = testManifest();
    const profileId = await seedActiveProfile(rig, manifest);
    const res = await catalogOneRoute.GET(
      new Request(`http://tracker.local/api/v1/catalog/${profileId}`),
      { params: Promise.resolve({ profileId }) },
    );
    expect(res.status).toBe(200);
    const envelope = (await res.json()) as {
      profiles: Array<{ profile_id: string }>;
      signature: string;
      catalogVersion: number;
      generatedAt: string;
    };
    expect(envelope.profiles).toHaveLength(1);
    expect(envelope.profiles[0]!.profile_id).toBe(profileId);
    expect(
      verifyBase64(
        envelope.signature,
        canonicalJson({
          catalogVersion: envelope.catalogVersion,
          generatedAt: envelope.generatedAt,
          profiles: envelope.profiles,
        }),
        rig.ctx.keys.publicKey,
      ),
    ).toBe(true);
  });
});

describe("F5 public per-IP flooding (health + catalog share 120/min)", () => {
  it("request 121 from one IP -> 429 rate_limited retryable", async () => {
    const ip = { "x-forwarded-for": "203.0.113.7" };
    let lastStatus = 200;
    let lastBody: { error?: { code?: string; retryable?: boolean } } = {};
    for (let i = 0; i < 121; i += 1) {
      const path = i % 2 === 0 ? "/api/v1/health" : "/api/v1/catalog";
      const res =
        path === "/api/v1/health"
          ? await healthRoute.GET(new Request("http://tracker.local/api/v1/health", { headers: ip }))
          : await catalogRoute.GET(new Request("http://tracker.local/api/v1/catalog", { headers: ip }));
      lastStatus = res.status;
      lastBody = (await res.json()) as typeof lastBody;
    }
    expect(lastStatus).toBe(429);
    expect(lastBody.error?.code).toBe("rate_limited");
    expect(lastBody.error?.retryable).toBe(true);
  });
});
