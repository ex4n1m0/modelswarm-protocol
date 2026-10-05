// Shared peer-lease binding checks (msp-v1 §3.3): a leaseId is unguessable
// and bound to the signing installation; referencing another installation's
// lease (or a nonexistent one) is 401 unauthorized, an expired lease is
// 410 lease_expired.

import { jsonError } from "@/lib/errors";
import type { LeaseRecord } from "@/lib/store";

export function leaseBindingError(
  lease: LeaseRecord | null,
  installationId: string,
  now: number,
): Response | null {
  if (!lease) return jsonError(401, "unauthorized", "unknown lease");
  if (lease.installationId !== installationId) {
    return jsonError(401, "unauthorized", "lease belongs to another installation");
  }
  if (lease.expiresAt <= now) {
    return jsonError(410, "lease_expired", "lease expired; re-register");
  }
  return null;
}
