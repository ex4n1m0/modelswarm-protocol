// ADR-041 swarm balance, REVISED per owner ruling 2026-10-17 (the
// torrent lesson): the gate keys on CAPACITY, not operational history —
// "I host a model with a certain capacity due to my hardware and model
// type; I can pull that many tokens per second, even if nobody is
// pulling from mine at that moment or in the past."
//
//   allowed while: tokens consumed in the trailing WINDOW
//                  <= k * capacity_tokens_per_sec(verified class) * WINDOW
//
// The verified CapacityClass is ASSIGNED from measured challenge timings
// (D7/ADR-012 — self-reported hardware never raises it): hardware +
// model type, exactly the owner's phrasing. A rolling window (default
// 1 h) IS the burst allowance and the cold start — a fresh host can pull
// its full window budget immediately; no bootstrap, no history debt, no
// unpopular-torrent spiral. The receipts ledger (granted + consumed)
// remains the evidence base for accounting, display, and the future
// reputation chassis — it just no longer GATES consumption.
//
// Ships DARK (computed + surfaced on every issuance; blocks only under
// MSP_SWARM_BALANCE_ENFORCE=on).

export const ENV_BALANCE_K = "MSP_SWARM_BALANCE_K";
export const ENV_BALANCE_ENFORCE = "MSP_SWARM_BALANCE_ENFORCE";
export const ENV_BALANCE_ENFORCE_CLOUD = "MSP_SWARM_BALANCE_ENFORCE_CLOUD";
export const ENV_BALANCE_WINDOW_SECS = "MSP_SWARM_BALANCE_WINDOW_SECS";

export const DEFAULT_BALANCE_K = 1.0;
export const DEFAULT_WINDOW_SECS = 3600;

/** ADR-042 §2: cumulative serving evidence required before the measured
 *  anchor replaces the class table (shorter samples are TTFT-noisy; the
 *  class fallback errs LOW, so under-evidence only under-allows). */
export const MIN_SERVED_SECONDS = 2;

/** Verified-class capacity table (policy constants, ADR-041 §2): the
 *  measured serving speed a peer of this class honestly sustains. */
export const CLASS_TOKENS_PER_SEC: Record<string, number> = {
  cpu: 8,
  gpu_entry: 15,
  gpu_mid: 40,
  gpu_high: 90,
};

export function balanceK(): number {
  const raw = Number.parseFloat(process.env[ENV_BALANCE_K] ?? "");
  return Number.isFinite(raw) && raw >= 0 ? raw : DEFAULT_BALANCE_K;
}

export function balanceWindowSecs(): number {
  const raw = Number.parseInt(process.env[ENV_BALANCE_WINDOW_SECS] ?? "", 10);
  return Number.isFinite(raw) && raw > 0 ? raw : DEFAULT_WINDOW_SECS;
}

export function balanceEnforced(): boolean {
  return (process.env[ENV_BALANCE_ENFORCE] ?? "").trim().toLowerCase() === "on";
}

/** ADR-042 §3: cloud-profile leases enforce the balance by DEFAULT (the
 *  capacity-stake access path is born limited); the operator kill-switch
 *  spells the env value "off". ADR-041's dark default for other leases is
 *  unchanged (balanceEnforced above). */
export function cloudBalanceEnforced(): boolean {
  return (process.env[ENV_BALANCE_ENFORCE_CLOUD] ?? "").trim().toLowerCase() !== "off";
}

export interface BalanceView {
  /** The verified capacity this peer may pull at (tokens/sec, all profiles). */
  capacityTokensPerSec: number;
  /** Where the anchor came from — "measured" (ADR-042 §2, receipt-backed
   *  trailing served tok/s) or "class" (ADR-041 table / cold start). */
  capacitySource: "measured" | "class";
  /** Tokens consumed inside the current window. */
  consumedInWindow: number;
  /** The current window budget (k * capacity * window). */
  budget: number;
  k: number;
  windowSecs: number;
  /** allowance still unspent. */
  headroom: number;
  /** consumption allowed under the current constants? */
  ok: boolean;
}

export function balanceDecision(
  capacityClass: string,
  consumedInWindow: number,
  measured?: { tokens: number; seconds: number },
): BalanceView {
  const k = balanceK();
  const windowSecs = balanceWindowSecs();
  // ADR-042 §2: the owner's anchor is the machine's OWN measured serving
  // throughput (tokens/second while serving — utilization-independent);
  // the class table covers cold start until MIN_SERVED_SECONDS of
  // receipt-backed serving time exists in the window.
  let capacityTokensPerSec = CLASS_TOKENS_PER_SEC[capacityClass] ?? CLASS_TOKENS_PER_SEC.cpu!;
  let capacitySource: "measured" | "class" = "class";
  if (measured && measured.seconds >= MIN_SERVED_SECONDS && measured.tokens > 0) {
    const measuredTokPerSec = measured.tokens / measured.seconds;
    if (measuredTokPerSec > 0) {
      capacityTokensPerSec = measuredTokPerSec;
      capacitySource = "measured";
    }
  }
  const budget = k * capacityTokensPerSec * windowSecs;
  const headroom = budget - consumedInWindow;
  return {
    capacityTokensPerSec,
    capacitySource,
    consumedInWindow,
    budget,
    k,
    windowSecs,
    headroom,
    ok: k === 0 || headroom >= 0, // k=0 disables the gate (policy off)
  };
}
