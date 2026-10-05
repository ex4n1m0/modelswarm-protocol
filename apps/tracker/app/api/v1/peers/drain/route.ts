import { jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { leaseBindingError } from "@/lib/leases";
import { LeaseIdSchema } from "@/lib/schemas";

// POST /api/v1/peers/drain (msp-v1 §3.3): marks the lease draining. Draining
// peers are excluded from GET /peers and from eligibility-lease issuance
// (403 ineligible) — deterministic, no grace.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, LeaseIdSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;
  const now = ctx.now();

  const lease = await ctx.store.getLease(body.leaseId);
  const binding = leaseBindingError(lease, installationId, now);
  if (binding || !lease) return binding ?? jsonOk({});

  await ctx.store.updateLease(body.leaseId, { draining: true });
  return jsonOk({ draining: true });
}
