// RelayAdmissionTicket issuance (ADR-038 §3.1/§3.3 — tracker half of the D12
// close). Pins: ticket on every successful heartbeat, the exact signed field
// set + canonical-JSON signature against the hub key, the frozen 600 s TTL,
// fresh nonces, ≤ 1024 B serialized form, and NO ticket on any failure path
// (unauthenticated, wrong installation, stale lease).
//
// Golden-vector discipline: the frozen vectors under protocol/vectors/
// (relay-ticket-1.json, relay-ticket-2-expired.json, relay-ticket-forge-1.json
// — landed by the Protocol seat at af1dc01) are consumed DIRECTLY here, the
// same files the Rust side pins (modelswarm-eligibility relay_ticket_vectors
// tests): this is the TS↔Rust ticket byte-parity gate. The tracker MINTS; the
// strict from_wire parser is the Rust consumer's — the forge mirror below
// re-derives the syntax/signature rows with the tracker's own primitives
// (canonicalJson, splitRelayAdmissionTicket, verifyBase64Url) and asserts the
// same verdicts. relay-admission-frame-1.json is the transport-layer frame
// vector (Network seat's codec) and is deliberately NOT consumed here.

import { beforeEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import * as heartbeatRoute from "@/app/api/v1/peers/heartbeat/route";
import {
  base64UrlEncode,
  canonicalJson,
  publicKeyOf,
  sha256Hex,
  signBase64,
  verifyBase64Url,
} from "@/lib/crypto";
import { issueRelayAdmissionTicket, splitRelayAdmissionTicket } from "@/lib/relay-ticket";
import { RELAY_TICKET_TTL_MS } from "@/lib/constants";
import {
  enroll,
  heartbeat,
  makeRig,
  register,
  seedActiveProfile,
  signedRequest,
  testManifest,
  HUB_SEED,
  type Enrollment,
  type TestRig,
} from "./helpers";

const here = dirname(fileURLToPath(import.meta.url));
const vectorsDir = join(here, "..", "..", "..", "protocol", "vectors");

let rig: TestRig;
let profileA: string;

beforeEach(async () => {
  rig = makeRig();
  profileA = await seedActiveProfile(rig, testManifest());
});

/** Decode + structurally validate one served ticket. */
function decodeTicket(token: string): {
  fields: Record<string, unknown>;
  json: string;
  signature: string;
} {
  const parts = splitRelayAdmissionTicket(token);
  expect(parts).not.toBeNull();
  const fields = JSON.parse(parts!.json) as Record<string, unknown>;
  return { fields, json: parts!.json, signature: parts!.signature };
}

describe("heartbeat mints a relay admission ticket (ADR-038 §3.3)", () => {
  let who: Enrollment;
  let leaseId: string;

  beforeEach(async () => {
    who = await enroll(rig, "198.51.100.41");
    ({ leaseId } = await register(rig, who, [profileA]));
  });

  it("carries relayTicket on a successful heartbeat, bound to server-side rows", async () => {
    rig.advance(5_000);
    const res = await heartbeat(rig, who, leaseId, { profiles: [profileA] });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { relayTicket: string; leaseExpiresAt: string };
    expect(typeof body.relayTicket).toBe("string");
    expect(body.relayTicket.length).toBeGreaterThan(0);

    const { fields, json, signature } = decodeTicket(body.relayTicket);
    // Exact signed field set (T2 content-blind): nothing but the five fields.
    expect(Object.keys(fields).sort()).toEqual(
      ["expires_at", "installation_id", "issued_at", "nonce", "peer_id"],
    );
    // installation_id from the AUTHENTICATED identity, peer_id from the
    // tracker-side registered lease row — never client input.
    expect(fields.installation_id).toBe(who.installationId);
    expect(fields.peer_id).toBe(who.peerId);
    // RFC-3339 millisecond timestamps.
    expect(fields.issued_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    expect(fields.expires_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    // 32 lowercase hex nonce.
    expect(fields.nonce).toMatch(/^[0-9a-f]{32}$/);

    // Canonical JSON (sorted keys) and hub-key Ed25519 signature over it.
    expect(json).toBe(canonicalJson(fields));
    expect(verifyBase64Url(signature, json, publicKeyOf(HUB_SEED))).toBe(true);

    // Frozen serialized-length bound (ADR-038 §3.1: ≤ 1024 bytes).
    expect(Buffer.byteLength(body.relayTicket, "utf8")).toBeLessThanOrEqual(1024);
  });

  it("TTL delta is EXACTLY 600 s and issued_at is the heartbeat clock", async () => {
    rig.advance(7_000);
    const before = rig.ctx.now();
    const res = await heartbeat(rig, who, leaseId);
    expect(res.status).toBe(200);
    const { fields } = decodeTicket(((await res.json()) as { relayTicket: string }).relayTicket);
    expect(Date.parse(fields.issued_at as string)).toBe(before);
    expect(RELAY_TICKET_TTL_MS).toBe(600_000); // the frozen constant itself
    expect(Date.parse(fields.expires_at as string) - Date.parse(fields.issued_at as string))
      .toBe(600_000);
  });

  it("mints a fresh nonce per issuance (one per heartbeat)", async () => {
    const first = decodeTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket);
    rig.advance(15_000); // past envelope timestamp staleness
    const second = decodeTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket);
    expect(first.fields.nonce).not.toBe(second.fields.nonce);
    // Distinct full tickets (fresh signature over the new canonical form).
    const t1 = splitRelayAdmissionTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket)!;
    const t2 = splitRelayAdmissionTicket(((await (await heartbeat(rig, who, leaseId)).json()) as { relayTicket: string }).relayTicket)!;
    expect(t1.signature).not.toBe(t2.signature);
  });
});

