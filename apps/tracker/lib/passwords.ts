// Website-account password hashing: node:crypto scrypt, promisified — no
// new dependencies (constraint: no argon2 / bcrypt / passlib). The stored
// form is self-describing so parameters can evolve without invalidating
// existing rows:
//
//   scrypt$<N>$<r>$<p>$<salt-base64>$<hash-base64>
//
// verifyPassword re-derives with the parameters parsed FROM the stored
// string (never the constants), so a future cost bump only changes what
// hashPassword writes. Comparison is timing-safe.

import { randomBytes, scrypt as scryptCallback, timingSafeEqual } from "node:crypto";
import { promisify } from "node:util";
import type { ScryptOptions } from "node:crypto";

// promisify() loses the typed overloads of scrypt; pin the shape we call.
const scryptAsync = promisify(scryptCallback) as (
  password: string,
  salt: Buffer,
  keylen: number,
  options: ScryptOptions,
) => Promise<Buffer>;

// N=2^16, r=8, p=1 (64 MiB memory — OWASP-aligned for interactive login).
// maxmem must exceed 128*N*r (~64 MiB); 256 MiB leaves generous headroom.
const SCRYPT_N = 65_536;
const SCRYPT_R = 8;
const SCRYPT_P = 1;
const SCRYPT_KEYLEN = 64;
const SCRYPT_SALT_BYTES = 16;
const SCRYPT_MAXMEM = 256 * 1024 * 1024;

interface ParsedStored {
  N: number;
  r: number;
  p: number;
  salt: Buffer;
  hash: Buffer;
}

/** Hash a password into the self-describing stored form. */
export async function hashPassword(password: string): Promise<string> {
  const salt = randomBytes(SCRYPT_SALT_BYTES);
  const derived = await scryptAsync(password, salt, SCRYPT_KEYLEN, {
    N: SCRYPT_N,
    r: SCRYPT_R,
    p: SCRYPT_P,
    maxmem: SCRYPT_MAXMEM,
  });
  return [
    "scrypt",
    SCRYPT_N,
    SCRYPT_R,
    SCRYPT_P,
    salt.toString("base64"),
    derived.toString("base64"),
  ].join("$");
}

/** Split a stored string into its parameters; null when malformed. */
function parseStored(stored: string): ParsedStored | null {
  const parts = stored.split("$");
  if (parts.length !== 6 || parts[0] !== "scrypt") return null;
  const N = Number.parseInt(parts[1]!, 10);
  const r = Number.parseInt(parts[2]!, 10);
  const p = Number.parseInt(parts[3]!, 10);
  if (!Number.isInteger(N) || N <= 0) return null;
  if (!Number.isInteger(r) || r <= 0) return null;
  if (!Number.isInteger(p) || p <= 0) return null;
  if (N * r > 2 ** 32) return null; // absurd cost — refuse rather than hang
  const salt = Buffer.from(parts[4]!, "base64");
  const hash = Buffer.from(parts[5]!, "base64");
  if (salt.length === 0 || hash.length === 0) return null;
  return { N, r, p, salt, hash };
}

/** Verify a password against a stored hash. Malformed stored values are
 *  simply wrong (false), never a thrown error. */
export async function verifyPassword(password: string, stored: string): Promise<boolean> {
  const parsed = parseStored(stored);
  if (!parsed) return false;
  try {
    const derived = await scryptAsync(password, parsed.salt, parsed.hash.length, {
      N: parsed.N,
      r: parsed.r,
      p: parsed.p,
      maxmem: Math.max(SCRYPT_MAXMEM, 128 * parsed.N * parsed.r * 2),
    });
    return derived.length === parsed.hash.length && timingSafeEqual(derived, parsed.hash);
  } catch {
    return false;
  }
}

let dummyHashCache: string | null = null;

/** A well-formed stored hash of an unguessable password. Login verifies
 *  against this when the email is unknown so the failure timing and the
 *  error body are indistinguishable from a wrong password (no user
 *  enumeration). */
export async function dummyPasswordHash(): Promise<string> {
  if (!dummyHashCache) {
    dummyHashCache = await hashPassword(randomBytes(32).toString("hex"));
  }
  return dummyHashCache;
}
