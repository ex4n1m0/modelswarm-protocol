// E capability/eligibility tokens: shape, binding, TTL cap, signature,
// no-capability rejection, and record-level expiry past the grace window.

import { beforeEach, describe, expect, it } from "vitest";
import { canonicalJson, verifyBase64Url } from "@/lib/crypto";
import {
  enroll,
  makeRig,
  passChallenge,
  register,
  requestEligibilityLease,
  seedActiveProfile,
  testManifest,
} from "./helpers";

let rig: ReturnType<typeof makeRig>;
let profileId: string;

beforeEach(async () => {
  rig = makeRig();
  profileId = await seedActiveProfile(rig, testManifest());
});

describe("E1 eligibility lease conforms to §5/ADR-012", () => {
  it("issued after a passed challenge: shape, binding, TTL cap, signature", async () => {
    const who = await enroll(rig, "198.51.100.20");
    const { leaseId, leaseExpiresAt } = await register(rig, who, [profileId]);
    await passChallenge(rig, who, leaseId, profileId, { firstTokenMs: 100, totalMs: 1500 });

    const res = await requestEligibilityLease(rig, who, leaseId, profileId);
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      token: string;
      lease: Record<string, unknown>;
    };

    // serialized form base64url(json).base64url(sig)
    const parts = body.token.split(".");
    expect(parts).toHaveLength(2);
    const json = Buffer.from(parts[0]!, "base64url").toString("utf8");
    const fields = JSON.parse(json) as Record<string, unknown>;
    expect(fields).toEqual(body.lease);

    // peer/profile/installation binding
    expect(fields.peer_id).toBe(who.peerId);
    expect(fields.installation_id).toBe(who.installationId);
    expect(fields.model_profile_id).toBe(profileId);
    expect(fields.can_host).toBe(true);
    expect(fields.can_consume).toBe(true);
    expect(fields.slots).toBe(2);
    expect(fields.verified_capacity).toBe("gpu_high"); // totalMs 1500 -> gpu_high
    expect(typeof fields.audit_epoch).toBe("number");
    expect(typeof fields.challenge_id).toBe("string");

    // TTL cap: never outlives the issuing lease by more than 60 s
    const expiry = Date.parse(fields.expires_at as string);
    expect(expiry).toBeLessThanOrEqual(Date.parse(leaseExpiresAt) + 60_000);
    expect(expiry).toBe(Date.parse(leaseExpiresAt) + 60_000);
    expect(expiry).toBeGreaterThan(rig.ctx.now());

    // signature verifies with the hub public key
    expect(verifyBase64Url(parts[1]!, json, rig.ctx.keys.publicKey)).toBe(true);
    // canonical serialization (sorted keys) is what is signed
    expect(json).toBe(canonicalJson(fields));

    // stored record matches (E3 lookup handle)
    const record = await rig.store.getTokenByNonce(fields.nonce as string);
    expect(record?.profileId).toBe(profileId);
    expect(record?.peerId).toBe(who.peerId);
  });

  it("reissue (refresh) yields a fresh nonce and stays capped", async () => {
    const who = await enroll(rig, "198.51.100.21");
    const { leaseId, leaseExpiresAt } = await register(rig, who, [profileId]);
    await passChallenge(rig, who, leaseId, profileId);
    const first = (await (await requestEligibilityLease(rig, who, leaseId, profileId)).json()) as {
      lease: { nonce: string; expires_at: string };
    };
    const second = (await (await requestEligibilityLease(rig, who, leaseId, profileId)).json()) as {
      lease: { nonce: string; expires_at: string };
    };
    expect(first.lease.nonce).not.toBe(second.lease.nonce);
    expect(Date.parse(second.lease.expires_at)).toBeLessThanOrEqual(Date.parse(leaseExpiresAt) + 60_000);
  });
});

describe("E2 issuance requires a passed challenge", () => {
  it("no passed challenge -> 403 no_capability", async () => {
    const who = await enroll(rig, "198.51.100.22");
    const { leaseId } = await register(rig, who, [profileId]);

    const res = await requestEligibilityLease(rig, who, leaseId, profileId);
    expect(res.status).toBe(403);
    const body = (await res.json()) as { error: { code: string } };
    expect(body.error.code).toBe("no_capability");

    // after passing the challenge it works
    await passChallenge(rig, who, leaseId, profileId);
    const ok = await requestEligibilityLease(rig, who, leaseId, profileId);
    expect(ok.status).toBe(200);
  });

  it("challenge for a different profile does not grant this one", async () => {
    const otherProfile = await seedActiveProfile(
      rig,
      testManifest({ hf_revision: "c".repeat(40), runtime: { name: "rt-test", version: "b2", build_hash: "2".repeat(64) } }),
    );
    const who = await enroll(rig, "198.51.100.23");
    const { leaseId } = await register(rig, who, [profileId, otherProfile]);
    await passChallenge(rig, who, leaseId, otherProfile);

    const res = await requestEligibilityLease(rig, who, leaseId, profileId);
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("no_capability");
  });
});

describe("E3 record-level authority dies with the lease past the 60 s grace", () => {
  it("usable within grace, rejected after lease expiry + 60 s", async () => {
    const who = await enroll(rig, "198.51.100.24");
    const { leaseId, leaseExpiresAt } = await register(rig, who, [profileId]);
    await passChallenge(rig, who, leaseId, profileId);
    const issued = (await (await requestEligibilityLease(rig, who, leaseId, profileId)).json()) as {
      lease: { nonce: string; expires_at: string };
    };

    const leaseExpiry = Date.parse(leaseExpiresAt);

    // still within the token's own lifetime and the lease grace
    rig.setNow(leaseExpiry + 30_000);
    const within = await rig.store.getTokenAuthority(issued.lease.nonce);
    expect(within.usable).toBe(true);

    // past the grace window the record is rejected. Because a correctly
    // issued token's expires_at equals lease expiry + 60 s, both expiry
    // conditions trip together; the rejection itself is what E3 pins.
    rig.setNow(Date.parse(issued.lease.expires_at) + 1);
    const past = await rig.store.getTokenAuthority(issued.lease.nonce);
    expect(past.usable).toBe(false);
    if (!past.usable) {
      expect(["lease_expired", "token_expired"]).toContain(past.reason);
    }

    // A record whose expires_at outlives the lease cap (e.g. a legacy row) is
    // rejected specifically by the lease-grace rule, before its own expiry.
    await rig.store.recordToken({
      nonce: issued.lease.nonce,
      leaseId,
      peerId: who.peerId,
      installationId: who.installationId,
      profileId,
      challengeId: "c".repeat(32),
      canHost: true,
      canConsume: true,
      slots: 1,
      capacityClass: "cpu",
      auditEpoch: 0,
      issuedAt: rig.ctx.now() - 90_000,
      expiresAt: leaseExpiry + 120_000, // not yet expired
    });
    const legacy = await rig.store.getTokenAuthority(issued.lease.nonce);
    expect(legacy.usable).toBe(false);
    if (!legacy.usable) expect(legacy.reason).toBe("lease_expired");

    // unknown nonce
    const unknown = await rig.store.getTokenAuthority("f".repeat(32));
    expect(unknown.usable).toBe(false);
  });
});
