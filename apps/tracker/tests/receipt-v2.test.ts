// Receipts v2 (ADR-030 §1/§2): the TS↔Rust byte-parity gate against the
// FROZEN golden vector protocol/vectors/receipt-v2-1.json, plus the
// intake-validation behavior rows (digest preimage, signatures,
// self-work exclusion, conservative attribution).

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { bs58Decode } from "@/lib/crypto";
import {
  cohortDigestOf,
  finalCanonical,
  perPeerShare,
  phase1Canonical,
  phase2Canonical,
  receiptDigest,
  validateForAccounting,
  type ReceiptV2,
} from "@/lib/receipt-v2";

const here = dirname(fileURLToPath(import.meta.url));
const vectorsDir = join(here, "../../../protocol/vectors");
const V = JSON.parse(readFileSync(join(vectorsDir, "receipt-v2-1.json"), "utf8")) as {
  receipt: ReceiptV2;
  phase1_canonical_json: string;
  phase2_canonical_json: string;
  final_canonical_json: string;
  receipt_digest: string;
  server_signing_seed_hex: string;
  requester_signing_seed_hex: string;
};

function seedKey(seedHex: string): Uint8Array {
  return new Uint8Array(
    (seedHex.match(/.{2}/g) ?? []).map((h) => parseInt(h, 16)),
  );
}

describe("receipt-v2-1 golden vector (ADR-030 §6, TS↔Rust parity)", () => {
  it("both canonical phases + final form + digest are byte-exact", () => {
    expect(phase1Canonical(V.receipt)).toBe(V.phase1_canonical_json);
    expect(phase2Canonical(V.receipt)).toBe(V.phase2_canonical_json);
    expect(finalCanonical(V.receipt)).toBe(V.final_canonical_json);
    expect(receiptDigest(V.receipt)).toBe(V.receipt_digest);
    expect(V.receipt_digest).toMatch(/^sha256:[0-9a-f]{64}$/);
  });

  it("cohort digest re-derives from the peer list, order-independent", () => {
    expect(cohortDigestOf(V.receipt.peers)).toBe(V.receipt.cohort_digest);
    expect(cohortDigestOf([...V.receipt.peers].reverse())).toBe(V.receipt.cohort_digest);
  });

  it("both signatures verify against the fixture seeds' public keys", () => {
    // The eligible ed25519 public key derives directly from the seed.
    const serverPub = serverPublicKeyOf(V.server_signing_seed_hex);
    const requesterPub = serverPublicKeyOf(V.requester_signing_seed_hex);
    expect(
      validateForAccounting(V.receipt, V.receipt_digest, serverPub, requesterPub),
    ).not.toBeNull();
  });
});

// Tiny local ed25519 pubkey-from-seed (the tracker crypto lib exposes
// publicKeyOf(secretKey) — reused here for the vector keys).
import { publicKeyOf } from "@/lib/crypto";
function serverPublicKeyOf(seedHex: string): Uint8Array {
  return publicKeyOf(seedKey(seedHex));
}

describe("intake validation (ADR-030 §3: nothing self-reported moves counters)", () => {
  const serverPub = serverPublicKeyOf(V.server_signing_seed_hex);
  const requesterPub = serverPublicKeyOf(V.requester_signing_seed_hex);

  it("wrong claimed digest -> not accounting evidence", () => {
    expect(validateForAccounting(V.receipt, "sha256:deadbeef", serverPub, requesterPub)).toBeNull();
  });

  it("missing either signature -> null", () => {
    const one = { ...V.receipt, requester_signature: "" };
    expect(validateForAccounting(one, receiptDigest(one) ?? "", serverPub, requesterPub)).toBeNull();
  });

  it("wrong keys -> null (both signatures must verify against ENROLLED keys)", () => {
    expect(validateForAccounting(V.receipt, V.receipt_digest, requesterPub, serverPub)).toBeNull();
  });

  it("self-work contributes zero; attribution is conservative", () => {
    const grants = validateForAccounting(V.receipt, V.receipt_digest, serverPub, requesterPub)!;
    expect(grants).toHaveLength(2);
    expect(grants.every((g) => g.endpoint_seconds === 3)).toBe(true);
    // The solo self-work receipt: zero grants.
    const solo: ReceiptV2 = {
      ...V.receipt,
      peers: [{ peer_id: "12D3KooWSolo", installation_id: "InstSolo", role: "server" }],
    };
    expect(perPeerShare(solo, 0)).toEqual({ endpoint_seconds: 0, verified_tokens: 0 });
  });
});

// ---- ADR-045: the presets golden vector (receipt-v2-2-presets.json) ----
// The B2 trap's regression test: the TS side must digest the object AS
// PARSED (zero stripping) — six DISTINCT digests prove the field cannot
// be dropped or normalized before digesting.

describe("receipt-v2-2-presets golden vector (ADR-045, digest sensitivity)", () => {
  const P = JSON.parse(
    readFileSync(join(vectorsDir, "receipt-v2-2-presets.json"), "utf8"),
  ) as {
    cases: Array<{
      preset: string;
      receipt: ReceiptV2;
      phase1_canonical_json: string;
      phase2_canonical_json: string;
      final_canonical_json: string;
      receipt_digest: string;
    }>;
  };

  it("carries exactly the frozen six-value receipt-side vocabulary", () => {
    expect(P.cases.map((c) => c.preset).sort()).toEqual(
      ["balanced", "deep", "fast", "maximum", "unspecified", "verified"].sort(),
    );
  });

  it("every case: preimages + digest byte-exact from the parsed object", () => {
    for (const c of P.cases) {
      expect(phase1Canonical(c.receipt)).toBe(c.phase1_canonical_json);
      expect(phase2Canonical(c.receipt)).toBe(c.phase2_canonical_json);
      expect(finalCanonical(c.receipt)).toBe(c.final_canonical_json);
      expect(receiptDigest(c.receipt)).toBe(c.receipt_digest);
    }
  });

  it("six DISTINCT digests — the preset field cannot be stripped or normalized pre-digest", () => {
    const digests = new Set(P.cases.map((c) => c.receipt_digest));
    expect(digests.size).toBe(6);
  });
});
