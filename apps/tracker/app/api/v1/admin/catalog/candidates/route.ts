import { getTrackerContext } from "@/lib/context";
import { deriveProfileId } from "@/lib/crypto";
import { jsonErrorFor, jsonOk } from "@/lib/errors";
import { guardAdminBody, requireAdmin } from "@/lib/guard";
import { CandidateSchema } from "@/lib/schemas";

// POST /api/v1/admin/catalog/candidates (msp-v1 §3.5): insert a schema-v2
// ProfileRecord with derived profile id (ADR-011), status candidate. The
// manifest is validated by zod (shape per catalog/schema-v2.json) and the id
// is derived, never caller-supplied. Immutability: re-publishing a different
// manifest under an existing profileId -> 400 invalid_body (ADR-005).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  const denied = requireAdmin(req, getTrackerContext());
  if (denied) return denied;

  const guarded = await guardAdminBody(req, CandidateSchema);
  if (!guarded.ok) return guarded.response;
  const { ctx, body } = guarded;

  const profileId = deriveProfileId(body.manifest);
  const result = await ctx.store.insertProfile({
    profileId,
    manifest: body.manifest,
    displayName: body.display_name,
    status: body.status ?? "candidate",
    provenance: body.provenance,
    createdAt: ctx.now(),
  });

  if (result === "conflict") {
    return jsonErrorFor(
      "invalid_body",
      `profileId ${profileId} already exists with a different manifest (immutable, ADR-005)`,
    );
  }
  return jsonOk({ profileId }, 201);
}
