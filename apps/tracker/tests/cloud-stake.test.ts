// ADR-040 §3 path (b): an installation ACTIVELY serving a cloud profile
// (registered + challenge-passed on its own lease) may consume ANY
// profile — the cloud stake. No stake and no local challenge = the
// unchanged no_capability refusal.

import { beforeEach, describe, expect, it } from "vitest";
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
import { deriveProfileId } from "@/lib/crypto";
import { balanceDecision } from "@/lib/balance";

let rig: TestRig;
let localProfile: string;
let cloudProfile: string;

const CLOUD_MANIFEST = {
  schema_version: 2,
  family: "cloud",
  provider: "deepseek",
  model: "deepseek-chat",
  context_tokens: 65536,
  max_output_tokens: 8192,
  wire: "openai-chat-v1",
};

beforeEach(async () => {
  rig = makeRig();
  localProfile = await seedActiveProfile(rig, testManifest());
  cloudProfile = deriveProfileId(CLOUD_MANIFEST);
  await rig.store.insertProfile({
    profileId: cloudProfile,
    manifest: CLOUD_MANIFEST as never,
    displayName: "DeepSeek Chat (cloud)",
    status: "active",
    provenance: undefined,
    createdAt: rig.ctx.now(),
  });
});

describe("ADR-040 path (b): the cloud stake", () => {
  it("a cloud-serving peer consumes a LOCAL profile it does not host", async () => {
    const who = await enroll(rig);
    const { leaseId } = await register(rig, who, [cloudProfile]);
    await passChallenge(rig, who, leaseId, cloudProfile); // the serving proof

    const res = await peersLease.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: localProfile }), rig.ctx.now),
    );
    expect(res.status).toBe(200);
    const body = (await res.json()) as { token: string; lease: { model_profile_id?: string; profile_id?: string } };
    expect(body.token).toBeTruthy();
  });

  it("no cloud stake and no local challenge still refuses (unchanged path (a))", async () => {
    const who = await enroll(rig);
    const other = await seedActiveProfile(rig, testManifest({ display_name: "Other" }));
    const { leaseId } = await register(rig, who, [other]); // hosts nothing cloud, no challenge for `other`... pass none
    const res = await peersLease.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: localProfile }), rig.ctx.now),
    );
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("no_capability");
  });

  it("a cloud profile on the lease WITHOUT a passed cloud challenge grants nothing", async () => {
    const who = await enroll(rig);
    const { leaseId } = await register(rig, who, [cloudProfile]); // never challenged
    const res = await peersLease.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: localProfile }), rig.ctx.now),
    );
    expect(res.status).toBe(403);
  });
});

// ADR-042 (owner ruling 2026-10-17 night): the CAPACITY stake — an
// actively-hosting peer (a passed challenge on ANY hosted profile, local
// included; no provider key anywhere) consumes cloud profiles, bounded by
// its machine's measured serving capacity. Cloud leases enforce the
// balance BY DEFAULT (born limited); the operator kill-switch is
// MSP_SWARM_BALANCE_ENFORCE_CLOUD=off.
describe("ADR-042: the capacity stake (local hosts pull cloud keys)", () => {
  type Who = Awaited<ReturnType<typeof enroll>>;

  beforeEach(() => {
    delete process.env.MSP_SWARM_BALANCE_ENFORCE;
    delete process.env.MSP_SWARM_BALANCE_ENFORCE_CLOUD;
  });

  async function localHost(): Promise<{ who: Who; leaseId: string }> {
    const who = await enroll(rig);
    const { leaseId } = await register(rig, who, [localProfile]);
    await passChallenge(rig, who, leaseId, localProfile); // serving proof: a LOCAL profile
    return { who, leaseId };
  }

  const cloudLease = (who: Who, leaseId: string) =>
    peersLease.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: cloudProfile }), rig.ctx.now),
    );

  it("a LOCAL-only host consumes a CLOUD profile with no key staged", async () => {
    const { who, leaseId } = await localHost();
    const res = await cloudLease(who, leaseId);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { token: string; balance: { capacitySource: string } };
    expect(body.token).toBeTruthy();
    expect(body.balance.capacitySource).toBe("class"); // cold start — no served receipts yet
  });

  it("cloud leases enforce the balance by default (born limited)", async () => {
    const { who, leaseId } = await localHost();
    const budget = balanceDecision("gpu_high", 0).budget; // challenge timings map the lease to the top class
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "c".repeat(64),
      tokens: budget + 1,
      at: rig.ctx.now(),
    });
    const res = await cloudLease(who, leaseId);
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("balance_exceeded");
  });

  it("the operator kill-switch (ENFORCE_CLOUD=off) re-opens over-budget cloud leases", async () => {
    process.env.MSP_SWARM_BALANCE_ENFORCE_CLOUD = "off";
    const { who, leaseId } = await localHost();
    const budget = balanceDecision("gpu_high", 0).budget;
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "d".repeat(64),
      tokens: budget + 1,
      at: rig.ctx.now(),
    });
    const res = await cloudLease(who, leaseId);
    expect(res.status).toBe(200);
  });

  it("the measured anchor (receipt-backed served tok/s) sets the cloud budget", async () => {
    const { who, leaseId } = await localHost();
    // 1800 served tokens over 36 served seconds = 50 tok/s measured —
    // BELOW the gpu_high class anchor, so the machine's own number governs
    // (ADR-042 §2): 200k consumed now exceeds the 180k measured window
    // budget even though it sits inside the class budget.
    await rig.store.recordGrantedAccounting([
      {
        receiptDigest: "e".repeat(64),
        peerId: who.peerId,
        installationId: who.installationId,
        profileId: localProfile,
        endpointSeconds: 36,
        verifiedTokens: 1800,
        grantedAt: rig.ctx.now(),
      },
    ]);
    await rig.store.recordConsumption({
      installationId: who.installationId,
      receiptDigest: "f".repeat(64),
      tokens: 200_000,
      at: rig.ctx.now(),
    });
    const res = await cloudLease(who, leaseId);
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("balance_exceeded");
  });
});
