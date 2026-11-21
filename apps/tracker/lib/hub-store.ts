// Hub storage for the OnlyHumans API port (ADR-013 D4, P3 slice 5).
//
// `HubStore` is the single interface the hub routes talk to. Two
// implementations, mirroring lib/store.ts's MemoryStore/PgStore split:
//   - UpstashHubStore: plain fetch to Upstash REST (no SDK), constructed only
//     when DM1_UPSTASH_REDIS_REST_URL + DM1_UPSTASH_REDIS_REST_TOKEN are set.
//   - MemoryHubStore:  Maps + injectable clock, used ONLY via explicit test
//     injection (setHubStore). Production FAILS CLOSED with 503 when the env
//     vars are absent (matching the reference) — never a silent memory serve.
//
// UNIVERSE ISOLATION (D4): this hub is a SEPARATE universe from
// onlyhumans.deepflux.space. Two independent layers keep them apart:
//   1. DEDICATED env vars (DM1_UPSTASH_REDIS_REST_URL / _TOKEN) — the tracker
//      never reads the reference's UPSTASH_REDIS_REST_URL / _TOKEN.
//   2. A `dm1:` prefix on EVERY key — even if both pointed at one Upstash
//      instance, dm1:peer:/room:/inbox:/presence never collide with the
//      reference's bare peer:/room:/inbox:/presence keyspace.
//
// OPAQUE-RECORD INVARIANT: records are stored VERBATIM (JSON.stringify of the
// validated body) and returned verbatim. PUBLIC-signature + key→id verification
// happens at the ROUTE layer (lib/hub-crypto.ts) before storage; this store
// never decodes keys or inspects `env_json`, and clients re-verify on read
// (hub.rs). Compatible with lib/store.ts's "no prompt/completion text" rule:
// the inbox's only free-form payload is opaque sealed ciphertext, bytes in /
// bytes out, never parsed or logged.
//
// TTLs mirror OnlyHumans-main/api/*.ts: peer/room 300 s, inbox 86400 s (last
// 32 items per box), presence 300 s (the whole zset self-destructs at 900 s).
// Expiry is row-aging in MemoryHubStore (expiresAt vs the injected clock) and
// server-side EX / score-pruning in UpstashHubStore — no timers or sweepers.
//
// RATE LIMITING is deliberately NOT here: the reference's redis `rl:` keys
// are replaced by the tracker's in-memory RateLimiter (lib/ratelimit.ts) at
// the route layer, per the D4 handoff.

import type { HubMailItem, HubRegistration, HubRoomHost } from "@/lib/hub-schemas";

export const HUB_PEER_TTL_S = 300;
export const HUB_ROOM_TTL_S = 300;
export const HUB_INBOX_TTL_S = 86400;
export const HUB_INBOX_MAX_PER_BOX = 32;
export const HUB_PRESENCE_TTL_S = 300;
/** The whole presence zset self-destructs if every app disappears. */
export const HUB_PRESENCE_SET_TTL_S = HUB_PRESENCE_TTL_S * 3;

export interface HubStore {
  getPeer(peerId: string): Promise<HubRegistration | null>;
  setPeer(peerId: string, record: HubRegistration): Promise<void>;
  getRoom(roomId: string): Promise<HubRoomHost | null>;
  /** SET with optional NX. Returns false when NX refused (a live record
   *  exists) — the room-creation election lost. */
  setRoom(roomId: string, record: HubRoomHost, nx: boolean): Promise<boolean>;
  /** Append a mixed-recipient batch: rpush each item, then per box refresh
   *  the TTL and trim to the last HUB_INBOX_MAX_PER_BOX items. */
  appendInbox(items: HubMailItem[]): Promise<void>;
  /** Destructive drain: return every queued item and clear the box. */
  drainInbox(peerId: string): Promise<HubMailItem[]>;
  /** Heartbeat: register the token's expiry, prune dead tokens, return the
   *  live count (including this token). */
  presenceBeat(token: string): Promise<number>;
  presenceLeave(token: string): Promise<void>;
}

// ---------------------------------------------------------------------------
// Upstash REST (plain fetch, no SDK) — mirrors OnlyHumans-main/api discipline.

interface UpstashResult {
  result?: unknown;
}

