// D16 (owner gate 2026-10-09 / ADR-028 §7): nonce-bound challengePrompt
// riding the existing wire field, strict canary-pin mode, and the D7 honest
// relabel's enforceable part (timing plausibility). Construction parity is
// covered by tests/crypto-vectors.test.ts against the golden vector.

import { beforeEach, describe, expect, it } from "vitest";
import { buildChallengePrompt, parseChallengePrompt } from "@/lib/challenge";
import {
  enroll,
  makeRig,
  register,
  seedActiveProfile,
  signedRequest,
  testManifest,
} from "./helpers";
import type { Enrollment } from "./helpers";
import * as challengeStart from "@/app/api/v1/peers/challenge/start/route";
import * as challengeComplete from "@/app/api/v1/peers/challenge/complete/route";

let rig: ReturnType<typeof makeRig>;
let profileId: string;
let otherProfileId: string;

const CANARY_A = "sha256:" + "ab".repeat(32);
const CANARY_OTHER = "sha256:" + "cd".repeat(32);

function start(who: Enrollment, leaseId: string, profile: string) {
  return challengeStart.POST(
    signedRequest(
      who.keys,
      who.installationId,
      who.session,
      "/api/v1/peers/challenge/start",
      JSON.stringify({ leaseId, profileId: profile }),
      rig.ctx.now,
    ),
  );
}

function complete(
  who: Enrollment,
  leaseId: string,
  profile: string,
  challengeId: string,
  timings: { firstTokenMs: number; totalMs: number },
) {
  return challengeComplete.POST(
    signedRequest(
      who.keys,
      who.installationId,
      who.session,
      "/api/v1/peers/challenge/complete",
      JSON.stringify({ leaseId, profileId: profile, challengeId, timings }),
      rig.ctx.now,
    ),
  );
}

/** Fresh rig with strict canary pins; re-seeds the same deterministic
 *  profile ids into the new store (deriveProfileId is content-derived, so
 *  the ids are stable across rigs). */
async function strictRig() {
  rig = makeRig({ canaryPins: new Map([[profileId, CANARY_A]]) });
  profileId = await seedActiveProfile(rig, testManifest());
  otherProfileId = await seedActiveProfile(
    rig,
    testManifest({ hf_repo: "example-org/another-model-gguf" }),
  );
}

beforeEach(async () => {
  rig = makeRig();
  profileId = await seedActiveProfile(rig, testManifest());
  otherProfileId = await seedActiveProfile(
    rig,
    testManifest({ hf_repo: "example-org/another-model-gguf" }),
  );
});

describe("D16 nonce-bound prompt (default, no pins configured)", () => {
  it("serves a prompt that embeds the challengeId and parses back", async () => {
    const who = await enroll(rig, "198.51.100.60");
    const { leaseId } = await register(rig, who, [profileId]);
    const res = await start(who, leaseId, profileId);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { challengeId: string; challengePrompt: string };
    expect(body.challengePrompt).toBe(buildChallengePrompt(body.challengeId, null));
    const parsed = parseChallengePrompt(body.challengePrompt);
    expect(parsed?.challengeId).toBe(body.challengeId);
    expect(parsed?.canaryDigest).toBeNull();
  });

  it("different challenge instances carry different prompts (no fixed constant)", async () => {
    const whoA = await enroll(rig, "198.51.100.61");
    const whoB = await enroll(rig, "198.51.100.62");
    const a = await register(rig, whoA, [profileId]);
    const b = await register(rig, whoB, [profileId]);
    const resA = (await (await start(whoA, a.leaseId, profileId)).json()) as { challengePrompt: string };
    const resB = (await (await start(whoB, b.leaseId, profileId)).json()) as { challengePrompt: string };
    expect(resA.challengePrompt).not.toBe(resB.challengePrompt);
  });

  it("re-start of an open challenge is idempotent (same nonce, same prompt)", async () => {
    const who = await enroll(rig, "198.51.100.63");
    const { leaseId } = await register(rig, who, [profileId]);
    const first = (await (await start(who, leaseId, profileId)).json()) as {
      challengeId: string;
      challengePrompt: string;
    };
    const second = (await (await start(who, leaseId, profileId)).json()) as {
      challengeId: string;
      challengePrompt: string;
    };
    expect(second.challengeId).toBe(first.challengeId);
    expect(second.challengePrompt).toBe(first.challengePrompt);
  });
});

