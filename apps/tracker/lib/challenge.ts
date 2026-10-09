// Hosting-challenge prompt construction + parsing (D16, owner gate
// 2026-10-09 / ADR-028 §7). The wire surface is UNCHANGED: the built value
// rides the existing `challengePrompt` response field of
// POST /peers/challenge/start (msp-v1 §3.3) — no new fields, and the frozen
// ChallengeCompleteSchema body is untouched.
//
// Format (byte-pinned by protocol/vectors/challenge-prompt-1.json, consumed
// by this suite and by modelswarm-tracker-api):
//
//   base:      "ModelSwarm readiness challenge <challengeId>"
//   with pin:  "ModelSwarm readiness challenge <challengeId>; greedy-canary sha256:<64 hex>"
//
// Properties:
// - Nonce-bound: the challengeId is a fresh 128-bit id per lease+profile, so
//   a solved prompt/timing pair cannot be precomputed against a fixed public
//   constant or replayed across challenges/installations (audit T1/§4-B).
// - Possession digests are HASH-ONLY (content-blind, ADR-001): the pinned
//   value is the SHA-256 of the greedy canary token stream produced offline
//   by the resolver pipeline under the pinned runtime (E0 determinism:
//   greedy decoding of a fixed prompt is stable across GPU vendors and
//   thread counts, so every honest host of the exact ModelProfileId
//   reproduces it). The hub never sees prompts or token streams — it
//   embeds and serves digests only.
// - The hub cannot observe the peer's engine: timings at challenge/complete
//   remain self-attested telemetry (D7 relabel), and digest-based
//   spot-verification by other peers is deferred to the reputation chassis
//   (audit §4-D) with its sampling rate sized from measured detection math
//   (Sarmenta 1-(1-s)^n; TBD-numeric until the E-A detection curves exist).

const PROMPT_PREFIX = "ModelSwarm readiness challenge ";
const CANARY_SUFFIX_PREFIX = "; greedy-canary ";

const CHALLENGE_ID_RE = /^[0-9a-f]{32}$/;
const CANARY_DIGEST_RE = /^sha256:[0-9a-f]{64}$/;

/** Canary pin map from `MSP_CANARY_PINS` (JSON: profileId -> "sha256:<64 hex>").
 *  Null when the env is unset (v0 legacy posture: nonce-bound prompt only).
 *  Malformed values fail closed at context build — strict mode is never
 *  silently downgraded. */
export function parseCanaryPins(raw: string | undefined): ReadonlyMap<string, string> | null {
  if (raw === undefined || raw.trim() === "") return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error("MSP_CANARY_PINS is set but not valid JSON (want {\"<profileId>\": \"sha256:<64 hex>\"})");
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("MSP_CANARY_PINS must be a JSON object of profileId -> sha256 digest");
  }
  const entries = Object.entries(parsed as Record<string, unknown>);
  if (entries.length > 256) {
    throw new Error("MSP_CANARY_PINS exceeds 256 entries");
  }
  const pins = new Map<string, string>();
  for (const [profileId, digest] of entries) {
    if (!/^msp1:[0-9a-f]{64}$/.test(profileId)) {
      throw new Error(`MSP_CANARY_PINS key is not a ModelProfileId: ${profileId.slice(0, 12)}…`);
    }
    if (typeof digest !== "string" || !CANARY_DIGEST_RE.test(digest)) {
      throw new Error(`MSP_CANARY_PINS[${profileId.slice(0, 12)}…] is not "sha256:<64 hex>"`);
    }
    pins.set(profileId, digest);
  }
  return pins;
}

/** Build the served `challengePrompt` value. `canaryDigest` is the pinned
 *  greedy-canary digest for the exact profile (or null in v0 legacy mode). */
export function buildChallengePrompt(challengeId: string, canaryDigest: string | null): string {
  if (!CHALLENGE_ID_RE.test(challengeId)) {
    throw new Error("challengeId must be 32 lowercase hex chars");
  }
  if (canaryDigest !== null && !CANARY_DIGEST_RE.test(canaryDigest)) {
    throw new Error("canaryDigest must be \"sha256:<64 hex>\"");
  }
  const base = PROMPT_PREFIX + challengeId;
  return canaryDigest === null ? base : base + CANARY_SUFFIX_PREFIX + canaryDigest;
}

export interface ParsedChallengePrompt {
  challengeId: string;
  canaryDigest: string | null;
}

/** Parse a served `challengePrompt` back into (challengeId, canary digest).
 *  Client-side hook: the node/desktop extracts the nonce and, when present,
 *  self-verifies its local greedy canary output against the pinned digest
 *  before reporting timings. Returns null on any deviation from the frozen
 *  format. */
export function parseChallengePrompt(prompt: string): ParsedChallengePrompt | null {
  if (!prompt.startsWith(PROMPT_PREFIX)) return null;
  const rest = prompt.slice(PROMPT_PREFIX.length);
  const semicolon = rest.indexOf(CANARY_SUFFIX_PREFIX);
  const idPart = semicolon === -1 ? rest : rest.slice(0, semicolon);
  if (!CHALLENGE_ID_RE.test(idPart)) return null;
  if (semicolon === -1) return { challengeId: idPart, canaryDigest: null };
  const digest = rest.slice(semicolon + CANARY_SUFFIX_PREFIX.length);
  if (!CANARY_DIGEST_RE.test(digest)) return null;
  return { challengeId: idPart, canaryDigest: digest };
}
