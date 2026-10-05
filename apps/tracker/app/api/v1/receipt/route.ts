import { jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { JobResultSchema } from "@/lib/schemas";

// POST /api/v1/receipt — Phase B canonical receipt intake (DESIGN.md).
// Same schema and storage as /events/job-result; both record only
// {receiptDigest, outcome}.

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