describe("no ticket on failure paths (ADR-038 §3.3: success-only issuance)", () => {
  it("unauthenticated request -> 401, no relayTicket anywhere in the body", async () => {
    const res = await heartbeatRoute.POST(
      new Request("http://tracker.local/api/v1/peers/heartbeat", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          leaseId: "0".repeat(32),
          activeProfiles: [],
          freeSlots: 1,
          queueMs: 0,
          draining: false,
        }),
      }),
    );
    expect(res.status).toBe(401);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.relayTicket).toBeUndefined();
  });

  it("another installation's lease -> 401 unauthorized, no ticket", async () => {
    const a = await enroll(rig, "198.51.100.42");
    const b = await enroll(rig, "198.51.100.43");
    const { leaseId } = await register(rig, a, [profileA]);
    const res = await heartbeatRoute.POST(
      signedRequest(
        b.keys,
        b.installationId,
        b.session,
        "/api/v1/peers/heartbeat",
        JSON.stringify({
          leaseId,
          activeProfiles: [],
          freeSlots: 1,
          queueMs: 0,
          draining: false,
        }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(401);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.relayTicket).toBeUndefined();
    expect(body).toEqual({ error: expect.any(Object) });
  });

  it("stale (expired) lease -> 410 lease_expired, no ticket", async () => {
    const who = await enroll(rig, "198.51.100.44");
    const { leaseId } = await register(rig, who, [profileA]);
    rig.advance(80_000); // past the 75 s row-aging TTL
    const res = await heartbeat(rig, who, leaseId);
    expect(res.status).toBe(410);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.relayTicket).toBeUndefined();
    // Unknown lease is the same refusal family.
    const ghost = await heartbeat(rig, who, "f".repeat(32));
    expect(ghost.status).toBe(401);
    expect(((await ghost.json()) as Record<string, unknown>).relayTicket).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Frozen golden vectors (protocol/vectors/, landed af1dc01) — the TS↔Rust
// ticket byte-parity gate. The Rust consumer pins the same files in
// crates/modelswarm-eligibility/tests/relay_ticket_vectors.rs; any drift in
// canonical bytes, signature form, TTL, deadline arithmetic, or the
// syntax/signature forge verdicts fails BOTH suites.

function loadVector(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(join(vectorsDir, name), "utf8")) as Record<string, unknown>;
}

const ACCEPT = loadVector("relay-ticket-1.json");
const EXPIRED = loadVector("relay-ticket-2-expired.json");
const FORGE = loadVector("relay-ticket-forge-1.json");
const ACCEPT_FIELDS = ACCEPT.fields as Record<string, string>;

/** The fixture key's public half (asserted to derive from the seed below). */
const FIXTURE_PUBLIC_KEY = publicKeyOf(
  Buffer.from(ACCEPT.signing_seed_hex as string, "hex"),
);

/** Sign a canonical payload with the recorded fixture key (re-derivation). */
function signWithFixtureSeed(json: string, seedHex: string): string {
  return Buffer.from(signBase64(json, Buffer.from(seedHex, "hex")), "base64").toString("base64url");
}

/** Mirror of the frozen ticket-form invariants (ADR-038 §3.4 checks 3–4):
 *  exact field set, RFC-3339 ms timestamps, 32-lower-hex nonce, the exactly
 *  600 s delta, and the ≤ 1024 B wire bound. Returns the first failing
 *  invariant, or null when the form is valid. Test-only mirror — the strict
 *  from_wire parser is the Rust consumer's; the tracker mints. */
function ticketFormError(fields: Record<string, unknown>, wire: string): string | null {
  const keys = Object.keys(fields).sort();
  if (JSON.stringify(keys) !== JSON.stringify(
    ["expires_at", "installation_id", "issued_at", "nonce", "peer_id"],
  )) return "field_set";
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(String(fields.issued_at))) {
    return "issued_at_format";
  }
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(String(fields.expires_at))) {
    return "expires_at_format";
  }
  if (!/^[0-9a-f]{32}$/.test(String(fields.nonce))) return "nonce";
  const delta = Date.parse(String(fields.expires_at)) - Date.parse(String(fields.issued_at));
  if (delta !== RELAY_TICKET_TTL_MS) return "ttl_delta";
  if (Buffer.byteLength(wire, "utf8") > 1024) return "oversized";
  return null;
}

