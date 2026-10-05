// Phase B endpoint coverage: session-authorize, receipt, audit, rendezvous,
// job-result, notices, audit-epoch ineligibility.

import { beforeEach, describe, expect, it } from "vitest";
import * as sessionAuthorize from "@/app/api/v1/session-authorize/route";
import * as receiptRoute from "@/app/api/v1/receipt/route";
import * as jobResult from "@/app/api/v1/events/job-result/route";
import * as auditRoute from "@/app/api/v1/audit/route";
import * as offerRoute from "@/app/api/v1/rendezvous/offer/route";
import * as answerRoute from "@/app/api/v1/rendezvous/answer/route";
import * as pendingRoute from "@/app/api/v1/rendezvous/pending/route";
import * as peersLeaseRoute from "@/app/api/v1/peers/lease/route";
import * as promoteRoute from "@/app/api/v1/admin/catalog/promote/route";
import { deriveProfileId } from "@/lib/crypto";
import {
  enroll,
  heartbeat,
  makeRig,
  passChallenge,
  register,
  requestEligibilityLease,
  seedActiveProfile,
  signedRequest,
  testManifest,
} from "./helpers";

let rig: ReturnType<typeof makeRig>;
let profileId: string;

beforeEach(async () => {
  rig = makeRig({ adminToken: "test-admin-token" });
  profileId = await seedActiveProfile(rig, testManifest());
});

