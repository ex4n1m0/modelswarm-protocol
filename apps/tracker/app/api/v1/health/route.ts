import { NextResponse } from "next/server";
import { MSP_PROTOCOL_VERSION, SERVICE_NAME } from "@/lib/version";

// Phase A skeleton: static liveness only. Leases, catalog signing, and peer
// endpoints arrive in Phase 1 (docs/acceptance/phase-1.md).

export function GET() {
  return NextResponse.json({
    status: "ok",
    service: SERVICE_NAME,
    protocol: MSP_PROTOCOL_VERSION,
  });
}
