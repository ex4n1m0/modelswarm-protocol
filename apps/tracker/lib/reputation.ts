// Reputation chassis v1 (ADR-044): a DERIVED standing grade computed on
// read from the granted ledger + suspension row — never stored, never
// self-reported. The served-throughput measure and thresholds are the
// SAME ones ADR-043 bucketed for fair-queue weight, so a peer's standing
// can never disagree with its queue priority.

import { DEFAULT_FAIR_WEIGHT_T2, DEFAULT_FAIR_WEIGHT_T3 } from "@/lib/constants";
import type { SuspensionRecord } from "@/lib/store";

export type StandingGrade =
  | "member"
  | "contributor"
  | "cornerstone"
  | "recovering"
  | "probation";

export interface Standing {
  grade: StandingGrade;
  /** Receipt-backed decayed served tok/s, rounded for display. */
  served_tok_per_sec: number;
  /** Decayed served token total (7-day half-life). */
  served_tokens_decayed: number;
  /** Persisted cumulative ADR-012 suspension count (0 = never). */
  strikes: number;
}

/** Compute the standing (ADR-044 §2). Pure; all inputs hub-derived. */
export function standingOf(
  servedTokensDecayed: number,
  endpointSecondsDecayed: number,
  suspension: SuspensionRecord | null,
  now: number,
  t2: number = DEFAULT_FAIR_WEIGHT_T2,
  t3: number = DEFAULT_FAIR_WEIGHT_T3,
): Standing {
  const tokPerSec =
    endpointSecondsDecayed > 0 ? servedTokensDecayed / endpointSecondsDecayed : 0;
  const strikes = suspension?.suspensionCount ?? 0;
  const suspended = suspension !== null && suspension.untilAt > now;
  let grade: StandingGrade;
  if (suspended) {
    grade = strikes >= 3 ? "probation" : "recovering";
  } else if (tokPerSec >= t3 && t3 > t2) {
    grade = "cornerstone";
  } else if (tokPerSec >= t2) {
    grade = "contributor";
  } else {
    grade = "member";
  }
  return {
    grade,
    served_tok_per_sec: Math.round(tokPerSec * 10) / 10,
    served_tokens_decayed: Math.round(servedTokensDecayed),
    strikes,
  };
}
