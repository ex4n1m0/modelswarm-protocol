// Shared test scaffolding: deterministic context with an injectable clock,
// keypairs, signed-request builders, and full enrollment/register flows.

import { createHash, randomBytes } from "node:crypto";
import { initTrackerContext, type TrackerContext } from "@/lib/context";
import { MemoryStore } from "@/lib/store";
import { RateLimiter } from "@/lib/ratelimit";
import {
  bs58Encode,
  deriveKeyId,
  derivePeerId,
  publicKeyOf,
  sha256Hex,
  base64UrlDecode,
} from "@/lib/crypto";
import { encodeAuthorization } from "@/lib/envelope";
import type { KeyPair } from "@/lib/crypto";
import { deriveProfileId } from "@/lib/crypto";

export const BASE_TIME = Date.UTC(2026, 10, 4, 12, 0, 0); // 2026-11-04T12:00:00Z
export const HUB_SEED = createHash("sha256").update("test-hub-seed").digest();

export interface TestRig {
  ctx: TrackerContext;
  store: MemoryStore;
  nowMs: number;
  advance(ms: number): void;
  setNow(ms: number): void;
}

export function makeRig(overrides: Partial<TrackerContext> = {}): TestRig {
  let nowMs = BASE_TIME;
  const now = () => nowMs;
  // The store shares the injectable clock so row-aging follows test time.
  const store = new MemoryStore(now);
  const ctx = initTrackerContext({
    store,
    now,
    keys: { secretKey: HUB_SEED, publicKey: publicKeyOf(HUB_SEED) },
    limiter: new RateLimiter(now),
    ...overrides,
  });
  return {
    ctx,
    store,
    get nowMs() {
      return nowMs;
    },
    set nowMs(value: number) {
      nowMs = value;
    },
    advance(ms: number) {
      nowMs += ms;
    },
    setNow(ms: number) {
      nowMs = ms;
    },
  };
}

// ---------------------------------------------------------------------------
// Keypairs / ids

export function makeKeypair(): KeyPair {
  const secretKey = new Uint8Array(randomBytes(32));
  return { secretKey, publicKey: publicKeyOf(secretKey) };
}

/** installationId = base58(sha256(pubKey)); peerId = ADR-020 multihash derivation. */
export function keyPairIds(keys: KeyPair): { installationId: string; peerId: string; pubKeyB58: string } {
  const pubKeyB58 = bs58Encode(keys.publicKey);
  return { installationId: deriveKeyId(keys.publicKey), peerId: derivePeerId(keys.publicKey), pubKeyB58 };
}

// ---------------------------------------------------------------------------
// Signed requests

export function randomNonce(): string {
  return randomBytes(16).toString("hex");
}

export interface EnvelopeOpts {
  method?: string;
  ts?: number; // epoch ms (default: rig now)
  nonce?: string;
  ip?: string;
  tamperBodyDigest?: boolean;
}

export function signedRequest(
  keys: KeyPair,
  installationId: string,
  session: string,
  path: string,
  body: string | undefined,
  now: () => number,
  opts: EnvelopeOpts = {},
): Request {
  const method = opts.method ?? (body === undefined ? "GET" : "POST");
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "x-msp-session": session,
  };
  if (opts.ip) headers["x-forwarded-for"] = opts.ip;
  const signedBody = body === undefined ? "" : body;
  let bodyDigest = `sha256:${sha256Hex(signedBody)}`;
  if (opts.tamperBodyDigest) bodyDigest = `sha256:${sha256Hex(signedBody + "x")}`;
  // The envelope signs the pathname (query strings are not part of the
  // signed payload), matching the server's url.pathname comparison.
  const pathOnly = path.split("?")[0]!;
  const { header } = encodeAuthorization(
    {
      installationId,
      method,
      path: pathOnly,
      ts: new Date(opts.ts ?? now()).toISOString(),
      nonce: opts.nonce ?? randomNonce(),
      bodyDigest,
    },
    keys.secretKey,
  );
  headers.authorization = header;
  const url = `http://tracker.local${path}`;
  if (body === undefined) return new Request(url, { method, headers });
  return new Request(url, { method, headers, body });
}

// ---------------------------------------------------------------------------
// HTTP flow helpers (drive the real route handlers)

import * as healthRoute from "@/app/api/v1/health/route";
import * as deviceStart from "@/app/api/v1/auth/device/start/route";
import * as deviceComplete from "@/app/api/v1/auth/device/complete/route";
import * as registerRoute from "@/app/api/v1/peers/register/route";
import * as heartbeatRoute from "@/app/api/v1/peers/heartbeat/route";
import * as drainRoute from "@/app/api/v1/peers/drain/route";
import * as challengeStart from "@/app/api/v1/peers/challenge/start/route";
import * as challengeComplete from "@/app/api/v1/peers/challenge/complete/route";
import * as peersLease from "@/app/api/v1/peers/lease/route";
import * as peersList from "@/app/api/v1/peers/route";

export { healthRoute, deviceStart, deviceComplete, registerRoute, heartbeatRoute, drainRoute, challengeStart, challengeComplete, peersLease, peersList };

export interface Enrollment {
  keys: KeyPair;
  installationId: string;
  peerId: string;
  pubKeyB58: string;
  session: string;
  sessionExpiresAt: string;
}

