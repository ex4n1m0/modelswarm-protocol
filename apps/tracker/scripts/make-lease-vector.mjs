// Generates the cross-language EligibilityLease golden vector
// (protocol/vectors/lease-hubkey-1.json). Deterministic: fixed fixture
// key (NOT the production hub key), fixed fields, no clock or randomness.
// The TS test (apps/tracker/tests/crypto-vectors.test.ts) and the Rust
// test (crates/modelswarm-eligibility/src/lease.rs) both verify this
// exact token — the byte-parity gate for ADR-012/ADR-026 lease interop.
//
// Run: node scripts/make-lease-vector.mjs   (from apps/tracker)
import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ed from "@noble/ed25519";

// Same sha512 injection as lib/crypto.ts (@noble v2 sync API requirement).
ed.etc.sha512Sync = (...m) =>
  createHash("sha512").update(ed.etc.concatBytes(...m)).digest();

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..");

function canonicalize(value) {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(canonicalize);
  const out = {};
  for (const key of Object.keys(value).sort()) out[key] = canonicalize(value[key]);
  return out;
}

const canonicalJson = (value) => JSON.stringify(canonicalize(value));

const b64url = (bytes) => Buffer.from(bytes).toString("base64url");

// Fixture signing key: 32 bytes of 0x07 (same style as the ADR-020
// [9u8;32] peer-id golden). Recorded in the vector; never production.
const seed = Buffer.alloc(32, 0x07);
const publicKey = ed.getPublicKey(seed);

// Timestamps via toISOString so the vector is byte-identical in shape to
// issueEligibilityLease output (milliseconds always present).
const issuedMs = Date.UTC(2026, 9, 8, 0, 0, 0); // 2026-10-08T00:00:00.000Z
const leaseExpiryMs = Date.UTC(2026, 9, 8, 0, 4, 0); // underlying tracker lease

const fields = {
  peer_id: "12D3KooWSrKnMZUcSxK8G7wmBbXdU8nFEfWGhLu6H8xjn8LmCSJb",
  installation_id: "b58-installation-golden-0001",
  model_profile_id: `msp1:${"ab".repeat(32)}`,
  issued_at: new Date(issuedMs).toISOString(),
  expires_at: new Date(leaseExpiryMs + 60_000).toISOString(), // exactly the grace cap
  lease_expires_at: new Date(leaseExpiryMs).toISOString(),
  can_host: true,
  can_consume: true,
  slots: 2,
  verified_capacity: "gpu_mid",
  audit_epoch: 7,
  challenge_id: `c3a1`.repeat(8),
  nonce: `9f1e`.repeat(16),
};

const json = canonicalJson(fields);
const signature = ed.sign(Buffer.from(json, "utf8"), seed);
const token = `${b64url(Buffer.from(json, "utf8"))}.${b64url(signature)}`;

const doc = {
  name: "lease-hubkey-1",
  description:
    "Cross-language EligibilityLease golden vector (ADR-012; serving gate ADR-026). " +
    "Signed by the fixture key recorded below — NOT the production hub key. " +
    "TS (issueEligibilityLease canonical form) and Rust (EligibilityLease::from_wire + verify) " +
    "must both accept this exact token; see crypto-vectors.test.ts and the lease.rs golden test.",
  signing_seed_hex: seed.toString("hex"),
  signing_public_key_hex: Buffer.from(publicKey).toString("hex"),
  verify_at: "2026-10-08T00:01:00Z",
  fields,
  token,
  // integrity pin for the generator itself
  canonical_json_sha256: createHash("sha256").update(json, "utf8").digest("hex"),
};

const outPath = join(repoRoot, "protocol", "vectors", "lease-hubkey-1.json");
writeFileSync(outPath, JSON.stringify(doc, null, 2) + "\n");
console.log(`wrote ${outPath}`);
console.log(`token = ${token}`);
