// ADR-012 suspension machinery (gate D6 / dependency-graph edge 5): the
// deterministic policy table + the one-shot evaluation helper the peer
// routes call on touch. Pure decision-making lives here; store mutations
// stay in the store implementations.
//
// Policy (frozen; vectors in tests/suspension.test.ts):
//   * failures counted: trailing 24 h, by PEER across leases (a fresh
//     registration does not reset the clock)
//   * threshold: 3 failures within the window => suspend
//   * backoff window escalates by prior suspension count:
//       1st 15 min · 2nd 1 h · 3rd 6 h · 4th+ 24 h (cap)
//   * recovery: purely row-aging — until_at in the past means eligible
//     again; the next touch restores the remembered slot count
//   * reason string: "challenge_failures" (bounded, no free text)

import type { TrackerContext } from "@/lib/context";

export const SUSPENSION_WINDOW_MS = 24 * 60 * 60 * 1000;
export const SUSPENSION_THRESHOLD = 3;
export const SUSPENSION_BACKOFF_MS = [15 * 60_000, 60 * 60_000, 6 * 60 * 60_000, 24 * 60 * 60_000] as const;
export const SUSPENSION_REASON = "challenge_failures";

export interface ChallengeFailureEvent {
  /** 'timeout' = ignored past deadline; 'invalid' = implausible completion. */
  failureClass: "timeout" | "invalid";
  /** When the failure became final (deadline for timeouts, completion for invalid). */
  at: number;
}

export interface SuspensionHistory {
  /** Prior suspensions for the peer (the one being decided is not counted). */
  suspensionCount: number;
}

export type SuspensionDecision =
  | { suspend: false }
  | { suspend: true; untilAt: number; reason: typeof SUSPENSION_REASON; backoffMs: number };

/** The deterministic policy table (pure; test-vector pinned). */
export function suspensionDecision(
  failures: ChallengeFailureEvent[],
  history: SuspensionHistory,
  now: number,
): SuspensionDecision {
  const inWindow = failures.filter((f) => now - f.at < SUSPENSION_WINDOW_MS && f.at <= now);
  if (inWindow.length < SUSPENSION_THRESHOLD) return { suspend: false };
  const idx = Math.min(Math.max(history.suspensionCount, 0), SUSPENSION_BACKOFF_MS.length - 1);
  const backoffMs = SUSPENSION_BACKOFF_MS[idx]!;
  return { suspend: true, untilAt: now + backoffMs, reason: SUSPENSION_REASON, backoffMs };
}

export interface SuspensionState {
  peerId: string;
  untilAt: number;
  reason: string;
  suspensionCount: number;
  prevMaxSlots: number;
}

/**
 * One peer-touch evaluation (heartbeat / lease / challenge routes):
 * 1. marks this lease's expired-open challenges failed('timeout'),
 * 2. recovers an aged-out suspension if due (restores remembered slots),
 * 3. re-evaluates the policy over the peer's windowed failures and
 *    applies a NEW suspension when the threshold is met (idempotent —
 *    an active suspension is never re-escalated by this path).
 * Returns the live suspension state (null when clear) so callers can
 * clamp their responses.
 */
export async function evaluateSuspension(
  ctx: TrackerContext,
  lease: { leaseId: string; peerId: string; maxSlots: number },
  now: number,
): Promise<SuspensionState | null> {
  // Expired-open challenges on THIS lease become failures (row-aging, no
  // background process).
  await ctx.store.failExpiredChallenges(lease.leaseId, now);

  const since = now - SUSPENSION_WINDOW_MS;
  const failures = await ctx.store.recentChallengeFailures(lease.peerId, since, now);
  const lastFailureAt = failures.reduce((m, f) => Math.max(m, f.at), 0);
  const existing = await ctx.store.getSuspension(lease.peerId);

  // REDEMPTION: a passed challenge newer than the newest failure is the
  // peer proving it serves again — full forgiveness (the row goes, the
  // escalation memory resets, the slots come back). Without this the
  // stale window would re-suspend forever.
  if (lastFailureAt > 0) {
    const lastPassAt = await ctx.store.lastChallengePassAt(lease.peerId);
    if (lastPassAt !== null && lastPassAt > lastFailureAt) {
      await ctx.store.clearSuspension(lease.peerId, now);
      return null;
    }
  }

  // Aged-out suspension: slots come back, the row STAYS as escalation
  // memory (count survives so the next breach escalates) until redemption
  // above or the failure window empties below.
  await ctx.store.recoverSuspensionIfDue(lease.peerId, now);
  const recovered = await ctx.store.getSuspension(lease.peerId);
  if (recovered && recovered.untilAt > now) return recovered;

  // No failures left in the window: the escalation memory ages out with
  // them — a clean slate is a clean slate.
  if (failures.length === 0) {
    if (recovered) await ctx.store.clearSuspension(lease.peerId, now);
    return null;
  }

  const decision = suspensionDecision(
    failures,
    { suspensionCount: recovered?.suspensionCount ?? 0 },
    now,
  );
  if (!decision.suspend) return null;

  // prevMaxSlots: a suspension is never based on the clamped 0 (the
  // register route already clamps); remember the larger of the current
  // lease's slots and any remembered value.
  const prev = Math.max(lease.maxSlots, recovered?.prevMaxSlots ?? 0, 1);
  const state: SuspensionState = {
    peerId: lease.peerId,
    untilAt: decision.untilAt,
    reason: decision.reason,
    suspensionCount: (recovered?.suspensionCount ?? 0) + 1,
    prevMaxSlots: prev,
  };
  await ctx.store.applySuspension(state, now);
  await ctx.store.pushNotice({ leaseId: lease.leaseId }, {
    type: "suspended",
    untilAt: new Date(decision.untilAt).toISOString(),
    reason: decision.reason,
  });
  return state;
}
