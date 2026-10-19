// ADR-044 reputation chassis — the derived-standing computation.

import { describe, expect, it } from "vitest";

import { standingOf } from "@/lib/reputation";
import type { SuspensionRecord } from "@/lib/store";

const NOW = Date.UTC(2026, 10, 4, 12, 0, 0);

function suspension(overrides: Partial<SuspensionRecord> = {}): SuspensionRecord {
  return {
    peerId: "PeerId".padEnd(12, "y"),
    untilAt: NOW + 60_000,
    reason: "challenge failures",
    suspensionCount: 1,
    prevMaxSlots: 1,
    ...overrides,
  };
}

describe("ADR-044 standingOf", () => {
  it("a fresh hosting peer is a member — base, not penalty", () => {
    const s = standingOf(0, 0, null, NOW);
    expect(s.grade).toBe("member");
    expect(s.strikes).toBe(0);
    expect(s.served_tok_per_sec).toBe(0);
  });

  it("contribution grades key on the SAME bars as fair-queue weight", () => {
    // 2 served tokens over 1 endpoint second = 2 tok/s: ≥ T2 (1), < T3 (5).
    expect(standingOf(2, 1, null, NOW).grade).toBe("contributor");
    // 5 over 1 = 5 tok/s: ≥ T3 → cornerstone.
    expect(standingOf(5, 1, null, NOW).grade).toBe("cornerstone");
    // Just under T2.
    expect(standingOf(0.9, 1, null, NOW).grade).toBe("member");
    // Long endpoint time dilutes the rate honestly.
    expect(standingOf(4, 10, null, NOW).grade).toBe("member");
  });

  it("an ACTIVE suspension overrides any contribution grade", () => {
    expect(standingOf(50, 1, suspension(), NOW).grade).toBe("recovering");
    // Expired suspension (untilAt in the past): row-aging recovers the
    // contribution grade automatically — the record alone never punishes.
    const expired = suspension({ untilAt: NOW - 1_000 });
    expect(standingOf(50, 1, expired, NOW).grade).toBe("cornerstone");
    // The strike count still displays for honesty.
    expect(standingOf(50, 1, expired, NOW).strikes).toBe(1);
  });

  it("probation = active suspension in the ADR-012 escalation band", () => {
    expect(standingOf(0, 0, suspension({ suspensionCount: 3 }), NOW).grade).toBe("probation");
    expect(standingOf(0, 0, suspension({ suspensionCount: 7 }), NOW).grade).toBe("probation");
  });

  it("rounds display fields without touching the inputs", () => {
    const s = standingOf(12_345.678, 100, null, NOW);
    expect(s.served_tok_per_sec).toBe(123.5);
    expect(s.served_tokens_decayed).toBe(12346);
  });

  it("operator thresholds flow through (same env knobs as ADR-043)", () => {
    expect(standingOf(0.5, 1, null, NOW, 0.5, 2).grade).toBe("contributor");
    expect(standingOf(2, 1, null, NOW, 0.5, 2).grade).toBe("cornerstone");
  });
});
