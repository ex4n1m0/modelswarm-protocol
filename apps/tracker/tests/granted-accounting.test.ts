// Granted verified-work accounting (ADR-030 §3/§4/§5, Phase B1): the
// /events/job-result intake with a full counter-signed ReceiptV2 — grants
// ONLY on validated evidence, replay-proof, sybil-ring zero — and the
// decayed /accounting read.

import { beforeEach, describe, expect, it } from "vitest";
import * as jobResult from "@/app/api/v1/events/job-result/route";
import * as accounting from "@/app/api/v1/accounting/route";
import { GRANTED_HALF_LIFE_SECS } from "@/lib/constants";
import { cohortDigestOf, receiptDigest, signReceiptRequester, signReceiptServer, type ReceiptV2 } from "@/lib/receipt-v2";
import { enroll, makeRig, register, seedActiveProfile, signedRequest, testManifest, type TestRig } from "./helpers";

let rig: TestRig;
let profileId: string;

beforeEach(async () => {
  rig = makeRig();
  profileId = await seedActiveProfile(rig, testManifest());
});

/** Builds a VALID two-phase receipt engaging the two enrolled rigs. */
async function buildReceipt(
  requester: Awaited<ReturnType<typeof enroll>>,
  server: Awaited<ReturnType<typeof enroll>>,
  extra?: Partial<ReceiptV2>,
): Promise<{ receipt: ReceiptV2; digest: string }> {
  const peers = [
    { peer_id: requester.peerId, installation_id: requester.installationId, role: "requester" },
    { peer_id: server.peerId, installation_id: server.installationId, role: "server" },
  ];
  const receipt: ReceiptV2 = {
    request_id: `req-${Math.random().toString(36).slice(2, 10)}`,
    profile_id: profileId,
    preset: "fast",
    mode_id: "single",
    cohort_digest: cohortDigestOf(peers),
    peers,
    started_at: new Date(rig.ctx.now() - 6000).toISOString(),
    ended_at: new Date(rig.ctx.now()).toISOString(),
    outcome: "completed",
    accepted_tokens: 19,
    verified_tokens: 19,
    endpoint_seconds: 6,
    usage_digest: "sha256:7f3a",
    budgets: { max_tokens: 256, deadline_ms: 30000 },
    server_signature: "",
    requester_signature: "",
    ...extra,
  };
  receipt.server_signature = signReceiptServer(receipt, server.keys.secretKey)!;
  receipt.requester_signature = signReceiptRequester(receipt, requester.keys.secretKey)!;
  const digest = receiptDigest(receipt)!.replace("sha256:", "");
  return { receipt, digest };
}

async function post(who: Awaited<ReturnType<typeof enroll>>, body: unknown) {
  return jobResult.POST(
    signedRequest(
      who.keys,
      who.installationId,
      who.session,
      "/api/v1/events/job-result",
      JSON.stringify(body),
      rig.ctx.now,
    ),
  );
}

