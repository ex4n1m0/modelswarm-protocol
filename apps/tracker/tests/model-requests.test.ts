// ADR-023: community model-request queue — public submit (rate-limited,
// dedupe, content-blind schema), admin list + resolve.

import { beforeEach, describe, expect, it } from "vitest";
import * as submitRoute from "@/app/api/v1/catalog/requests/route";
import * as listRoute from "@/app/api/v1/admin/catalog/requests/route";
import * as resolveRoute from "@/app/api/v1/admin/catalog/requests/resolve/route";
import { makeRig } from "./helpers";

const ADMIN = { "x-msp-admin": "test-admin-token" };
const SHA =
  "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e";
const REV = "91cad51170dc346986eccefdc2dd33a9da36ead9";

function validBody(over: Record<string, unknown> = {}) {
  return {
    hf_repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
    hf_revision: REV,
    artifact_path: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
    artifact_sha256: SHA,
    artifact_bytes: 1_117_320_736,
    quant_method: "q4_k_m",
    quant_bits: 4,
    display_name: "Qwen2.5 1.5B Instruct (Q4_K_M)",
    ...over,
  };
}

function post(body: unknown, headers: Record<string, string> = {}): Request {
  return new Request("http://tracker.local/api/v1/catalog/requests", {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
}

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig({ adminToken: "test-admin-token" });
});

describe("POST /catalog/requests (public)", () => {
  it("queues a valid request and records the source ip", async () => {
    const res = await submitRoute.POST(
      post(validBody(), { "x-forwarded-for": "203.0.113.9" }),
    );
    expect(res.status).toBe(201);
    expect(await res.json()).toMatchObject({ status: "queued" });

    const queue = await rig.store.listModelRequests();
    expect(queue).toHaveLength(1);
    expect(queue[0]).toMatchObject({
      hfRepo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
      artifactSha256: SHA,
      requestedFrom: "203.0.113.9",
      resolution: "",
    });
  });

  it("dedupes on artifact sha256 (already-requested, single row)", async () => {
    await submitRoute.POST(post(validBody()));
    const again = await submitRoute.POST(
      post(validBody({ display_name: "same artifact, other name" })),
    );
    expect(await again.json()).toMatchObject({ status: "already-requested" });
    expect(await rig.store.listModelRequests()).toHaveLength(1);
  });

  it("rejects inference-shaped fields (content-blind schema)", async () => {
    const res = await submitRoute.POST(
      post(validBody({ prompt: "hello swarm" })), // unknown key
    );
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: { code: string } };
    expect(body.error.code).toBe("invalid_body");
  });

  it("rate-limits beyond the enrollment grade (10/min/ip)", async () => {
    let last: Response | undefined;
    for (let i = 0; i < 11; i += 1) {
      last = await submitRoute.POST(
        post(validBody({ artifact_sha256: SHA.replace(/e$/, String(i)) })),
      );
    }
    expect(last!.status).toBe(429);
  });
});

describe("GET /admin/catalog/requests", () => {
  it("requires the admin token", async () => {
    const res = await listRoute.GET(
      new Request("http://tracker.local/api/v1/admin/catalog/requests"),
    );
    expect(res.status).toBe(403);
  });

  it("lists open requests first, oldest first", async () => {
    for (let i = 0; i < 3; i += 1) {
      await rig.advance(1_000);
      await rig.store.insertModelRequest({
        artifactSha256: SHA.replace(/e$/, String(i)),
        hfRepo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        hfRevision: REV,
        artifactPath: "a.gguf",
        artifactBytes: 1,
        quantMethod: "q4_k_m",
        quantBits: 4,
        displayName: `m${i}`,
        note: "",
        requestedFrom: "unknown",
      });
    }
    await rig.store.resolveModelRequest(2, "promoted");
    const res = await listRoute.GET(
      new Request("http://tracker.local/api/v1/admin/catalog/requests", {
        headers: ADMIN,
      }),
    );
    const body = (await res.json()) as { requests: Array<{ id: number }> };
    expect(body.requests.map((r) => r.id)).toEqual([1, 3, 2]);
  });
});

describe("POST /admin/catalog/requests/resolve", () => {
  it("closes an open request exactly once", async () => {
    await submitRoute.POST(post(validBody()));
    const ok = await resolveRoute.POST(
      new Request("http://tracker.local/api/v1/admin/catalog/requests/resolve", {
        method: "POST",
        headers: { "content-type": "application/json", ...ADMIN },
        body: JSON.stringify({ id: 1, resolution: "promoted" }),
      }),
    );
    expect(ok.status).toBe(200);
    const repeat = await resolveRoute.POST(
      new Request("http://tracker.local/api/v1/admin/catalog/requests/resolve", {
        method: "POST",
        headers: { "content-type": "application/json", ...ADMIN },
        body: JSON.stringify({ id: 1, resolution: "rejected" }),
      }),
    );
    expect(repeat.status).toBe(404);
    const [rec] = await rig.store.listModelRequests();
    expect(rec.resolution).toBe("promoted");
    expect(rec.resolvedAt).toBeGreaterThan(0);
  });
});
