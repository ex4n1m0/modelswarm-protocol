// EligibilityLease (ADR-012) construction + verified-capacity policy.

import {
  base64UrlEncode,
  canonicalJson,
  randomId128,
  signBase64,
} from "@/lib/crypto";
import { TOKEN_GRACE_MS } from "@/lib/constants";
import type { CapacityClass, ChallengeRecord, LeaseRecord } from "@/lib/store";

export interface EligibilityLeaseFields {
  peer_id: string;
  installation_id: string;
  model_profile_id: string;
  issued_at: string;
  expires_at: string;
  can_host: boolean;
  can_consume: boolean;
  slots: number;
  verified_capacity: CapacityClass;
  audit_epoch: number;
  challenge_id: string;
  nonce: string;
}

export interface IssuedEligibilityLease {
  token: string; // base64url(json) "." base64url(sig)
  fields: EligibilityLeaseFields;
  signature: string;
}

/**
 * Deterministic verified-capacity table from measured challenge timings
 * (ADR-012: self-reported hardware never raises capacity).
 */
export function capacityFromTimings(totalMs: number): CapacityClass {
  if (totalMs <= 2_000) return "gpu_high";
  if (totalMs <= 8_000) return "gpu_mid";
  if (totalMs <= 30_000) return "gpu_entry";
  return "cpu";
}

export function capacityRank(c: CapacityClass): number {
  return { cpu: 0, gpu_entry: 1, gpu_mid: 2, gpu_high: 3 }[c];
}

/**
 * Build + sign an EligibilityLease. `expires_at` is hard-capped at the
 * issuing lease's expiry + 60 s grace (msp-v1 §5 / ADR-012).
 */
export function issueEligibilityLease(
  lease: LeaseRecord,
  challenge: ChallengeRecord,
  profileId: string,
  auditEpoch: number,
  now: number,
  secretKey: Uint8Array,
): IssuedEligibilityLease {
  const fields: EligibilityLeaseFields = {
    peer_id: lease.peerId,
    installation_id: lease.installationId,
    model_profile_id: profileId,
    issued_at: new Date(now).toISOString(),
    expires_at: new Date(lease.expiresAt + TOKEN_GRACE_MS).toISOString(),
    can_host: true,
    can_consume: true,
    slots: lease.maxSlots,
    verified_capacity: lease.capacityClass,
    audit_epoch: auditEpoch,
    challenge_id: challenge.challengeId,
    nonce: randomId128(),
  };
  const json = canonicalJson(fields);
  // Detached Ed25519 signature over the canonical JSON, base64url-encoded
  // (msp-v1 §5 serialized form: base64url(json) "." base64url(sig)).
  const signatureBytes = Buffer.from(signBase64(json, secretKey), "base64");
  const signature = Buffer.from(signatureBytes).toString("base64url");
  const token = `${base64UrlEncode(json)}.${signature}`;
  return { token, fields, signature };
}

/** Parse + structurally check a serialized eligibility lease (no key check). */
export function splitEligibilityLease(token: string): { json: string; signature: string } | null {
  const parts = token.split(".");
  if (parts.length !== 2 || parts[0].length === 0 || parts[1].length === 0) return null;
  return { json: Buffer.from(parts[0], "base64url").toString("utf8"), signature: parts[1] };
}
