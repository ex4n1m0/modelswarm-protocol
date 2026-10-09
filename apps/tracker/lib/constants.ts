// Frozen timing/size constants (msp-v1 §2–§5). All expiry is row-aging
// against these values plus the injected clock — never a timer.

/** Peer lease TTL; must stay within the frozen 60–90 s band (msp-v1 §3.3). */
export const LEASE_TTL_MS = 75_000;

/** Capability/eligibility-lease grace over the issuing lease (msp-v1 §5). */
export const TOKEN_GRACE_MS = 60_000;

/** Hosting-challenge completion deadline. */
export const CHALLENGE_TTL_MS = 120_000;

/** X-MSP-Session lifetime (msp-v1 §2.3). */
export const SESSION_TTL_MS = 24 * 60 * 60 * 1000;

/** Device enrollment code lifetime. */
export const DEVICE_CODE_TTL_MS = 15 * 60 * 1000;

/** Cooperative session-authorization validity (Phase B metadata endpoint). */
export const SESSION_AUTH_TTL_MS = 10 * 60 * 1000;

/**
 * Hosting-challenge prompt PREFIX. The served value is nonce-bound per
 * challenge instance — `"ModelSwarm readiness challenge <challengeId>"`,
 * optionally `"; greedy-canary sha256:<64 hex>"` when the profile has a
 * pinned possession digest (D16, gate 2026-10-09 / ADR-028 §7; construction
 * frozen in lib/challenge.ts, byte-pinned by
 * protocol/vectors/challenge-prompt-1.json). Deliberately NOT a stored
 * column, so no schema field is prompt-shaped.
 */

// -- write-time retention (T5; row-aging only, no timers — ADR-001) -------
// Write-only/high-churn tables prune themselves at write time using the
// rendezvous-mailbox pattern (bounded + TTL); see lib/store.ts.

/** peer_observations: newest rows kept per peer (240 ≈ 2 h at 30 s cadence). */
export const OBSERVATION_MAX_PER_PEER = 240;
/** peer_observations: hard age cutoff. */
export const OBSERVATION_TTL_MS = 7 * 24 * 60 * 60 * 1000;
/** device_codes / capability_tokens: rows linger this long past expiry
 *  (debugging grace) before write-time pruning. */
export const EXPIRED_ROW_GRACE_MS = 24 * 60 * 60 * 1000;
