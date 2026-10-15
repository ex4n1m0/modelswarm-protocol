// RelayAdmissionTicket minting (ADR-038 §3.1/§3.3 — the D12 close).
//
// The ticket is a tracker-signed JSON object with EXACTLY the field set
// {installation_id, peer_id, issued_at, expires_at, nonce} (snake_case, the
// §5 lease discipline), Ed25519-signed by the hub key over the canonical
// JSON (msp-v1 §2.2) with the signature omitted, serialized in the
// capabilityToken form `base64url(canonical JSON) "." base64url(signature)`.
// TTL is frozen at exactly 600 s and the serialized form stays ≤ 1024 bytes.
//
// Content-blind by construction (T2): minted from installation identity, the
// tracker-side registered peer row, and a clock — nothing else. No profile
// ids, no addresses, no capacity metadata; the ticket never touches
// eligibility, §6 stream logic, or receipts. Signed with the SAME hub key as
// the catalog envelope and eligibility leases (`ctx.keys.secretKey`) — no
// second key surface.

import {
  base64UrlEncode,
  canonicalJson,
  randomId128,
  signBase64,
} from "@/lib/crypto";
import { RELAY_TICKET_TTL_MS } from "@/lib/constants";

/** The exact signed field set (ADR-038 §3.1; `deny_unknown_fields` on the
 *  consumer side — the relay/`modelswarm-eligibility` parses strictly). */
export interface RelayTicketFields {
  installation_id: string;
  peer_id: string;
  /** RFC-3339 millisecond timestamps; `expires_at − issued_at == 600_000 ms`. */
  issued_at: string;
  expires_at: string;
  /** 32 lowercase hex chars, fresh per issuance. */
  nonce: string;
}

export interface IssuedRelayTicket {
  /** base64url(canonical JSON) "." base64url(signature) — the wire string. */
  token: string;
  fields: RelayTicketFields;
  signature: string;
}

/**
 * Mint + sign one RelayAdmissionTicket. Callers MUST pass `installationId`
 * from the AUTHENTICATED identity (envelope verification) and `peerId` from
 * the tracker-side lease/registered row — never client input (ADR-038 §3.3).
 * `now` is the injectable context clock, as everywhere in the tracker.
 */
export function issueRelayAdmissionTicket(
  installationId: string,
  peerId: string,
  now: number,
  secretKey: Uint8Array,
): IssuedRelayTicket {
  const fields: RelayTicketFields = {
    installation_id: installationId,
    peer_id: peerId,
    issued_at: new Date(now).toISOString(),
    expires_at: new Date(now + RELAY_TICKET_TTL_MS).toISOString(),
    nonce: randomId128(),
  };
  const json = canonicalJson(fields);
  // Detached Ed25519 signature over the canonical JSON (signature omitted
  // entirely from the signed form), base64url wire encoding — exactly the
  // eligibility-lease discipline (lib/eligibility.ts, msp-v1 §5).
  const signatureBytes = Buffer.from(signBase64(json, secretKey), "base64");
  const signature = Buffer.from(signatureBytes).toString("base64url");
  const token = `${base64UrlEncode(json)}.${signature}`;
  return { token, fields, signature };
}

/** Split a serialized ticket into its canonical JSON and base64url signature
 *  (structural check only — key/expiry verification is the relay's job). */
export function splitRelayAdmissionTicket(
  token: string,
): { json: string; signature: string } | null {
  const parts = token.split(".");
  if (parts.length !== 2 || parts[0].length === 0 || parts[1].length === 0) return null;
  return {
    json: Buffer.from(parts[0], "base64url").toString("utf8"),
    signature: parts[1],
  };
}
