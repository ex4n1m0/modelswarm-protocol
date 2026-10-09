// F abuse controls: per-installation rate limit, 413 size cap, malformed
// bodies (never logged), inference-shaped rejection, per-IP flooding on
// enrollment, cheap garbage rejection.

import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import * as deviceStart from "@/app/api/v1/auth/device/start/route";
import * as registerRoute from "@/app/api/v1/peers/register/route";
import * as peersList from "@/app/api/v1/peers/route";
import * as modelRequestsRoute from "@/app/api/v1/catalog/requests/route";
import * as sessionAuthorize from "@/app/api/v1/session-authorize/route";
import * as receiptRoute from "@/app/api/v1/receipt/route";
import {
  enroll,
  keyPairIds,
  makeKeypair,
  makeRig,
  register,
  seedActiveProfile,
  signedRequest,
  testManifest,
  randomNonce,
} from "./helpers";

let rig: ReturnType<typeof makeRig>;
let profileId: string;

beforeEach(async () => {
  rig = makeRig();
  profileId = await seedActiveProfile(rig, testManifest());
});

describe("F1 per-installation rate limit on peer endpoints", () => {
  it("request 61 within one minute -> 429 rate_limited retryable", async () => {
    const who = await enroll(rig, "198.51.100.30");
    await register(rig, who, [profileId]);

    let lastStatus = 200;
    let lastBody: { error?: { code?: string; retryable?: boolean } } = {};
    for (let i = 0; i < 61; i += 1) {
      const res = await peersList.GET(
        signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers", undefined, rig.ctx.now),
      );
      lastStatus = res.status;
      lastBody = (await res.json()) as typeof lastBody;
    }
    expect(lastStatus).toBe(429);
    expect(lastBody.error?.code).toBe("rate_limited");
    expect(lastBody.error?.retryable).toBe(true);
  });
});

describe("F2 payload size cap", () => {
  it("bodies over 64 KiB -> 413 payload_too_large (before auth)", async () => {
    const who = await enroll(rig, "198.51.100.31");
    const bigBody = JSON.stringify({
      peerId: who.peerId,
      addresses: ["/ip4/10.0.0.1/tcp/4001"],
      profiles: [profileId],
      maxSlots: 2,
      runtime: { name: "r", build: "x".repeat(70_000) },
    });
    expect(Buffer.byteLength(bigBody)).toBeGreaterThan(64 * 1024);
    const res = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", bigBody, rig.ctx.now),
    );
    expect(res.status).toBe(413);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("payload_too_large");
  });
});

describe("F3 malformed bodies", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("invalid JSON -> 400 invalid_body and the raw body is never logged", async () => {
    const who = await enroll(rig, "198.51.100.32");
    const marker = `MARKER-${randomNonce()}`;
    const badBody = `{"peerId": "${marker}", this is not json`;
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const logSpy = vi.spyOn(console, "log").mockImplementation(() => {});

    const res = await registerRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", badBody, rig.ctx.now),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");

    for (const call of [...errorSpy.mock.calls, ...logSpy.mock.calls]) {
      const line = JSON.stringify(call);
      expect(line.includes(marker)).toBe(false);
    }
  });

  it("schema-violating JSON -> 400 invalid_body (also unlogged)", async () => {
    const who = await enroll(rig, "198.51.100.33");
    const marker = `MARKER-${randomNonce()}`;
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const logSpy = vi.spyOn(console, "log").mockImplementation(() => {});

    const res = await registerRoute.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/peers/register",
        JSON.stringify({ peerId: marker, addresses: [], profiles: [], maxSlots: "eight" }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
    for (const call of [...errorSpy.mock.calls, ...logSpy.mock.calls]) {
      expect(JSON.stringify(call).includes(marker)).toBe(false);
    }
  });
});

describe("F4 inference-shaped payloads are structurally rejected", () => {
  const inferenceBodies = [
    { messages: [{ role: "user", content: "hello" }] },
    { prompt: "hello" },
    { input: "hello" },
  ];

  it.each(inferenceBodies.map((b) => [JSON.stringify(b)]))(
    "register rejects %s by schema absence",
    async (body) => {
      const who = await enroll(rig, "198.51.100.34");
      const res = await registerRoute.POST(
        signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", body, rig.ctx.now),
      );
      expect(res.status).toBe(400);
      expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
    },
  );

  it("session-authorize and receipt also reject prompt-like fields", async () => {
    const who = await enroll(rig, "198.51.100.35");
    await register(rig, who, [profileId]);

    const auth = await sessionAuthorize.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/session-authorize",
        JSON.stringify({ peer_ids: [who.peerId], profile_id: profileId, mode: "single", messages: [] }),
        rig.ctx.now,
      ),
    );
    expect(auth.status).toBe(400);

    const receipt = await receiptRoute.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/receipt",
        JSON.stringify({ receiptDigest: "0".repeat(64), outcome: "stop", prompt: "x" }),
        rig.ctx.now,
      ),
    );
    expect(receipt.status).toBe(400);
  });
});