export class UpstashHubStore implements HubStore {
  private url: string;
  constructor(
    url: string,
    private token: string,
    private clock: () => number = Date.now,
  ) {
    this.url = url.replace(/\/$/, "");
  }

  private key(suffix: string): string {
    return `dm1:${suffix}`;
  }

  private async set(key: string, value: string, opts: { ex: number; nx?: boolean }): Promise<boolean> {
    let u = `${this.url}/set/${encodeURIComponent(key)}/${encodeURIComponent(value)}?EX=${opts.ex}`;
    if (opts.nx) u += "&NX";
    const r = await fetch(u, { headers: { Authorization: `Bearer ${this.token}` } });
    if (!r.ok) throw new Error(`upstash set ${r.status}: ${await r.text()}`);
    const j = (await r.json()) as UpstashResult;
    return j.result === "OK"; // null when NX refused
  }

  private async get(key: string): Promise<string | null> {
    const r = await fetch(`${this.url}/get/${encodeURIComponent(key)}`, {
      headers: { Authorization: `Bearer ${this.token}` },
    });
    if (!r.ok) throw new Error(`upstash get ${r.status}: ${await r.text()}`);
    const j = (await r.json()) as UpstashResult;
    return typeof j.result === "string" ? j.result : null;
  }

  private async pipe(commands: unknown[][]): Promise<UpstashResult[]> {
    const r = await fetch(`${this.url}/pipeline`, {
      method: "POST",
      headers: { Authorization: `Bearer ${this.token}`, "Content-Type": "application/json" },
      body: JSON.stringify(commands),
    });
    if (!r.ok) throw new Error(`upstash pipe ${r.status}: ${await r.text()}`);
    const j = (await r.json()) as unknown;
    if (!Array.isArray(j)) throw new Error("upstash pipe: unexpected body");
    return j as UpstashResult[];
  }

  async getPeer(peerId: string): Promise<HubRegistration | null> {
    return parseRecord<HubRegistration>(await this.get(this.key(`peer:${peerId}`)));
  }

  async setPeer(peerId: string, record: HubRegistration): Promise<void> {
    await this.set(this.key(`peer:${peerId}`), JSON.stringify(record), { ex: HUB_PEER_TTL_S });
  }

  async getRoom(roomId: string): Promise<HubRoomHost | null> {
    return parseRecord<HubRoomHost>(await this.get(this.key(`room:${roomId}`)));
  }

  async setRoom(roomId: string, record: HubRoomHost, nx: boolean): Promise<boolean> {
    return this.set(this.key(`room:${roomId}`), JSON.stringify(record), { ex: HUB_ROOM_TTL_S, nx });
  }

  async appendInbox(items: HubMailItem[]): Promise<void> {
    const commands: unknown[][] = [];
    for (const it of items) commands.push(["rpush", this.key(`inbox:${it.to}`), JSON.stringify(it)]);
    for (const to of new Set(items.map((it) => it.to))) {
      commands.push(["expire", this.key(`inbox:${to}`), String(HUB_INBOX_TTL_S)]);
      commands.push(["ltrim", this.key(`inbox:${to}`), String(-HUB_INBOX_MAX_PER_BOX), "-1"]);
    }
    await this.pipe(commands);
  }

  async drainInbox(peerId: string): Promise<HubMailItem[]> {
    const out = await this.pipe([
      ["lrange", this.key(`inbox:${peerId}`), "0", "-1"],
      ["del", this.key(`inbox:${peerId}`)],
    ]);
    const raw = out[0]?.result;
    if (!Array.isArray(raw)) return [];
    return raw
      .map((s) => parseRecord<HubMailItem>(typeof s === "string" ? s : null))
      .filter((x): x is HubMailItem => x !== null);
  }

  async presenceBeat(token: string): Promise<number> {
    const expiresAt = Math.floor(this.clock() / 1000) + HUB_PRESENCE_TTL_S;
    await this.pipe([
      ["zadd", this.key("presence"), String(expiresAt), token],
      ["expire", this.key("presence"), String(HUB_PRESENCE_SET_TTL_S)],
    ]);
    return this.pruneAndCount();
  }

  async presenceLeave(token: string): Promise<void> {
    await this.pipe([["zrem", this.key("presence"), token]]);
  }

  private async pruneAndCount(): Promise<number> {
    const now = Math.floor(this.clock() / 1000);
    const res = await this.pipe([
      ["zremrangebyscore", this.key("presence"), "-inf", String(now)],
      ["zcard", this.key("presence")],
    ]);
    return Number(res[1]?.result ?? 0);
  }
}

