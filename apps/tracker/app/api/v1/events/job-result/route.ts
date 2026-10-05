import { jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { JobResultSchema } from "@/lib/schemas";

// POST /api/v1/events/job-result (msp-v1 §3.3/§7): signed {receiptDigest,
// outcome}. Stores digest + outcome only — timings and outcome, never prompt
// text (ADR-001).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const guarded = await guardPeerRequest(req, JobResultSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body, installationId } = guarded;

  await ctx.store.recordReceipt({
    receiptDigest: body.receiptDigest,
    outcome: body.outcome,
    installationId,
    receivedAt: ctx.now(),
  });
  return jsonOk({ recorded: true });
}
