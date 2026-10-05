// Cryptographic primitives for the tracker: canonical JSON (msp-v1 §2.2 /
// ADR-011), SHA-256 digests, Ed25519 signing, base58 (Bitcoin alphabet) for
// installation/peer id derivation, and the manifest-derived ModelProfileId.
//
// The canonicalization is byte-for-byte the same algorithm as
// scripts/validate-vectors.mjs (recursive key sort + JSON.stringify) and is
// cross-checked against the golden vectors in protocol/vectors by the test
// suite (TS / Rust / validator triple parity).

import { createHash } from "node:crypto";
import * as ed from "@noble/ed25519";

// @noble/ed25519 v2 exposes synchronous sign/verify only after a sha512
// implementation is injected. node:crypto provides it; no extra dependency.
ed.etc.sha512Sync = (...messages: Uint8Array[]): Uint8Array =>
  createHash("sha512").update(ed.etc.concatBytes(...messages)).digest();

/** Recursively sort object keys (lexicographic). Arrays keep order. */
function canonicalize(value: unknown): unknown {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(canonicalize);
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value as Record<string, unknown>).sort()) {
    out[key] = canonicalize((value as Record<string, unknown>)[key]);
  }
  return out;
}

/** Canonical JSON per msp-v1 §2.2 / ADR-011 (sorted keys, compact). */
export function canonicalJson(value: unknown): string {
  return JSON.stringify(canonicalize(value));
}

/** Lowercase hex SHA-256 of the UTF-8 encoding of `input`. */
export function sha256Hex(input: string | Uint8Array): string {
  const data = typeof input === "string" ? Buffer.from(input, "utf8") : Buffer.from(input);
  return createHash("sha256").update(data).digest("hex");
}

/** Raw SHA-256 bytes. */
export function sha256(input: string | Uint8Array): Uint8Array {
  const data = typeof input === "string" ? Buffer.from(input, "utf8") : Buffer.from(input);
  return new Uint8Array(createHash("sha256").update(data).digest());
}

/** Body digest wire form: "sha256:<hex>" (empty string digest for GETs). */
export function bodyDigestOf(rawBody: string): string {
  return `sha256:${sha256Hex(rawBody)}`;
}

// ---------------------------------------------------------------------------
// Ed25519

export interface KeyPair {
  secretKey: Uint8Array; // 32-byte seed
  publicKey: Uint8Array; // 32-byte compressed point
}

/** Derive the Ed25519 public key of a 32-byte seed. */
export function publicKeyOf(secretKey: Uint8Array): Uint8Array {
  return ed.getPublicKey(secretKey);
}

/** Ed25519 detached signature (64 bytes), returned as base64 (msp-v1 §2.2). */
export function signBase64(payload: string | Uint8Array, secretKey: Uint8Array): string {
  const data = typeof payload === "string" ? Buffer.from(payload, "utf8") : payload;
  return Buffer.from(ed.sign(data, secretKey)).toString("base64");
}

/** Verify a base64 Ed25519 signature over `payload`. */
export function verifyBase64(
  signatureB64: string,
  payload: string | Uint8Array,
  publicKey: Uint8Array,
): boolean {
  return verifyWithEncoding(signatureB64, "base64", payload, publicKey);
}

/** Verify a base64url Ed25519 signature (eligibility-lease wire form). */
export function verifyBase64Url(
  signatureB64Url: string,
  payload: string | Uint8Array,
  publicKey: Uint8Array,
): boolean {
  return verifyWithEncoding(signatureB64Url, "base64url", payload, publicKey);
}

