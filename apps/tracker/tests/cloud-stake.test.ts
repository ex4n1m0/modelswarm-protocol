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
