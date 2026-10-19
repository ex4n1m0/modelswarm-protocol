import { jsonOk } from "@/lib/errors";
import { guardPeerGet } from "@/lib/guard";
import {
  DEFAULT_FAIR_WEIGHT_T2,
  DEFAULT_FAIR_WEIGHT_T3,
  GRANTED_HALF_LIFE_SECS,
} from "@/lib/constants";
import { standingOf } from "@/lib/reputation";

// GET /api/v1/accounting?peer_id=… (ADR-030 §5 read surface): the peer's
// granted verified-work counters — decayed (7-day half-life, the frozen
// INITIAL value) and raw — plus the ADR-044 derived standing. Counts and
// timings only; the raw rows stay in the store as the auditable evidence
// base. This is the surface fair queueing and the reputation chassis
// consume (M10 ordering: they read, they never self-report).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const guarded = await guardPeerGet(req);
  if (!guarded.ok) return guarded.response;
  const { ctx } = guarded;
  const peerId = new URL(req.url).searchParams.get("peer_id") ?? "";
  if (!peerId) {
    return jsonOk({
      halfLifeSecs: GRANTED_HALF_LIFE_SECS,
      ...emptyCounters(),
      standing: standingOf(0, 0, null, ctx.now()),
    });
  }
  const counters = await ctx.store.grantedCounters(peerId, GRANTED_HALF_LIFE_SECS);
  const suspension = await ctx.store.getSuspension(peerId);
  const standing = standingOf(
    counters.verifiedTokensDecayed,
    counters.endpointSecondsDecayed,
    suspension,
    ctx.now(),
    fairThreshold(process.env.MSP_FAIR_WEIGHT_T2, DEFAULT_FAIR_WEIGHT_T2),
    fairThreshold(process.env.MSP_FAIR_WEIGHT_T3, DEFAULT_FAIR_WEIGHT_T3),
  );
  return jsonOk({ peerId, halfLifeSecs: GRANTED_HALF_LIFE_SECS, ...counters, standing });
}

function emptyCounters() {
  return {
    endpointSecondsDecayed: 0,
    verifiedTokensDecayed: 0,
    endpointSecondsRaw: 0,
    verifiedTokensRaw: 0,
    rows: 0,
  };
}

/// ADR-043/044 shared env-threshold parse (heartbeat route keeps its own
/// copy — Next.js route isolation).
function fairThreshold(raw: string | undefined, fallback: number): number {
  const parsed = Number.parseFloat(raw ?? "");
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