/** Defensive parse: a corrupt stored blob reads as absent, never throws. */
function parseRecord<T>(raw: string | null): T | null {
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// In-memory fallback (tests / unconfigured env) — row-aging TTL, no timers.

interface Aged<T> {
  value: T;
  expiresAt: number; // epoch ms
}

export class MemoryHubStore implements HubStore {
  private peers = new Map<string, Aged<HubRegistration>>();
  private rooms = new Map<string, Aged<HubRoomHost>>();
  private inboxes = new Map<string, Aged<HubMailItem[]>>();
  private presence = new Map<string, number>(); // token -> expiresAt (epoch ms)

  constructor(private clock: () => number = Date.now) {}

  private live<T>(entry: Aged<T> | undefined): T | null {
    return entry && entry.expiresAt > this.clock() ? entry.value : null;
  }

  async getPeer(peerId: string): Promise<HubRegistration | null> {
    return this.live(this.peers.get(peerId));
  }

  async setPeer(peerId: string, record: HubRegistration): Promise<void> {
    this.peers.set(peerId, { value: record, expiresAt: this.clock() + HUB_PEER_TTL_S * 1000 });
  }

  async getRoom(roomId: string): Promise<HubRoomHost | null> {
    return this.live(this.rooms.get(roomId));
  }

  async setRoom(roomId: string, record: HubRoomHost, nx: boolean): Promise<boolean> {
    if (nx && this.live(this.rooms.get(roomId)) !== null) return false;
    this.rooms.set(roomId, { value: record, expiresAt: this.clock() + HUB_ROOM_TTL_S * 1000 });
    return true;
  }

  async appendInbox(items: HubMailItem[]): Promise<void> {
    const now = this.clock();
    for (const to of new Set(items.map((it) => it.to))) {
      const existing = this.live(this.inboxes.get(to)) ?? [];
      const merged = [...existing, ...items.filter((it) => it.to === to)];
      this.inboxes.set(to, {
        value: merged.slice(-HUB_INBOX_MAX_PER_BOX),
        expiresAt: now + HUB_INBOX_TTL_S * 1000,
      });
    }
  }

  async drainInbox(peerId: string): Promise<HubMailItem[]> {
    const items = this.live(this.inboxes.get(peerId)) ?? [];
    this.inboxes.delete(peerId);
    return items;
  }

  async presenceBeat(token: string): Promise<number> {
    this.presence.set(token, this.clock() + HUB_PRESENCE_TTL_S * 1000);
    this.prunePresence();
    return this.presence.size;
  }

  async presenceLeave(token: string): Promise<void> {
    this.presence.delete(token);
  }

  private prunePresence(): void {
    const now = this.clock();
    for (const [token, expiresAt] of this.presence) {
      if (expiresAt <= now) this.presence.delete(token);
    }
  }
}

// ---------------------------------------------------------------------------
// Factory + test injection point (mirrors store.ts getStore / context init).

/** No injected store and DM1_UPSTASH_* absent. Routes map this to 503 (fail
 *  closed) so production never silently serves from memory; the reference 503s
 *  the same way. Tests inject MemoryHubStore explicitly via setHubStore. */
export class HubNotConfiguredError extends Error {
  constructor() {
    super("hub storage not configured");
    this.name = "HubNotConfiguredError";
  }
}

let singleton: HubStore | null = null;

/** Env-driven hub store. The injectable clock keeps TTL row-aging coherent
 *  with the route context clock (never Date.now() directly in routes). An
 *  explicitly injected store (tests) wins; otherwise Upstash when configured;
 *  otherwise throws HubNotConfiguredError (NOT a memory fallback). */
export function getHubStore(clock: () => number = Date.now): HubStore {
  if (singleton) return singleton;
  const url = process.env.DM1_UPSTASH_REDIS_REST_URL;
  const token = process.env.DM1_UPSTASH_REDIS_REST_TOKEN;
  if (url && token) {
    singleton = new UpstashHubStore(url, token, clock);
    return singleton;
  }
  throw new HubNotConfiguredError();
}

/** Replace the hub store — the test injection point (null resets to lazy). */
export function setHubStore(store: HubStore | null): void {
  singleton = store;
}
