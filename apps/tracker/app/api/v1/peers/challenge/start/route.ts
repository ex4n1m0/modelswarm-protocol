import { randomId128 } from "@/lib/crypto";
import { buildChallengePrompt } from "@/lib/challenge";
import { jsonError, jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { ChallengeStartSchema } from "@/lib/schemas";
import { CHALLENGE_TTL_MS } from "@/lib/constants";

// POST /api/v1/peers/challenge/start (msp-v1 §3.3): one open challenge per
// lease+profile (idempotent while open). D16 (gate 2026-10-09 / ADR-028 §7):
// the served challengePrompt is NONCE-BOUND — "ModelSwarm readiness
// challenge <challengeId>" with the fresh 128-bit per-instance id — plus the
// profile's pinned greedy-canary digest when one is configured, riding the
// existing wire field (no schema change). Hash-only: the hub never sees the
// canary token stream (content-blind, ADR-001).

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

  // Strict canary mode (D16): when pins are configured, every challengeable
  // profile MUST have a pinned digest — an unpinned profile can no longer
  // earn eligibility leases at all (fail closed; the pin source is
  // owner-maintained env until a signed catalog field lands via its own ADR).
  const canaryDigest = ctx.canaryPins?.get(body.profileId) ?? null;
  if (ctx.canaryPins !== null && canaryDigest === null) {
    return jsonError(
      403,
      "no_capability",
      "strict canary mode: profile has no pinned greedy-canary digest (MSP_CANARY_PINS)",
    );
  }

  const open = await ctx.store.getOpenChallenge(body.leaseId, body.profileId);
  const challengeId = open?.challengeId ?? randomId128();
  const prompt = buildChallengePrompt(challengeId, canaryDigest);
  if (!open) {
    await ctx.store.createChallenge({
      challengeId,
      leaseId: body.leaseId,
      profileId: body.profileId,
      prompt,
      issuedAt: now,
      deadlineAt: now + CHALLENGE_TTL_MS,
      outcome: "open" as const,
      firstTokenMs: null,
      totalMs: null,
      completedAt: null,
    });
  }

  return jsonOk({
    challengeId,
    challengePrompt: prompt,
    deadlineAt: new Date((open?.deadlineAt ?? now + CHALLENGE_TTL_MS)).toISOString(),
  });
}
