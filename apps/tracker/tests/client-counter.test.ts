// Owner directive 2026-10-17: the unified client counter — every installer
// download (any OS) and every self-update artifact fetch counts as ONE
// client served. The updater pulls versioned artifacts through the same
// counted /api/download route, so the total is the sum of all counted
// files; the test pins the arithmetic and the stats exposure.

import { beforeEach, describe, expect, it } from "vitest";
import { makeRig } from "./helpers";

let rig: ReturnType<typeof makeRig>;
beforeEach(() => {
  rig = makeRig();
});

describe("unified client counter", () => {
  it("sums all OS installers and update artifacts as one client each", async () => {
    // A fresh install on Windows, an update on Linux, an update on macOS.
    await rig.store.bumpDownloadCount("ModelSwarm-Setup-0.2.30-windows-x64.exe");
    await rig.store.bumpDownloadCount("ModelSwarm-0.2.30-linux-amd64.deb");
    await rig.store.bumpDownloadCount("ModelSwarm-0.2.30-macos-arm64.dmg");
    await rig.store.bumpDownloadCount("ModelSwarm-0.2.30-linux-amd64.deb"); // second update
    expect(await rig.store.clientServesTotal()).toBe(4);
  });

  it("is zero on a fresh tracker", async () => {
    expect(await rig.store.clientServesTotal()).toBe(0);
  });
});