describe("F6 device/start flooding", () => {
  it("11 requests/min from one IP -> 429 on the 11th and no rows for rejected calls", async () => {
    const keys = makeKeypair();
    const { installationId, pubKeyB58 } = keyPairIds(keys);
    let ok = 0;
    let lastStatus = 200;
    for (let i = 0; i < 11; i += 1) {
      const res = await deviceStart.POST(
        new Request("http://tracker.local/api/v1/auth/device/start", {
          method: "POST",
          headers: { "content-type": "application/json", "x-forwarded-for": "203.0.113.99" },
          body: JSON.stringify({ installationId, pubKey: pubKeyB58 }),
        }),
      );
      lastStatus = res.status;
      if (res.status === 200) ok += 1;
      else await res.json(); // drain body
    }
    expect(ok).toBe(10);
    expect(lastStatus).toBe(429);
    // exactly the 10 accepted calls created device codes — the 429th created none
    const storeInternals = rig.store as unknown as { deviceAuths: Map<string, unknown> };
    expect(storeInternals.deviceAuths.size).toBe(10);
  });
});

describe("F7 garbage to peer endpoints is rejected cheaply", () => {
  it("no 5xx; statuses within [400,401,413,429]; counted against the unsigned IP limit", async () => {
    const garbage: Array<Record<string, string>> = [
      { authorization: "MSP1 !!!!" },
      { authorization: "Bearer something" },
      { authorization: "MSP1 " + Buffer.from("not-json").toString("base64url") },
      { "x-msp-session": "only-session" },
      {},
    ];
    let lastStatus = 0;
    for (let i = 0; i < 61; i += 1) {
      const headers = garbage[i % garbage.length]!;
      const res = await peersList.GET(
        new Request("http://tracker.local/api/v1/peers?profile_id=x", {
          headers: { "x-forwarded-for": "203.0.113.120", ...headers },
        }),
      );
      expect([400, 401, 413, 429]).toContain(res.status);
      if (res.status !== 429) {
        const body = (await res.json()) as { error: { code: string } };
        expect(["unsigned_request", "unauthorized", "invalid_body"]).toContain(body.error.code);
      }
      lastStatus = res.status;
      if (i < 60) expect(res.status).not.toBe(429); // under the unsigned limit
    }
    // the 61st unsigned call from this IP crossed the 60/min limit
    expect(lastStatus).toBe(429);
  });
});

describe("F8 unknown-installation envelope floods hit the per-IP unsigned bucket (T4)", () => {
  it("well-formed envelopes naming an unenrolled id: 401s, then 429 after 60/min", async () => {
    // A real keypair signs real envelopes, but the installation never
    // enrolled — previously these burned a DB read per request without
    // touching ANY limiter bucket.
    const keys = makeKeypair();
    const { installationId } = keyPairIds(keys);
    let sawUnknownInstallation = false;
    let lastStatus = 200;
    for (let i = 0; i < 61; i += 1) {
      const res = await peersList.GET(
        signedRequest(keys, installationId, "any", "/api/v1/peers", undefined, rig.ctx.now, {
          ip: "203.0.113.121",
        }),
      );
      lastStatus = res.status;
      if (res.status === 401) {
        const body = (await res.json()) as { error: { code: string } };
        expect(body.error.code).toBe("unknown_installation");
        sawUnknownInstallation = true;
      } else if (res.status !== 429) {
        await res.json();
      }
    }
    expect(sawUnknownInstallation).toBe(true);
    expect(lastStatus).toBe(429);
  });
});

describe("F9 /catalog/requests body is read under the shared 64 KiB cap (T4)", () => {
  it("oversized bodies -> 413 payload_too_large before any parse", async () => {
    const bigNote = "x".repeat(70_000);
    const res = await modelRequestsRoute.POST(
      new Request("http://tracker.local/api/v1/catalog/requests", {
        method: "POST",
        headers: { "content-type": "application/json", "x-forwarded-for": "203.0.113.130" },
        body: JSON.stringify({ note: bigNote }),
      }),
    );
    expect(res.status).toBe(413);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("payload_too_large");
    // no row was created by the rejected call
    expect(await rig.store.listModelRequests()).toHaveLength(0);
  });
});
