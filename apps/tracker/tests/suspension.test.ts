// ADR-012 suspension wiring (gate D6 / dependency-graph edge 5): the
// deterministic policy table's vectors + the end-to-end lifecycle over the
// real routes — timeout sweeps, invalid-output ticks, peer-keyed
// suspension (re-registration cannot escape it), roster clamping, lease
// refusal with the honest window, notice propagation, and row-aging
// recovery that restores the remembered slot count.

import { beforeEach, describe, expect, it } from "vitest";
import { makeRig, seedActiveProfile, testManifest, enroll, register, heartbeat, listPeers, type TestRig } from "./helpers";
import {
  suspensionDecision,
  SUSPENSION_THRESHOLD,
  SUSPENSION_WINDOW_MS,
  type ChallengeFailureEvent,
} from "@/lib/suspension";
import { signedRequest } from "./helpers";
import { challengeStart as challengeStartRoute, challengeComplete as challengeCompleteRoute, peersLease as peersLeaseRoute } from "./helpers";

let rig: TestRig;
let profileA: string;

beforeEach(async () => {
  rig = makeRig();
  profileA = await seedActiveProfile(rig, testManifest());
});

describe("ADR-012 policy table (deterministic vectors)", () => {
  const T = 1_000_000_000_000;
  const fail = (failureClass: "timeout" | "invalid", at: number): ChallengeFailureEvent => ({ failureClass, at });

  it("below the threshold never suspends", () => {
    for (let n = 0; n < SUSPENSION_THRESHOLD; n++) {
      const failures = Array.from({ length: n }, (_, i) => fail("timeout", T - i * 1000));
      expect(suspensionDecision(failures, { suspensionCount: 0 }, T).suspend).toBe(false);
    }
  });

  it("threshold in the window suspends for the first backoff tier (15 min)", () => {
    const failures = Array.from({ length: SUSPENSION_THRESHOLD }, (_, i) => fail("timeout", T - i * 1000));
    const d = suspensionDecision(failures, { suspensionCount: 0 }, T);
    expect(d.suspend).toBe(true);
    if (d.suspend) {
      expect(d.untilAt).toBe(T + 15 * 60_000);
      expect(d.reason).toBe("challenge_failures");
    }
  });

  it("mixed classes count together and escalate by prior count", () => {
    const failures: ChallengeFailureEvent[] = [
      fail("timeout", T - 60_000),
      fail("invalid", T - 30_000),
      fail("invalid", T - 10_000),
    ];
    const second = suspensionDecision(failures, { suspensionCount: 1 }, T);
    const third = suspensionDecision(failures, { suspensionCount: 2 }, T);
    const capped = suspensionDecision(failures, { suspensionCount: 9 }, T);
    if (second.suspend && third.suspend && capped.suspend) {
      expect(second.untilAt).toBe(T + 60 * 60_000); // 2nd: 1 h
      expect(third.untilAt).toBe(T + 6 * 60 * 60_000); // 3rd: 6 h
      expect(capped.untilAt).toBe(T + 24 * 60 * 60_000); // cap: 24 h
    } else {
      throw new Error("expected all three to suspend");
    }
  });

  it("failures older than the window do not count", () => {
    const stale = Array.from({ length: SUSPENSION_THRESHOLD }, (_, i) =>
      fail("timeout", T - SUSPENSION_WINDOW_MS - 60_000 - i * 1000),
    );
    expect(suspensionDecision(stale, { suspensionCount: 0 }, T).suspend).toBe(false);
  });

  it("future-dated failures are ignored (clock sanity)", () => {
    const future = Array.from({ length: SUSPENSION_THRESHOLD }, (_, i) => fail("timeout", T + 60_000 + i));
    expect(suspensionDecision(future, { suspensionCount: 0 }, T).suspend).toBe(false);
  });
});

