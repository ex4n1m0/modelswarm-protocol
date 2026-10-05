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
 * Fixed hosting-challenge prompt. Per-profile constant served from code —
 * deliberately NOT a stored column, so no schema field is prompt-shaped.
 */
export const CHALLENGE_PROMPT = "ModelSwarm readiness challenge";
