// Hardening pins (2026-10-08 review): the dev signing key must never be
// reachable in production, and rendezvous mailboxes must stay bounded.

import { afterEach, describe, expect, it } from "vitest";
import { initTrackerContext } from "@/lib/context";
import { MemoryStore } from "@/lib/store";

describe("hub signing key fail-closed (production)", () => {
  const savedSeed = process.env.MSP_HUB_SEED;
  const savedNodeEnv = process.env.NODE_ENV;
  const savedVercelEnv = process.env.VERCEL_ENV;
  // process.env's well-known keys are typed read-only; tests mutate anyway.
  const setEnv = (key: string, value: string | undefined) => {
    const env = process.env as Record<string, string | undefined>;
    if (value === undefined) delete env[key];
    else env[key] = value;
  };

  afterEach(() => {
    setEnv("MSP_HUB_SEED", savedSeed);
    setEnv("NODE_ENV", savedNodeEnv);
    setEnv("VERCEL_ENV", savedVercelEnv);
  });

  it("throws at context build when production has no MSP_HUB_SEED", () => {
    setEnv("MSP_HUB_SEED", undefined);
    setEnv("VERCEL_ENV", "production");
    expect(() => initTrackerContext()).toThrow(/MSP_HUB_SEED/);
  });

  it("throws on a malformed seed in production too (never silently downgraded)", () => {
    setEnv("MSP_HUB_SEED", "not-hex-at-all");
    setEnv("VERCEL_ENV", "production");
    expect(() => initTrackerContext()).toThrow(/MSP_HUB_SEED/);
  });

  it("accepts a well-formed seed in production", () => {
    setEnv("MSP_HUB_SEED", "07".repeat(32));
    setEnv("VERCEL_ENV", "production");
    expect(() => initTrackerContext()).not.toThrow();
  });
});

describe("rendezvous mailboxes are bounded", () => {
  it("keeps only the most recent RENDEZVOUS_MAX_PER_MAILBOX items", async () => {
    const store = new MemoryStore(() => 0);
    const now = 1_000_000;
    for (let i = 0; i < 70; i += 1) {
      await store.pushRendezvous({
        toPeerId: "peer-a",
        fromPeerId: `sender-${i}`,
        kind: "offer",
        payload: `payload-${i}`,
        receivedAt: now + i,
      });
    }
    const drained = await store.drainRendezvous("peer-a");
    expect(drained).toHaveLength(64);
    // Oldest dropped, newest kept.
    expect(drained[0]!.fromPeerId).toBe("sender-6");
    expect(drained[63]!.fromPeerId).toBe("sender-69");
  });

  it("drops items older than the TTL on push", async () => {
    const store = new MemoryStore(() => 0);
    const now = 10_000_000;
    await store.pushRendezvous({
      toPeerId: "peer-b",
      fromPeerId: "old",
      kind: "offer",
      payload: "p",
      receivedAt: now - 25 * 60 * 60 * 1000,
    });
    await store.pushRendezvous({
      toPeerId: "peer-b",
      fromPeerId: "fresh",
      kind: "offer",
      payload: "p",
      receivedAt: now,
    });
    const drained = await store.drainRendezvous("peer-b");
    expect(drained.map((d) => d.fromPeerId)).toEqual(["fresh"]);
  });
});
