// POST /api/v1/account/machines/[installationId]/revoke — mark one of the
// signed-in user's machines revoked (frees the installation for another
// account to bind). Malformed, not-found, or not-owned ids all answer the
// same 404 unknown_machine.

import { jsonErrorFor, jsonOk, logEvent } from "@/lib/errors";
import { guardAccountRequest, requireJsonContentType } from "@/lib/guard";
import { requireAccountUser } from "@/lib/account-auth";
import { BASE58_ID_PATTERN } from "@/lib/schemas";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(
  req: Request,
  { params }: { params: Promise<{ installationId: string }> },
): Promise<Response> {
  const contentType = requireJsonContentType(req);
  if (contentType) return contentType;
  const guarded = await guardAccountRequest(req);
  if (!guarded.ok) return guarded.response;
  const auth = await requireAccountUser(req, guarded.ctx);
  if (!auth.ok) return auth.response;

  const { installationId } = await params;
  // Shape-gate the path param before any store call: >64-char garbage must
  // not reach Postgres' varchar(64) (a 500), and Memory/Pg must agree.
  if (!BASE58_ID_PATTERN.test(installationId)) {
    return jsonErrorFor("unknown_machine", "no machine with that id for this account");
  }
  const ok = await guarded.ctx.store.revokeAccountMachine(auth.userId, installationId);
  if (!ok) {
    return jsonErrorFor("unknown_machine", "no machine with that id for this account");
  }
  logEvent("info", { event: "account_machine_revoke", userId: auth.userId, installationId });
  return jsonOk({ ok: true });
}
