// ADR-041 swarm balance: the decayed two-sided reciprocity check.
// Pure decision-making here; the ledger reads live in the store.
//
//   allowed while: decayed_consumed <= k * decayed_granted + BOOTSTRAP
//
// In steady state that is exactly "pull at most k× your push rate"
// (owner ruling 2026-10-17: k = 1 to start, operator-tunable). Both
// sides decay with the same half-life, so the window and the anti-farm
// horizon are one mechanism. Constants come from env (policy, not
// releases); enforcement ships DARK first (measured, surfaced, never
// blocking) until real traffic shapes them.

export const ENV_BALANCE_K = "MSP_SWARM_BALANCE_K";
export const ENV_BALANCE_BOOTSTRAP = "MSP_SWARM_BOOTSTRAP_TOKENS";
export const ENV_BALANCE_ENFORCE = "MSP_SWARM_BALANCE_ENFORCE";

export const DEFAULT_BALANCE_K = 1.0;
export const DEFAULT_BOOTSTRAP_TOKENS = 100_000;

export function balanceK(): number {
  const raw = Number.parseFloat(process.env[ENV_BALANCE_K] ?? "");
  return Number.isFinite(raw) && raw >= 0 ? raw : DEFAULT_BALANCE_K;
}

export function bootstrapTokens(): number {
  const raw = Number.parseInt(process.env[ENV_BALANCE_BOOTSTRAP] ?? "", 10);
  return Number.isFinite(raw) && raw >= 0 ? raw : DEFAULT_BOOTSTRAP_TOKENS;
}

export function balanceEnforced(): boolean {
  return (process.env[ENV_BALANCE_ENFORCE] ?? "").trim().toLowerCase() === "on";
}

export interface BalanceView {
  /** Decayed granted tokens (all profiles, the push side). */
  granted: number;
  /** Decayed consumed tokens (the pull side). */
  consumed: number;
  k: number;
  bootstrap: number;
  /** The allowance still unspent under the rule. */
  headroom: number;
  /** consumption allowed under the current constants? */
  ok: boolean;
}

export function balanceDecision(granted: number, consumed: number): BalanceView {
  const k = balanceK();
  const bootstrap = bootstrapTokens();
  const headroom = k * granted + bootstrap - consumed;
  return {
    granted,
    consumed,
    k,
    bootstrap,
    headroom,
    ok: k === 0 || headroom >= 0, // k=0 disables the gate (policy off)
  };
}