describe("relay-ticket-1 accept vector (ADR-038 §6.2, TS↔Rust parity)", () => {
  it("the fixture seed derives the recorded public key", () => {
    expect(Buffer.from(FIXTURE_PUBLIC_KEY).toString("hex")).toBe(
      ACCEPT.signing_public_key_hex,
    );
  });

  it("payload is the byte-exact canonical JSON of the recorded fields", () => {
    const parts = splitRelayAdmissionTicket(ACCEPT.token as string)!;
    expect(parts).not.toBeNull();
    expect(parts.json).toBe(ACCEPT.canonical_json);
    expect(parts.json).toBe(canonicalJson(ACCEPT_FIELDS));
    expect(sha256Hex(parts.json)).toBe(ACCEPT.canonical_json_sha256);
    expect(JSON.parse(parts.json)).toEqual(ACCEPT_FIELDS);
  });

  it("signature verifies against the fixture key; flipped byte and wrong key fail", () => {
    const parts = splitRelayAdmissionTicket(ACCEPT.token as string)!;
    expect(verifyBase64Url(parts.signature, parts.json, FIXTURE_PUBLIC_KEY)).toBe(true);
    const flipped = (Number.parseInt(parts.signature[0]!, 36) ^ 1).toString(36) + parts.signature.slice(1);
    expect(verifyBase64Url(flipped, parts.json, FIXTURE_PUBLIC_KEY)).toBe(false);
    // A different valid key (the rig hub key) must also fail.
    expect(verifyBase64Url(parts.signature, parts.json, publicKeyOf(HUB_SEED))).toBe(false);
  });

  it("frozen shape: exact field set, exactly 600 s delta, 32-hex nonce, ≤ 1024 B", () => {
    expect(ticketFormError(ACCEPT_FIELDS, ACCEPT.token as string)).toBeNull();
    expect(RELAY_TICKET_TTL_MS).toBe(600_000);
    // Admission deadline arithmetic (the relay's T4): expires_at + 120 s.
    expect(Date.parse(ACCEPT_FIELDS.expires_at!) + 120_000).toBe(
      Date.parse(ACCEPT.expected_admission_deadline as string),
    );
    // Live at the recorded mid-life verify_at (issued_at − 120 s ≤ now ≤ deadline).
    const now = Date.parse(ACCEPT.verify_at as string);
    expect(now).toBeGreaterThanOrEqual(Date.parse(ACCEPT_FIELDS.issued_at!) - 120_000);
    expect(now).toBeLessThanOrEqual(Date.parse(ACCEPT_FIELDS.expires_at!) + 120_000);
  });

  it("the tracker issuer reproduces the vector's deterministic fields (nonce excepted)", () => {
    const issued = issueRelayAdmissionTicket(
      ACCEPT_FIELDS.installation_id!,
      ACCEPT_FIELDS.peer_id!,
      Date.parse(ACCEPT_FIELDS.issued_at!),
      Buffer.from(ACCEPT.signing_seed_hex as string, "hex"),
    );
    const parts = splitRelayAdmissionTicket(issued.token)!;
    const got = JSON.parse(parts.json) as Record<string, unknown>;
    const expected = { ...ACCEPT_FIELDS };
    delete expected.nonce;
    delete got.nonce;
    expect(got).toEqual(expected);
    expect(verifyBase64Url(parts.signature, parts.json, FIXTURE_PUBLIC_KEY)).toBe(true);
  });

  it("the wire form reconstructs byte-exactly from the frozen primitives", () => {
    // Rebuilding the serialized ticket from canonicalJson + the base64url
    // Ed25519 discipline (exactly what lib/relay-ticket.ts does) must
    // reproduce the recorded vector token — pinning the whole serialization
    // discipline, not just its parse.
    const json = canonicalJson(ACCEPT_FIELDS);
    const signature = signWithFixtureSeed(json, ACCEPT.signing_seed_hex as string);
    expect(`${base64UrlEncode(json)}.${signature}`).toBe(ACCEPT.token);
  });
});

