// ADR-043 fair queueing — tracker seat tests: the queue-weight class
// bucketing and the byte-compatibility rule (a base lease's signed payload
// omits queue_weight entirely, so pre-ADR-043 golden vectors and in-flight
// leases stay valid; only >1 weights materialize on the wire).

import { describe, expect, it } from "vitest";

import {
  issueEligibilityLease,
  queueWeightClass,
  splitEligibilityLease,
} from "@/lib/eligibility";
import { publicKeyOf, verifyBase64Url } from "@/lib/crypto";
import { HUB_SEED } from "./helpers";
import type { ChallengeRecord, LeaseRecord } from "@/lib/store";

const secretKey = HUB_SEED;
const publicKey = publicKeyOf(secretKey);

const lease: LeaseRecord = {
  leaseId: "a".repeat(32),
  installationId: "InstallId".padEnd(12, "x"),
  peerId: "PeerId".padEnd(12, "y"),
  createdAt: 0,
  expiresAt: Date.UTC(2026, 10, 4, 12, 1, 15),
  profiles: ["msp1:" + "1".repeat(64)],
  addresses: ["/ip4/10.0.0.1/tcp/4001"],
  maxSlots: 2,
  freeSlots: 1,
  queueMs: 3,
  draining: false,
  capacityClass: "gpu_mid",
  auditEpoch: 4,
  lastSeenAt: 0,
};

const challenge: ChallengeRecord = {
  challengeId: "b".repeat(32),
  leaseId: lease.leaseId,
  profileId: lease.profiles[0]!,
  prompt: "ModelSwarm readiness challenge",
  issuedAt: 0,
  deadlineAt: 120_000,
  outcome: "passed",
  firstTokenMs: 100,
  totalMs: 900,
  completedAt: 90_000,
};

describe("ADR-043 queueWeightClass bucketing", () => {
  it("base / sustained / heavy classes at the owner-approved 1/5 defaults", () => {
    expect(queueWeightClass(0, 1, 5)).toBe(1);
    expect(queueWeightClass(0.99, 1, 5)).toBe(1);
    expect(queueWeightClass(1, 1, 5)).toBe(2);
    expect(queueWeightClass(4.99, 1, 5)).toBe(2);
    expect(queueWeightClass(5, 1, 5)).toBe(4);
    expect(queueWeightClass(40, 1, 5)).toBe(4);
  });

  it("t3 only applies when strictly above t2 (degenerate config guard)", () => {
    // t3 == t2: the >= t3 branch would double-fire; the guard keeps the
    // ladder monotone (>= t3 must also be > t2).
    expect(queueWeightClass(1, 1, 1)).toBe(2);
    expect(queueWeightClass(2, 1, 1)).toBe(2);
  });

  it("custom operator thresholds bucket from the new bars", () => {
    expect(queueWeightClass(0.5, 0.5, 2)).toBe(2);
    expect(queueWeightClass(2, 0.5, 2)).toBe(4);
  });
});

describe("ADR-043 lease serialization byte-compatibility", () => {
  it("a base (weight 1) lease omits queue_weight from the signed payload", () => {
    const issued = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey);
    const parts = splitEligibilityLease(issued.token)!;
    expect(parts.json).not.toContain("queue_weight");
    expect(issued.fields.queue_weight).toBeUndefined();
    // and the signature still verifies over exactly that payload
    expect(verifyBase64Url(parts.signature, parts.json, publicKey)).toBe(true);
  });

  it("an explicit weight-1 argument behaves exactly like the default", () => {
    const a = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey);
    const b = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey, 1);
    expect(splitEligibilityLease(b.token)!.json).not.toContain("queue_weight");
    expect(b.fields.queue_weight).toBeUndefined();
    // Same signed-field SHAPE (nonce/issued_at differ per issuance by design).
    const { nonce: _na, ...aRest } = a.fields;
    const { nonce: _nb, ...bRest } = b.fields;
    expect(aRest).toEqual(bRest);
  });

  it("a weight-4 lease carries queue_weight inside the signed payload", () => {
    const issued = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey, 4);
    const parts = splitEligibilityLease(issued.token)!;
    expect(parts.json).toContain('"queue_weight":4');
    expect(issued.fields.queue_weight).toBe(4);
  });

  it("weights below 1 clamp to base (never 0, never fractional)", () => {
    const zero = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey, 0);
    expect(zero.fields.queue_weight).toBeUndefined();
    const frac = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey, 2.9);
    expect(frac.fields.queue_weight).toBe(2);
  });
});