describe("POST /api/v1/session-authorize", () => {
  it("validates roster leases and returns metadata only", async () => {
    const a = await enroll(rig, "198.51.100.50");
    const b = await enroll(rig, "198.51.100.51");
    await register(rig, a, [profileId]);
    await register(rig, b, [profileId]);

    const res = await sessionAuthorize.POST(
      signedRequest(
        a.keys,
        a.installationId,
        a.session,
        "/api/v1/session-authorize",
        JSON.stringify({ peer_ids: [a.peerId, b.peerId], profile_id: profileId, mode: "speculative" }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(201);
    const body = (await res.json()) as {
      session_id: string;
      peer_ids: string[];
      profile_id: string;
      mode: string;
    };
    expect(body).toEqual({
      session_id: expect.stringMatching(/^[0-9a-f]{32}$/),
      peer_ids: [a.peerId, b.peerId],
      profile_id: profileId,
      mode: "speculative",
    });
    // no prompt-shaped fields exist in the response
    expect(Object.keys(body).sort()).toEqual(["mode", "peer_ids", "profile_id", "session_id"]);
  });

  it("roster member without an active lease -> 403 ineligible", async () => {
    const a = await enroll(rig, "198.51.100.52");
    const ghost = await enroll(rig, "198.51.100.53"); // enrolled but never registered
    await register(rig, a, [profileId]);
    const res = await sessionAuthorize.POST(
      signedRequest(
        a.keys,
        a.installationId,
        a.session,
        "/api/v1/session-authorize",
        JSON.stringify({ peer_ids: [ghost.peerId], profile_id: profileId, mode: "single" }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("ineligible");
  });

  it("unknown profile -> 400 unknown_profile", async () => {
    const a = await enroll(rig, "198.51.100.54");
    await register(rig, a, [profileId]);
    const res = await sessionAuthorize.POST(
      signedRequest(
        a.keys,
        a.installationId,
        a.session,
        "/api/v1/session-authorize",
        JSON.stringify({ peer_ids: [a.peerId], profile_id: "msp1:" + "5".repeat(64), mode: "single" }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unknown_profile");
  });
});

describe("POST /api/v1/receipt + /api/v1/events/job-result", () => {
  it("store digest + outcome only; both endpoints behave identically", async () => {
    const who = await enroll(rig, "198.51.100.55");
    await register(rig, who, [profileId]);
    const digest = "a".repeat(64);

    const res = await receiptRoute.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/receipt",
        JSON.stringify({ receiptDigest: digest, outcome: "stop" }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ recorded: true });

    const digest2 = "b".repeat(64);
    const res2 = await jobResult.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/events/job-result",
        JSON.stringify({ receiptDigest: digest2, outcome: "length" }),
        rig.ctx.now,
      ),
    );
    expect(res2.status).toBe(200);
    expect(await res2.json()).toEqual({ recorded: true });

    const stored = await rig.store.getReceipt(digest);
    expect(stored).toMatchObject({ receiptDigest: digest, outcome: "stop", installationId: who.installationId });
  });
});

describe("POST /api/v1/audit", () => {
  it("admin bumps audit epochs; stale-epoch lease becomes ineligible", async () => {
    const who = await enroll(rig, "198.51.100.56");
    const { leaseId } = await register(rig, who, [profileId]);
    await passChallenge(rig, who, leaseId, profileId);

    // issuance works pre-sweep
    const ok = await requestEligibilityLease(rig, who, leaseId, profileId);
    expect(ok.status).toBe(200);

    const auditRes = await auditRoute.POST(
      new Request("http://tracker.local/api/v1/audit", {
        method: "POST",
        headers: { "content-type": "application/json", "x-msp-admin": "test-admin-token" },
        body: JSON.stringify({ peer_ids: [who.peerId] }),
      }),
    );
    expect(auditRes.status).toBe(200);
    const audit = (await auditRes.json()) as { updated: number; peers: Array<{ audit_epoch: number }> };
    expect(audit.updated).toBe(1);
    expect(audit.peers[0]!.audit_epoch).toBe(1);

    // lease pinned to epoch 0 -> refresh rejected without a token lookup
    const stale = await requestEligibilityLease(rig, who, leaseId, profileId);
    expect(stale.status).toBe(403);
    expect(((await stale.json()) as { error: { code: string } }).error.code).toBe("ineligible");

    // re-registering picks up the current epoch; issuance works again
    const re = await register(rig, who, [profileId]);
    await passChallenge(rig, who, re.leaseId, profileId);
    const fresh = await requestEligibilityLease(rig, who, re.leaseId, profileId);
    expect(fresh.status).toBe(200);
  });

  it("non-admin -> 403", async () => {
    const res = await auditRoute.POST(
      new Request("http://tracker.local/api/v1/audit", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ peer_ids: ["SomePeerId1234"] }),
      }),
    );
    expect(res.status).toBe(403);
  });
});

describe("rendezvous mailbox", () => {
  it("offer/answer delivered via pending and deleted on read", async () => {
    const a = await enroll(rig, "198.51.100.57");
    const b = await enroll(rig, "198.51.100.58");
    await register(rig, a, [profileId]);
    await register(rig, b, [profileId]);

    const offerRes = await offerRoute.POST(
      signedRequest(
        a.keys,
        a.installationId,
        a.session,
        "/api/v1/rendezvous/offer",
        JSON.stringify({ toPeerId: b.peerId, offer: "opaque-sdp-offer" }),
        rig.ctx.now,
      ),
    );
    expect(offerRes.status).toBe(200);
    expect(await offerRes.json()).toEqual({ accepted: true });

    const answerRes = await answerRoute.POST(
      signedRequest(
        b.keys,
        b.installationId,
        b.session,
        "/api/v1/rendezvous/answer",
        JSON.stringify({ toPeerId: a.peerId, answer: "opaque-sdp-answer" }),
        rig.ctx.now,
      ),
    );
    expect(answerRes.status).toBe(200);

    const bPending = await pendingRoute.GET(
      signedRequest(b.keys, b.installationId, b.session, "/api/v1/rendezvous/pending", undefined, rig.ctx.now),
    );
    expect(bPending.status).toBe(200);
    const bItems = (await bPending.json()) as { items: Array<Record<string, unknown>> };
    expect(bItems.items).toHaveLength(1);
    expect(bItems.items[0]).toMatchObject({ fromPeerId: a.peerId, offer: "opaque-sdp-offer" });

    // delete-on-read
    const bAgain = (await (await pendingRoute.GET(
      signedRequest(b.keys, b.installationId, b.session, "/api/v1/rendezvous/pending", undefined, rig.ctx.now),
    )).json()) as { items: unknown[] };
    expect(bAgain.items).toHaveLength(0);

    const aPending = (await (await pendingRoute.GET(
      signedRequest(a.keys, a.installationId, a.session, "/api/v1/rendezvous/pending", undefined, rig.ctx.now),
    )).json()) as { items: Array<Record<string, unknown>> };
    expect(aPending.items).toHaveLength(1);
    expect(aPending.items[0]).toMatchObject({ fromPeerId: b.peerId, answer: "opaque-sdp-answer" });
  });

  it("offer without an active lease -> 403 ineligible; oversized payload -> 400", async () => {
    const a = await enroll(rig, "198.51.100.59");
    const b = await enroll(rig, "198.51.100.60");
    // a never registered
    const noLease = await offerRoute.POST(
      signedRequest(
        a.keys,
        a.installationId,
        a.session,
        "/api/v1/rendezvous/offer",
        JSON.stringify({ toPeerId: b.peerId, offer: "x" }),
        rig.ctx.now,
      ),
    );
    expect(noLease.status).toBe(403);

    await register(rig, a, [profileId]);
    const tooBig = await offerRoute.POST(
      signedRequest(
        a.keys,
        a.installationId,
        a.session,
        "/api/v1/rendezvous/offer",
        JSON.stringify({ toPeerId: b.peerId, offer: "o".repeat(4097) }),
        rig.ctx.now,
      ),
    );
    expect(tooBig.status).toBe(400);
    expect(((await tooBig.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });
});

describe("heartbeat notices (revocation propagation)", () => {
  it("blocked peers and catalog promotions surface as drained notices", async () => {
    const who = await enroll(rig, "198.51.100.61");
    const other = await enroll(rig, "198.51.100.62");
    const { leaseId } = await register(rig, who, [profileId]);
    await register(rig, other, [profileId]);

    await rig.store.blockPeer(other.peerId, "test");
    // promotion through the admin route broadcasts catalog_update
    const candidateId = deriveProfileId(testManifest({ hf_revision: "d".repeat(40) }));
    await rig.store.insertProfile({
      profileId: candidateId,
      manifest: testManifest({ hf_revision: "d".repeat(40) }) as never,
      displayName: "Cand2",
      status: "candidate",
      createdAt: rig.ctx.now(),
    });
    const promotedRes = await promoteRoute.POST(
      new Request("http://tracker.local/api/v1/admin/catalog/promote", {
        method: "POST",
        headers: { "content-type": "application/json", "x-msp-admin": "test-admin-token" },
        body: JSON.stringify({ profileId: candidateId }),
      }),
    );
    expect(promotedRes.status).toBe(200);

    const hb = await heartbeat(rig, who, leaseId);
    const body = (await hb.json()) as { notices: Array<Record<string, unknown>> };
    const types = body.notices.map((n) => n.type).sort();
    expect(types).toEqual(["catalog_update", "revoked_peers"]);
    const revoked = body.notices.find((n) => n.type === "revoked_peers") as { peerIds: string[] };
    expect(revoked.peerIds).toEqual([other.peerId]);
    const upd = body.notices.find((n) => n.type === "catalog_update") as { catalogVersion: number };
    expect(upd.catalogVersion).toBeGreaterThan(1);

    // drained: a second heartbeat has nothing left
    const hb2 = await heartbeat(rig, who, leaseId);
    expect(((await hb2.json()) as { notices: unknown[] }).notices).toHaveLength(0);
  });
});

describe("peers/lease route guard order", () => {
  it("expired lease -> 410 before challenge check", async () => {
    const who = await enroll(rig, "198.51.100.63");
    const { leaseId } = await register(rig, who, [profileId]);
    rig.advance(80_000);
    const res = await peersLeaseRoute.POST(
      signedRequest(
        who.keys,
        who.installationId,
        who.session,
        "/api/v1/peers/lease",
        JSON.stringify({ leaseId, profileId }),
        rig.ctx.now,
      ),
    );
    expect(res.status).toBe(410);
  });
});
