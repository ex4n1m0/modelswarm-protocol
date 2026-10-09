import { jsonError, jsonOk } from "@/lib/errors";
import { capacityFromTimings, capacityRank } from "@/lib/eligibility";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { ChallengeCompleteSchema } from "@/lib/schemas";

// POST /api/v1/peers/challenge/complete (msp-v1 §3.3): verifies the deadline,
// applies timing-plausibility checks, marks the challenge passed, stores the
// zod-validated timings. Issues no token by itself — issuance moved to
// POST /peers/lease (ADR-012).
//
// D7 HONEST LABEL (owner gate 2026-10-09 / ADR-028 §7): the timings are
// SELF-REPORTED telemetry. The hub cannot observe the challenger's engine,
// so `capacityClass` (served here and in GET /peers) and the lease field
// `verified_capacity` are labels derived from self-attested numbers — used
// for roster ranking, NOT hub-verified measurements. The field names are
// frozen wire shapes (byte-pinned by protocol/vectors/lease-hubkey-1.json);
// the honesty lives in this label, the docs, and the D16 nonce-bound prompt
// that at least kills precomputed/replayed timing pairs. Digest-based
// possession spot-verification is deferred to the reputation chassis
// (Sarmenta sampling; see lib/challenge.ts).

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

  // Plausibility (audit T1/§4-A): a zero-duration completion or a first
  // token after the total is not credible telemetry. These reject obvious
  // fabrications; they do NOT make the numbers verified (see D7 note above).
  const { firstTokenMs, totalMs } = body.timings;
  if (totalMs <= 0) {
    return jsonError(400, "invalid_body", "timings.totalMs must be > 0");
  }
  if (firstTokenMs > totalMs) {
    return jsonError(400, "invalid_body", "timings.firstTokenMs must be <= timings.totalMs");
  }

  await ctx.store.passChallenge(body.challengeId, firstTokenMs, totalMs, now);

  // Self-reported capacity label (D7): the deterministic table maps the
  // challenger's OWN number to a class; only an upgrade over the lease's
  // current label is persisted (self-reported hardware never raises it
  // beyond what its own timings claim — ADR-012).
  const capacityClass = capacityFromTimings(totalMs);
  if (capacityRank(capacityClass) > capacityRank(lease.capacityClass)) {
    await ctx.store.updateLease(body.leaseId, { capacityClass });
  }

  return jsonOk({ passed: true, challengeId: body.challengeId, capacityClass });
}
