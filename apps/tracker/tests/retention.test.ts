// T5 write-time retention (MemoryStore invariants; PgStore mirrors the same
// policy in SQL — the rendezvous-mailbox pattern applied to the write-only
// and high-churn tables). No timers anywhere: pruning rides the injected
// clock at write time.

import { beforeEach, describe, expect, it } from "vitest";
import { MemoryStore } from "@/lib/store";
import {
  EXPIRED_ROW_GRACE_MS,
  OBSERVATION_MAX_PER_PEER,
  OBSERVATION_TTL_MS,
} from "@/lib/constants";
import { BASE_TIME, makeRig } from "./helpers";

let rig: ReturnType<typeof makeRig>;
let store: MemoryStore;

beforeEach(() => {
  rig = makeRig();
  store = rig.store;
});

const obs = (peerId: string, at: number) => ({
  leaseId: "l".repeat(32),
  peerId,
  freeSlots: 1,
  queueMs: 0,
  observedAt: at,
});

describe("peer_observations: per-peer cap + TTL at write time", () => {
  it("keeps only the newest N rows per peer", async () => {
    const t = rig.nowMs;
    for (let i = 0; i < OBSERVATION_MAX_PER_PEER + 20; i += 1) {
      await store.recordObservation(obs("peerA", t + i));
    }
    const internals = store as unknown as { observations: Array<{ peerId: string }> };
    const kept = internals.observations.filter((o) => o.peerId === "peerA");
    expect(kept.length).toBe(OBSERVATION_MAX_PER_PEER);
  });

  it("drops rows older than the TTL regardless of peer", async () => {
    const t = rig.nowMs;
    await store.recordObservation(obs("old", t - OBSERVATION_TTL_MS - 1));
    await store.recordObservation(obs("new", t));
    const internals = store as unknown as { observations: Array<{ peerId: string }> };
    expect(internals.observations.map((o) => o.peerId)).toEqual(["new"]);
  });

  it("per-peer caps are independent", async () => {
    const t = rig.nowMs;
    for (let i = 0; i < OBSERVATION_MAX_PER_PEER + 5; i += 1) {
      await store.recordObservation(obs("p1", t + i));
      await store.recordObservation(obs("p2", t + i));
    }
    const internals = store as unknown as { observations: Array<{ peerId: string }> };
    expect(internals.observations.filter((o) => o.peerId === "p1").length).toBe(OBSERVATION_MAX_PER_PEER);
    expect(internals.observations.filter((o) => o.peerId === "p2").length).toBe(OBSERVATION_MAX_PER_PEER);
  });
});

describe("sessions: census-safe pruning", () => {
  const session = (installationId: string, token: string, expiresAt: number) => ({
    token,
    installationId,
    createdAt: 0,
    expiresAt,
  });

  it("deletes expired-beyond-grace sessions but keeps one tombstone per installation (cap census stays cumulative)", async () => {
    const t = rig.nowMs;
    const ancient = t - EXPIRED_ROW_GRACE_MS - 10_000;
    await store.createSession(session("inst-old", "t1", ancient));
    await store.createSession(session("inst-old", "t2", ancient + 1));
    await store.createSession(session("inst-old", "t3", ancient + 2));
    await store.createSession(session("inst-live", "t4", t + 60_000));
    // Each creating write prunes; inst-old keeps exactly its newest row.
    const internals = store as unknown as { sessions: Map<string, { installationId: string }> };
    const remaining = [...internals.sessions.values()].map((s) => s.installationId).sort();
    expect(remaining).toEqual(["inst-live", "inst-old"]);
    // the census basis is untouched
    expect(await store.countEnrolledInstallations()).toBe(2);
  });

  it("never removes the only row of an installation", async () => {
    const t = rig.nowMs;
    await store.createSession(session("solo", "only", t - EXPIRED_ROW_GRACE_MS - 60_000));
    await store.createSession(session("other", "live", t + 60_000));
    expect(await store.countEnrolledInstallations()).toBe(2);
    expect(await store.getSession("only")).not.toBeNull();
  });
});