/** Full device enrollment: start -> (approve via store hook) -> complete. */
export async function enroll(rig: TestRig, ip?: string): Promise<Enrollment> {
  const keys = makeKeypair();
  const { installationId, pubKeyB58 } = keyPairIds(keys);
  const startRes = await deviceStart.POST(
    new Request("http://tracker.local/api/v1/auth/device/start", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(ip ? { "x-forwarded-for": ip } : {}),
      },
      body: JSON.stringify({ installationId, pubKey: pubKeyB58 }),
    }),
  );
  if (startRes.status !== 200) throw new Error(`device/start failed: ${startRes.status} ${await startRes.text()}`);
  const start = (await startRes.json()) as { deviceCode: string; userCode: string };
  await rig.store.approveDevice(start.deviceCode);
  const completeBody = JSON.stringify({ deviceCode: start.deviceCode });
  const completeReq = signedRequest(
    keys,
    installationId,
    "",
    "/api/v1/auth/device/complete",
    completeBody,
    rig.ctx.now,
    { ip },
  );
  const completeRes = await deviceComplete.POST(completeReq);
  if (completeRes.status !== 200) throw new Error(`device/complete failed: ${completeRes.status} ${await completeRes.text()}`);
  const session = (await completeRes.json()) as { token: string; expiresAt: string };
  return {
    keys,
    installationId,
    peerId: derivePeerId(keys.publicKey),
    pubKeyB58,
    session: session.token,
    sessionExpiresAt: session.expiresAt,
  };
}

export async function register(
  rig: TestRig,
  who: Enrollment,
  profiles: string[],
  opts: { ip?: string; maxSlots?: number } = {},
): Promise<{ leaseId: string; leaseExpiresAt: string }> {
  const body = JSON.stringify({
    peerId: who.peerId,
    addresses: ["/ip4/10.0.0.1/tcp/4001", "/dns/peer.example.com/tcp/4001"],
    profiles,
    maxSlots: opts.maxSlots ?? 2,
    runtime: { name: "test-runtime", build: "b0001" },
  });
  const res = await registerRoute.POST(
    signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/register", body, rig.ctx.now, { ip: opts.ip }),
  );
  if (res.status !== 200) throw new Error(`register failed: ${res.status} ${await res.text()}`);
  return (await res.json()) as { leaseId: string; leaseExpiresAt: string };
}

export async function heartbeat(
  rig: TestRig,
  who: Enrollment,
  leaseId: string,
  opts: { freeSlots?: number; draining?: boolean; profiles?: string[] } = {},
): Promise<Response> {
  const body = JSON.stringify({
    leaseId,
    activeProfiles: opts.profiles ?? [],
    freeSlots: opts.freeSlots ?? 1,
    queueMs: 5,
    draining: opts.draining ?? false,
  });
  return heartbeatRoute.POST(
    signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/heartbeat", body, rig.ctx.now),
  );
}

export async function listPeers(
  rig: TestRig,
  who: Enrollment,
  query = "",
): Promise<Response> {
  return peersList.GET(
    signedRequest(who.keys, who.installationId, who.session, `/api/v1/peers${query}`, undefined, rig.ctx.now),
  );
}

/** Register + pass the hosting challenge for one profile. */
export async function passChallenge(
  rig: TestRig,
  who: Enrollment,
  leaseId: string,
  profileId: string,
  timings = { firstTokenMs: 120, totalMs: 1500 },
): Promise<string> {
  const startRes = await challengeStart.POST(
    signedRequest(
      who.keys,
      who.installationId,
      who.session,
      "/api/v1/peers/challenge/start",
      JSON.stringify({ leaseId, profileId }),
      rig.ctx.now,
    ),
  );
  if (startRes.status !== 200) throw new Error(`challenge/start failed: ${startRes.status} ${await startRes.text()}`);
  const { challengeId } = (await startRes.json()) as { challengeId: string };
  const completeRes = await challengeComplete.POST(
    signedRequest(
      who.keys,
      who.installationId,
      who.session,
      "/api/v1/peers/challenge/complete",
      JSON.stringify({ leaseId, profileId, challengeId, timings }),
      rig.ctx.now,
    ),
  );
  if (completeRes.status !== 200) throw new Error(`challenge/complete failed: ${completeRes.status} ${await completeRes.text()}`);
  return challengeId;
}

export async function requestEligibilityLease(
  rig: TestRig,
  who: Enrollment,
  leaseId: string,
  profileId: string,
): Promise<Response> {
  return peersLease.POST(
    signedRequest(
      who.keys,
      who.installationId,
      who.session,
      "/api/v1/peers/lease",
      JSON.stringify({ leaseId, profileId }),
      rig.ctx.now,
    ),
  );
}

export async function drain(rig: TestRig, who: Enrollment, leaseId: string): Promise<Response> {
  return drainRoute.POST(
    signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/drain", JSON.stringify({ leaseId }), rig.ctx.now),
  );
}

// ---------------------------------------------------------------------------
// Catalog fixtures

/** A schema-v2-valid manifest derived from the golden vector shape. */
export function testManifest(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schema_version: 2,
    hf_repo: "example-org/example-model-gguf",
    hf_revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    artifact_hashes: [
      { path: "model-q4_k_m.gguf", sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
    ],
    tokenizer_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    chat_template_hash: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    architecture_hash: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    quantization: { method: "Q4_K_M", bits: 4 },
    runtime: {
      name: "rt-test",
      version: "b0-test",
      build_hash: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    },
    decoding_abi_version: 1,
    speculative_capabilities: ["proposal_tokens"],
    ...overrides,
  };
}

/** Insert an ACTIVE profile directly through the store (admin seed). */
export async function seedActiveProfile(
  rig: TestRig,
  manifest: Record<string, unknown>,
  displayName = "Test Profile",
): Promise<string> {
  const profileId = deriveProfileId(manifest);
  await rig.store.insertProfile({
    profileId,
    manifest: manifest as never,
    displayName,
    status: "active",
    createdAt: rig.ctx.now(),
  });
  return profileId;
}

export function decodeB64Url(text: string): string {
  return Buffer.from(base64UrlDecode(text)).toString("utf8");
}
