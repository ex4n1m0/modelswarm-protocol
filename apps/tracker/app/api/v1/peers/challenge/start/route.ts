import { randomId128 } from "@/lib/crypto";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { ChallengeStartSchema } from "@/lib/schemas";
import { CHALLENGE_PROMPT, CHALLENGE_TTL_MS } from "@/lib/constants";

// POST /api/v1/peers/challenge/start (msp-v1 §3.3): one open challenge per
// lease+profile (idempotent while open). The prompt is the fixed protocol
// constant; only the id and deadline are server state.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, ChallengeStartSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;
  const now = ctx.now();

  const lease = await ctx.store.getLease(body.leaseId);
  const binding = leaseBindingError(lease, installationId, now);
  if (binding) return binding;

  const profile = await ctx.store.getProfile(body.profileId);
  if (!profile || profile.status !== "active") {
    return jsonError(400, "unknown_profile", `profile ${body.profileId} is not in the active catalog`);
  }

  const open = await ctx.store.getOpenChallenge(body.leaseId, body.profileId);
  const challenge =
    open ??
    {
      challengeId: randomId128(),
      leaseId: body.leaseId,
      profileId: body.profileId,
      prompt: CHALLENGE_PROMPT,
      issuedAt: now,
      deadlineAt: now + CHALLENGE_TTL_MS,
      outcome: "open" as const,
      firstTokenMs: null,
      totalMs: null,
      completedAt: null,
    };
  if (!open) await ctx.store.createChallenge(challenge);

  return jsonOk({
    challengeId: challenge.challengeId,
    challengePrompt: CHALLENGE_PROMPT,
    deadlineAt: new Date(challenge.deadlineAt).toISOString(),
  });
}