describe("relay-ticket-2-expired vector (expiry, not signature, is the failure)", () => {
  it("same token; signature STILL verifies at the late verify_at", () => {
    expect(EXPIRED.token).toBe(ACCEPT.token);
    const parts = splitRelayAdmissionTicket(EXPIRED.token as string)!;
    expect(verifyBase64Url(parts.signature, parts.json, FIXTURE_PUBLIC_KEY)).toBe(true);
  });

  it("the ±120 s skew rule expires it: verify_at past expires_at + grace", () => {
    expect(EXPIRED.skew_grace_secs).toBe(120);
    const now = Date.parse(EXPIRED.verify_at as string);
    const deadline = Date.parse(EXPIRED.expires_at as string) + 120_000;
    expect(now).toBeGreaterThan(deadline); // → permission_denied(202) / expired
    // ...while the mid-life anchor of relay-ticket-1 stays inside the window.
    expect(Date.parse(ACCEPT.verify_at as string)).toBeLessThanOrEqual(
      Date.parse(ACCEPT_FIELDS.expires_at!) + 120_000,
    );
  });
});

describe("relay-ticket-forge-1 corpus (tracker-side mirror of the §3.4 verdicts)", () => {
  // The relay-context rows (peer_mismatch, installation_mismatch) need an
  // authenticated connection — they are the relay's checks, not the tracker's;
  // here we mirror only what the ticket FORM and SIGNATURE let any consumer
  // decide, re-deriving each row from the accept vector exactly as the Rust
  // forge test does. relay-admission-frame-1.json (the protobuf frames) is
  // the transport seat's vector and is intentionally not consumed here.
  const token = ACCEPT.token as string;
  const parts = splitRelayAdmissionTicket(token)!;
  const canonical = parts.json;
  const seedHex = ACCEPT.signing_seed_hex as string;
  const otherSeedHex = "39".repeat(32);
  const remote = FORGE.authenticated_remote_peer_id as string;

  it("corpus shape: rows carry the frozen status+reason pairs", () => {
    const rows = FORGE.rows as Array<{ id: string; expected_status: string; expected_reason: string }>;
    expect(rows.length).toBe(12);
    for (const row of rows) {
      expect(row.expected_status).toMatch(/^(malformed_message\(400\)|permission_denied\(202\))$/);
      expect(row.expected_reason).toMatch(/^(malformed|bad_signature|peer_mismatch|installation_mismatch)$/);
    }
  });

  it("bad_signature: a flipped signature-segment char fails verification", () => {
    const sigStart = token.lastIndexOf(".") + 1;
    const first = token[sigStart]!;
    const flipped = token.slice(0, sigStart) + (first === "A" ? "B" : "A") + token.slice(sigStart + 1);
    const fParts = splitRelayAdmissionTicket(flipped)!;
    expect(verifyBase64Url(fParts.signature, fParts.json, FIXTURE_PUBLIC_KEY)).toBe(false);
  });

  it("wrong_key: the same payload re-signed by a different key fails", () => {
    const forged = `${base64UrlEncode(canonical)}.${signWithFixtureSeed(canonical, otherSeedHex)}`;
    const fParts = splitRelayAdmissionTicket(forged)!;
    expect(verifyBase64Url(fParts.signature, fParts.json, FIXTURE_PUBLIC_KEY)).toBe(false);
  });

  it("binding rows stay relay-side; the mutated tickets are internally valid (order note)", () => {
    // peer_mismatch: a VALIDLY-SIGNED ticket for peer B is well-formed and
    // verifies — only the relay's connection binding refuses it. The order
    // note (signature precedes binding) is mirrored by the both-bad case:
    // wrong key + wrong peer reports bad_signature, because verification
    // runs before any binding check could.
    const otherPeer = { ...ACCEPT_FIELDS, peer_id: "12D3KooWErVrN9uT2YwALa5kSMkE2h6W8YmQz3Xp7F1cBdGtRv" };
    const validForB = `${base64UrlEncode(canonicalJson(otherPeer))}.${signWithFixtureSeed(canonicalJson(otherPeer), seedHex)}`;
    const vParts = splitRelayAdmissionTicket(validForB)!;
    expect(verifyBase64Url(vParts.signature, vParts.json, FIXTURE_PUBLIC_KEY)).toBe(true);
    expect(ticketFormError(otherPeer, validForB)).toBeNull();
    expect(otherPeer.peer_id).not.toBe(remote); // the relay-side mismatch fact

    const bothBad = `${base64UrlEncode(canonicalJson(otherPeer))}.${signWithFixtureSeed(canonicalJson(otherPeer), otherSeedHex)}`;
    const bParts = splitRelayAdmissionTicket(bothBad)!;
    expect(verifyBase64Url(bParts.signature, bParts.json, FIXTURE_PUBLIC_KEY)).toBe(false);
  });

  it("unknown_field: a strict field-set parse refuses extra keys", () => {
    const extra = JSON.parse(canonical) as Record<string, unknown>;
    extra.extra_key = 1;
    expect(ticketFormError(extra, token)).toBe("field_set");
    // A strict consumer's signed-form round-trip also breaks: the payload no
    // longer re-canonicalizes to the frozen signature preimage.
    expect(canonicalJson(extra)).not.toBe(canonical);
  });

  it("missing_field: dropping nonce breaks the exact field set", () => {
    const missing = { ...ACCEPT_FIELDS } as Record<string, unknown>;
    delete missing.nonce;
    expect(ticketFormError(missing, token)).toBe("field_set");
  });

  it("non_canonical_segment: valid JSON, wrong key order fails canonicality", () => {
    // JSON.stringify of the vector's own field order (installation_id first)
    // is a valid but NON-canonical encoding — a strict consumer requires
    // payload === canonicalJson(JSON.parse(payload)).
    const reordered = JSON.stringify(ACCEPT_FIELDS);
    expect(reordered).not.toBe(canonicalJson(JSON.parse(reordered)));
    expect(canonicalJson(ACCEPT_FIELDS)).toBe(canonical);
  });

  it("wrong_ttl_delta: 601 s validly signed — signature ok, delta wrong", () => {
    const ttlBad = { ...ACCEPT_FIELDS, expires_at: "2026-10-15T12:10:01.000Z" };
    const json = canonicalJson(ttlBad);
    const wire = `${base64UrlEncode(json)}.${signWithFixtureSeed(json, seedHex)}`;
    const wParts = splitRelayAdmissionTicket(wire)!;
    expect(verifyBase64Url(wParts.signature, wParts.json, FIXTURE_PUBLIC_KEY)).toBe(true);
    expect(ticketFormError(ttlBad, wire)).toBe("ttl_delta");
  });

  it("nonce_wrong_charset: uppercase and 31-char nonces fail, signature ok", () => {
    for (const nonce of ["A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6", "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d"]) {
      const bad = { ...ACCEPT_FIELDS, nonce };
      const json = canonicalJson(bad);
      const wire = `${base64UrlEncode(json)}.${signWithFixtureSeed(json, seedHex)}`;
      const wParts = splitRelayAdmissionTicket(wire)!;
      expect(verifyBase64Url(wParts.signature, wParts.json, FIXTURE_PUBLIC_KEY)).toBe(true);
      expect(ticketFormError(bad, wire)).toBe("nonce");
    }
  });

  it("bad_timestamp: a plain-date issued_at fails the RFC-3339 form", () => {
    const tsBad = { ...ACCEPT_FIELDS, issued_at: "2026-10-15" };
    expect(ticketFormError(tsBad, token)).toBe("issued_at_format");
  });

  it("single_segment: no '.' separator does not even split", () => {
    expect(splitRelayAdmissionTicket(token.replace(".", ""))).toBeNull();
  });

  it("oversized_ticket: > 1024 B wire is refused before anything else", () => {
    const huge = `${"A".repeat(1030)}.${parts.signature}`;
    expect(Buffer.byteLength(huge, "utf8")).toBeGreaterThan(1024);
    expect(ticketFormError(JSON.parse(canonical), huge)).toBe("oversized");
  });
});