describe("device_codes: expired-beyond-grace rows are swept at write time", () => {
  it("keeps fresh and pending codes, drops long-expired ones", async () => {
    const t = rig.nowMs;
    await store.createDeviceAuth({
      deviceCode: "dead",
      installationId: "i1",
      userCode: "ABCDEFGH",
      verifyUrl: "https://x/verify",
      approved: false,
      expiresAt: t - EXPIRED_ROW_GRACE_MS - 1,
    });
    await store.createDeviceAuth({
      deviceCode: "fresh",
      installationId: "i2",
      userCode: "JKLMNOPQ",
      verifyUrl: "https://x/verify",
      approved: false,
      expiresAt: t + 60_000,
    });
    expect(await store.getDeviceAuth("fresh")).not.toBeNull();
    expect(await store.getDeviceAuth("dead")).toBeNull();
  });
});

describe("capability_tokens: grace-expired tokens are swept at write time", () => {
  it("drops tokens expired beyond the grace, keeps usable ones", async () => {
    const t = rig.nowMs;
    const base = {
      leaseId: "l".repeat(32),
      peerId: "peer",
      installationId: "i",
      profileId: "msp1:" + "0".repeat(64),
      challengeId: "c".repeat(32),
      canHost: true,
      canConsume: true,
      slots: 1,
      capacityClass: "cpu" as const,
      auditEpoch: 0,
      issuedAt: t - 100_000,
    };
    await store.recordToken({ ...base, nonce: "dead", expiresAt: t - EXPIRED_ROW_GRACE_MS - 1 });
    await store.recordToken({ ...base, nonce: "live", expiresAt: t + 60_000 });
    expect(await store.getTokenByNonce("live")).not.toBeNull();
    expect(await store.getTokenByNonce("dead")).toBeNull();
  });
});

describe("session_authorizations: expired rows are dropped at write time", () => {
  it("keeps only unexpired rows", async () => {
    const t = rig.nowMs;
    const auth = (sessionId: string, expiresAt: number) => ({
      sessionId,
      peerIds: ["peer"],
      profileId: "msp1:" + "0".repeat(64),
      mode: "single",
      createdAt: t,
      expiresAt,
    });
    await store.recordSessionAuthorization(auth("expired", t - 1));
    await store.recordSessionAuthorization(auth("live", t + 60_000));
    const internals = store as unknown as { sessionAuthorizations: Array<{ sessionId: string }> };
    expect(internals.sessionAuthorizations.map((a) => a.sessionId)).toEqual(["live"]);
  });
});

describe("peer_notices: dead-lease queues are dropped on {all:true} fan-out", () => {
  it("notices for leases dead beyond the grace are not fanned out and old queues are dropped", async () => {
    const t = rig.nowMs;
    const deadLease = {
      leaseId: "dead".padEnd(32, "0"),
      installationId: "i1",
      peerId: "p1",
      createdAt: t - 100_000,
      expiresAt: t - EXPIRED_ROW_GRACE_MS - 1,
      profiles: [],
      addresses: [],
      maxSlots: 1,
      freeSlots: 1,
      queueMs: 0,
      draining: false,
      capacityClass: "cpu" as const,
      auditEpoch: 0,
      lastSeenAt: t - 100_000,
    };
    const liveLease = { ...deadLease, leaseId: "live".padEnd(32, "0"), expiresAt: t + 60_000 };
    await store.createLease(deadLease);
    await store.createLease(liveLease);
    await store.pushNotice({ leaseId: deadLease.leaseId }, { type: "revoked_tokens", tokenNonces: ["x"] });
    await store.pushNotice({ all: true }, { type: "catalog_update", catalogVersion: 2 });
    const drainedDead = await store.drainNotices(deadLease.leaseId);
    const drainedLive = await store.drainNotices(liveLease.leaseId);
    // the dead lease's queued notice was pruned by the fan-out write; the
    // live lease received exactly the new notice
    expect(drainedDead).toEqual([]);
    expect(drainedLive).toEqual([{ type: "catalog_update", catalogVersion: 2 }]);
  });
});

describe("BASE_TIME sanity", () => {
  it("fixed clock keeps row-aging deterministic", () => {
    expect(rig.nowMs).toBe(BASE_TIME);
  });
});
