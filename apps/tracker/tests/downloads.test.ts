// I5: counted installer downloads at /api/download/<file> + stats exposure.

import { beforeEach, describe, expect, it } from "vitest";
import * as downloadRoute from "@/app/api/download/[file]/route";
import * as statsRoute from "@/app/api/v1/stats/route";
import { makeRig } from "./helpers";

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig();
});

const hit = (file: string, ip?: string) =>
  downloadRoute.GET(
    new Request(`http://tracker.local/api/download/${file}`, {
      headers: ip ? { "x-forwarded-for": ip } : {},
    }),
    { params: Promise.resolve({ file }) },
  );

describe("I5 GET /api/download/<file>", () => {
  it("rejects non-installer and traversal names", async () => {
    for (const bad of ["../config.json", "notes.txt", "a/b.exe", ".exe"]) {
      const res = await hit(bad);
      expect(res.status, bad).toBe(400);
    }
  });

  it("404s for installer-shaped names that do not exist on disk", async () => {
    const res = await hit("ModelSwarm-9.9.9-nope.exe");
    expect(res.status).toBe(404);
  });

  it("counts and redirects when the file exists", async () => {
    // The dev workspace ships real installers under public/downloads.
    const res = await hit("ModelSwarm-Setup-0.2.1-windows-x64.exe");
    expect([302, 404]).toContain(res.status); // 302 locally; 404 on runners without artifacts
    if (res.status === 302) {
      expect(res.headers.get("location")).toContain("/downloads/ModelSwarm-Setup-0.2.1-windows-x64.exe");
      const stats = await (await statsRoute.GET(new Request("http://tracker.local/api/v1/stats"))).json();
      expect(stats.downloads["ModelSwarm-Setup-0.2.1-windows-x64.exe"]).toBeGreaterThan(0);
    }
  });

  it("bump + read roundtrip through the store", async () => {
    expect(await rig.store.bumpDownloadCount("x.deb")).toBe(1);
    expect(await rig.store.bumpDownloadCount("x.deb")).toBe(2);
    expect(await rig.store.downloadCounts()).toEqual({ "x.deb": 2 });
  });

  it("rate-limits per IP at the public class (T4: unthrottled counted DB writes otherwise)", async () => {
    const file = "ModelSwarm-9.9.9-flood.exe"; // name-shaped; 404s are fine — the limit fires first
    let lastStatus = 200;
    for (let i = 0; i < 121; i += 1) {
      const res = await hit(file, "203.0.113.140");
      lastStatus = res.status;
      if (lastStatus === 429) break;
    }
    expect(lastStatus).toBe(429);
    const body = (await hit(file, "203.0.113.140").then((r) => r.json())) as {
      error: { code: string; retryable: boolean };
    };
    expect(body.error.code).toBe("rate_limited");
    expect(body.error.retryable).toBe(true);
  });
});
