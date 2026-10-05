import { jsonError, jsonOk } from "@/lib/errors";
import { capacityFromTimings, capacityRank } from "@/lib/eligibility";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { ChallengeCompleteSchema } from "@/lib/schemas";

// POST /api/v1/peers/challenge/complete (msp-v1 §3.3): verifies the deadline,
// marks the challenge passed, stores the zod-validated timings. Issues no
// token by itself — issuance moved to POST /peers/lease (ADR-012).
// verified_capacity derives from measured totalMs (self-reported hardware can
// never raise it) via the deterministic table in lib/eligibility.ts.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, ChallengeCompleteSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;
  const now = ctx.now();

  const lease = await ctx.store.getLease(body.leaseId);
  const binding = leaseBindingError(lease, installationId, now);
  if (binding || !lease) return binding ?? jsonOk({});

  const challenge = await ctx.store.getChallenge(body.challengeId);
  if (!challenge || challenge.leaseId !== body.leaseId || challenge.profileId !== body.profileId) {
    return jsonError(400, "invalid_body", "unknown challengeId for this lease+profile");
  }
  if (challenge.outcome !== "open") {
    return jsonError(403, "forbidden", "challenge already finalized");
  }
  if (now > challenge.deadlineAt) {
    return jsonError(403, "forbidden", "challenge deadline exceeded");
  }

  await ctx.store.passChallenge(
    body.challengeId,
    body.timings.firstTokenMs,
    body.timings.totalMs,
    now,
  );

  const capacityClass = capacityFromTimings(body.timings.totalMs);
  if (capacityRank(capacityClass) > capacityRank(lease.capacityClass)) {
    await ctx.store.updateLease(body.leaseId, { capacityClass });
  }

  return jsonOk({ passed: true, challengeId: body.challengeId, capacityClass });
}
