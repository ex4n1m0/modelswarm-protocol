// Storage layer for the tracker.
//
// `TrackerStore` is the single interface every route talks to. Two
// implementations:
//   - MemoryStore: Maps, used in tests and when DATABASE_URL is absent.
//   - PgStore:     Postgres via `pg`, SQL mirroring migrations/0001_init.sql,
//                  constructed only when DATABASE_URL is set.
//
// Invariants (ADR-001 / msp-v1 §3.3):
//   - Every expiry is row-aging: a stored `expiresAt` compared against the
//     injected clock at read time. There is no timer, sweeper, or cron
//     anywhere in this file or its callers.
//   - All timestamps cross the interface as epoch milliseconds; PgStore
//     converts to/from timestamptz columns.
//   - Nothing that could resemble prompt/completion text is ever stored:
//     the only free-form strings are enumerated protocol metadata (ids,
//     multiaddrs, opaque rendezvous blobs, notice objects).

import type { Pool } from "pg";
import { CHALLENGE_PROMPT } from "@/lib/constants";

export type ProfileStatus = "candidate" | "active" | "deprecated";
export type CapacityClass = "cpu" | "gpu_entry" | "gpu_mid" | "gpu_high";
export type ChallengeOutcome = "open" | "passed" | "failed";

export interface Manifest {
  schema_version: 2;
  hf_repo: string;
  hf_revision: string;
  artifact_hashes: Array<{ path: string; sha256: string }>;
  tokenizer_hash: string;
  chat_template_hash: string;
  architecture_hash: string;
  quantization: { method: string; bits: number };
  runtime: { name: string; version: string; build_hash: string };
  decoding_abi_version: number;
  speculative_capabilities: string[];
  [key: string]: unknown; // closed by the zod schema at the edge; index kept for canonicalization
}

export interface Provenance {
  resolvedBy?: string;
  resolvedAt?: string;
  reviewedBy?: string;
  licenseEvidenceUrl?: string;
}

/** catalog/schema-v2.json ProfileRecord: manifest + mutable UI wrapper. */
export interface ProfileRecord {
  profileId: string;
  manifest: Manifest;
  displayName: string;
  status: ProfileStatus;
  provenance?: Provenance;
  createdAt: number;
}

export interface InstallationRecord {
  installationId: string;
  pubKeyB58: string;
  createdAt: number;
}

export interface DeviceAuthRecord {
  deviceCode: string;
  installationId: string;
  userCode: string;
  verifyUrl: string;
  expiresAt: number;
  approved: boolean;
}

export interface SessionRecord {
  token: string;
  installationId: string;
  createdAt: number;
  expiresAt: number;
}

export interface LeaseRecord {
  leaseId: string;
  installationId: string;
  peerId: string;
  createdAt: number;
  expiresAt: number;
  profiles: string[];
  addresses: string[];
  maxSlots: number;
  freeSlots: number;
  queueMs: number;
  draining: boolean;
  capacityClass: CapacityClass;
  auditEpoch: number;
  lastSeenAt: number;
}

export interface PeerView {
  peerId: string;
  addresses: string[];
  queueMs: number;
  freeSlots: number;
  leaseExpiresAt: number;
  lastSeenAt: number;
  capacityClass: CapacityClass;
}

export interface ChallengeRecord {
  challengeId: string;
  leaseId: string;
  profileId: string;
  prompt: string;
  issuedAt: number;
  deadlineAt: number;
  outcome: ChallengeOutcome;
  firstTokenMs: number | null;
  totalMs: number | null;
  completedAt: number | null;
}

/** EligibilityLease / capability token record (ADR-006 + ADR-012). */
export interface TokenRecord {
  nonce: string;
  leaseId: string;
  peerId: string;
  installationId: string;
  profileId: string;
  challengeId: string;
  canHost: boolean;
  canConsume: boolean;
  slots: number;
  capacityClass: CapacityClass;
  auditEpoch: number;
  issuedAt: number;
  /** Hard cap: issuing lease expiresAt + 60 s grace (msp-v1 §5). */
  expiresAt: number;
}

export type TokenAuthority =
  | { usable: true; record: TokenRecord }
  | { usable: false; reason: "unknown_token" | "token_expired" | "lease_expired"; record: TokenRecord | null };

export type Notice =
  | { type: "revoked_peers"; peerIds: string[] }
  | { type: "revoked_tokens"; tokenNonces: string[] }
  | { type: "catalog_update"; catalogVersion: number };

export interface RendezvousItem {
  fromPeerId: string;
  kind: "offer" | "answer";
  payload: string;
  receivedAt: number;
}

export interface ReceiptRecord {
  receiptDigest: string;
  outcome: string;
  installationId: string;
  receivedAt: number;
}

export type InsertProfileResult = "inserted" | "exists_same" | "conflict";

export interface TrackerStore {
  /** Clock, shared with the request context (injectable; never Date.now directly). */
  now(): number;

  // -- installations & device enrollment -----------------------------------
  upsertInstallation(rec: InstallationRecord): Promise<void>;
  getInstallation(installationId: string): Promise<InstallationRecord | null>;

  createDeviceAuth(rec: DeviceAuthRecord): Promise<void>;
  getDeviceAuth(deviceCode: string): Promise<DeviceAuthRecord | null>;
  /** Ops-runbook stand-in (approval hook); test-only in Phase B. */
  approveDevice(deviceCode: string): Promise<boolean>;

  createSession(rec: SessionRecord): Promise<void>;
  getSession(token: string): Promise<SessionRecord | null>;

  // -- replay protection ----------------------------------------------------
  /** Single-use per installation within the window; true = fresh nonce. */
  consumeNonce(installationId: string, nonce: string, windowMs: number): Promise<boolean>;