describe("job-result intake with ReceiptV2 (ADR-030 §3)", () => {
  it("a valid counter-signed receipt grants the non-requester peers", async () => {
    const requester = await enroll(rig);
    const server = await enroll(rig);
    await register(rig, server, [profileId]);
    const { receipt, digest } = await buildReceipt(requester, server);

    const res = await post(requester, { receiptDigest: digest, outcome: "stop", receipt });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { reason: string; grants: Array<{ peerId: string; endpointSeconds: number }> };
    expect(body.reason).toBe("granted");
    expect(body.grants).toHaveLength(1);
    expect(body.grants[0].peerId).toBe(server.peerId);
    expect(body.grants[0].endpointSeconds).toBe(6);

    // The read surface: raw + decayed (fresh row -> full weight).
    const counters = await rig.store.grantedCounters(server.peerId, GRANTED_HALF_LIFE_SECS);
    expect(counters.rows).toBe(1);
    expect(counters.endpointSecondsRaw).toBe(6);
    expect(counters.endpointSecondsDecayed).toBeCloseTo(6, 5);
  });

  it("replaying the same receipt is a no-op (replay-proof key)", async () => {
    const requester = await enroll(rig);
    const server = await enroll(rig);
    const { receipt, digest } = await buildReceipt(requester, server);
    await post(requester, { receiptDigest: digest, outcome: "stop", receipt });
    await post(requester, { receiptDigest: digest, outcome: "stop", receipt });
    const counters = await rig.store.grantedCounters(server.peerId, GRANTED_HALF_LIFE_SECS);
    expect(counters.rows).toBe(1);
    expect(counters.endpointSecondsRaw).toBe(6);
  });

  it("wrong claimed digest, missing signature, and ring installations grant nothing (honest reasons)", async () => {
    const requester = await enroll(rig);
    const server = await enroll(rig);
    const { receipt, digest } = await buildReceipt(requester, server);

    const wrong = await post(requester, { receiptDigest: "c".repeat(64), outcome: "stop", receipt });
    expect(((await wrong.json()) as { reason: string }).reason).toBe("receipt_not_valid_evidence");

    const unsigned = { ...receipt, requester_signature: "" };
    const unsignedDigest = receiptDigest(unsigned as ReceiptV2)!.replace("sha256:", "");
    const missing = await post(requester, { receiptDigest: unsignedDigest, outcome: "stop", receipt: unsigned });
    expect(((await missing.json()) as { reason: string }).reason).toBe("receipt_not_valid_evidence");

    // A sybil ring: the server installation ALSO plays verifier.
    const ringPeers = [
      ...receipt.peers,
      { peer_id: `${server.peerId}2`, installation_id: server.installationId, role: "verifier" },
    ];
    const ringReceipt = { ...receipt, peers: ringPeers, cohort_digest: cohortDigestOf(ringPeers) };
    ringReceipt.server_signature = signReceiptServer(ringReceipt, server.keys.secretKey)!;
    ringReceipt.requester_signature = signReceiptRequester(ringReceipt, requester.keys.secretKey)!;
    const ringDigest = receiptDigest(ringReceipt)!.replace("sha256:", "");
    const ringed = await post(requester, { receiptDigest: ringDigest, outcome: "stop", receipt: ringReceipt });
    expect(((await ringed.json()) as { reason: string }).reason).toBe("sybil_ring_installations");

    const counters = await rig.store.grantedCounters(server.peerId, GRANTED_HALF_LIFE_SECS);
    expect(counters.rows).toBe(0);
  });

  // ADR-045 S-5 (rev 2, 2026-10-20): intake rejects out-of-vocabulary
  // preset values at the schema layer — the honest invalid_body failure,
  // never a relabel and never a silent strip. "FAST" is the pre-freeze
  // legacy literal this fixture itself used to carry.
  it.each(["turbo", "FAST"])(
    "out-of-vocabulary preset %j is rejected with invalid_body — no relabel, no rows",
    async (bad) => {
      const requester = await enroll(rig);
      const server = await enroll(rig);
      const { receipt, digest } = await buildReceipt(requester, server, { preset: bad });

      const res = await post(requester, { receiptDigest: digest, outcome: "stop", receipt });
      expect(res.status).toBe(400);
      expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");

      // Schema rejection happens before any storage: no digest row, no grants.
      expect(await rig.store.getReceipt(digest)).toBeNull();
      const counters = await rig.store.grantedCounters(server.peerId, GRANTED_HALF_LIFE_SECS);
      expect(counters.rows).toBe(0);
    },
  );

  it("signatures must verify against ENROLLED keys — a third party's key grants nothing", async () => {
    const requester = await enroll(rig);
    const server = await enroll(rig);
    const outsider = await enroll(rig);
    const { receipt, digest } = await buildReceipt(requester, server);
    // Re-sign the server phase with the OUTSIDER's key.
    const forged = { ...receipt, server_signature: signReceiptServer(receipt, outsider.keys.secretKey)! };
    const res = await post(requester, { receiptDigest: digest, outcome: "stop", receipt: forged });
    expect(((await res.json()) as { reason: string }).reason).toBe("receipt_not_valid_evidence");
    const counters = await rig.store.grantedCounters(server.peerId, GRANTED_HALF_LIFE_SECS);
    expect(counters.rows).toBe(0);
  });

  it("decay: a row one half-life old weighs half (ADR-030 §5)", async () => {
    const requester = await enroll(rig);
    const server = await enroll(rig);
    const { receipt, digest } = await buildReceipt(requester, server);
    await post(requester, { receiptDigest: digest, outcome: "stop", receipt });
    // Age the row by exactly one half-life.
    rig.advance(GRANTED_HALF_LIFE_SECS * 1000);
    const counters = await rig.store.grantedCounters(server.peerId, GRANTED_HALF_LIFE_SECS);
    expect(counters.endpointSecondsRaw).toBe(6);
    expect(counters.endpointSecondsDecayed).toBeCloseTo(3, 5);
  });

  it("GET /accounting returns the decayed counters for the asking peer", async () => {
    const requester = await enroll(rig);
    const server = await enroll(rig);
    const { receipt, digest } = await buildReceipt(requester, server);
    await post(requester, { receiptDigest: digest, outcome: "stop", receipt });
    const res = await accounting.GET(
      signedRequest(server.keys, server.installationId, server.session, `/api/v1/accounting?peer_id=${server.peerId}`, undefined, rig.ctx.now),
    );
    expect(res.status).toBe(200);
    const body = (await res.json()) as { endpointSecondsRaw: number; rows: number; halfLifeSecs: number };
    expect(body.rows).toBe(1);
    expect(body.endpointSecondsRaw).toBe(6);
    expect(body.halfLifeSecs).toBe(GRANTED_HALF_LIFE_SECS);
  });
});
