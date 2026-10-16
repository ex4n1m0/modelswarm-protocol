import { jsonOk } from "@/lib/errors";
import { guardPeerGet } from "@/lib/guard";
import { GRANTED_HALF_LIFE_SECS } from "@/lib/constants";

// GET /api/v1/accounting?peer_id=… (ADR-030 §5 read surface): the peer's
// granted verified-work counters — decayed (7-day half-life, the frozen
// INITIAL value) and raw. Counts and timings only; the raw rows stay in
// the store as the auditable evidence base. This is the surface fair
// queueing and the reputation chassis will consume (M10 ordering: they
// read, they never self-report).

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const guarded = await guardPeerGet(req);
  if (!guarded.ok) return guarded.response;
  const { ctx } = guarded;
  const peerId = new URL(req.url).searchParams.get("peer_id") ?? "";
  if (!peerId) {
    return jsonOk({ halfLifeSecs: GRANTED_HALF_LIFE_SECS, ...emptyCounters() });
  }
  const counters = await ctx.store.grantedCounters(peerId, GRANTED_HALF_LIFE_SECS);
  return jsonOk({ peerId, halfLifeSecs: GRANTED_HALF_LIFE_SECS, ...counters });
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
