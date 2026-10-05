// Request-scoped dependencies for every route: store, clock, hub signing
// keys, rate limiter, and admin/public knobs. The default context is built
// lazily from env; tests override it wholesale with initTrackerContext.

import { getStore, type TrackerStore } from "@/lib/store";
import { publicKeyOf, type KeyPair } from "@/lib/crypto";
import { RateLimiter, DEFAULT_RATE_LIMITS, type RateLimits } from "@/lib/ratelimit";

export interface TrackerContext {
  store: TrackerStore;
  /** Injectable clock (epoch ms). Never call Date.now() directly in routes. */
  now(): number;
  /** Hub Ed25519 signing key (catalog envelope + eligibility leases). */
  keys: KeyPair;
  limiter: RateLimiter;
  limits: RateLimits;
  /** Admin bearer token for X-MSP-Admin (env ADMIN_TOKEN). Null = admin disabled. */
  adminToken: string | null;
  /** Device verification URL returned by /auth/device/start. */
  verifyUrl: string;
}

/** Development-only deterministic seed so local/CI builds can sign catalogs
 *  without secrets. Production sets MSP_HUB_SEED (64 hex chars). */
const DEV_SEED_HEX = "9d61b862b05cbaeba8a3b3a5d8f1b1b7a4c1e2f3a5b6c7d8e9f0a1b2c3d4e5f6";

function keysFromEnv(): KeyPair {
  const seedHex = process.env.MSP_HUB_SEED;
  let seed: Uint8Array;
  if (seedHex && /^[0-9a-f]{64}$/i.test(seedHex)) {
    seed = new Uint8Array(Buffer.from(seedHex, "hex"));
  } else {
    seed = new Uint8Array(Buffer.from(DEV_SEED_HEX, "hex"));
  }
  return { secretKey: seed, publicKey: publicKeyOf(seed) };
}

function envInt(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const value = Number.parseInt(raw, 10);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

function buildDefaultContext(): TrackerContext {
  const now = () => Date.now();
  const store = getStore(now);
  return {
    store,
    now,
    keys: keysFromEnv(),
    limiter: new RateLimiter(now),
    limits: {
      peerPerInstallation: envInt("MSP_RATE_PEER", DEFAULT_RATE_LIMITS.peerPerInstallation),
      unsignedPerIp: envInt("MSP_RATE_UNSIGNED", DEFAULT_RATE_LIMITS.unsignedPerIp),
      enrollPerIp: envInt("MSP_RATE_ENROLL", DEFAULT_RATE_LIMITS.enrollPerIp),
      publicPerIp: envInt("MSP_RATE_PUBLIC", DEFAULT_RATE_LIMITS.publicPerIp),
    },
    adminToken: process.env.ADMIN_TOKEN ?? null,
    verifyUrl: process.env.DEVICE_VERIFY_URL ?? "https://modelswarm.deepflux.space/verify",
  };
}

let ctx: TrackerContext | null = null;

/** Lazily-created default context (env-driven). */
export function getTrackerContext(): TrackerContext {
  if (!ctx) ctx = buildDefaultContext();
  return ctx;
}

/**
 * Replace the context — the test injection point. Any field not provided
 * falls back to a fresh default (a new MemoryStore when DATABASE_URL is
 * unset, so tests never share state with the default context).
 */
export function initTrackerContext(overrides: Partial<TrackerContext> = {}): TrackerContext {
  const base = buildDefaultContext();
  ctx = {
    ...base,
    ...overrides,
    // `now` is a plain function property; spread copies it, but guard anyway.
    now: overrides.now ?? base.now,
  };
  // Keep the store clock coherent with the context clock when only `now` is
  // overridden and the caller did not pass a store.
  if (overrides.now && !overrides.store) {
    ctx.store = getStore(ctx.now);
  }
  return ctx;
}

/** Test helper: fingerprint of the hub public key (hex). */
export function hubPublicKeyHex(c: TrackerContext): string {
  return Buffer.from(c.keys.publicKey).toString("hex");
}