describe("ADR-012 suspension lifecycle (routes end-to-end)", () => {
  it("three timed-out challenges suspend; the swarm sees zero slots; recovery restores them", async () => {
    const who = await enroll(rig);
    const { leaseId } = await register(rig, who, [profileA], { maxSlots: 3 });
    let lastBeat: { notices: { type: string; untilAt?: string }[]; suspendedUntilAt: string | null } | null = null;

    // Three challenges that are started and then ignored past their 120 s
    // deadline. The sweep runs on heartbeat (the regular peer touch). The
    // lease TTL is 75 s, so a mid-window heartbeat keeps it alive while
    // the challenge deadline (120 s) passes.
    for (let i = 0; i < SUSPENSION_THRESHOLD; i++) {
      const start = await challengeStartRoute.POST(
        signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/challenge/start", JSON.stringify({ leaseId, profileId: profileA }), rig.ctx.now),
      );
      expect(start.status).toBe(200);
      rig.advance(60_000);
      expect((await heartbeat(rig, who, leaseId)).status).toBe(200); // lease alive
      rig.advance(61_000); // challenge deadline (120 s) now past
      const hb = await heartbeat(rig, who, leaseId);
      expect(hb.status).toBe(200);
      lastBeat = (await hb.json()) as {
        notices: { type: string; untilAt?: string }[];
        suspendedUntilAt: string | null;
      };
    }

    // The third sweep crossed the threshold: THAT heartbeat already
    // carried the suspension notice + the honest window (evaluation runs
    // before the drain in the same request).
    expect(lastBeat!.suspendedUntilAt).not.toBeNull();
    const notice = lastBeat!.notices.find((n) => n.type === "suspended");
    expect(notice).toBeDefined();
    expect(notice!.untilAt).toBe(lastBeat!.suspendedUntilAt);
    const hb2 = await heartbeat(rig, who, leaseId);
    const hb2Body = (await hb2.json()) as { suspendedUntilAt: string | null };
    expect(hb2Body.suspendedUntilAt).toBe(lastBeat!.suspendedUntilAt);

    // The roster reports ZERO free slots no matter what the peer claims.
    const claiming = await heartbeat(rig, who, leaseId, { freeSlots: 3 });
    expect(claiming.status).toBe(200);
    const roster = await listPeers(rig, who, `?profile_id=${profileA}`);
    const rosterBody = (await roster.json()) as { peers: { peerId: string; freeSlots: number }[] };
    expect(rosterBody.peers.find((p) => p.peerId === who.peerId)?.freeSlots).toBe(0);

    // Lease issuance refuses with the honest reason.
    const lease403 = await peersLeaseRoute.POST(
      signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/lease", JSON.stringify({ leaseId, profileId: profileA }), rig.ctx.now),
    );
    expect(lease403.status).toBe(403);
    const errBody = (await lease403.json()) as { error: { code: string; message: string } };
    expect(errBody.error.message).toContain("suspended until");

    // Re-registration cannot escape: a fresh lease is clamped to 0 slots.
    const re = await register(rig, who, [profileA], { maxSlots: 4 });
    const reLease = (await rig.store.getLease(re.leaseId))!;
    expect(reLease.maxSlots).toBe(0);

    // Recovery + redemption: the 15-minute first-tier window ages out,
    // but the STALE failure window re-suspends ESCALATED (count 2 -> 1 h)
    // until the peer redeems with a PASSED challenge newer than its last
    // failure — then full forgiveness: null window, slots restored.
    const windowEnd = Number(new Date(lastBeat!.suspendedUntilAt!).getTime());
    let beat: Response;
    for (let guard = 0; rig.ctx.now() <= windowEnd && guard < 40; guard++) {
      rig.advance(60_000);
      beat = await heartbeat(rig, who, re.leaseId);
      expect(beat.status).toBe(200);
    }
    rig.advance(60_000); // past the window — stale failures still inside 24 h
    beat = await heartbeat(rig, who, re.leaseId);
    expect(beat.status).toBe(200);
    const escalated = (await beat.json()) as { suspendedUntilAt: string | null };
    expect(escalated.suspendedUntilAt).not.toBeNull();
    expect(new Date(escalated.suspendedUntilAt!).getTime()).toBeGreaterThan(
      windowEnd + 30 * 60_000,
    ); // escalated tier

    // Redemption: one honest passed challenge.
    const { passChallenge } = await import("./helpers");
    await passChallenge(rig, who, re.leaseId, profileA);
    beat = await heartbeat(rig, who, re.leaseId);
    expect(beat.status).toBe(200);
    const recBody = (await beat.json()) as { suspendedUntilAt: string | null };
    expect(recBody.suspendedUntilAt).toBeNull();
    const afterLease = (await rig.store.getLease(re.leaseId))!;
    // The REMEMBERED count is what was live at suspension time (3) — the
    // re-registration's claimed 4 never inflated it (untrusted input).
    expect(afterLease.maxSlots).toBe(3);
    const roster2 = await listPeers(rig, who, `?profile_id=${profileA}`);
    const roster2Body = (await roster2.json()) as { peers: { peerId: string; freeSlots: number }[] };
    expect(roster2Body.peers.find((p) => p.peerId === who.peerId)?.freeSlots).toBeGreaterThanOrEqual(0);
  });

  it("an implausible completion is an invalid-class failure (three of them suspend)", async () => {
    const who = await enroll(rig);
    const { leaseId } = await register(rig, who, [profileA]);

    for (let i = 0; i < SUSPENSION_THRESHOLD; i++) {
      rig.advance(1_000); // spacing: fresh rate-limit windows per cycle
      const start = await challengeStartRoute.POST(
        signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/challenge/start", JSON.stringify({ leaseId, profileId: profileA }), rig.ctx.now),
      );
      expect(start.status).toBe(200);
      const { challengeId } = (await start.json()) as { challengeId: string };
      // Invalid timings (totalMs = 0): an 'invalid'-class failure tick.
      const res = await challengeCompleteRoute.POST(
        signedRequest(who.keys, who.installationId, who.session, "/api/v1/peers/challenge/complete", JSON.stringify({ leaseId, profileId: profileA, challengeId, timings: { firstTokenMs: 0, totalMs: 0 } }), rig.ctx.now),
      );
      expect(res.status).toBe(400);
      expect(((await res.json()) as { error: { message: string } }).error.message).toContain("totalMs");
    }

    const failures = await rig.store.recentChallengeFailures(who.peerId, 0, rig.ctx.now());
    expect(failures).toHaveLength(SUSPENSION_THRESHOLD);
    expect(failures.every((f) => f.failureClass === "invalid")).toBe(true);

    const hb = await heartbeat(rig, who, leaseId);
    const body = (await hb.json()) as { suspendedUntilAt: string | null };
    expect(body.suspendedUntilAt).not.toBeNull();
  });
});