  // -- leases ---------------------------------------------------------------
  createLease(rec: LeaseRecord): Promise<void>;
  getLease(leaseId: string): Promise<LeaseRecord | null>;
  updateLease(
    leaseId: string,
    patch: Partial<
      Pick<
        LeaseRecord,
        | "expiresAt"
        | "profiles"
        | "addresses"
        | "freeSlots"
        | "queueMs"
        | "draining"
        | "capacityClass"
        | "auditEpoch"
        | "lastSeenAt"
      >
    >,
  ): Promise<boolean>;
  /** Active (non-expired, non-draining) lease for a peer, newest first. */
  findActiveLeaseByPeer(peerId: string): Promise<LeaseRecord | null>;
  /** Active lease held by the signing installation (sender identity). */
  findActiveLeaseByInstallation(installationId: string): Promise<LeaseRecord | null>;
  /** Persist one heartbeat observation (metrics only, peer_observations). */
  recordObservation(rec: {
    leaseId: string;
    peerId: string;
    freeSlots: number;
    queueMs: number;
    observedAt: number;
  }): Promise<void>;
  /** Directory view: non-expired, non-draining peers, optionally by profile. */
  listPeers(filter: { profileId?: string; limit: number }): Promise<PeerView[]>;
  /** Distinct peers with a live, non-draining lease (public counter). */
  countOnlinePeers(): Promise<number>;
  /** Same population, per hosted profile id (public per-model counter). */
  countOnlinePeersByProfile(): Promise<Record<string, number>>;
  /** Authoritative audit epoch for a peer (0 default). Leases pin the epoch at
   *  registration; sweeps bump this counter only (ADR-012). */
  peerEpoch(peerId: string): Promise<number>;
  /** Bump the peer's authoritative audit epoch; returns the new value. */
  bumpPeerEpoch(peerId: string): Promise<number>;

  // -- hosting challenges ----------------------------------------------------
  createChallenge(rec: ChallengeRecord): Promise<void>;
  getChallenge(challengeId: string): Promise<ChallengeRecord | null>;
  getOpenChallenge(leaseId: string, profileId: string): Promise<ChallengeRecord | null>;
  passChallenge(
    challengeId: string,
    firstTokenMs: number,
    totalMs: number,
    completedAt: number,
  ): Promise<boolean>;
  getPassedChallenge(leaseId: string, profileId: string): Promise<ChallengeRecord | null>;

  // -- capability tokens / eligibility leases --------------------------------
  recordToken(rec: TokenRecord): Promise<void>;
  getTokenByNonce(nonce: string): Promise<TokenRecord | null>;
  /** Record-level validity (E3): token unexpired AND issuing lease within the
   *  60 s grace window. Row-aging only — no timers. */
  getTokenAuthority(nonce: string): Promise<TokenAuthority>;
  revokeToken(nonce: string): Promise<boolean>;

  // -- catalog ----------------------------------------------------------------
  insertProfile(rec: ProfileRecord): Promise<InsertProfileResult>;
  getProfile(profileId: string): Promise<ProfileRecord | null>;
  listProfiles(status?: ProfileStatus): Promise<ProfileRecord[]>;
  promoteProfile(profileId: string): Promise<boolean>;
  catalogVersion(): Promise<number>;

  // -- notices (revocation propagation, delete-on-read per lease) -------------
  pushNotice(target: { all: true } | { leaseId: string }, notice: Notice): Promise<void>;
  drainNotices(leaseId: string): Promise<Notice[]>;

  // -- rendezvous mailbox ------------------------------------------------------
  pushRendezvous(item: RendezvousItem & { toPeerId: string }): Promise<void>;
  drainRendezvous(toPeerId: string): Promise<Array<RendezvousItem>>;

  // -- receipts -----------------------------------------------------------------
  recordReceipt(rec: ReceiptRecord): Promise<void>;
  getReceipt(receiptDigest: string): Promise<ReceiptRecord | null>;

  // -- rate-limit counters (persisted observability; the live limiter is the
  //    in-memory fixed-window buckets per msp-v1 §3.4) -------------------------
  bumpRateCounter(bucketKey: string, windowStart: number): Promise<number>;

  // -- blocked peers -------------------------------------------------------------
  blockPeer(peerId: string, reason: string): Promise<void>;
  isPeerBlocked(peerId: string): Promise<boolean>;
  listBlockedPeers(): Promise<string[]>;

  // -- session authorizations (Phase B metadata endpoint) ------------------------
  recordSessionAuthorization(rec: {
    sessionId: string;
    peerIds: string[];
    profileId: string;
    mode: string;
    createdAt: number;
    expiresAt: number;
  }): Promise<void>;
}

// ===========================================================================
// MemoryStore
// ===========================================================================

export class MemoryStore implements TrackerStore {
  private installations = new Map<string, InstallationRecord>();
  private deviceAuths = new Map<string, DeviceAuthRecord>();
  private sessions = new Map<string, SessionRecord>();
  private nonces = new Map<string, Map<string, number>>();
  private leases = new Map<string, LeaseRecord>();
  private challenges = new Map<string, ChallengeRecord>();
  private tokens = new Map<string, TokenRecord>();
  private profiles = new Map<string, ProfileRecord>();
  private notices = new Map<string, Notice[]>();
  private rendezvous = new Map<string, Array<RendezvousItem>>();
  private receipts = new Map<string, ReceiptRecord>();
  private observations: Array<{
    leaseId: string;
    peerId: string;
    freeSlots: number;
    queueMs: number;
    observedAt: number;
  }> = [];
  private rateCounters = new Map<string, number>();
  private blocked = new Map<string, string>();
  private auditEpochs = new Map<string, number>();
  private sessionAuthorizations: Array<{
    sessionId: string;
    peerIds: string[];
    profileId: string;
    mode: string;
    createdAt: number;
    expiresAt: number;
  }> = [];
  private version = 1;

  constructor(private clock: () => number = Date.now) {}

  now(): number {
    return this.clock();
  }

  async upsertInstallation(rec: InstallationRecord): Promise<void> {
    this.installations.set(rec.installationId, { ...rec });
  }

  async getInstallation(installationId: string): Promise<InstallationRecord | null> {
    return this.installations.get(installationId) ?? null;
  }

  async createDeviceAuth(rec: DeviceAuthRecord): Promise<void> {
    this.deviceAuths.set(rec.deviceCode, { ...rec });
  }

  async getDeviceAuth(deviceCode: string): Promise<DeviceAuthRecord | null> {
    return this.deviceAuths.get(deviceCode) ?? null;
  }

  async approveDevice(deviceCode: string): Promise<boolean> {
    const rec = this.deviceAuths.get(deviceCode);
    if (!rec) return false;
    rec.approved = true;
    return true;
  }

  async createSession(rec: SessionRecord): Promise<void> {
    this.sessions.set(rec.token, { ...rec });
  }

