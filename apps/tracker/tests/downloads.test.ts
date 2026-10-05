// I5: counted installer downloads at /api/download/<file> + stats exposure.

import { beforeEach, describe, expect, it } from "vitest";
import * as downloadRoute from "@/app/api/download/[file]/route";
import * as statsRoute from "@/app/api/v1/stats/route";
import { makeRig } from "./helpers";

let rig: ReturnType<typeof makeRig>;

beforeEach(() => {
  rig = makeRig();
});

const hit = (file: string) =>
  downloadRoute.GET(new Request(`http://tracker.local/api/download/${file}`), {
    params: Promise.resolve({ file }),
  });

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
});
