// G catalog administration: admin-only promotion, append-only publishing.

import { beforeEach, describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import * as candidates from "@/app/api/v1/admin/catalog/candidates/route";
import * as promote from "@/app/api/v1/admin/catalog/promote/route";
import * as catalogRoute from "@/app/api/v1/catalog/route";
import * as registerRoute from "@/app/api/v1/peers/register/route";
import { deriveProfileId } from "@/lib/crypto";
import { enroll, makeRig, seedActiveProfile, signedRequest, testManifest } from "./helpers";

const here = dirname(fileURLToPath(import.meta.url));
const ADMIN = { "x-msp-admin": "test-admin-token" };

function adminRequest(path: string, body: unknown, headers: Record<string, string> = ADMIN): Request {
  return new Request(`http://tracker.local${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
}

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig({ adminToken: "test-admin-token" });
});

describe("G1 admin-only, append-only catalog", () => {
  it("non-admin candidate insert -> 403 forbidden", async () => {
    const res = await candidates.POST(
      adminRequest("/api/v1/admin/catalog/candidates", { manifest: testManifest(), display_name: "X" }, {}),
    );
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("forbidden");
  });

  it("wrong token -> 403", async () => {
    const res = await candidates.POST(
      adminRequest("/api/v1/admin/catalog/candidates", { manifest: testManifest(), display_name: "X" }, { "x-msp-admin": "wrong" }),
    );
    expect(res.status).toBe(403);
  });

  it("insert -> candidate invisible publicly; promote -> active + catalog_update notice", async () => {
    const manifest = testManifest();
    const profileId = deriveProfileId(manifest);

    const insertRes = await candidates.POST(
      adminRequest("/api/v1/admin/catalog/candidates", { manifest, display_name: "Cand" }),
    );
    expect(insertRes.status).toBe(201);
    expect(((await insertRes.json()) as { profileId: string }).profileId).toBe(profileId);

    // candidate not in public catalog yet
    const before = (await (await catalogRoute.GET(new Request("http://tracker.local/api/v1/catalog"))).json()) as {
      profiles: unknown[];
      catalogVersion: number;
    };
    expect(before.profiles).toHaveLength(0);

    const promoteRes = await promote.POST(adminRequest("/api/v1/admin/catalog/promote", { profileId }));
    expect(promoteRes.status).toBe(200);
    expect(await promoteRes.json()).toEqual({ profileId, status: "active" });

    const after = (await (await catalogRoute.GET(new Request("http://tracker.local/api/v1/catalog"))).json()) as {
      profiles: Array<{ profile_id: string }>;
      catalogVersion: number;
    };
    expect(after.profiles).toHaveLength(1);
    expect(after.profiles[0]!.profile_id).toBe(profileId);
    expect(after.catalogVersion).toBeGreaterThan(before.catalogVersion);

    // promote unknown -> 404 unknown_profile
    const unknown = await promote.POST(adminRequest("/api/v1/admin/catalog/promote", { profileId: "msp1:" + "7".repeat(64) }));
    expect(unknown.status).toBe(404);
    expect(((await unknown.json()) as { error: { code: string } }).error.code).toBe("unknown_profile");
  });

  it("immutability: same profileId with a different manifest -> 400 invalid_body", async () => {
    // With derived ids a differing manifest always derives a differing id, so
    // the guard is exercised against a seeded (legacy/direct) row whose stored
    // manifest does not match its id.
    const manifestA = testManifest();
    const manifestB = testManifest({ hf_revision: "e".repeat(40) });
    const idOfA = deriveProfileId(manifestA);
    await rig.store.insertProfile({
      profileId: idOfA,
      manifest: manifestB as never, // seeded mismatch on purpose
      displayName: "Legacy",
      status: "candidate",
      createdAt: rig.ctx.now(),
    });
    const res = await candidates.POST(
      adminRequest("/api/v1/admin/catalog/candidates", { manifest: manifestA, display_name: "A" }),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });

  it("re-publishing the identical manifest is idempotent (201)", async () => {
    const manifest = testManifest();
    const first = await candidates.POST(
      adminRequest("/api/v1/admin/catalog/candidates", { manifest, display_name: "Same" }),
    );
    expect(first.status).toBe(201);
    const second = await candidates.POST(
      adminRequest("/api/v1/admin/catalog/candidates", { manifest, display_name: "Same" }),
    );
    expect(second.status).toBe(201);
  });
});

describe("G2 no public insert path to model_profiles", () => {
  it("source scan: only admin routes reference insertProfile/promoteProfile", () => {
    const appDir = join(here, "..", "app", "api", "v1");
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) walk(full);
        else if (entry === "route.ts") {
          const src = readFileSync(full, "utf8");
          if (/insertProfile|promoteProfile/.test(src) && !full.includes(join("admin", "catalog"))) {
            offenders.push(full);
          }
        }
      }
    };
    walk(appDir);
    expect(offenders).toEqual([]);
  });

  it("registering cannot smuggle a manifest: candidate-shaped body -> 400", async () => {
    const who = await enroll(rig, "198.51.100.40");
    const res = await registerRoute.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/peers/register",
        JSON.stringify({
          peerId: who.peerId,
          addresses: ["/ip4/10.0.0.1/tcp/1"],
          profiles: ["msp1:" + "3".repeat(64)],
          maxSlots: 1,
          runtime: { name: "r", build: "b" },
          manifest: testManifest(),
        }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(400); // strict schema: manifest not a field
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });

  it("profiles seeded via the store are the only catalog source (behavioral)", async () => {
    await seedActiveProfile(rig, testManifest());
    const who = await enroll(rig, "198.51.100.41");
    // A peer cannot insert; only admin routes can (covered above).
    const res = await registerRoute.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/peers/register",
        JSON.stringify({
          peerId: who.peerId,
          addresses: ["/ip4/10.0.0.1/tcp/1"],
          profiles: ["msp1:" + "4".repeat(64)],
          maxSlots: 1,
          runtime: { name: "r", build: "b" },
        }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unknown_profile");
  });
});