  async getSession(token: string): Promise<SessionRecord | null> {
    return this.sessions.get(token) ?? null;
  }

  async consumeNonce(installationId: string, nonce: string, windowMs: number): Promise<boolean> {
    const now = this.clock();
    let seen = this.nonces.get(installationId);
    if (!seen) {
      seen = new Map();
      this.nonces.set(installationId, seen);
    }
    // Row-aging eviction: drop entries outside the replay window.
    for (const [n, ts] of seen) {
      if (now - ts > windowMs) seen.delete(n);
    }
    if (seen.has(nonce)) return false;
    seen.set(nonce, now);
    return true;
  }

  async createLease(rec: LeaseRecord): Promise<void> {
    this.leases.set(rec.leaseId, { ...rec });
  }

  async getLease(leaseId: string): Promise<LeaseRecord | null> {
    const rec = this.leases.get(leaseId);
    return rec ? { ...rec } : null;
  }

  async updateLease(
    leaseId: string,
    patch: Partial<
      Pick<
        LeaseRecord,
        | "expiresAt"
        | "profiles"
        | "addresses"
        | "freeSlots"
        | "queueMs"
        | "draining"
        | "capacityClass"
        | "auditEpoch"
        | "lastSeenAt"
      >
    >,
  ): Promise<boolean> {
    const rec = this.leases.get(leaseId);
    if (!rec) return false;
    Object.assign(rec, patch);
    return true;
  }

  private isLive(rec: LeaseRecord, now: number): boolean {
    return rec.expiresAt > now && !rec.draining;
  }

  async findActiveLeaseByPeer(peerId: string): Promise<LeaseRecord | null> {
    const now = this.clock();
    let best: LeaseRecord | null = null;
    for (const rec of this.leases.values()) {
      if (rec.peerId === peerId && this.isLive(rec, now)) {
        if (!best || rec.createdAt > best.createdAt) best = rec;
      }
    }
    return best ? { ...best } : null;
  }

  async findActiveLeaseByInstallation(installationId: string): Promise<LeaseRecord | null> {
    const now = this.clock();
    let best: LeaseRecord | null = null;
    for (const rec of this.leases.values()) {
      if (rec.installationId === installationId && this.isLive(rec, now)) {
        if (!best || rec.createdAt > best.createdAt) best = rec;
      }
    }
    return best ? { ...best } : null;
  }

  async recordObservation(rec: {
    leaseId: string;
    peerId: string;
    freeSlots: number;
    queueMs: number;
    observedAt: number;
  }): Promise<void> {
    this.observations.push({ ...rec });
    if (this.observations.length > 10_000) this.observations.splice(0, this.observations.length - 10_000);
  }

  async listPeers(filter: { profileId?: string; limit: number }): Promise<PeerView[]> {
    const now = this.clock();
    const out: PeerView[] = [];
    for (const rec of this.leases.values()) {
      if (!this.isLive(rec, now)) continue;
      if (filter.profileId && !rec.profiles.includes(filter.profileId)) continue;
      out.push({
        peerId: rec.peerId,
        addresses: [...rec.addresses],
        queueMs: rec.queueMs,
        freeSlots: rec.freeSlots,
        leaseExpiresAt: rec.expiresAt,
        lastSeenAt: rec.lastSeenAt,
        capacityClass: rec.capacityClass,
      });
    }
    out.sort((a, b) => (a.peerId < b.peerId ? -1 : 1));
    return out.slice(0, filter.limit);
  }

  async countOnlinePeers(): Promise<number> {
    const now = this.clock();
    const live = new Set<string>();
    for (const rec of this.leases.values()) {
      if (this.isLive(rec, now) && !live.has(rec.peerId)) live.add(rec.peerId);
    }
    return live.size;
  }

  async countOnlinePeersByProfile(): Promise<Record<string, number>> {
    const now = this.clock();
    const perProfile = new Map<string, Set<string>>();
    for (const rec of this.leases.values()) {
      if (!this.isLive(rec, now)) continue;
      for (const profile of rec.profiles) {
        const set = perProfile.get(profile) ?? new Set<string>();
        set.add(rec.peerId);
        perProfile.set(profile, set);
      }
    }
    const out: Record<string, number> = {};
    for (const [profile, peers] of perProfile) out[profile] = peers.size;
    return out;
  }

  async peerEpoch(peerId: string): Promise<number> {
    return this.auditEpochs.get(peerId) ?? 0;
  }

  async bumpPeerEpoch(peerId: string): Promise<number> {
    const next = (this.auditEpochs.get(peerId) ?? 0) + 1;
    this.auditEpochs.set(peerId, next);
    return next;
  }

  async createChallenge(rec: ChallengeRecord): Promise<void> {
    this.challenges.set(rec.challengeId, { ...rec });
  }

  async getChallenge(challengeId: string): Promise<ChallengeRecord | null> {
    const rec = this.challenges.get(challengeId);
    return rec ? { ...rec } : null;
  }

  async getOpenChallenge(leaseId: string, profileId: string): Promise<ChallengeRecord | null> {
    for (const rec of this.challenges.values()) {
      if (rec.leaseId === leaseId && rec.profileId === profileId && rec.outcome === "open") {
        return { ...rec };
      }
    }
    return null;
  }

  async passChallenge(
    challengeId: string,
    firstTokenMs: number,
    totalMs: number,
    completedAt: number,
  ): Promise<boolean> {
    const rec = this.challenges.get(challengeId);
    if (!rec) return false;
    rec.outcome = "passed";
    rec.firstTokenMs = firstTokenMs;
    rec.totalMs = totalMs;
    rec.completedAt = completedAt;
    return true;
  }

  async getPassedChallenge(leaseId: string, profileId: string): Promise<ChallengeRecord | null> {
    for (const rec of this.challenges.values()) {
      if (
        rec.leaseId === leaseId &&
        rec.profileId === profileId &&
        rec.outcome === "passed" &&
        this.clock() <= rec.deadlineAt + 60_000 // grace for record-level lookups
      ) {
        return { ...rec };
      }
    }
    return null;
  }

  async recordToken(rec: TokenRecord): Promise<void> {
    this.tokens.set(rec.nonce, { ...rec });
  }

