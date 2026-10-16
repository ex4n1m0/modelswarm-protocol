import { jsonOk } from "@/lib/errors";
import { guardPeerRequest } from "@/lib/guard";
import { JobResultSchema } from "@/lib/schemas";
import { bs58Decode } from "@/lib/crypto";
import { validateForAccounting, type ReceiptV2 } from "@/lib/receipt-v2";
import type { GrantedAccountingRow } from "@/lib/store";

// POST /api/v1/events/job-result (msp-v1 §3.3/§7): signed {receiptDigest,
// outcome}. Stores digest + outcome only — timings and outcome, never prompt
// text (ADR-001).
//
// ADR-030 Phase B1 (additive, optional `receipt`): when the full
// counter-signed ReceiptV2 rides the event, intake VALIDATES it — the
// frozen digest preimage, BOTH signatures against the ENROLLED keys of
// the signing installations, and the grant exclusions (self-work zero;
// installation-distinctness as the initial sybil-ring guard) — and
// appends granted_accounting rows. Counters move ONLY on receipts that
// pass every check; anything else records the digest row exactly as v1
// did and grants nothing, with the honest reason in the response.

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

function keyOf(installation: { pubKeyB58: string } | null): Uint8Array | null {
  if (!installation) return null;
  const key = bs58Decode(installation.pubKeyB58);
  if (key === null || key.length !== 32) return null;
  return key;
}

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

  // v1-compat: the no-receipt response stays byte-identical (the
  // phaseb.test pin); the extra fields appear only when a receipt rides.
  if (!body.receipt) {
    return jsonOk({ recorded: true });
  }
  const receipt = body.receipt as ReceiptV2;

  // Enrolled keys for both signing roles; unknown installation = not
  // accounting evidence (a receipt signed by an unenrolled key grants
  // nothing).
  const serverPeer = receipt.peers.find((p) => p.role === "server");
  const requesterPeer = receipt.peers.find((p) => p.role === "requester");
  if (!serverPeer || !requesterPeer) {
    return jsonOk({ recorded: true, grants: [], reason: "roles_incomplete" });
  }
  // Initial sybil-ring guard (ADR-030 §4): the engaged installations must
  // be pairwise distinct — one installation playing several roles is a
  // ring. Subnet/ASN diversity rides this same hook when that data
  // exists at intake.
  const distinct = new Set(receipt.peers.map((p) => p.installation_id));
  if (distinct.size !== receipt.peers.length) {
    return jsonOk({ recorded: true, grants: [], reason: "sybil_ring_installations" });
  }
  const [serverInst, requesterInst] = await Promise.all([
    ctx.store.getInstallation(serverPeer.installation_id),
    ctx.store.getInstallation(requesterPeer.installation_id),
  ]);
  const serverKey = keyOf(serverInst);
  const requesterKey = keyOf(requesterInst);
  if (!serverKey || !requesterKey) {
    return jsonOk({ recorded: true, grants: [], reason: "signer_not_enrolled" });
  }

  const grants = validateForAccounting(
    receipt,
    `sha256:${body.receiptDigest}`,
    serverKey,
    requesterKey,
  );
  if (grants === null || grants.length === 0) {
    return jsonOk({ recorded: true, grants: [], reason: "receipt_not_valid_evidence" });
  }
  const rows: GrantedAccountingRow[] = grants.map((g) => ({
    receiptDigest: body.receiptDigest,
    peerId: g.peer_id,
    installationId: g.installation_id,
    profileId: receipt.profile_id,
    endpointSeconds: g.endpoint_seconds,
    verifiedTokens: g.verified_tokens,
    grantedAt: ctx.now(),
  }));
  await ctx.store.recordGrantedAccounting(rows);
  return jsonOk({
    recorded: true,
    grants: rows.map((r) => ({
      peerId: r.peerId,
      endpointSeconds: r.endpointSeconds,
      verifiedTokens: r.verifiedTokens,
    })),
    reason: "granted",
  });
}
