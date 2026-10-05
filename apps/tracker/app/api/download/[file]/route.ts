// GET /api/download/<file> — counted installer download (Phase I5).
// Website convenience, NOT part of the msp-v1 tracker contract: increments
// the per-file counter and redirects to the static artifact under
// /downloads/. File names are strictly shaped and must exist on disk, so
// this can neither proxy arbitrary paths nor count phantom files.

import { getTrackerContext } from "@/lib/context";
import { jsonError } from "@/lib/errors";
import { existsSync } from "node:fs";
import { join } from "node:path";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]*\.(exe|deb|AppImage|dmg)$/;

export async function GET(
  req: Request,
  ctx: { params: Promise<{ file: string }> },
): Promise<Response> {
  const { file } = await ctx.params;
  if (!NAME_PATTERN.test(file)) {
    return jsonError(400, "invalid_body", "not a downloadable installer name", false);
  }
  const publicDir = join(process.cwd(), "public", "downloads");
  if (!existsSync(join(publicDir, file))) {
    return jsonError(404, "unknown_profile", "no such installer", false);
  }
  const store = getTrackerContext().store;
  await store.bumpDownloadCount(file);
  // Same-origin redirect: works on the custom domain and preview URLs alike.
  return Response.redirect(new URL(`/downloads/${file}`, req.url).toString(), 302);
}