describe("D16 strict canary mode (MSP_CANARY_PINS configured)", () => {
  it("embeds the pinned digest in the served prompt", async () => {
    await strictRig();
    const who = await enroll(rig, "198.51.100.64");
    const { leaseId } = await register(rig, who, [profileId]);
    const body = (await (await start(who, leaseId, profileId)).json()) as {
      challengePrompt: string;
    };
    const parsed = parseChallengePrompt(body.challengePrompt);
    expect(parsed?.canaryDigest).toBe(CANARY_A);
    expect(body.challengePrompt).toBe(buildChallengePrompt(parsed!.challengeId, CANARY_A));
  });

  it("refuses challenges for unpinned profiles (fail closed, no lease path)", async () => {
    await strictRig();
    const who = await enroll(rig, "198.51.100.65");
    const { leaseId } = await register(rig, who, [otherProfileId]);
    const res = await start(who, leaseId, otherProfileId);
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("no_capability");
  });

  it("two pinned profiles never share a served prompt (nonce + per-profile digest)", async () => {
    rig = makeRig({
      canaryPins: new Map([
        [profileId, CANARY_A],
        [otherProfileId, CANARY_OTHER],
      ]),
    });
    profileId = await seedActiveProfile(rig, testManifest());
    otherProfileId = await seedActiveProfile(
      rig,
      testManifest({ hf_repo: "example-org/another-model-gguf" }),
    );
    const who = await enroll(rig, "198.51.100.66");
    const { leaseId } = await register(rig, who, [profileId, otherProfileId]);
    const p1 = (await (await start(who, leaseId, profileId)).json()) as { challengePrompt: string };
    const p2 = (await (await start(who, leaseId, otherProfileId)).json()) as { challengePrompt: string };
    expect(p1.challengePrompt).toContain(CANARY_A);
    expect(p2.challengePrompt).toContain(CANARY_OTHER);
    expect(p1.challengePrompt).not.toBe(p2.challengePrompt);
  });
});

describe("D7 timing plausibility at challenge/complete", () => {
  it("rejects zero-duration completions", async () => {
    const who = await enroll(rig, "198.51.100.67");
    const { leaseId } = await register(rig, who, [profileId]);
    const { challengeId } = (await (await start(who, leaseId, profileId)).json()) as {
      challengeId: string;
    };
    const res = await complete(who, leaseId, profileId, challengeId, { firstTokenMs: 0, totalMs: 0 });
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });

  it("rejects firstTokenMs > totalMs and leaves the challenge open", async () => {
    const who = await enroll(rig, "198.51.100.68");
    const { leaseId } = await register(rig, who, [profileId]);
    const { challengeId } = (await (await start(who, leaseId, profileId)).json()) as {
      challengeId: string;
    };
    const res = await complete(who, leaseId, profileId, challengeId, { firstTokenMs: 900, totalMs: 500 });
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
    // the challenge stays open for an honest retry
    const retry = await complete(who, leaseId, profileId, challengeId, { firstTokenMs: 100, totalMs: 500 });
    expect(retry.status).toBe(200);
  });

  it("plausible self-reported timings still pass and label the class (D7: label, not proof)", async () => {
    const who = await enroll(rig, "198.51.100.69");
    const { leaseId } = await register(rig, who, [profileId]);
    const { challengeId } = (await (await start(who, leaseId, profileId)).json()) as {
      challengeId: string;
    };
    const res = await complete(who, leaseId, profileId, challengeId, { firstTokenMs: 100, totalMs: 1500 });
    expect(res.status).toBe(200);
    expect(((await res.json()) as { passed: boolean; capacityClass: string }).capacityClass).toBe("gpu_high");
  });
});
