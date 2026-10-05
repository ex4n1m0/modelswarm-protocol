// Pg-backed enrollment round-trip: the same flows as enrollment.test.ts but
// against a REAL Postgres (DATABASE_URL; skipped otherwise). Exists because
// v0.2.4 shipped an ON CONFLICT that only fails against real Postgres — the
// MemoryStore suite cannot catch SQL-level breakage.
import { execSync } from "node:child_process";
import { beforeAll, describe, expect, it } from "vitest";
import * as deviceStart from "@/app/api/v1/auth/device/start/route";
import * as deviceComplete from "@/app/api/v1/auth/device/complete/route";
import * as registerRoute from "@/app/api/v1/peers/register/route";
import { initTrackerContext } from "@/lib/context";
import { PgStore } from "@/lib/store";
import { keyPairIds, makeKeypair, seedActiveProfile, signedRequest, testManifest } from "./helpers";

const dbUrl = process.env.DATABASE_URL;

describe.skipIf(!dbUrl)("Pg enrollment round-trip", () => {
  let store: PgStore;
  let nowMs: number;
  let profileId: string;

  beforeAll(() => {
    // The CI postgres job migrates before tests; local runs may not have.
    execSync("npm run db:migrate", {
      env: { ...process.env, DATABASE_URL: dbUrl },
      stdio: "pipe",
    });
    nowMs = 1_800_000_000_000;
    store = new PgStore(dbUrl!, () => nowMs);
    initTrackerContext({
      store,
      now: () => nowMs,
      adminToken: "test-admin-token",
    });
  }, 60_000);

  it("device start -> approve by user code -> complete -> register, all on Postgres", async () => {
    const rigLike = { ctx: { now: () => nowMs }, store } as unknown as Parameters<
      typeof seedActiveProfile
    >[0];
    profileId = await seedActiveProfile(rigLike, testManifest());

    const keys = makeKeypair();
    const ids = keyPairIds(keys);
    const ip = { "x-forwarded-for": "203.0.113.50" };

    const startRes = await deviceStart.POST(
      new Request("http://t.local/api/v1/auth/device/start", {
        method: "POST",
        headers: { "content-type": "application/json", ...ip },
        body: JSON.stringify({ installationId: ids.installationId, pubKey: ids.pubKeyB58 }),
      }),
    );
    expect(startRes.status).toBe(200);
    const start = (await startRes.json()) as { deviceCode: string; userCode: string };
    expect(start.userCode).toMatch(/^[A-Z2-9]{8}$/);

    // A retry of device/start must not 500 (the peer_keys history insert).
    const retryRes = await deviceStart.POST(
      new Request("http://t.local/api/v1/auth/device/start", {
        method: "POST",
        headers: { "content-type": "application/json", ...ip },
        body: JSON.stringify({ installationId: ids.installationId, pubKey: ids.pubKeyB58 }),
      }),
    );
    expect(retryRes.status).toBe(200);

    expect(await store.approveDeviceByUserCode(start.userCode)).toBe(true);

    const completeRes = await deviceComplete.POST(
      signedRequest(
        keys,
        ids.installationId,
        "",
        "/api/v1/auth/device/complete",
        JSON.stringify({ deviceCode: start.deviceCode }),
        () => nowMs,
        { ip: ip["x-forwarded-for"] },
      ),
    );
    expect(completeRes.status).toBe(200);
    const session = (await completeRes.json()) as { token: string };
    expect(session.token.length).toBeGreaterThanOrEqual(32);

    const registerRes = await registerRoute.POST(
      signedRequest(
        keys,
        ids.installationId,
        session.token,
        "/api/v1/peers/register",
        JSON.stringify({
          peerId: ids.peerId,
          addresses: ["/ip4/10.0.0.9/tcp/4001"],
          profiles: [profileId],
          maxSlots: 1,
          runtime: { name: "llama.cpp", build: "b11407" },
        }),
        () => nowMs,
        { ip: ip["x-forwarded-for"] },
      ),
    );
    expect(registerRes.status).toBe(200);
    const reg = (await registerRes.json()) as { leaseId: string };
    expect(reg.leaseId).toBeTruthy();
  });
});
