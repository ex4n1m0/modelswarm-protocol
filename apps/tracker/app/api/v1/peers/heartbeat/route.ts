import { jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { HeartbeatSchema } from "@/lib/schemas";
import { LEASE_TTL_MS } from "@/lib/constants";
import { issueRelayAdmissionTicket } from "@/lib/relay-ticket";

// POST /api/v1/peers/heartbeat (msp-v1 §3.3): extends the lease (row-aging
// TTL), records an observation, and drains revocation notices (delete-on-read).
// Every SUCCESSFUL heartbeat also mints one RelayAdmissionTicket (ADR-038
// §3.3): issuance to heartbeat-current installations rides this existing
// surface — no new auth machinery, no new route, no new rate limit. The
// ticket is server-side-minted from the AUTHENTICATED installation and the
// tracker-side lease row (peerId registered at /peers/register); neither id
// is ever taken from the request body. Failure paths (401/410 below) return
// BEFORE minting — no ticket for unauthenticated or stale-lease callers.

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

  // Same hub key surface as catalog envelopes and eligibility leases
  // (ADR-038 §3.4 key distribution; deliberately NOT a second key).
  const relayTicket = issueRelayAdmissionTicket(
    installationId,
    lease.peerId,
    now,
    ctx.keys.secretKey,
  );

  const notices = await ctx.store.drainNotices(body.leaseId);
  // ADR-039: the operator-configured community relay multiaddr, handed to
  // every enrolled installation on each heartbeat (empty string = not
  // configured; absent env = the feature is off — deploy-order additive,
  // the relayTicket precedent).
  const relayAddr = (process.env.MSP_RELAY_ADDR ?? "").trim();
  return jsonOk({
    leaseExpiresAt: new Date(expiresAt).toISOString(),
    relayTicket: relayTicket.token,
    relayAddr,
    notices,
  });
}
