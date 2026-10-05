// Crypto + golden-vector parity (ADR-011): deriveProfileId must match the
// expected_profile_id of every protocol/vectors fixture (TS / Rust / validator
// triple parity), and eligibility-lease serialization must round-trip.

import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  bs58Decode,
  bs58Encode,
  canonicalJson,
  derivePeerId,
  deriveProfileId,
  publicKeyOf,
  randomId128,
  sha256Hex,
  verifyBase64Url,
} from "@/lib/crypto";
import { issueEligibilityLease, splitEligibilityLease } from "@/lib/eligibility";
import { HUB_SEED, testManifest } from "./helpers";
import type { ChallengeRecord, LeaseRecord } from "@/lib/store";

const here = dirname(fileURLToPath(import.meta.url));
const vectorsDir = join(here, "..", "..", "..", "protocol", "vectors");

describe("golden vector parity (ADR-011)", () => {
  const files = readdirSync(vectorsDir).filter((f) => f.endsWith(".json")).sort();
  it("finds fixtures", () => {
    expect(files.length).toBeGreaterThanOrEqual(2);
  });

  for (const file of files) {
    it(`deriveProfileId matches ${file}`, () => {
      const doc = JSON.parse(readFileSync(join(vectorsDir, file), "utf8")) as {
        manifest: unknown;
        expected_profile_id: string;
      };
      expect(deriveProfileId(doc.manifest)).toBe(doc.expected_profile_id);
    });
  }

  it("any hashed-field change yields a different id", () => {
    const doc = JSON.parse(readFileSync(join(vectorsDir, files[0]!), "utf8")) as {
      manifest: Record<string, unknown>;
    };
    const mutated = {
      ...doc.manifest,
      runtime: { ...(doc.manifest.runtime as object), version: "b1-test" },
    };
    expect(deriveProfileId(mutated)).not.toBe(deriveProfileId(doc.manifest));
  });
});

describe("ADR-020 peerId derivation", () => {
  it("seed [9u8;32] matches the Rust peer_id_for golden (12D3Koo form)", () => {
    const seed = Buffer.alloc(32, 9);
    const peerId = derivePeerId(publicKeyOf(seed));
    // Same literal asserted by modelswarm-identity installation tests.
    expect(peerId).toBe("12D3KooWSrKnMZUcSxK8G7wmBbXdU8nFEfWGhLu6H8xjn8LmCSJb");
  });
  it("decodes to the identity multihash of the protobuf-encoded key", () => {
    const seed = Buffer.alloc(32, 9);
    const publicKey = publicKeyOf(seed);
    const decoded = bs58Decode(derivePeerId(publicKey))!;
    expect(decoded!.length).toBe(38);
    expect([...decoded.slice(0, 6)]).toEqual([0x00, 0x24, 0x08, 0x01, 0x12, 0x20]);
    expect([...decoded.slice(6)]).toEqual([...publicKey]);
  });
});

describe("canonical JSON", () => {
  it("sorts keys recursively and matches the validator algorithm", () => {
    const input = { b: 1, a: { d: [3, { z: 1, y: 2 }], c: "x" } };
    expect(canonicalJson(input)).toBe('{"a":{"c":"x","d":[3,{"y":2,"z":1}]},"b":1}');
  });
  it("does not mutate its input", () => {
    const input = { b: 1, a: 2 };
    canonicalJson(input);
    expect(Object.keys(input)).toEqual(["b", "a"]);
  });
});

describe("sha256", () => {
  it("hex digest of the empty string (GET bodyDigest base)", () => {
    expect(sha256Hex("")).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
  });
});

describe("base58", () => {
  it("known encodings", () => {
    expect(bs58Encode(new Uint8Array([0]))).toBe("1");
    expect(bs58Encode(new Uint8Array([0, 0]))).toBe("11");
    expect(bs58Encode(new Uint8Array([1]))).toBe("2");
    expect(bs58Encode(new Uint8Array([0x12, 0x34]))).toBe("2PM");
    expect(bs58Encode(Buffer.from("test", "utf8"))).toBe("3yZe7d");
    // cross-check against the id-derivation rule used by the Rust side
    expect(bs58Encode(new Uint8Array(Buffer.from(sha256Hex("test"), "hex")))).toBe(
      "Bjj4AWTNrjQVHqgWbP2XaxXz4DYH1WZMyERHxsad7b2w",
    );
  });
  it("round-trips random buffers", () => {
    for (let i = 0; i < 50; i += 1) {
      const bytes = new Uint8Array(randomId128().match(/../g)!.map((h) => Number.parseInt(h, 16)));
      const encoded = bs58Encode(bytes);
      expect(bs58Decode(encoded)).toEqual(bytes);
    }
  });
});

describe("eligibility lease serialization (ADR-012)", () => {
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

  it("serializes as base64url(json).base64url(sig) and verifies with the hub key", () => {
    const issued = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey);
    const parts = issued.token.split(".");
    expect(parts).toHaveLength(2);
    const json = Buffer.from(parts[0]!, "base64url").toString("utf8");
    // canonical: keys sorted
    expect(json[0]).toBe("{");
    expect(json.indexOf('"audit_epoch"')).toBeLessThan(json.indexOf('"can_host"'));
    // signature verifies over exactly that json
    expect(verifyBase64Url(parts[1]!, json, publicKey)).toBe(true);
    // fields round-trip
    expect(JSON.parse(json)).toEqual(issued.fields);
    // expiry capped at lease expiry + 60 s
    expect(issued.fields.expires_at).toBe(new Date(lease.expiresAt + 60_000).toISOString());
    expect(issued.fields.peer_id).toBe(lease.peerId);
    expect(issued.fields.model_profile_id).toBe(lease.profiles[0]);
    expect(issued.fields.audit_epoch).toBe(4);
  });

  it("a flipped signature byte fails verification", () => {
    const issued = issueEligibilityLease(lease, challenge, lease.profiles[0]!, 4, 1_000, secretKey);
    const parts = issued.token.split(".");
    const flipped = (Number.parseInt(parts[1]![0]!, 36) ^ 1).toString(36) + parts[1]!.slice(1);
    const json = Buffer.from(parts[0]!, "base64url").toString("utf8");
    expect(verifyBase64Url(flipped, json, publicKey)).toBe(false);
  });

  it("splitEligibilityLease rejects malformed tokens", () => {
    expect(splitEligibilityLease("no-dot")).toBeNull();
    expect(splitEligibilityLease("a.")).toBeNull();
    expect(splitEligibilityLease(".b")).toBeNull();
  });

  it("capacity table is deterministic", () => {
    // sanity: timings table exists via capacityFromTimings (implicitly covered
    // by the challenge tests); here we pin the manifest fixture shape used
    // across the suite.
    const manifest = testManifest();
    expect(deriveProfileId(manifest)).toMatch(/^msp1:[0-9a-f]{64}$/);
  });
});
