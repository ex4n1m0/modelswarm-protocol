import { jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { HeartbeatSchema } from "@/lib/schemas";
import { LEASE_TTL_MS } from "@/lib/constants";

// POST /api/v1/peers/heartbeat (msp-v1 §3.3): extends the lease (row-aging
// TTL), records an observation, and drains revocation notices (delete-on-read).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, HeartbeatSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;
  const now = ctx.now();

  const lease = await ctx.store.getLease(body.leaseId);
  const binding = leaseBindingError(lease, installationId, now);
  if (binding || !lease) return binding ?? jsonOk({});

  const expiresAt = now + LEASE_TTL_MS;
  await ctx.store.updateLease(body.leaseId, {
    expiresAt,
    freeSlots: body.freeSlots,
    queueMs: body.queueMs,
    draining: body.draining || lease.draining,
    lastSeenAt: now,
  });
  await ctx.store.recordObservation({
    leaseId: body.leaseId,
    peerId: lease.peerId,
    freeSlots: body.freeSlots,
    queueMs: body.queueMs,
    observedAt: now,
  });

  const notices = await ctx.store.drainNotices(body.leaseId);
  return jsonOk({ leaseExpiresAt: new Date(expiresAt).toISOString(), notices });
}