  async getTokenByNonce(nonce: string): Promise<TokenRecord | null> {
    const rec = this.tokens.get(nonce);
    return rec ? { ...rec } : null;
  }

  async getTokenAuthority(nonce: string): Promise<TokenAuthority> {
    const now = this.clock();
    const rec = this.tokens.get(nonce);
    if (!rec) return { usable: false, reason: "unknown_token", record: null };
    if (now > rec.expiresAt) return { usable: false, reason: "token_expired", record: rec };
    const lease = this.leases.get(rec.leaseId);
    // E3: authority dies with the issuing lease, past the 60 s grace.
    if (!lease || now > lease.expiresAt + 60_000) {
      return { usable: false, reason: "lease_expired", record: rec };
    }
    return { usable: true, record: { ...rec } };
  }

  async revokeToken(nonce: string): Promise<boolean> {
    const existed = this.tokens.delete(nonce);
    if (existed) await this.pushNotice({ all: true }, { type: "revoked_tokens", tokenNonces: [nonce] });
    return existed;
  }

  async insertProfile(rec: ProfileRecord): Promise<InsertProfileResult> {
    const existing = this.profiles.get(rec.profileId);
    if (existing) {
      return JSON.stringify(sorted(existing.manifest)) === JSON.stringify(sorted(rec.manifest))
        ? "exists_same"
        : "conflict";
    }
    this.profiles.set(rec.profileId, { ...rec });
    this.version += 1;
    return "inserted";
  }

  async getProfile(profileId: string): Promise<ProfileRecord | null> {
    const rec = this.profiles.get(profileId);
    return rec ? { ...rec } : null;
  }

  async listProfiles(status?: ProfileStatus): Promise<ProfileRecord[]> {
    const out = [...this.profiles.values()]
      .filter((p) => !status || p.status === status)
      .sort((a, b) => (a.profileId < b.profileId ? -1 : 1));
    return out.map((p) => ({ ...p }));
  }

  async promoteProfile(profileId: string): Promise<boolean> {
    const rec = this.profiles.get(profileId);
    if (!rec) return false;
    if (rec.status !== "active") {
      rec.status = "active";
      this.version += 1;
    }
    return true;
  }

  async catalogVersion(): Promise<number> {
    return this.version;
  }

  async pushNotice(target: { all: true } | { leaseId: string }, notice: Notice): Promise<void> {
    const targets: string[] =
      "all" in target ? [...this.leases.keys()] : [target.leaseId];
    for (const leaseId of targets) {
      const queue = this.notices.get(leaseId) ?? [];
      queue.push(structuredClone(notice));
      this.notices.set(leaseId, queue);
    }
  }

  async drainNotices(leaseId: string): Promise<Notice[]> {
    const queue = this.notices.get(leaseId) ?? [];
    this.notices.set(leaseId, []);
    return queue;
  }

  async pushRendezvous(item: RendezvousItem & { toPeerId: string }): Promise<void> {
    const mailbox = this.rendezvous.get(item.toPeerId) ?? [];
    mailbox.push({ fromPeerId: item.fromPeerId, kind: item.kind, payload: item.payload, receivedAt: item.receivedAt });
    this.rendezvous.set(item.toPeerId, mailbox);
  }

  async drainRendezvous(toPeerId: string): Promise<RendezvousItem[]> {
    const mailbox = this.rendezvous.get(toPeerId) ?? [];
    this.rendezvous.set(toPeerId, []);
    return mailbox;
  }

  async recordReceipt(rec: ReceiptRecord): Promise<void> {
    this.receipts.set(rec.receiptDigest, { ...rec });
  }

  async getReceipt(receiptDigest: string): Promise<ReceiptRecord | null> {
    return this.receipts.get(receiptDigest) ?? null;
  }

  async bumpRateCounter(bucketKey: string, windowStart: number): Promise<number> {
    const key = `${windowStart}:${bucketKey}`;
    const next = (this.rateCounters.get(key) ?? 0) + 1;
    this.rateCounters.set(key, next);
    return next;
  }

  async blockPeer(peerId: string, reason: string): Promise<void> {
    this.blocked.set(peerId, reason);
    await this.pushNotice({ all: true }, { type: "revoked_peers", peerIds: [peerId] });
  }

  async isPeerBlocked(peerId: string): Promise<boolean> {
    return this.blocked.has(peerId);
  }

  async listBlockedPeers(): Promise<string[]> {
    return [...this.blocked.keys()].sort();
  }

  async recordSessionAuthorization(rec: {
    sessionId: string;
    peerIds: string[];
    profileId: string;
    mode: string;
    createdAt: number;
    expiresAt: number;
  }): Promise<void> {
    this.sessionAuthorizations.push({ ...rec });
  }
}

function sorted(value: unknown): unknown {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(sorted);
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value as Record<string, unknown>).sort()) {
    out[key] = sorted((value as Record<string, unknown>)[key]);
  }
  return out;
}

// ===========================================================================
// PgStore — SQL mirrors migrations/0001_init.sql. Instantiated only when
// DATABASE_URL is set; the `pg` module is imported lazily so builds without
// a database never pull it into the bundle.
// ===========================================================================

type AnyPool = Pool;

export class PgStore implements TrackerStore {
  private _pool: AnyPool | null = null;
  private poolPromise: Promise<AnyPool> | null = null;

  constructor(
    private connectionString: string,
    private clock: () => number = Date.now,
  ) {}

  now(): number {
    return this.clock();
  }

  private async pool(): Promise<AnyPool> {
    if (this._pool) return this._pool;
    if (!this.poolPromise) {
      this.poolPromise = (async () => {
        const pg = (await import("pg")).default;
        this._pool = new pg.Pool({ connectionString: this.connectionString, max: 5 });
        return this._pool;
      })();
    }
    return this.poolPromise;
  }

  private async query(sql: string, params: unknown[] = []): Promise<unknown[]> {
    const pool = await this.pool();
    const result = await pool.query(sql, params as never[]);
    return result.rows as unknown[];
  }

  private static iso(ms: number): string {
    return new Date(ms).toISOString();
  }

