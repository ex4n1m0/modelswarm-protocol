// Fixed-window rate limiter (msp-v1 §3.4 defaults, tunable via env).
// Deliberately in-memory and timer-free: windows are computed from the
// injectable clock (row-aging style), never from background eviction jobs.

export interface RateLimits {
  /** per installation on peer endpoints (signed) */
  peerPerInstallation: number;
  /** per IP on unsigned calls to peer endpoints */
  unsignedPerIp: number;
  /** per IP on enrollment (auth/device/*) */
  enrollPerIp: number;
  /** per IP on /health + /catalog */
  publicPerIp: number;
}

export const DEFAULT_RATE_LIMITS: RateLimits = {
  peerPerInstallation: 60,
  unsignedPerIp: 60,
  enrollPerIp: 10,
  publicPerIp: 120,
};

const WINDOW_MS = 60_000;

interface Bucket {
  count: number;
  window: number;
}

export class RateLimiter {
  private buckets = new Map<string, Bucket>();

  constructor(
    private now: () => number,
    public limits: RateLimits = { ...DEFAULT_RATE_LIMITS },
  ) {}

  /**
   * Count one hit against `key` in the current fixed minute window.
   * Returns true when the hit is allowed, false when the limit is exceeded.
   */
  hit(key: string, limitPerMinute: number): boolean {
    const window = Math.floor(this.now() / WINDOW_MS);
    const current = this.buckets.get(key);
    if (!current || current.window !== window) {
      this.buckets.set(key, { count: 1, window });
      // Lazy pruning of stale entries for the same key keeps the map bounded
      // without any background timer.
      if (this.buckets.size > 10_000) this.prune(window);
      return 1 <= limitPerMinute;
    }
    current.count += 1;
    return current.count <= limitPerMinute;
  }

  /** Inspect without counting (used when a request is rejected earlier anyway). */
  isExceeded(key: string, limitPerMinute: number): boolean {
    const window = Math.floor(this.now() / WINDOW_MS);
    const current = this.buckets.get(key);
    return !!current && current.window === window && current.count > limitPerMinute;
  }

  private prune(currentWindow: number): void {
    for (const [key, bucket] of this.buckets) {
      if (bucket.window !== currentWindow) this.buckets.delete(key);
    }
  }

  reset(): void {
    this.buckets.clear();
  }
}

/** Extract the best-effort client IP for per-IP buckets. */
export function clientIpOf(req: Request): string {
  const fwd = req.headers.get("x-forwarded-for");
  if (fwd) return fwd.split(",")[0]!.trim();
  return req.headers.get("x-real-ip") ?? "unknown";
}

export type LimiterClass = "peer" | "unsigned" | "enroll" | "public";
