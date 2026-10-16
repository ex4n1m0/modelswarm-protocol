// Receipts v2 (ADR-030 §1/§2): the tracker-side mirror of
// modelswarm_eligibility::receipt_v2 — canonical signing phases, the
// frozen receiptDigest, signature verification, and the grant exclusions
// applied at intake. Pure TS over the shared crypto lib so the two sides
// cannot drift (the relay-ticket discipline); byte-parity is pinned
// against protocol/vectors/receipt-v2-1.json in tests.

import { base64UrlEncode, canonicalJson, sha256Hex, verifyBase64Url } from "@/lib/crypto";
import * as ed from "@noble/ed25519";

/** The ADR-030 §1 object on the wire (both signatures present at rest). */
export interface ReceiptV2 {
  request_id: string;
  profile_id: string;
  preset: string;
  mode_id: string;
  cohort_digest: string;
  peers: Array<{ peer_id: string; installation_id: string; role: string }>;
  started_at: string;
  ended_at: string;
  outcome: string;
  accepted_tokens: number;
  verified_tokens: number;
  endpoint_seconds: number;
  usage_digest: string;
  budgets: Record<string, unknown>;
  server_signature: string;
  requester_signature: string;
}

/** `cohort_digest` (§1): sha256 over the canonical JSON of the sorted
 * "peer_id:role" labels. */
export function cohortDigestOf(peers: ReceiptV2["peers"]): string {
  const labels = peers.map((p) => `${p.peer_id}:${p.role}`).sort();
  return `sha256:${sha256Hex(canonicalJson(labels))}`;
}

function canonicalWith(
  receipt: ReceiptV2,
  server: boolean,
  requester: boolean,
): string | null {
  const clone: Record<string, unknown> = { ...receipt };
  if (!server) delete clone.server_signature;
  if (!requester) delete clone.requester_signature;
  try {
    return canonicalJson(clone);
  } catch {
    return null;
  }
}

export function phase1Canonical(receipt: ReceiptV2): string | null {
  return canonicalWith(receipt, false, false);
}

export function phase2Canonical(receipt: ReceiptV2): string | null {
  return canonicalWith(receipt, true, false);
}

export function finalCanonical(receipt: ReceiptV2): string | null {
  return canonicalWith(receipt, true, true);
}

/** The frozen `receiptDigest` (§2): sha256 over the final counter-signed
 * canonical JSON. */
export function receiptDigest(receipt: ReceiptV2): string | null {
  const canonical = finalCanonical(receipt);
  return canonical === null ? null : `sha256:${sha256Hex(canonical)}`;
}

/** Verifies both signatures against the enrolled public keys of the
 * signing installations. */
export function verifyReceiptSignatures(
  receipt: ReceiptV2,
  serverKey: Uint8Array,
  requesterKey: Uint8Array,
): boolean {
  const phase1 = phase1Canonical(receipt);
  const phase2 = phase2Canonical(receipt);
  if (phase1 === null || phase2 === null) return false;
  return (
    verifyBase64Url(receipt.server_signature, phase1, serverKey) &&
    verifyBase64Url(receipt.requester_signature, phase2, requesterKey)
  );
}

/** Grant exclusion (§4): the installations this receipt grants accounting
 * to — everyone except the requesting installation (self-work is zero).
 * Sybil-ring diversity is applied at intake on top (subnet/ASN knowledge
 * lives tracker-side). */
export function grantedPeers(
  receipt: ReceiptV2,
  requestingInstallation: string,
): ReceiptV2["peers"] {
  return receipt.peers.filter((p) => p.installation_id !== requestingInstallation);
}

/** Per-peer attribution (the initial frozen rule, conservative):
 * integer division across granted peers, remainder discarded. */
export function perPeerShare(
  receipt: ReceiptV2,
  grantedCount: number,
): { endpoint_seconds: number; verified_tokens: number } {
  if (grantedCount === 0) return { endpoint_seconds: 0, verified_tokens: 0 };
  return {
    endpoint_seconds: Math.floor(receipt.endpoint_seconds / grantedCount),
    verified_tokens: Math.floor(receipt.verified_tokens / grantedCount),
  };
}

/** Intake validation (§3): accounting moves only on receipts that pass
 * EVERY check — digest preimage, both signatures against enrolled keys,
 * non-empty peer set. Returns the granted contributions, or null when the
 * receipt is not accounting evidence. */
export function validateForAccounting(
  receipt: ReceiptV2,
  claimedDigest: string,
  serverKey: Uint8Array,
  requesterKey: Uint8Array,
): Array<{
  peer_id: string;
  installation_id: string;
  endpoint_seconds: number;
  verified_tokens: number;
}> | null {
  if (receipt.peers.length === 0) return null;
  if (receipt.server_signature.length === 0 || receipt.requester_signature.length === 0) {
    return null;
  }
  const digest = receiptDigest(receipt);
  if (digest === null || digest !== claimedDigest) return null;
  if (!verifyReceiptSignatures(receipt, serverKey, requesterKey)) return null;
  const requester = receipt.peers.find((p) => p.role === "requester");
  if (!requester) return null;
  const granted = grantedPeers(receipt, requester.installation_id);
  const share = perPeerShare(receipt, granted.length);
  return granted.map((p) => ({
    peer_id: p.peer_id,
    installation_id: p.installation_id,
    endpoint_seconds: share.endpoint_seconds,
    verified_tokens: share.verified_tokens,
  }));
}

// ---------------------------------------------------------------------------
// Signing helpers (test/tool surface; the tracker's runtime role is
// VERIFICATION — production receipts are signed by the peers, Rust side).

/** Phase 1: the serving/coordinating peer signs the both-absent form. */
export function signReceiptServer(receipt: ReceiptV2, secretKey: Uint8Array): string | null {
  const preimage = phase1Canonical(receipt);
  return preimage === null ? null : base64UrlEncode(ed.sign(Buffer.from(preimage, "utf8"), secretKey));
}

/** Phase 2: the requesting peer counter-signs the server-present form. */
export function signReceiptRequester(receipt: ReceiptV2, secretKey: Uint8Array): string | null {
  const preimage = phase2Canonical(receipt);
  return preimage === null ? null : base64UrlEncode(ed.sign(Buffer.from(preimage, "utf8"), secretKey));
}