  async upsertInstallation(rec: InstallationRecord): Promise<void> {
    await this.query(
      `INSERT INTO installations (installation_id, public_key_b58, created_at)
       VALUES ($1, $2, $3)
       ON CONFLICT (installation_id) DO UPDATE SET public_key_b58 = EXCLUDED.public_key_b58`,
      [rec.installationId, rec.pubKeyB58, PgStore.iso(rec.createdAt)],
    );
    await this.query(
      `INSERT INTO peer_keys (installation_id, public_key_b58, added_at)
       VALUES ($1, $2, $3)
       ON CONFLICT (installation_id) DO UPDATE SET public_key_b58 = EXCLUDED.public_key_b58`,
      [rec.installationId, rec.pubKeyB58, PgStore.iso(rec.createdAt)],
    );
  }

  async getInstallation(installationId: string): Promise<InstallationRecord | null> {
    const rows = await this.query(
      `SELECT installation_id, public_key_b58, created_at FROM installations WHERE installation_id = $1`,
      [installationId],
    );
    const row = rows[0] as
      | { installation_id: string; public_key_b58: string; created_at: Date }
      | undefined;
    if (!row) return null;
    return {
      installationId: row.installation_id,
      pubKeyB58: row.public_key_b58,
      createdAt: row.created_at.getTime(),
    };
  }

