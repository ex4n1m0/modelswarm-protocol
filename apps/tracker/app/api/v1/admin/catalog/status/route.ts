// POST /api/v1/admin/catalog/status — set a profile's lifecycle status
// (candidate | active | deprecated). The ops lever promote never had:
// demoting an active profile that must leave the live catalog (e.g. a
// client-compat break, a licensing problem) previously required direct DB
// access. Idempotent; the manifest never changes here (immutability is
// ADR-005's — status is lifecycle, not content).

import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardAdminBody, requireAdmin } from "@/lib/guard";
import { getTrackerContext } from "@/lib/context";
import { z } from "zod";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const StatusSchema = z
  .object({
    profileId: z.string().regex(/^msp1:[0-9a-f]{64}$/, "msp1:<64 hex>"),
    status: z.enum(["candidate", "active", "deprecated"]),
  })
  .strict();

export async function POST(req: Request) {
  const ctx = getTrackerContext();
  const denied = requireAdmin(req, ctx);
  if (denied) return denied;
  const guarded = await guardAdminBody(req, StatusSchema);
  if (!guarded.ok) return guarded.response;
  const body = guarded.body as { profileId: string; status: "candidate" | "active" | "deprecated" };

  const ok = await ctx.store.setProfileStatus(body.profileId, body.status);
  if (!ok) {
    return jsonErrorFor("unknown_profile", `profile ${String(body.profileId)} not found`);
  }
  return jsonOk({ profileId: body.profileId, status: body.status });
}