function verifyWithEncoding(
  signatureText: string,
  encoding: "base64" | "base64url",
  payload: string | Uint8Array,
  publicKey: Uint8Array,
): boolean {
  try {
    const sig = Buffer.from(signatureText, encoding);
    if (sig.length !== 64) return false;
    const data = typeof payload === "string" ? Buffer.from(payload, "utf8") : payload;
    return ed.verify(sig, data, publicKey);
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// base64url (Authorization header + capability/eligibility token wire form)

export function base64UrlEncode(data: Uint8Array | string): string {
  const buf = typeof data === "string" ? Buffer.from(data, "utf8") : Buffer.from(data);
  return buf.toString("base64url");
}

export function base64UrlDecode(text: string): Uint8Array {
  return new Uint8Array(Buffer.from(text, "base64url"));
}

// ---------------------------------------------------------------------------
// base58 (Bitcoin alphabet) — installationId/peerId derivation per msp-v1 §2.1:
// base58(SHA-256(public key)). Pure TS so the TS and Rust sides cannot drift.

const B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const B58_INDEX: Record<string, number> = {};
for (let i = 0; i < B58_ALPHABET.length; i += 1) B58_INDEX[B58_ALPHABET[i]!] = i;

export function bs58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  let num = 0n;
  for (const byte of bytes) num = (num << 8n) | BigInt(byte);
  let out = "";
  while (num > 0n) {
    const rem = num % 58n;
    out = B58_ALPHABET[Number(rem)] + out;
    num /= 58n;
  }
  // Each leading zero byte encodes as a literal "1".
  for (const byte of bytes) {
    if (byte !== 0) break;
    out = `1${out}`;
  }
  return out;
}

export function bs58Decode(text: string): Uint8Array | null {
  if (text.length === 0) return new Uint8Array(0);
  let num = 0n;
  for (const ch of text) {
    const digit = B58_INDEX[ch];
    if (digit === undefined) return null;
    num = num * 58n + BigInt(digit);
  }
  const bytes: number[] = [];
  while (num > 0n) {
    bytes.unshift(Number(num & 0xffn));
    num >>= 8n;
  }
  for (const ch of text) {
    if (ch === "1") bytes.unshift(0);
    else break;
  }
  return new Uint8Array(bytes);
}

/**
 * Derive the id the protocol binds to a raw Ed25519 public key:
 * base58(SHA-256(pubKey)) — used for `installationId` (msp-v1 §2.1).
 */
export function deriveKeyId(publicKey: Uint8Array): string {
  return bs58Encode(sha256(publicKey));
}

/**
 * Derive the wire `peerId` from a raw Ed25519 public key (ADR-020): the
 * libp2p identity-multihash PeerId — base58(0x00 0x24 ‖ protobuf(pubKey))
 * where protobuf(pubKey) is the 36-byte libp2p `PublicKey` proto
 * (0x08 0x01 = Type Ed25519, 0x12 0x20 = Data tag + length 32, then the key).
 * The result is the "12D3Koo…" form (52 chars) — byte-identical to the Rust
 * `modelswarm_identity::peer_id_for` AND to `libp2p::PeerId::from_public_key`
 * (cross-checked by the F11 libp2p-backend test). Registration validates the
 * peerId↔pubKey binding with THIS derivation; the old
 * `peerId == installationId` equality was the Phase B–E placeholder and is
 * gone (hard switch, nothing deployed).
 */
export function derivePeerId(publicKey: Uint8Array): string {
  if (publicKey.length !== 32) return ""; // Ed25519 keys are exactly 32 bytes
  const protobuf = new Uint8Array(4 + publicKey.length);
  protobuf[0] = 0x08; // field 1 (Type) varint tag
  protobuf[1] = 0x01; // Ed25519
  protobuf[2] = 0x12; // field 2 (Data) length-delimited tag
  protobuf[3] = 0x20; // length 32 (the key bytes)
  protobuf.set(publicKey, 4);
  const multihash = new Uint8Array(2 + protobuf.length);
  multihash[0] = 0x00; // identity-multihash code
  multihash[1] = 0x24; // digest length (36 = the protobuf)
  multihash.set(protobuf, 2);
  return bs58Encode(multihash);
}

// ---------------------------------------------------------------------------
// ModelProfileId (ADR-011)

/** "msp1:" + sha256-hex over the canonical JSON of the manifest. */
export function deriveProfileId(manifest: unknown): string {
  return `msp1:${sha256Hex(canonicalJson(manifest))}`;
}

// ---------------------------------------------------------------------------
// Random ids / opaque tokens

import { randomBytes, timingSafeEqual } from "node:crypto";

/** 128-bit random id as 32 lowercase hex chars (leaseId, challengeId, …). */
export function randomId128(): string {
  return randomBytes(16).toString("hex");
}

/** Opaque unguessable session/device token (256-bit, hex). */
export function randomToken(): string {
  return randomBytes(32).toString("hex");
}

/** 8-char human-friendly user code for device verification. */
export function randomUserCode(): string {
  const alphabet = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
  let out = "";
  for (let i = 0; i < 8; i += 1) out += alphabet[randomBytes(1)[0]! % alphabet.length];
  return out;
}

/** Constant-time string equality (admin token comparison). */
export function safeEqual(a: string, b: string): boolean {
  const ha = createHash("sha256").update(a, "utf8").digest();
  const hb = createHash("sha256").update(b, "utf8").digest();
  return timingSafeEqual(ha, hb);
}