  async createDeviceAuth(rec: DeviceAuthRecord): Promise<void> {
    await this.query(
      `INSERT INTO device_codes (device_code, installation_id, user_code, verify_url, approved, expires_at)
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [rec.deviceCode, rec.installationId, rec.userCode, rec.verifyUrl, rec.approved, PgStore.iso(rec.expiresAt)],
    );
  }

  async getDeviceAuth(deviceCode: string): Promise<DeviceAuthRecord | null> {
    const rows = await this.query(
      `SELECT device_code, installation_id, user_code, verify_url, approved, expires_at
       FROM device_codes WHERE device_code = $1`,
      [deviceCode],
    );
    const row = rows[0] as
      | {
          device_code: string;
          installation_id: string;
          user_code: string;
          verify_url: string;
          approved: boolean;
          expires_at: Date;
        }
      | undefined;
    if (!row) return null;
    return {
      deviceCode: row.device_code,
      installationId: row.installation_id,
      userCode: row.user_code,
      verifyUrl: row.verify_url,
      approved: row.approved,
      expiresAt: row.expires_at.getTime(),
    };
  }

  async approveDevice(deviceCode: string): Promise<boolean> {
    const rows = await this.query(
      `UPDATE device_codes SET approved = true WHERE device_code = $1 RETURNING device_code`,
      [deviceCode],
    );
    return rows.length > 0;
  }

  async createSession(rec: SessionRecord): Promise<void> {
    await this.query(
      `INSERT INTO sessions (token, installation_id, expires_at) VALUES ($1, $2, $3)`,
      [rec.token, rec.installationId, PgStore.iso(rec.expiresAt)],
    );
  }

  async getSession(token: string): Promise<SessionRecord | null> {
    const rows = await this.query(
      `SELECT token, installation_id, expires_at FROM sessions WHERE token = $1`,
      [token],
    );
    const row = rows[0] as { token: string; installation_id: string; expires_at: Date } | undefined;
    if (!row) return null;
    return {
      token: row.token,
      installationId: row.installation_id,
      createdAt: 0,
      expiresAt: row.expires_at.getTime(),
    };
  }

  async consumeNonce(installationId: string, nonce: string, windowMs: number): Promise<boolean> {
    const nowIso = PgStore.iso(this.clock());
    // Row-aging cleanup of the same installation's expired nonces.
    await this.query(
      `DELETE FROM nonces WHERE installation_id = $1 AND seen_at < $2`,
      [installationId, PgStore.iso(this.clock() - windowMs)],
    );
    try {
      await this.query(
        `INSERT INTO nonces (installation_id, nonce, seen_at) VALUES ($1, $2, $3)`,
        [installationId, nonce, nowIso],
      );
      return true;
    } catch {
      return false; // duplicate nonce inside the window
    }
  }

  async createLease(rec: LeaseRecord): Promise<void> {
    await this.query(
      `INSERT INTO peer_leases
         (lease_id, installation_id, peer_id, profiles, addresses, max_slots, free_slots,
          queue_ms, draining, capacity_class, audit_epoch, expires_at, last_heartbeat_at)
       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)`,
      [
        rec.leaseId,
        rec.installationId,
        rec.peerId,
        JSON.stringify(rec.profiles),
        JSON.stringify(rec.addresses),
        rec.maxSlots,
        rec.freeSlots,
        rec.queueMs,
        rec.draining,
        rec.capacityClass,
        rec.auditEpoch,
        PgStore.iso(rec.expiresAt),
        PgStore.iso(rec.lastSeenAt),
      ],
    );
  }

  private static rowToLease(row: Record<string, unknown>): LeaseRecord {
    return {
      leaseId: row.lease_id as string,
      installationId: row.installation_id as string,
      peerId: row.peer_id as string,
      createdAt: (row.created_at as Date).getTime(),
      expiresAt: (row.expires_at as Date).getTime(),
      profiles: row.profiles as string[],
      addresses: row.addresses as string[],
      maxSlots: row.max_slots as number,
      freeSlots: row.free_slots as number,
      queueMs: row.queue_ms as number,
      draining: row.draining as boolean,
      capacityClass: row.capacity_class as CapacityClass,
      auditEpoch: Number(row.audit_epoch),
      lastSeenAt: (row.last_heartbeat_at as Date).getTime(),
    };
  }

  async getLease(leaseId: string): Promise<LeaseRecord | null> {
    const rows = await this.query(
      `SELECT * FROM peer_leases WHERE lease_id = $1`,
      [leaseId],
    );
    const row = rows[0];
    return row ? PgStore.rowToLease(row as Record<string, unknown>) : null;
  }

  async updateLease(
    leaseId: string,
    patch: Partial<
      Pick<
        LeaseRecord,
        | "expiresAt"
        | "profiles"
        | "addresses"
        | "freeSlots"
        | "queueMs"
        | "draining"
        | "capacityClass"
        | "auditEpoch"
        | "lastSeenAt"
      >
    >,
  ): Promise<boolean> {
    const sets: string[] = [];
    const params: unknown[] = [];
    const add = (column: string, value: unknown) => {
      params.push(value);
      sets.push(`${column} = $${params.length}`);
    };
    if (patch.expiresAt !== undefined) add("expires_at", PgStore.iso(patch.expiresAt));
    if (patch.profiles !== undefined) add("profiles", JSON.stringify(patch.profiles));
    if (patch.addresses !== undefined) add("addresses", JSON.stringify(patch.addresses));
    if (patch.freeSlots !== undefined) add("free_slots", patch.freeSlots);
    if (patch.queueMs !== undefined) add("queue_ms", patch.queueMs);
    if (patch.draining !== undefined) add("draining", patch.draining);
    if (patch.capacityClass !== undefined) add("capacity_class", patch.capacityClass);
    if (patch.auditEpoch !== undefined) add("audit_epoch", patch.auditEpoch);
    if (patch.lastSeenAt !== undefined) add("last_heartbeat_at", PgStore.iso(patch.lastSeenAt));
    if (sets.length === 0) return false;
    params.push(leaseId);
    const rows = await this.query(
      `UPDATE peer_leases SET ${sets.join(", ")} WHERE lease_id = $${params.length} RETURNING lease_id`,
      params,
    );
    return rows.length > 0;
  }

  async findActiveLeaseByPeer(peerId: string): Promise<LeaseRecord | null> {
    const rows = await this.query(
      `SELECT * FROM peer_leases
       WHERE peer_id = $1 AND expires_at > $2 AND draining = false
       ORDER BY created_at DESC LIMIT 1`,
      [peerId, PgStore.iso(this.clock())],
    );
    const row = rows[0];
    return row ? PgStore.rowToLease(row as Record<string, unknown>) : null;
  }

  async findActiveLeaseByInstallation(installationId: string): Promise<LeaseRecord | null> {
    const rows = await this.query(
      `SELECT * FROM peer_leases
       WHERE installation_id = $1 AND expires_at > $2 AND draining = false
       ORDER BY created_at DESC LIMIT 1`,
      [installationId, PgStore.iso(this.clock())],
    );
    const row = rows[0];
    return row ? PgStore.rowToLease(row as Record<string, unknown>) : null;
  }

  async recordObservation(rec: {
    leaseId: string;
    peerId: string;
    freeSlots: number;
    queueMs: number;
    observedAt: number;
  }): Promise<void> {
    await this.query(
      `INSERT INTO peer_observations (lease_id, peer_id, free_slots, queue_ms, observed_at)
       VALUES ($1, $2, $3, $4, $5)`,
      [rec.leaseId, rec.peerId, rec.freeSlots, rec.queueMs, PgStore.iso(rec.observedAt)],
    );
  }

  async listPeers(filter: { profileId?: string; limit: number }): Promise<PeerView[]> {
    const rows = await this.query(
      `SELECT * FROM peer_leases
       WHERE expires_at > $1 AND draining = false
         AND ($2::text IS NULL OR profiles @> to_jsonb(ARRAY[$2::text]))
       ORDER BY peer_id ASC
       LIMIT $3`,
      [PgStore.iso(this.clock()), filter.profileId ?? null, filter.limit],
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      peerId: row.peer_id as string,
      addresses: row.addresses as string[],
      queueMs: row.queue_ms as number,
      freeSlots: row.free_slots as number,
      leaseExpiresAt: (row.expires_at as Date).getTime(),
      lastSeenAt: (row.last_heartbeat_at as Date).getTime(),
      capacityClass: row.capacity_class as CapacityClass,
    }));
  }

  async countOnlinePeers(): Promise<number> {
    const rows = await this.query(
      `SELECT COUNT(DISTINCT peer_id) AS n FROM peer_leases
       WHERE expires_at > $1 AND draining = false`,
      [PgStore.iso(this.clock())],
    );
    return Number((rows[0] as { n: string | number }).n);
  }

  async countOnlinePeersByProfile(): Promise<Record<string, number>> {
    const rows = await this.query(
      `SELECT p.profile AS profile_id, COUNT(DISTINCT l.peer_id) AS n
       FROM peer_leases l, jsonb_array_elements_text(l.profiles) AS p(profile)
       WHERE l.expires_at > $1 AND l.draining = false
       GROUP BY p.profile`,
      [PgStore.iso(this.clock())],
    );
    const out: Record<string, number> = {};
    for (const row of rows as Record<string, unknown>[]) {
      out[row.profile_id as string] = Number(row.n);
    }
    return out;
  }

  async peerEpoch(peerId: string): Promise<number> {
    const rows = await this.query(
      `SELECT audit_epoch FROM peer_audit_state WHERE peer_id = $1`,
      [peerId],
    );
    if (rows.length === 0) return 0;
    return Number((rows[0] as { audit_epoch: string | number }).audit_epoch);
  }

  async bumpPeerEpoch(peerId: string): Promise<number> {
    const rows = await this.query(
      `INSERT INTO peer_audit_state (peer_id, audit_epoch) VALUES ($1, 1)
       ON CONFLICT (peer_id)
       DO UPDATE SET audit_epoch = peer_audit_state.audit_epoch + 1, updated_at = now()
       RETURNING audit_epoch`,
      [peerId],
    );
    return Number((rows[0] as { audit_epoch: string | number }).audit_epoch);
  }

  async createChallenge(rec: ChallengeRecord): Promise<void> {
    await this.query(
      `INSERT INTO hosting_challenges (challenge_id, lease_id, profile_id, deadline_at, outcome)
       VALUES ($1, $2, $3, $4, $5)`,
      [rec.challengeId, rec.leaseId, rec.profileId, PgStore.iso(rec.deadlineAt), rec.outcome],
    );
  }

  private static rowToChallenge(row: Record<string, unknown>): ChallengeRecord {
    return {
      challengeId: row.challenge_id as string,
      leaseId: row.lease_id as string,
      profileId: row.profile_id as string,
      prompt: CHALLENGE_PROMPT,
      issuedAt: (row.issued_at as Date).getTime(),
      deadlineAt: (row.deadline_at as Date).getTime(),
      outcome: row.outcome as ChallengeOutcome,
      firstTokenMs: row.first_token_ms === null ? null : Number(row.first_token_ms),
      totalMs: row.total_ms === null ? null : Number(row.total_ms),
      completedAt: row.completed_at === null ? null : (row.completed_at as Date).getTime(),
    };
  }

  async getChallenge(challengeId: string): Promise<ChallengeRecord | null> {
    const rows = await this.query(
      `SELECT * FROM hosting_challenges WHERE challenge_id = $1`,
      [challengeId],
    );
    const row = rows[0];
    return row ? PgStore.rowToChallenge(row as Record<string, unknown>) : null;
  }

  async getOpenChallenge(leaseId: string, profileId: string): Promise<ChallengeRecord | null> {
    const rows = await this.query(
      `SELECT * FROM hosting_challenges
       WHERE lease_id = $1 AND profile_id = $2 AND outcome = 'open'
       ORDER BY issued_at DESC LIMIT 1`,
      [leaseId, profileId],
    );
    const row = rows[0];
    return row ? PgStore.rowToChallenge(row as Record<string, unknown>) : null;
  }

  async passChallenge(
    challengeId: string,
    firstTokenMs: number,
    totalMs: number,
    completedAt: number,
  ): Promise<boolean> {
    const rows = await this.query(
      `UPDATE hosting_challenges
       SET outcome = 'passed', first_token_ms = $1, total_ms = $2, completed_at = $3
       WHERE challenge_id = $4 RETURNING challenge_id`,
      [firstTokenMs, totalMs, PgStore.iso(completedAt), challengeId],
    );
    return rows.length > 0;
  }

  async getPassedChallenge(leaseId: string, profileId: string): Promise<ChallengeRecord | null> {
    const rows = await this.query(
      `SELECT * FROM hosting_challenges
       WHERE lease_id = $1 AND profile_id = $2 AND outcome = 'passed'
       ORDER BY completed_at DESC LIMIT 1`,
      [leaseId, profileId],
    );
    const row = rows[0];
    if (!row) return null;
    const rec = PgStore.rowToChallenge(row as Record<string, unknown>);
    if (this.clock() > rec.deadlineAt + 60_000) return null;
    return rec;
  }

  async recordToken(rec: TokenRecord): Promise<void> {
    await this.query(
      `INSERT INTO capability_tokens
         (nonce, lease_id, peer_id, installation_id, profile_id, challenge_id,
          can_host, can_consume, slots, capacity_class, audit_epoch, issued_at, expires_at)
       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)`,
      [
        rec.nonce,
        rec.leaseId,
        rec.peerId,
        rec.installationId,
        rec.profileId,
        rec.challengeId,
        rec.canHost,
        rec.canConsume,
        rec.slots,
        rec.capacityClass,
        rec.auditEpoch,
        PgStore.iso(rec.issuedAt),
        PgStore.iso(rec.expiresAt),
      ],
    );
  }

  private static rowToToken(row: Record<string, unknown>): TokenRecord {
    return {
      nonce: row.nonce as string,
      leaseId: row.lease_id as string,
      peerId: row.peer_id as string,
      installationId: row.installation_id as string,
      profileId: row.profile_id as string,
      challengeId: row.challenge_id as string,
      canHost: row.can_host as boolean,
      canConsume: row.can_consume as boolean,
      slots: row.slots as number,
      capacityClass: row.capacity_class as CapacityClass,
      auditEpoch: Number(row.audit_epoch),
      issuedAt: (row.issued_at as Date).getTime(),
      expiresAt: (row.expires_at as Date).getTime(),
    };
  }

  async getTokenByNonce(nonce: string): Promise<TokenRecord | null> {
    const rows = await this.query(
      `SELECT * FROM capability_tokens WHERE nonce = $1`,
      [nonce],
    );
    const row = rows[0];
    return row ? PgStore.rowToToken(row as Record<string, unknown>) : null;
  }

  async getTokenAuthority(nonce: string): Promise<TokenAuthority> {
    const rows = await this.query(`SELECT * FROM capability_tokens WHERE nonce = $1`, [nonce]);
    const row = rows[0];
    if (!row) return { usable: false, reason: "unknown_token", record: null };
    const rec = PgStore.rowToToken(row as Record<string, unknown>);
    const now = this.clock();
    if (now > rec.expiresAt) return { usable: false, reason: "token_expired", record: rec };
    const leaseRows = await this.query(
      `SELECT expires_at FROM peer_leases WHERE lease_id = $1`,
      [rec.leaseId],
    );
    const lease = leaseRows[0] as { expires_at: Date } | undefined;
    if (!lease || now > lease.expires_at.getTime() + 60_000) {
      return { usable: false, reason: "lease_expired", record: rec };
    }
    return { usable: true, record: rec };
  }

  async revokeToken(nonce: string): Promise<boolean> {
    const rows = await this.query(
      `DELETE FROM capability_tokens WHERE nonce = $1 RETURNING nonce`,
      [nonce],
    );
    if (rows.length === 0) return false;
    await this.pushNotice({ all: true }, { type: "revoked_tokens", tokenNonces: [nonce] });
    return true;
  }

  async insertProfile(rec: ProfileRecord): Promise<InsertProfileResult> {
    const existing = await this.getProfile(rec.profileId);
    if (existing) {
      return JSON.stringify(sorted(existing.manifest)) === JSON.stringify(sorted(rec.manifest))
        ? "exists_same"
        : "conflict";
    }
    await this.query(
      `INSERT INTO model_profiles (profile_id, manifest, display_name, status, provenance, created_at)
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [
        rec.profileId,
        JSON.stringify(rec.manifest),
        rec.displayName,
        rec.status,
        rec.provenance ? JSON.stringify(rec.provenance) : null,
        PgStore.iso(rec.createdAt),
      ],
    );
    await this.query(`UPDATE catalog_state SET version = version + 1 WHERE singleton = true`);
    return "inserted";
  }

  async getProfile(profileId: string): Promise<ProfileRecord | null> {
    const rows = await this.query(
      `SELECT profile_id, manifest, display_name, status, provenance, created_at
       FROM model_profiles WHERE profile_id = $1`,
      [profileId],
    );
    const row = rows[0] as
      | {
          profile_id: string;
          manifest: unknown;
          display_name: string;
          status: ProfileStatus;
          provenance: unknown;
          created_at: Date;
        }
      | undefined;
    if (!row) return null;
    return {
      profileId: row.profile_id,
      manifest: row.manifest as Manifest,
      displayName: row.display_name,
      status: row.status,
      provenance: (row.provenance as Provenance | null) ?? undefined,
      createdAt: row.created_at.getTime(),
    };
  }

  async listProfiles(status?: ProfileStatus): Promise<ProfileRecord[]> {
    const rows = await this.query(
      `SELECT profile_id, manifest, display_name, status, provenance, created_at
       FROM model_profiles WHERE ($1::text IS NULL OR status = $1::text)
       ORDER BY profile_id ASC`,
      [status ?? null],
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      profileId: row.profile_id as string,
      manifest: row.manifest as Manifest,
      displayName: row.display_name as string,
      status: row.status as ProfileStatus,
      provenance: (row.provenance as Provenance | null) ?? undefined,
      createdAt: (row.created_at as Date).getTime(),
    }));
  }

  async promoteProfile(profileId: string): Promise<boolean> {
    const rows = await this.query(
      `UPDATE model_profiles SET status = 'active' WHERE profile_id = $1 RETURNING profile_id`,
      [profileId],
    );
    if (rows.length === 0) return false;
    await this.query(`UPDATE catalog_state SET version = version + 1 WHERE singleton = true`);
    return true;
  }

  async catalogVersion(): Promise<number> {
    const rows = await this.query(`SELECT version FROM catalog_state WHERE singleton = true`);
    return Number((rows[0] as { version: string | number }).version);
  }

  async pushNotice(target: { all: true } | { leaseId: string }, notice: Notice): Promise<void> {
    if ("all" in target) {
      await this.query(
        `INSERT INTO peer_notices (lease_id, notice)
         SELECT lease_id, $1::jsonb FROM peer_leases`,
        [JSON.stringify(notice)],
      );
    } else {
      await this.query(
        `INSERT INTO peer_notices (lease_id, notice) VALUES ($1, $2)`,
        [target.leaseId, JSON.stringify(notice)],
      );
    }
  }

  async drainNotices(leaseId: string): Promise<Notice[]> {
    const rows = await this.query(
      `DELETE FROM peer_notices WHERE lease_id = $1 RETURNING notice`,
      [leaseId],
    );
    return rows.map((row) => (row as { notice: Notice }).notice);
  }

  async pushRendezvous(item: RendezvousItem & { toPeerId: string }): Promise<void> {
    await this.query(
      `INSERT INTO rendezvous (to_peer_id, from_peer_id, kind, payload)
       VALUES ($1, $2, $3, $4)`,
      [item.toPeerId, item.fromPeerId, item.kind, item.payload],
    );
  }

  async drainRendezvous(toPeerId: string): Promise<RendezvousItem[]> {
    const rows = await this.query(
      `DELETE FROM rendezvous WHERE to_peer_id = $1 RETURNING from_peer_id, kind, payload, received_at`,
      [toPeerId],
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      fromPeerId: row.from_peer_id as string,
      kind: row.kind as "offer" | "answer",
      payload: row.payload as string,
      receivedAt: (row.received_at as Date).getTime(),
    }));
  }

  async recordReceipt(rec: ReceiptRecord): Promise<void> {
    await this.query(
      `INSERT INTO job_receipts (receipt_digest, outcome, installation_id, received_at)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT (receipt_digest) DO NOTHING`,
      [rec.receiptDigest, rec.outcome, rec.installationId, PgStore.iso(rec.receivedAt)],
    );
  }

  async getReceipt(receiptDigest: string): Promise<ReceiptRecord | null> {
    const rows = await this.query(
      `SELECT receipt_digest, outcome, installation_id, received_at FROM job_receipts
       WHERE receipt_digest = $1`,
      [receiptDigest],
    );
    const row = rows[0] as
      | { receipt_digest: string; outcome: string; installation_id: string; received_at: Date }
      | undefined;
    if (!row) return null;
    return {
      receiptDigest: row.receipt_digest,
      outcome: row.outcome,
      installationId: row.installation_id,
      receivedAt: row.received_at.getTime(),
    };
  }

  async bumpRateCounter(bucketKey: string, windowStart: number): Promise<number> {
    const rows = await this.query(
      `INSERT INTO rate_counters (bucket_key, window_started_at, hits) VALUES ($1, $2, 1)
       ON CONFLICT (bucket_key, window_started_at)
       DO UPDATE SET hits = rate_counters.hits + 1 RETURNING hits`,
      [bucketKey, PgStore.iso(windowStart)],
    );
    return Number((rows[0] as { hits: number }).hits);
  }

  async blockPeer(peerId: string, reason: string): Promise<void> {
    await this.query(
      `INSERT INTO blocked_peers (peer_id, reason) VALUES ($1, $2)
       ON CONFLICT (peer_id) DO UPDATE SET reason = EXCLUDED.reason`,
      [peerId, reason],
    );
    await this.pushNotice({ all: true }, { type: "revoked_peers", peerIds: [peerId] });
  }

  async isPeerBlocked(peerId: string): Promise<boolean> {
    const rows = await this.query(`SELECT 1 FROM blocked_peers WHERE peer_id = $1`, [peerId]);
    return rows.length > 0;
  }

  async listBlockedPeers(): Promise<string[]> {
    const rows = await this.query(`SELECT peer_id FROM blocked_peers ORDER BY peer_id ASC`);
    return (rows as Record<string, unknown>[]).map((row) => row.peer_id as string);
  }

  async recordSessionAuthorization(rec: {
    sessionId: string;
    peerIds: string[];
    profileId: string;
    mode: string;
    createdAt: number;
    expiresAt: number;
  }): Promise<void> {
    await this.query(
      `INSERT INTO session_authorizations (session_id, profile_id, mode, peer_ids, expires_at)
       VALUES ($1, $2, $3, $4, $5)`,
      [rec.sessionId, rec.profileId, rec.mode, JSON.stringify(rec.peerIds), PgStore.iso(rec.expiresAt)],
    );
  }
}

/** The challenge prompt is a fixed per-profile protocol constant (see
 *  lib/constants.ts); never stored as a column. */
export { CHALLENGE_PROMPT as CHALLENGE_PROMPT_DEFAULT } from "@/lib/constants";

// ---------------------------------------------------------------------------
// Factory

export function getStore(clock: () => number = Date.now): TrackerStore {
  const url = process.env.DATABASE_URL;
  if (url) return new PgStore(url, clock);
  return new MemoryStore(clock);
}
