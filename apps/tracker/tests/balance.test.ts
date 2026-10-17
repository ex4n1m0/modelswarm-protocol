// ADR-041 swarm balance, CAPACITY rule (owner revision 2026-10-17, the
// torrent lesson): the gate keys on the verified serving class (hardware
// + model type, assigned from measured challenge timings) — NEVER on
// operational history. Rolling window = the burst allowance AND the cold
// start (a fresh host pulls its full window budget immediately; no
// bootstrap, no unpopular-torrent spiral).

import { beforeEach, describe, expect, it } from "vitest";
import {
  balanceDecision,
  CLASS_TOKENS_PER_SEC,
  DEFAULT_BALANCE_K,
  DEFAULT_WINDOW_SECS,
} from "@/lib/balance";
import {
  makeRig,
  seedActiveProfile,
  testManifest,
  enroll,
  register,
  passChallenge,
  peersLease,
  heartbeat,
  signedRequest,
  type TestRig,
} from "./helpers";

describe("ADR-041 capacity policy (pure)", () => {
  it("budget = k × class capacity × window", () => {
    const d = balanceDecision("gpu_mid", 0);
    expect(d.capacityTokensPerSec).toBe(CLASS_TOKENS_PER_SEC.gpu_mid);
    expect(d.budget).toBe(DEFAULT_BALANCE_K * CLASS_TOKENS_PER_SEC.gpu_mid! * DEFAULT_WINDOW_SECS);
    expect(d.ok).toBe(true);
    expect(d.headroom).toBe(d.budget);
  });

  it("within budget passes; beyond fails — no history input exists", () => {
    const budget = balanceDecision("gpu_mid", 0).budget;
    expect(balanceDecision("gpu_mid", budget).ok).toBe(true);
    expect(balanceDecision("gpu_mid", budget + 1).ok).toBe(false);
  });

  it("unknown class falls back to the honest floor (cpu)", () => {
    expect(balanceDecision("mystery", 0).capacityTokensPerSec).toBe(CLASS_TOKENS_PER_SEC.cpu);
  });
});

describe("ADR-041 lease-route integration (dark by default)", () => {
  let rig: TestRig;
  let profile: string;
  beforeEach(async () => {
    rig = makeRig();
    profile = await seedActiveProfile(rig, testManifest());
    delete process.env.MSP_SWARM_BALANCE_ENFORCE;
    delete process.env.MSP_SWARM_BALANCE_K;
  });

  async function eligibleLease() {
    const who = await enroll(rig);
    const { leaseId } = await register(rig, who, [profile]);
    await passChallenge(rig, who, leaseId, profile);
    return { who, leaseId };
  }

  const call = (who: { keys: ReturnType<typeof Object> } & any, leaseId: string) =>
    peersLease.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: profile }), rig.ctx.now),
    );

  it("every issuance carries the balance view; over-budget still 200 (dark)", async () => {
    const { who, leaseId } = await eligibleLease();
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "a".repeat(64),
      tokens: balanceDecision("gpu_high", 0).budget + 5_000,
      at: rig.ctx.now(),
    });
    const res = await call(who, leaseId);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { balance: { ok: boolean; consumedInWindow: number } };
    expect(body.balance.ok).toBe(false);
    expect(body.balance.consumedInWindow).toBeGreaterThan(balanceDecision("gpu_high", 0).budget);
  });

  it("enforcement on: over-budget blocks; the window refills with time", async () => {
    process.env.MSP_SWARM_BALANCE_ENFORCE = "on";
    const { who, leaseId } = await eligibleLease();
    const budget = balanceDecision("gpu_high", 0).budget; // the challenge timings map the lease to the top class; over-spend past even that
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "b".repeat(64),
      tokens: budget + 5_000,
      at: rig.ctx.now(),
    });
    const blocked = await call(who, leaseId);
    expect(blocked.status).toBe(403);
    expect(((await blocked.json()) as { error: { code: string } }).error.code).toBe("balance_exceeded");
    // The window refills: after a full window with no consumption the same
    // peer is back in budget (the torrent-lesson fix — nothing about its
    // SERVING history mattered at any point). The lease TTL is 75 s, so
    // walk the hour one beat at a time.
    for (let t = 0; t < DEFAULT_WINDOW_SECS + 120; t += 60) {
      rig.advance(60_000);
      await heartbeat(rig, who, leaseId); // keep the 75 s lease alive
    }
    // The passed challenge aged out over the walk — pass a fresh one
    // (this is the normal serving cadence) and the lease is back.
    await passChallenge(rig, who, leaseId, profile);
    const ok = await call(who, leaseId);
    expect(ok.status).toBe(200);
    const body = (await ok.json()) as { balance: { ok: boolean } };
    expect(body.balance.ok).toBe(true);
  });
});
