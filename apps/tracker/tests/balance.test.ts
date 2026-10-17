// ADR-041 swarm balance: policy vectors + the lease-route integration
// (dark mode by default — always computed and surfaced, never blocking;
// enforcement only under MSP_SWARM_BALANCE_ENFORCE=on).

import { beforeEach, describe, expect, it } from "vitest";
import { balanceDecision, DEFAULT_BALANCE_K, DEFAULT_BOOTSTRAP_TOKENS } from "@/lib/balance";
import {
  makeRig,
  seedActiveProfile,
  testManifest,
  enroll,
  register,
  passChallenge,
  peersLease,
  signedRequest,
  type TestRig,
} from "./helpers";

describe("ADR-041 policy table (pure)", () => {
  it("defaults: k=1, bootstrap on, gate satisfied", () => {
    expect(DEFAULT_BALANCE_K).toBe(1);
    expect(DEFAULT_BOOTSTRAP_TOKENS).toBeGreaterThan(0);
    const d = balanceDecision(0, 0);
    expect(d.ok).toBe(true); // bootstrap covers the cold start
    expect(d.headroom).toBe(DEFAULT_BOOTSTRAP_TOKENS);
  });

  it("pull within k*push + bootstrap passes; beyond fails", () => {
    expect(balanceDecision(1_000, 1_000).ok).toBe(true); // exactly k=1
    expect(balanceDecision(1_000, 1_000 + DEFAULT_BOOTSTRAP_TOKENS).ok).toBe(true); // boundary
    expect(balanceDecision(1_000, 1_000 + DEFAULT_BOOTSTRAP_TOKENS + 1).ok).toBe(false);
  });

  it("decayed numbers arrive from the ledger — the fn is scale-free", () => {
    const d = balanceDecision(12_345.6, 999);
    expect(d.headroom).toBeCloseTo(12_345.6 + DEFAULT_BOOTSTRAP_TOKENS - 999, 5);
  });
});

describe("ADR-041 lease-route integration", () => {
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

  it("every issuance carries the balance view (dark: never blocks)", async () => {
    const { who, leaseId } = await eligibleLease();
    // Over-balance by pure consumption, zero granted:
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "a".repeat(64),
      tokens: DEFAULT_BOOTSTRAP_TOKENS + 5_000,
      at: rig.ctx.now(),
    });
    const res = await peersLease.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: profile }), rig.ctx.now),
    );
    expect(res.status).toBe(200); // dark mode: surfaced, not blocking
    const body = (await res.json()) as { balance: { ok: boolean; consumed: number } };
    expect(body.balance.ok).toBe(false); // the honest number is right there
    expect(body.balance.consumed).toBeGreaterThan(DEFAULT_BOOTSTRAP_TOKENS);
  });

  it("enforcement on: over-balance blocks with balance_exceeded; serving refills", async () => {
    process.env.MSP_SWARM_BALANCE_ENFORCE = "on";
    const { who, leaseId } = await eligibleLease();
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "b".repeat(64),
      tokens: DEFAULT_BOOTSTRAP_TOKENS + 5_000,
      at: rig.ctx.now(),
    });
    const call = () =>
      peersLease.POST(
        signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: profile }), rig.ctx.now),
      );
    const blocked = await call();
    expect(blocked.status).toBe(403);
    expect(((await blocked.json()) as { error: { code: string } }).error.code).toBe("balance_exceeded");

    // Serve enough (granted tokens for this peer) and the same call passes.
    await rig.store.recordGrantedAccounting([
      {
        receiptDigest: "c".repeat(64),
        peerId: who.peerId,
        installationId: who.installationId,
        profileId: profile,
        endpointSeconds: 60,
        verifiedTokens: 20_000,
        grantedAt: rig.ctx.now(),
      },
    ]);
    const ok = await call();
    expect(ok.status).toBe(200);
  });
});
