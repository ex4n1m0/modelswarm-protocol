// Catalog envelope construction (msp-v1 §4 + ADR-011): schema-v2
// ProfileRecords in, Ed25519-signed envelope out.

import { canonicalJson, signBase64 } from "@/lib/crypto";
import type { ProfileRecord } from "@/lib/store";

/** ProfileRecord wire form (snake_case per catalog/schema-v2.json). */
export function profileToWire(rec: ProfileRecord): Record<string, unknown> {
  const out: Record<string, unknown> = {
    manifest: rec.manifest,
    profile_id: rec.profileId,
    display_name: rec.displayName,
    status: rec.status,
  };
  if (rec.provenance) out.provenance = rec.provenance;
  return out;
}

export interface CatalogEnvelope {
  catalogVersion: number;
  generatedAt: string;
  profiles: Record<string, unknown>[];
  signature: string;
}

/**
 * Sign {catalogVersion, generatedAt, profiles} canonically with the hub key.
 * The signature is standard base64 (msp-v1 §4), detached from the envelope.
 */
export function signCatalog(
  payload: { catalogVersion: number; generatedAt: string; profiles: Record<string, unknown>[] },
  secretKey: Uint8Array,
): CatalogEnvelope {
  return { ...payload, signature: signBase64(canonicalJson(payload), secretKey) };
}
