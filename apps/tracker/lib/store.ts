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
import { buildChallengePrompt } from "@/lib/challenge";
import {
  EXPIRED_ROW_GRACE_MS,
  OBSERVATION_MAX_PER_PEER,
  OBSERVATION_TTL_MS,
} from "@/lib/constants";

// Bounded rendezvous mailboxes (store-invariant extension 2026-10-08):
// rows for peers that never drain are pruned at WRITE time — TTL first,
// then a per-mailbox cap keeping the most recent items.
export const RENDEZVOUS_TTL_MS = 24 * 60 * 60 * 1000;
export const RENDEZVOUS_MAX_PER_MAILBOX = 64;

export type ProfileStatus = "candidate" | "active" | "deprecated";
export type CapacityClass = "cpu" | "gpu_entry" | "gpu_mid" | "gpu_high";
export type ChallengeOutcome = "open" | "passed" | "failed";

export interface LocalManifest {
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

/** ADR-040 cloud profile manifest: provider + exact model id + policy is
 *  the exactness (no artifact exists to hash). */
export interface CloudManifest {
  schema_version: 2;
  family: "cloud";
  provider: "deepseek" | "openrouter";
  model: string;
  context_tokens: number;
  max_output_tokens: number;
  wire: "openai-chat-v1";
  [key: string]: unknown;
}

export type Manifest = LocalManifest | CloudManifest;

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
  /** Claim handshake (migration 0010, ADR-013 P2 gate (a)): the account
   *  user who typed the userCode, the label they chose, and whether the
   *  DEVICE has confirmed. claimedByUserId set + claimConfirmedAt null =
   *  pending claim — binding happens only at confirm. All null on rows the
   *  admin/auto-approve paths approved without a claim. */
  claimedByUserId: string | null;
  claimedLabel: string | null;
  claimedAt: number | null;
  claimConfirmedAt: number | null;
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
  /** ADR-012 suspension forensics: which class ended a 'failed' challenge. */
  failureClass?: "timeout" | "invalid" | null;
}

/** Peer-keyed suspension state (ADR-012; migration 0005). */
export interface SuspensionRecord {
  peerId: string;
  untilAt: number;
  reason: string;
  suspensionCount: number;
  prevMaxSlots: number;
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
  | { type: "catalog_update"; catalogVersion: number }
  /** ADR-012 suspension propagation (msp-v1 §3.3; migration 0005). */
  | { type: "suspended"; untilAt: string; reason: string };

export interface RendezvousItem {
  fromPeerId: string;
  kind: "offer" | "answer";
  payload: string;
  receivedAt: number;
}

export interface GrantedAccountingRow {
  receiptDigest: string;
  peerId: string;
  installationId: string;
  profileId: string;
  endpointSeconds: number;
  verifiedTokens: number;
  grantedAt: number;
}

export interface ReceiptRecord {
  receiptDigest: string;
  outcome: string;
  installationId: string;
  receivedAt: number;
}

export type InsertProfileResult = "inserted" | "exists_same" | "conflict";

/** Community model request (ADR-023) — artifact pointers only, never catalog
 * content; the owner resolves each into a candidate profile via the
 * resolver script and the existing promote flow. */
export interface ModelRequestRecord {
  id: number;
  artifactSha256: string;
  hfRepo: string;
  hfRevision: string;
  artifactPath: string;
  artifactBytes: number;
  quantMethod: string;
  quantBits: number;
  displayName: string;
  note: string;
  requestedFrom: string;
  createdAt: number;
  resolvedAt: number | null;
  /** "" while open; "promoted" | "rejected" after the owner acts. */
  resolution: "" | "promoted" | "rejected";
}

export type InsertModelRequestResult = "inserted" | "duplicate";

// -- website accounts (migration 0008) ----------------------------------------

/** Website account user: email + self-describing scrypt password hash. */
export interface AccountUserRecord {
  userId: string;
  email: string;
  passwordHash: string;
  displayName: string;
  createdAt: number;
}

/** Server session behind the msw_session cookie (14-day TTL since P2 gate
 *  (b); row-aging). */
export interface UserSessionRecord {
  token: string;
  userId: string;
  createdAt: number;
  expiresAt: number;
}

export type AccountMachineStatus = "active" | "revoked";

/** One account↔installation binding row; approvedAt is null only for rows
 *  written by hand (dashboard approval always sets it). lastSeenAt is the
 *  signed machine heartbeat's touch (migration 0009) — null while the
 *  binding has never been seen (fresh binds reset it; row-aging display
 *  only, no timers). signalingDeviceId (migration 0011) maps the machine to
 *  its rd15 signaling mailbox for in-app connect — null while sharing is
 *  off or no heartbeat carried a mapping. label is null until the owner
 *  names the machine or a heartbeat auto-populates it from the display
 *  name (only while NULL — renames win). */
export interface AccountMachineRecord {
  userId: string;
  installationId: string;
  label: string | null;
  status: AccountMachineStatus;
  addedAt: number;
  approvedAt: number | null;
  lastSeenAt: number | null;
  signalingDeviceId: string | null;
}

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
  /** The /verify page hook: approve by the user-visible pairing code. */
  approveDeviceByUserCode(userCode: string): Promise<boolean>;

  createSession(rec: SessionRecord): Promise<void>;
  getSession(token: string): Promise<SessionRecord | null>;
  /** Distinct installations holding an enrollment session ever — the
   *  auto-approval cap basis (bounded Sybil exposure). */
  countEnrolledInstallations(): Promise<number>;

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
  /** Increment + return the download counter for one installer file. */
  bumpDownloadCount(file: string): Promise<number>;
  /** All download counters (public, for the website buttons). */
  downloadCounts(): Promise<Record<string, number>>;
  /** Owner directive 2026-10-17: the unified CLIENT counter — every
   *  installer download (any OS) AND every self-update artifact fetch
   *  counts as exactly one client served (updates pull the versioned
   *  artifact through the same counted route). */
  clientServesTotal(): Promise<number>;
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
  // -- ADR-012 suspension (migration 0005) -----------------------------------
  /** Marks one open challenge failed with its class (invalid completions). */
  failChallenge(challengeId: string, failureClass: "timeout" | "invalid", at: number): Promise<boolean>;
  /** Row-aging sweep: open challenges past deadline become failed('timeout'). */
  failExpiredChallenges(leaseId: string, now: number): Promise<number>;
  /** The peer's failed challenges inside [since, now] (across ALL leases). */
  recentChallengeFailures(
    peerId: string,
    sinceMs: number,
    nowMs: number,
  ): Promise<{ failureClass: "timeout" | "invalid"; at: number }[]>;
  getSuspension(peerId: string): Promise<SuspensionRecord | null>;
  /** Persists the suspension AND clamps the peer's lease rows to 0 slots. */
  applySuspension(state: SuspensionRecord, now: number): Promise<void>;
  /** Aged-out suspension: restores slots; KEEPS the row (escalation
   *  memory) with until_at in the past until redemption clears it. */
  recoverSuspensionIfDue(peerId: string, now: number): Promise<void>;
  /** Full redemption / aged-out memory: the row is deleted entirely. */
  clearSuspension(peerId: string, now: number): Promise<void>;
  /** The peer's newest passed challenge (redemption evidence), if any. */
  lastChallengePassAt(peerId: string): Promise<number | null>;

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
  /** Lifecycle status lever (admin/catalog/status): content stays
   *  immutable (ADR-005) — status is candidate|active|deprecated. */
  setProfileStatus(profileId: string, status: ProfileStatus): Promise<boolean>;
  promoteProfile(profileId: string): Promise<boolean>;
  catalogVersion(): Promise<number>;

  // -- community model requests (ADR-023) --------------------------------------
  /** Insert a request; dedupes on artifactSha256 ("duplicate" when the exact
   *  artifact was already requested). Idempotent from the client's view. */
  insertModelRequest(
    rec: Omit<ModelRequestRecord, "id" | "createdAt" | "resolvedAt" | "resolution">,
  ): Promise<InsertModelRequestResult>;
  /** Open requests first (oldest first), then resolved — the owner's queue. */
  listModelRequests(): Promise<ModelRequestRecord[]>;
  /** Mark a request resolved; false when unknown or already resolved. */
  resolveModelRequest(id: number, resolution: "promoted" | "rejected"): Promise<boolean>;

  // -- notices (revocation propagation, delete-on-read per lease) -------------
  pushNotice(target: { all: true } | { leaseId: string }, notice: Notice): Promise<void>;
  drainNotices(leaseId: string): Promise<Notice[]>;

  // -- rendezvous mailbox ------------------------------------------------------
  pushRendezvous(item: RendezvousItem & { toPeerId: string }): Promise<void>;
  drainRendezvous(toPeerId: string): Promise<Array<RendezvousItem>>;

  // -- receipts -----------------------------------------------------------------
  recordReceipt(rec: ReceiptRecord): Promise<void>;
  /** ADR-030 §3: granted verified-work accounting rows — appended ONLY by
   * intake-validated ReceiptV2 processing; (receipt_digest, peer_id) is
   * the replay-proof key (re-posts are no-ops). */
  recordGrantedAccounting(rows: GrantedAccountingRow[]): Promise<void>;
  /** Decayed granted counters for one peer (ADR-030 §5 half-life form):
   * each row contributes value * 0.5^(age/halfLife). Plus raw totals. */
  grantedCounters(
    peerId: string,
    halfLifeSecs: number,
  ): Promise<{
    endpointSecondsDecayed: number;
    verifiedTokensDecayed: number;
    endpointSecondsRaw: number;
    verifiedTokensRaw: number;
    rows: number;
  }>;
  getReceipt(receiptDigest: string): Promise<ReceiptRecord | null>;
  /** ADR-041 consumed side: one row per (installation, receipt) — the
   *  posting installation IS the requester. Re-posts are no-ops. */
  recordConsumption(row: { installationId: string; receiptDigest: string; tokens: number; at: number }): Promise<void>;
  /** Decayed consumed tokens for one installation (same half-life form
   *  as grantedCounters — display/accounting continuity). */
  consumedTokensDecayed(installationId: string, halfLifeSecs: number): Promise<{ decayed: number; raw: number; rows: number }>;
  /** ADR-041 (capacity rule): plain tokens consumed in the trailing
   *  window — the rate gate's input. */
  consumedInWindow(installationId: string, windowSecs: number): Promise<number>;
  /** ADR-042 §2 (measured capacity anchor): tokens served and endpoint
   *  seconds accumulated by one PEER in the trailing window, from
   *  intake-validated granted-accounting rows — tokens/seconds is the
   *  machine's own sustained serving throughput (utilization-independent). */
  servedThroughputInWindow(
    peerId: string,
    windowSecs: number,
  ): Promise<{ tokens: number; seconds: number }>;

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

  // -- website accounts (migration 0008) -----------------------------------------
  /** Insert an account user; duplicates (unique email) throw — routes check
   *  getAccountUserByEmail first and map the throw to 409 email_taken. */
  createAccountUser(rec: AccountUserRecord): Promise<void>;
  getAccountUserByEmail(email: string): Promise<AccountUserRecord | null>;
  getAccountUserById(userId: string): Promise<AccountUserRecord | null>;
  /** Admin password reset: swap the hash AND delete every user_sessions
   *  row of that user — a reset that left a stolen 30-day cookie alive
   *  would defeat its purpose. False when the user is unknown. */
  setAccountUserPassword(userId: string, passwordHash: string, now: number): Promise<boolean>;
  createUserSession(rec: UserSessionRecord): Promise<void>;
  /** Raw row read; expiry is judged by the CALLER against the clock
   *  (mirroring getSession usage). */
  getUserSession(token: string): Promise<UserSessionRecord | null>;
  deleteUserSession(token: string): Promise<void>;
  /** Gate (b) affordance: delete EVERY session row of the user ("sign out
   *  everywhere"). Also the shared core of setAccountUserPassword's kill. */
  deleteAllUserSessions(userId: string): Promise<void>;
  listAccountMachines(userId: string): Promise<AccountMachineRecord[]>;
  /** The single active owner of an installation, if any. */
  getActiveAccountForInstallation(installationId: string): Promise<{ userId: string } | null>;
  /** Create/refresh the binding as 'active' (flips the user's own revoked
   *  row back). "already_bound" when the installation is active under a
   *  DIFFERENT account — the caller maps that to 409 already_bound. label
   *  may be null (claim-handshake binds; heartbeat auto-populates later). */
  bindMachineToAccount(rec: {
    userId: string;
    installationId: string;
    label: string | null;
    now: number;
  }): Promise<"created" | "already_bound">;
  /** Revoke one of the user's machines (frees the installation for another
   *  account to bind). Re-binding ALWAYS goes through
   *  bindMachineToAccount's exclusivity check — there is deliberately no
   *  status='active' path here. False when unknown or not owned. */
  revokeAccountMachine(userId: string, installationId: string): Promise<boolean>;
  setAccountMachineLabel(userId: string, installationId: string, label: string): Promise<boolean>;
  /** Machine heartbeat contact (migrations 0009 + 0011): stamp lastSeenAt,
   *  set-or-clear signalingDeviceId (present means set — null clears, so a
   *  sharing-off machine drops out of the controllable directory; absent
   *  keeps the stored mapping for bare liveness payloads), and
   *  auto-populate a NULL label from the display name (renames win). True
   *  iff an active row was touched — no active binding (never bound, or
   *  revoked) is false, which the heartbeat route maps to 404
   *  unknown_machine. */
  updateMachineContact(
    installationId: string,
    contact: { signalingDeviceId?: string | null; displayName?: string },
    now: number,
  ): Promise<boolean>;
  /** Device pairing-code lookup for the dashboard approve flow (newest
   *  matching row, mirroring approveDeviceByUserCode's update set). */
  getDeviceAuthByUserCode(userCode: string): Promise<DeviceAuthRecord | null>;
  /** Claim handshake (gate (a)): the dashboard approve writes a PENDING
   *  claim — no bind. "already_claimed" when a DIFFERENT user's claim is
   *  pending (same user re-approving refreshes idempotently); "unknown"
   *  when no live code matches (caller keeps the no-liveness-oracle 404). */
  claimDeviceByUserCode(
    userCode: string,
    userId: string,
    label: string | null,
    now: number,
  ): Promise<"claimed" | "already_claimed" | "unknown">;
  /** Device-side confirm (accept=true: stamp claim_confirmed_at AND flip
   *  approved — the caller then binds via bindMachineToAccount) or deny
   *  (accept=false: clear the claim so the owner can re-approve). Returns
   *  the updated record, or null when the deviceCode is unknown. */
  resolveDeviceClaim(deviceCode: string, accept: boolean, now: number): Promise<DeviceAuthRecord | null>;
}

// ===========================================================================
// MemoryStore
// ===========================================================================

export class MemoryStore implements TrackerStore {
  private installations = new Map<string, InstallationRecord>();
  private deviceAuths = new Map<string, DeviceAuthRecord>();
  private sessions = new Map<string, SessionRecord>();
  private nonces = new Map<string, Map<string, number>>();
  private downloads = new Map<string, { count: number; lastAt: number }>();
  private leases = new Map<string, LeaseRecord>();
  private challenges = new Map<string, ChallengeRecord>();
  private suspensions = new Map<string, SuspensionRecord>();
  private tokens = new Map<string, TokenRecord>();
  private profiles = new Map<string, ProfileRecord>();
  private modelRequests = new Map<number, ModelRequestRecord>();
  private nextModelRequestId = 1;
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
  private accountUsers = new Map<string, AccountUserRecord>(); // key: userId
  private accountUsersByEmail = new Map<string, string>(); // email -> userId
  private userSessions = new Map<string, UserSessionRecord>();
  private accountMachines = new Map<string, AccountMachineRecord>(); // key: `${userId}:${installationId}`
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
    // Write-time prune (T5): codes that expired more than a day ago serve
    // nothing — neither approval nor the /verify queue can reach them.
    const cutoff = this.clock() - EXPIRED_ROW_GRACE_MS;
    for (const [code, auth] of this.deviceAuths) {
      if (auth.expiresAt < cutoff) this.deviceAuths.delete(code);
    }
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

  async approveDeviceByUserCode(userCode: string): Promise<boolean> {
    for (const rec of this.deviceAuths.values()) {
      if (rec.userCode === userCode) {
        rec.approved = true;
        return true;
      }
    }
    return false;
  }

  async getDeviceAuthByUserCode(userCode: string): Promise<DeviceAuthRecord | null> {
    // Newest matching row (Pg orders by created_at DESC; the record has no
    // createdAt, so the fixed-TTL expiresAt stands in) — the same population
    // approveDeviceByUserCode updates.
    let best: DeviceAuthRecord | null = null;
    for (const rec of this.deviceAuths.values()) {
      if (rec.userCode === userCode && (!best || rec.expiresAt > best.expiresAt)) best = rec;
    }
    return best ? { ...best } : null;
  }

  async claimDeviceByUserCode(
    userCode: string,
    userId: string,
    label: string | null,
    now: number,
  ): Promise<"claimed" | "already_claimed" | "unknown"> {
    let best: DeviceAuthRecord | null = null;
    for (const rec of this.deviceAuths.values()) {
      if (rec.userCode === userCode && (!best || rec.expiresAt > best.expiresAt)) best = rec;
    }
    if (!best || best.expiresAt <= this.clock()) return "unknown";
    if (best.claimedByUserId !== null && best.claimedByUserId !== userId) return "already_claimed";
    best.claimedByUserId = userId;
    best.claimedLabel = label;
    best.claimedAt = now;
    best.claimConfirmedAt = null; // same-user re-claim: back to pending
    return "claimed";
  }

  async resolveDeviceClaim(deviceCode: string, accept: boolean, now: number): Promise<DeviceAuthRecord | null> {
    const rec = this.deviceAuths.get(deviceCode);
    if (!rec) return null;
    if (accept) {
      // Confirm: the claim becomes binding-approved. The CALLER performs the
      // account_machines bind (needs the claiming user + label).
      rec.claimConfirmedAt = now;
      rec.approved = true;
    } else {
      // Deny: clear the claim; the code stays usable for a fresh approval.
      rec.claimedByUserId = null;
      rec.claimedLabel = null;
      rec.claimedAt = null;
      rec.claimConfirmedAt = null;
    }
    return { ...rec };
  }

  async createSession(rec: SessionRecord): Promise<void> {
    this.sessions.set(rec.token, { ...rec });
    // Write-time prune (T5), CENSUS-SAFE: expired sessions may be deleted
    // EXCEPT the newest row per installation — countEnrolledInstallations
    // counts DISTINCT installation_id over sessions (the
    // DEVICE_APPROVAL_CAP basis) and must stay cumulative, so each
    // installation keeps exactly one tombstone row at minimum.
    const cutoff = this.clock() - EXPIRED_ROW_GRACE_MS;
    // The newest expiry per installation is the tombstone that survives;
    // ties both survive (err toward keeping rows — the census is cumulative).
    const newestExpiryPerInstallation = new Map<string, number>();
    for (const s of this.sessions.values()) {
      const newest = newestExpiryPerInstallation.get(s.installationId);
      if (newest === undefined || s.expiresAt > newest) {
        newestExpiryPerInstallation.set(s.installationId, s.expiresAt);
      }
    }
    for (const [token, s] of this.sessions) {
      if (
        s.expiresAt < cutoff &&
        s.expiresAt !== newestExpiryPerInstallation.get(s.installationId)
      ) {
        this.sessions.delete(token);
      }
    }
  }

  async getSession(token: string): Promise<SessionRecord | null> {
    return this.sessions.get(token) ?? null;
  }

  async countEnrolledInstallations(): Promise<number> {
    return new Set([...this.sessions.values()].map((s) => s.installationId)).size;
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
    // Write-time prune (T5; rendezvous pattern): one row per heartbeat is
    // the highest-churn table and is read by nothing — bound it per peer
    // (newest N) plus a hard TTL, then keep the global cap as a backstop.
    const ttlCutoff = rec.observedAt - OBSERVATION_TTL_MS;
    this.observations = this.observations.filter((o) => o.observedAt >= ttlCutoff);
    const perPeer = this.observations.filter((o) => o.peerId === rec.peerId);
    if (perPeer.length >= OBSERVATION_MAX_PER_PEER) {
      const excess = perPeer.length - OBSERVATION_MAX_PER_PEER + 1;
      const drop = new Set(perPeer.slice(0, excess));
      this.observations = this.observations.filter((o) => !drop.has(o));
    }
    this.observations.push({ ...rec });
    if (this.observations.length > 10_000) this.observations.splice(0, this.observations.length - 10_000);
  }

  async listPeers(filter: { profileId?: string; limit: number }): Promise<PeerView[]> {
    const now = this.clock();
    const out: PeerView[] = [];
    for (const rec of this.leases.values()) {
      if (!this.isLive(rec, now)) continue;
      if (filter.profileId && !rec.profiles.includes(filter.profileId)) continue;
      // ADR-012: an actively suspended peer reports ZERO free slots no
      // matter what its heartbeats claim (the roster is the swarm's truth).
      const suspended = this.suspensions.get(rec.peerId);
      const freeSlots = suspended && suspended.untilAt > now ? 0 : rec.freeSlots;
      out.push({
        peerId: rec.peerId,
        addresses: [...rec.addresses],
        queueMs: rec.queueMs,
        freeSlots,
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

  async bumpDownloadCount(file: string): Promise<number> {
    const entry = this.downloads.get(file) ?? { count: 0, lastAt: this.clock() };
    entry.count += 1;
    entry.lastAt = this.clock();
    this.downloads.set(file, entry);
    return entry.count;
  }

  async downloadCounts(): Promise<Record<string, number>> {
    const out: Record<string, number> = {};
    for (const [file, entry] of this.downloads) out[file] = entry.count;
    return out;
  }

  async clientServesTotal(): Promise<number> {
    let total = 0;
    for (const entry of this.downloads.values()) total += entry.count;
    return total;
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

  async failChallenge(challengeId: string, failureClass: "timeout" | "invalid", at: number): Promise<boolean> {
    const rec = this.challenges.get(challengeId);
    if (!rec || rec.outcome !== "open") return false;
    rec.outcome = "failed";
    rec.failureClass = failureClass;
    rec.completedAt = failureClass === "invalid" ? at : null;
    return true;
  }

  async failExpiredChallenges(leaseId: string, now: number): Promise<number> {
    let marked = 0;
    for (const rec of this.challenges.values()) {
      if (rec.leaseId === leaseId && rec.outcome === "open" && now > rec.deadlineAt) {
        rec.outcome = "failed";
        rec.failureClass = "timeout";
        rec.completedAt = null;
        marked += 1;
      }
    }
    return marked;
  }

  async recentChallengeFailures(
    peerId: string,
    sinceMs: number,
    nowMs: number,
  ): Promise<{ failureClass: "timeout" | "invalid"; at: number }[]> {
    const leaseIds = new Set(
      [...this.leases.values()].filter((l) => l.peerId === peerId).map((l) => l.leaseId),
    );
    const out: { failureClass: "timeout" | "invalid"; at: number }[] = [];
    for (const rec of this.challenges.values()) {
      if (!leaseIds.has(rec.leaseId) || rec.outcome !== "failed") continue;
      // 'when': the deadline for ignored challenges, the completion for
      // invalid ones (matching the Pg SQL below).
      const at = rec.failureClass === "invalid" ? (rec.completedAt ?? rec.deadlineAt) : rec.deadlineAt;
      if (at >= sinceMs && at <= nowMs) {
        out.push({ failureClass: rec.failureClass ?? "timeout", at });
      }
    }
    return out;
  }

  async getSuspension(peerId: string): Promise<SuspensionRecord | null> {
    const rec = this.suspensions.get(peerId);
    return rec ? { ...rec } : null;
  }

  async applySuspension(state: SuspensionRecord, _now: number): Promise<void> {
    this.suspensions.set(state.peerId, { ...state });
    for (const rec of this.leases.values()) {
      if (rec.peerId === state.peerId && rec.maxSlots > 0) {
        rec.maxSlots = 0;
        rec.freeSlots = 0;
      }
    }
  }

  async recoverSuspensionIfDue(peerId: string, now: number): Promise<void> {
    const rec = this.suspensions.get(peerId);
    if (!rec || rec.untilAt > now) return;
    for (const lease of this.leases.values()) {
      if (lease.peerId === peerId && lease.maxSlots === 0) {
        lease.maxSlots = rec.prevMaxSlots;
        lease.freeSlots = Math.min(lease.freeSlots, rec.prevMaxSlots);
      }
    }
    // The row stays (suspension_count = escalation memory) with a spent
    // window; clearSuspension removes it on redemption or window-empty.
    rec.untilAt = now - 1;
  }

  async clearSuspension(peerId: string, _now: number): Promise<void> {
    const rec = this.suspensions.get(peerId);
    this.suspensions.delete(peerId);
    if (!rec) return;
    for (const lease of this.leases.values()) {
      if (lease.peerId === peerId && lease.maxSlots === 0) {
        lease.maxSlots = rec.prevMaxSlots;
        lease.freeSlots = Math.min(lease.freeSlots, rec.prevMaxSlots);
      }
    }
  }

  async lastChallengePassAt(peerId: string): Promise<number | null> {
    const leaseIds = new Set(
      [...this.leases.values()].filter((l) => l.peerId === peerId).map((l) => l.leaseId),
    );
    let newest: number | null = null;
    for (const rec of this.challenges.values()) {
      if (leaseIds.has(rec.leaseId) && rec.outcome === "passed" && rec.completedAt !== null) {
        newest = Math.max(newest ?? 0, rec.completedAt);
      }
    }
    return newest;
  }

  async recordToken(rec: TokenRecord): Promise<void> {
    this.tokens.set(rec.nonce, { ...rec });
    // Write-time prune (T5): one row per issuance; rows whose grace window
    // closed more than a day ago can never be authoritative again (E3).
    const cutoff = this.clock() - EXPIRED_ROW_GRACE_MS;
    for (const [nonce, token] of this.tokens) {
      if (token.expiresAt < cutoff) this.tokens.delete(nonce);
    }
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

  async setProfileStatus(profileId: string, status: ProfileStatus): Promise<boolean> {
    const rec = this.profiles.get(profileId);
    if (!rec) return false;
    rec.status = status;
    this.version += 1;
    return true;
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

  async insertModelRequest(
    rec: Omit<ModelRequestRecord, "id" | "createdAt" | "resolvedAt" | "resolution">,
  ): Promise<InsertModelRequestResult> {
    for (const existing of this.modelRequests.values()) {
      if (existing.artifactSha256 === rec.artifactSha256) return "duplicate";
    }
    this.modelRequests.set(this.nextModelRequestId, {
      ...rec,
      id: this.nextModelRequestId,
      createdAt: this.now(),
      resolvedAt: null,
      resolution: "",
    });
    this.nextModelRequestId += 1;
    return "inserted";
  }

  async listModelRequests(): Promise<ModelRequestRecord[]> {
    return [...this.modelRequests.values()]
      .sort((a, b) => {
        const aOpen = a.resolvedAt === null ? 0 : 1;
        const bOpen = b.resolvedAt === null ? 0 : 1;
        return aOpen - bOpen || a.createdAt - b.createdAt || a.id - b.id;
      })
      .map((r) => ({ ...r }));
  }

  async resolveModelRequest(id: number, resolution: "promoted" | "rejected"): Promise<boolean> {
    const rec = this.modelRequests.get(id);
    if (!rec || rec.resolvedAt !== null) return false;
    rec.resolvedAt = this.now();
    rec.resolution = resolution;
    return true;
  }

  async pushNotice(target: { all: true } | { leaseId: string }, notice: Notice): Promise<void> {
    const now = this.clock();
    // Write-time prune (T5): leases dead for over a day will never heartbeat
    // to drain a queue — drop their existing queues AND exclude them from
    // the {all:true} fan-out (one orphan row per dead lease otherwise).
    const deadCutoff = now - EXPIRED_ROW_GRACE_MS;
    if ("all" in target) {
      for (const [leaseId, lease] of this.leases) {
        if (lease.expiresAt < deadCutoff) this.notices.delete(leaseId);
      }
    }
    const targets: string[] =
      "all" in target
        ? [...this.leases.entries()]
            .filter(([, lease]) => lease.expiresAt >= deadCutoff)
            .map(([leaseId]) => leaseId)
        : [target.leaseId];
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
    // Bounded mailboxes: drop items older than the TTL and keep the most
    // recent RENDEZVOUS_MAX_PER_MAILBOX — an enrolled peer pushing 4 KiB
    // blobs at a mailbox that never drains must not grow memory unboundedly.
    const cutoff = item.receivedAt - RENDEZVOUS_TTL_MS;
    const mailbox = (this.rendezvous.get(item.toPeerId) ?? []).filter(
      (m) => m.receivedAt >= cutoff,
    );
    mailbox.push({ fromPeerId: item.fromPeerId, kind: item.kind, payload: item.payload, receivedAt: item.receivedAt });
    if (mailbox.length > RENDEZVOUS_MAX_PER_MAILBOX) {
      mailbox.splice(0, mailbox.length - RENDEZVOUS_MAX_PER_MAILBOX);
    }
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

  private granted = new Map<string, GrantedAccountingRow>();

  async recordGrantedAccounting(rows: GrantedAccountingRow[]): Promise<void> {
    for (const row of rows) {
      this.granted.set(`${row.receiptDigest}:${row.peerId}`, { ...row });
    }
  }

  async grantedCounters(
    peerId: string,
    halfLifeSecs: number,
  ): Promise<{
    endpointSecondsDecayed: number;
    verifiedTokensDecayed: number;
    endpointSecondsRaw: number;
    verifiedTokensRaw: number;
    rows: number;
  }> {
    let endpointSecondsDecayed = 0;
    let verifiedTokensDecayed = 0;
    let endpointSecondsRaw = 0;
    let verifiedTokensRaw = 0;
    let rows = 0;
    for (const row of this.granted.values()) {
      if (row.peerId !== peerId) continue;
      rows += 1;
      const ageSecs = Math.max(0, (this.now() - row.grantedAt) / 1000);
      const weight = Math.pow(0.5, ageSecs / halfLifeSecs);
      endpointSecondsDecayed += row.endpointSeconds * weight;
      verifiedTokensDecayed += row.verifiedTokens * weight;
      endpointSecondsRaw += row.endpointSeconds;
      verifiedTokensRaw += row.verifiedTokens;
    }
    return { endpointSecondsDecayed, verifiedTokensDecayed, endpointSecondsRaw, verifiedTokensRaw, rows };
  }

  private consumed = new Map<string, { installationId: string; receiptDigest: string; tokens: number; at: number }>();

  async recordConsumption(row: { installationId: string; receiptDigest: string; tokens: number; at: number }): Promise<void> {
    this.consumed.set(`${row.installationId}:${row.receiptDigest}`, { ...row });
  }

  async consumedInWindow(installationId: string, windowSecs: number): Promise<number> {
    const cutoff = this.now() - windowSecs * 1000;
    let total = 0;
    for (const row of this.consumed.values()) {
      if (row.installationId === installationId && row.at >= cutoff) total += row.tokens;
    }
    return total;
  }

  async servedThroughputInWindow(
    peerId: string,
    windowSecs: number,
  ): Promise<{ tokens: number; seconds: number }> {
    const cutoff = this.now() - windowSecs * 1000;
    let tokens = 0;
    let seconds = 0;
    for (const row of this.granted.values()) {
      if (row.peerId === peerId && row.grantedAt >= cutoff) {
        tokens += row.verifiedTokens;
        seconds += row.endpointSeconds;
      }
    }
    return { tokens, seconds };
  }

  async consumedTokensDecayed(
    installationId: string,
    halfLifeSecs: number,
  ): Promise<{ decayed: number; raw: number; rows: number }> {
    let decayed = 0;
    let raw = 0;
    let rows = 0;
    for (const row of this.consumed.values()) {
      if (row.installationId !== installationId) continue;
      rows += 1;
      const ageSecs = Math.max(0, (this.now() - row.at) / 1000);
      decayed += row.tokens * Math.pow(0.5, ageSecs / halfLifeSecs);
      raw += row.tokens;
    }
    return { decayed, raw, rows };
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
    // Write-time prune (T5): write-only rows with a 10-minute TTL; expired
    // rows can never be read usefully.
    this.sessionAuthorizations = this.sessionAuthorizations.filter((a) => a.expiresAt > this.clock());
    this.sessionAuthorizations.push({ ...rec });
  }

  async createAccountUser(rec: AccountUserRecord): Promise<void> {
    // Mirrors the Pg unique violation on email: duplicates throw, the route
    // checks first and maps the throw to 409 email_taken. (userId is a
    // 128-bit random id — collisions are not a practical path.)
    if (this.accountUsersByEmail.has(rec.email)) {
      throw new Error("email_taken");
    }
    this.accountUsers.set(rec.userId, { ...rec });
    this.accountUsersByEmail.set(rec.email, rec.userId);
  }

  async getAccountUserByEmail(email: string): Promise<AccountUserRecord | null> {
    const userId = this.accountUsersByEmail.get(email);
    const rec = userId ? this.accountUsers.get(userId) : undefined;
    return rec ? { ...rec } : null;
  }

  async getAccountUserById(userId: string): Promise<AccountUserRecord | null> {
    const rec = this.accountUsers.get(userId);
    return rec ? { ...rec } : null;
  }

  async setAccountUserPassword(userId: string, passwordHash: string, _now: number): Promise<boolean> {
    const rec = this.accountUsers.get(userId);
    if (!rec) return false;
    rec.passwordHash = passwordHash;
    // Kill every live session: a reset must invalidate stolen cookies too.
    await this.deleteAllUserSessions(userId);
    return true;
  }

  async createUserSession(rec: UserSessionRecord): Promise<void> {
    this.userSessions.set(rec.token, { ...rec });
    // Write-time prune (T5), CENSUS-SAFE tombstone mirror of createSession /
    // PgStore.createUserSession: expired-beyond-grace rows are deleted
    // EXCEPT the newest EXPIRED one per user (Pg ranks the same subset via
    // ROW_NUMBER) — live rows are never pruned and never displace the
    // tombstone, so a "has this user ever logged in" census over the table
    // stays honest and no user ever loses every row.
    const cutoff = this.clock() - EXPIRED_ROW_GRACE_MS;
    const newestExpiredPerUser = new Map<string, number>();
    for (const s of this.userSessions.values()) {
      if (s.expiresAt >= cutoff) continue;
      const newest = newestExpiredPerUser.get(s.userId);
      if (newest === undefined || s.expiresAt > newest) {
        newestExpiredPerUser.set(s.userId, s.expiresAt);
      }
    }
    for (const [token, s] of this.userSessions) {
      if (s.expiresAt < cutoff && s.expiresAt !== newestExpiredPerUser.get(s.userId)) {
        this.userSessions.delete(token);
      }
    }
  }

  async getUserSession(token: string): Promise<UserSessionRecord | null> {
    const rec = this.userSessions.get(token);
    return rec ? { ...rec } : null;
  }

  async deleteUserSession(token: string): Promise<void> {
    this.userSessions.delete(token);
  }

  async deleteAllUserSessions(userId: string): Promise<void> {
    for (const [token, session] of this.userSessions) {
      if (session.userId === userId) this.userSessions.delete(token);
    }
  }

  async listAccountMachines(userId: string): Promise<AccountMachineRecord[]> {
    return [...this.accountMachines.values()]
      .filter((m) => m.userId === userId)
      .sort((a, b) => b.addedAt - a.addedAt || (a.installationId < b.installationId ? -1 : 1))
      .map((m) => ({ ...m }));
  }

  async getActiveAccountForInstallation(installationId: string): Promise<{ userId: string } | null> {
    for (const m of this.accountMachines.values()) {
      if (m.installationId === installationId && m.status === "active") {
        return { userId: m.userId };
      }
    }
    return null;
  }

  async bindMachineToAccount(rec: {
    userId: string;
    installationId: string;
    label: string | null;
    now: number;
  }): Promise<"created" | "already_bound"> {
    // The partial unique index's rule, in code: one ACTIVE owner per
    // installation. A revoked row (same or other user) never blocks.
    for (const m of this.accountMachines.values()) {
      if (m.installationId === rec.installationId && m.status === "active" && m.userId !== rec.userId) {
        return "already_bound";
      }
    }
    const existing = this.accountMachines.get(`${rec.userId}:${rec.installationId}`);
    if (existing) {
      existing.label = rec.label;
      existing.status = "active";
      existing.approvedAt = rec.now;
      // Liveness and the signaling mapping are per-binding: a re-activated
      // row has not been seen under it and carries no connect mapping until
      // the machine heartbeats one (mirrors the Pg upsert's NULL resets).
      existing.lastSeenAt = null;
      existing.signalingDeviceId = null;
    } else {
      this.accountMachines.set(`${rec.userId}:${rec.installationId}`, {
        userId: rec.userId,
        installationId: rec.installationId,
        label: rec.label,
        status: "active",
        addedAt: rec.now,
        approvedAt: rec.now,
        lastSeenAt: null,
        signalingDeviceId: null,
      });
    }
    return "created";
  }

  async revokeAccountMachine(userId: string, installationId: string): Promise<boolean> {
    const rec = this.accountMachines.get(`${userId}:${installationId}`);
    if (!rec) return false;
    rec.status = "revoked";
    return true;
  }

  async setAccountMachineLabel(userId: string, installationId: string, label: string): Promise<boolean> {
    const rec = this.accountMachines.get(`${userId}:${installationId}`);
    if (!rec) return false;
    rec.label = label;
    return true;
  }

  async updateMachineContact(
    installationId: string,
    contact: { signalingDeviceId?: string | null; displayName?: string },
    now: number,
  ): Promise<boolean> {
    // At most one ACTIVE row per installation (the partial unique index's
    // rule, enforced in code on this side) — touch it and report existence.
    let touched = false;
    for (const rec of this.accountMachines.values()) {
      if (rec.installationId === installationId && rec.status === "active") {
        rec.lastSeenAt = now;
        if (contact.signalingDeviceId !== undefined) rec.signalingDeviceId = contact.signalingDeviceId;
        // Auto-populate only while unnamed: an owner rename (or an earlier
        // displayName) always wins over the machine's self-report.
        if (rec.label === null && contact.displayName !== undefined) rec.label = contact.displayName;
        touched = true;
      }
    }
    return touched;
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
    // peer_keys is append-only history (PK = installation_id, added_at);
    // the CURRENT key lives in installations. A retry with the same
    // timestamp is a no-op — never an upsert target by installation_id.
    await this.query(
      `INSERT INTO peer_keys (installation_id, public_key_b58, added_at)
       VALUES ($1, $2, $3)
       ON CONFLICT DO NOTHING`,
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
    // Write-time prune (T5; rendezvous pattern): expired-beyond-grace
    // codes are dead rows — neither approval nor /verify reaches them.
    await this.query(
      `DELETE FROM device_codes WHERE expires_at < now() - make_interval(secs => $1)`,
      [Math.floor(EXPIRED_ROW_GRACE_MS / 1000)],
    );
    await this.query(
      `INSERT INTO device_codes (device_code, installation_id, user_code, verify_url, approved, expires_at)
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [rec.deviceCode, rec.installationId, rec.userCode, rec.verifyUrl, rec.approved, PgStore.iso(rec.expiresAt)],
    );
  }

  async getDeviceAuth(deviceCode: string): Promise<DeviceAuthRecord | null> {
    const rows = await this.query(
      `SELECT device_code, installation_id, user_code, verify_url, approved, expires_at,
              claimed_by_user, claimed_label, claimed_at, claim_confirmed_at
       FROM device_codes WHERE device_code = $1`,
      [deviceCode],
    );
    const row = rows[0];
    return row ? PgStore.rowToDeviceAuth(row as Record<string, unknown>) : null;
  }

  async approveDevice(deviceCode: string): Promise<boolean> {
    const rows = await this.query(
      `UPDATE device_codes SET approved = true WHERE device_code = $1 RETURNING device_code`,
      [deviceCode],
    );
    return rows.length > 0;
  }

  async approveDeviceByUserCode(userCode: string): Promise<boolean> {
    const rows = await this.query(
      `UPDATE device_codes SET approved = true WHERE user_code = $1 RETURNING device_code`,
      [userCode],
    );
    return rows.length > 0;
  }

  async getDeviceAuthByUserCode(userCode: string): Promise<DeviceAuthRecord | null> {
    // Newest matching row — the same population approveDeviceByUserCode
    // updates (it flips every matching row; binding needs the one newest).
    const rows = await this.query(
      `SELECT device_code, installation_id, user_code, verify_url, approved, expires_at,
              claimed_by_user, claimed_label, claimed_at, claim_confirmed_at
       FROM device_codes WHERE user_code = $1
       ORDER BY created_at DESC LIMIT 1`,
      [userCode],
    );
    const row = rows[0];
    return row ? PgStore.rowToDeviceAuth(row as Record<string, unknown>) : null;
  }

  async claimDeviceByUserCode(
    userCode: string,
    userId: string,
    label: string | null,
    now: number,
  ): Promise<"claimed" | "already_claimed" | "unknown"> {
    // Targets exactly the newest live code with that userCode — the same row
    // getDeviceAuthByUserCode reads. Same-user re-claims refresh (back to
    // pending); a different user's pending claim blocks (already_claimed).
    const rows = await this.query(
      `UPDATE device_codes d SET
         claimed_by_user = $2, claimed_label = $3, claimed_at = $4, claim_confirmed_at = NULL
       WHERE d.user_code = $1
         AND d.expires_at > $4::timestamptz
         AND (d.claimed_by_user IS NULL OR d.claimed_by_user = $2)
         AND d.created_at = (SELECT MAX(created_at) FROM device_codes WHERE user_code = $1)
       RETURNING device_code`,
      [userCode, userId, label, PgStore.iso(now)],
    );
    if (rows.length > 0) return "claimed";
    // Conflict classification for the caller's 404-vs-409: an expired or
    // missing code is "unknown" (no liveness oracle), someone else's pending
    // claim on a live code is "already_claimed".
    const current = await this.getDeviceAuthByUserCode(userCode);
    if (current && current.expiresAt > now && current.claimedByUserId !== null && current.claimedByUserId !== userId) {
      return "already_claimed";
    }
    return "unknown";
  }

  async resolveDeviceClaim(deviceCode: string, accept: boolean, now: number): Promise<DeviceAuthRecord | null> {
    // Confirm: stamp claim_confirmed_at AND flip approved — the caller then
    // binds via bindMachineToAccount. Deny: clear the claim entirely so the
    // owner can re-approve with the same still-live code.
    const sql = accept
      ? `UPDATE device_codes SET claim_confirmed_at = $2, approved = true
         WHERE device_code = $1
         RETURNING device_code, installation_id, user_code, verify_url, approved, expires_at,
                   claimed_by_user, claimed_label, claimed_at, claim_confirmed_at`
      : `UPDATE device_codes SET claimed_by_user = NULL, claimed_label = NULL,
                                 claimed_at = NULL, claim_confirmed_at = NULL
         WHERE device_code = $1
         RETURNING device_code, installation_id, user_code, verify_url, approved, expires_at,
                   claimed_by_user, claimed_label, claimed_at, claim_confirmed_at`;
    const rows = await this.query(sql, accept ? [deviceCode, PgStore.iso(now)] : [deviceCode]);
    const row = rows[0];
    return row ? PgStore.rowToDeviceAuth(row as Record<string, unknown>) : null;
  }

  async createSession(rec: SessionRecord): Promise<void> {
    // Write-time prune (T5), CENSUS-SAFE: expired-beyond-grace sessions are
    // deleted EXCEPT the newest row per installation —
    // countEnrolledInstallations (DISTINCT installation_id) is the
    // DEVICE_APPROVAL_CAP basis and must stay cumulative, so every
    // installation keeps one tombstone row.
    await this.query(
      `DELETE FROM sessions WHERE ctid IN (
         SELECT ctid FROM (
           SELECT ctid,
                  ROW_NUMBER() OVER (PARTITION BY installation_id ORDER BY expires_at DESC) AS rn
           FROM sessions WHERE expires_at < now() - make_interval(secs => $1)
         ) ranked WHERE rn > 1
       )`,
      [Math.floor(EXPIRED_ROW_GRACE_MS / 1000)],
    );
    await this.query(
      `INSERT INTO sessions (token, installation_id, expires_at) VALUES ($1, $2, $3)`,
      [rec.token, rec.installationId, PgStore.iso(rec.expiresAt)],
    );
  }

  async getSession(token: string): Promise<SessionRecord | null> {
    const rows = await this.query(
      `SELECT token, installation_id, created_at, expires_at FROM sessions WHERE token = $1`,
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

  async countEnrolledInstallations(): Promise<number> {
    const rows = await this.query(
      `SELECT COUNT(DISTINCT installation_id) AS n FROM sessions`,
    );
    return Number((rows[0] as { n: string | number } | undefined)?.n ?? 0);
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
    // Write-time prune (T5; rendezvous pattern): peer_observations grows one
    // row per heartbeat and is read by nothing — bound it per peer (newest
    // N) plus a hard TTL, at write time.
    await this.query(
      `DELETE FROM peer_observations WHERE observed_at < now() - make_interval(secs => $1)`,
      [Math.floor(OBSERVATION_TTL_MS / 1000)],
    );
    await this.query(
      `DELETE FROM peer_observations WHERE ctid IN (
         SELECT ctid FROM (
           SELECT ctid,
                  ROW_NUMBER() OVER (PARTITION BY peer_id ORDER BY observed_at DESC) AS rn
           FROM peer_observations WHERE peer_id = $1
         ) ranked WHERE rn > $2
       )`,
      [rec.peerId, OBSERVATION_MAX_PER_PEER - 1],
    );
    await this.query(
      `INSERT INTO peer_observations (lease_id, peer_id, free_slots, queue_ms, observed_at)
       VALUES ($1, $2, $3, $4, $5)`,
      [rec.leaseId, rec.peerId, rec.freeSlots, rec.queueMs, PgStore.iso(rec.observedAt)],
    );
  }

  async listPeers(filter: { profileId?: string; limit: number }): Promise<PeerView[]> {
    const rows = await this.query(
      `SELECT l.*, s.until_at AS suspended_until
         FROM peer_leases l
         LEFT JOIN peer_suspensions s ON s.peer_id = l.peer_id AND s.until_at > $1::timestamptz
        WHERE l.expires_at > $1 AND l.draining = false
          AND ($2::text IS NULL OR l.profiles @> to_jsonb(ARRAY[$2::text]))
        ORDER BY l.peer_id ASC
        LIMIT $3`,
      [PgStore.iso(this.clock()), filter.profileId ?? null, filter.limit],
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      peerId: row.peer_id as string,
      addresses: row.addresses as string[],
      queueMs: row.queue_ms as number,
      // ADR-012: actively suspended peers roster at zero free slots.
      freeSlots: row.suspended_until === null || row.suspended_until === undefined
        ? (row.free_slots as number)
        : 0,
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

  async bumpDownloadCount(file: string): Promise<number> {
    const rows = await this.query(
      `INSERT INTO download_counts (file, count, last_at)
       VALUES ($1, 1, now())
       ON CONFLICT (file) DO UPDATE SET count = download_counts.count + 1, last_at = now()
       RETURNING count`,
      [file],
    );
    return Number((rows[0] as { count: string | number }).count);
  }

  async clientServesTotal(): Promise<number> {
    const rows = await this.query(
      `SELECT COALESCE(SUM(count), 0) AS total FROM download_counts`,
      [],
    );
    return Number((rows[0] as { total: string | number }).total);
  }

  async downloadCounts(): Promise<Record<string, number>> {
    const rows = await this.query(`SELECT file, count FROM download_counts`, []);
    const out: Record<string, number> = {};
    for (const row of rows as Record<string, unknown>[]) {
      out[row.file as string] = Number(row.count);
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

  // (failure_class persists via failChallenge/failExpiredChallenges only —
  // challenges are never created in a failed state.)

  private static rowToChallenge(row: Record<string, unknown>): ChallengeRecord {
    return {
      challengeId: row.challenge_id as string,
      leaseId: row.lease_id as string,
      profileId: row.profile_id as string,
      // The prompt is never a stored column: rows rehydrate to the base
      // nonce-bound form; the serving route rebuilds the full value (with
      // canary suffix, when pinned) from the challengeId (D16).
      prompt: buildChallengePrompt(row.challenge_id as string, null),
      issuedAt: (row.issued_at as Date).getTime(),
      deadlineAt: (row.deadline_at as Date).getTime(),
      outcome: row.outcome as ChallengeOutcome,
      firstTokenMs: row.first_token_ms === null ? null : Number(row.first_token_ms),
      totalMs: row.total_ms === null ? null : Number(row.total_ms),
      completedAt: row.completed_at === null ? null : (row.completed_at as Date).getTime(),
      failureClass: (row.failure_class as "timeout" | "invalid" | null) ?? null,
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

  async failChallenge(challengeId: string, failureClass: "timeout" | "invalid", at: number): Promise<boolean> {
    const rows = await this.query(
      `UPDATE hosting_challenges
         SET outcome = 'failed', failure_class = $2,
             completed_at = CASE WHEN $2 = 'invalid' THEN $3::timestamptz ELSE NULL END
       WHERE challenge_id = $1 AND outcome = 'open'
       RETURNING challenge_id`,
      [challengeId, failureClass, PgStore.iso(at)],
    );
    return rows.length > 0;
  }

  async failExpiredChallenges(leaseId: string, now: number): Promise<number> {
    const rows = await this.query(
      `UPDATE hosting_challenges
         SET outcome = 'failed', failure_class = 'timeout', completed_at = NULL
       WHERE lease_id = $1 AND outcome = 'open' AND deadline_at < $2::timestamptz
       RETURNING challenge_id`,
      [leaseId, PgStore.iso(now)],
    );
    return rows.length;
  }

  async recentChallengeFailures(
    peerId: string,
    sinceMs: number,
    nowMs: number,
  ): Promise<{ failureClass: "timeout" | "invalid"; at: number }[]> {
    const rows = await this.query(
      `SELECT c.failure_class,
              COALESCE(c.completed_at, c.deadline_at) AS at
         FROM hosting_challenges c
         JOIN peer_leases l ON l.lease_id = c.lease_id
        WHERE l.peer_id = $1 AND c.outcome = 'failed'
          AND COALESCE(c.completed_at, c.deadline_at) BETWEEN $2::timestamptz AND $3::timestamptz`,
      [peerId, PgStore.iso(sinceMs), PgStore.iso(nowMs)],
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      failureClass: (row.failure_class as "timeout" | "invalid") ?? "timeout",
      at: (row.at as Date).getTime(),
    }));
  }

  private static rowToSuspension(row: Record<string, unknown>): SuspensionRecord {
    return {
      peerId: row.peer_id as string,
      untilAt: (row.until_at as Date).getTime(),
      reason: row.reason as string,
      suspensionCount: Number(row.suspension_count),
      prevMaxSlots: Number(row.prev_max_slots),
    };
  }

  async getSuspension(peerId: string): Promise<SuspensionRecord | null> {
    const rows = await this.query(
      `SELECT * FROM peer_suspensions WHERE peer_id = $1`,
      [peerId],
    );
    const row = rows[0];
    return row ? PgStore.rowToSuspension(row as Record<string, unknown>) : null;
  }

  async applySuspension(state: SuspensionRecord, now: number): Promise<void> {
    await this.query(
      `INSERT INTO peer_suspensions (peer_id, until_at, reason, suspension_count, prev_max_slots, suspended_at)
       VALUES ($1, $2, $3, $4, $5, $6)
       ON CONFLICT (peer_id) DO UPDATE
         SET until_at = $2, reason = $3, suspension_count = $4,
             prev_max_slots = GREATEST(peer_suspensions.prev_max_slots, $5)`,
      [state.peerId, PgStore.iso(state.untilAt), state.reason, state.suspensionCount, state.prevMaxSlots, PgStore.iso(now)],
    );
    await this.query(
      `UPDATE peer_leases SET max_slots = 0, free_slots = 0
       WHERE peer_id = $1 AND max_slots > 0`,
      [state.peerId],
    );
  }

  async recoverSuspensionIfDue(peerId: string, now: number): Promise<void> {
    const rows = await this.query(
      `UPDATE peer_suspensions
          SET until_at = $2::timestamptz - make_interval(secs => 1)
        WHERE peer_id = $1 AND until_at <= $2::timestamptz
        RETURNING prev_max_slots`,
      [peerId, PgStore.iso(now)],
    );
    const row = rows[0] as Record<string, unknown> | undefined;
    if (!row) return;
    await this.query(
      `UPDATE peer_leases
         SET max_slots = $2, free_slots = LEAST(free_slots, $2)
       WHERE peer_id = $1 AND max_slots = 0`,
      [peerId, Number(row.prev_max_slots)],
    );
  }

  async clearSuspension(peerId: string, _now: number): Promise<void> {
    const rows = await this.query(
      `DELETE FROM peer_suspensions WHERE peer_id = $1 RETURNING prev_max_slots`,
      [peerId],
    );
    const row = rows[0] as Record<string, unknown> | undefined;
    if (!row) return;
    await this.query(
      `UPDATE peer_leases
         SET max_slots = $2, free_slots = LEAST(free_slots, $2)
       WHERE peer_id = $1 AND max_slots = 0`,
      [peerId, Number(row.prev_max_slots)],
    );
  }

  async lastChallengePassAt(peerId: string): Promise<number | null> {
    const rows = await this.query(
      `SELECT MAX(c.completed_at) AS newest
         FROM hosting_challenges c
         JOIN peer_leases l ON l.lease_id = c.lease_id
        WHERE l.peer_id = $1 AND c.outcome = 'passed' AND c.completed_at IS NOT NULL`,
      [peerId],
    );
    const row = rows[0] as Record<string, unknown> | undefined;
    return row && row.newest !== null ? (row.newest as Date).getTime() : null;
  }

  async recordToken(rec: TokenRecord): Promise<void> {
    // Write-time prune (T5): rows whose grace window closed more than a day
    // ago can never be authoritative again (E3); the expires_at index exists.
    await this.query(
      `DELETE FROM capability_tokens WHERE expires_at < now() - make_interval(secs => $1)`,
      [Math.floor(EXPIRED_ROW_GRACE_MS / 1000)],
    );
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
        // MemoryStore treats provenance as optional; the column is NOT NULL
        // DEFAULT '{}' — an explicit NULL would defeat the default (and did,
        // in the first real-Pg suite run).
        rec.provenance ? JSON.stringify(rec.provenance) : "{}",
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

  async setProfileStatus(profileId: string, status: ProfileStatus): Promise<boolean> {
    const rows = await this.query(
      `UPDATE model_profiles SET status = $2 WHERE profile_id = $1 RETURNING profile_id`,
      [profileId, status],
    );
    return rows.length > 0;
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

  async insertModelRequest(
    rec: Omit<ModelRequestRecord, "id" | "createdAt" | "resolvedAt" | "resolution">,
  ): Promise<InsertModelRequestResult> {
    const rows = await this.query(
      `INSERT INTO model_requests
         (artifact_sha256, hf_repo, hf_revision, artifact_path, artifact_bytes,
          quant_method, quant_bits, display_name, note, requested_from)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
       ON CONFLICT (artifact_sha256) DO NOTHING
       RETURNING id`,
      [
        rec.artifactSha256,
        rec.hfRepo,
        rec.hfRevision,
        rec.artifactPath,
        rec.artifactBytes,
        rec.quantMethod,
        rec.quantBits,
        rec.displayName,
        rec.note ?? "",
        rec.requestedFrom,
      ],
    );
    return rows.length === 0 ? "duplicate" : "inserted";
  }

  async listModelRequests(): Promise<ModelRequestRecord[]> {
    const rows = await this.query(
      `SELECT id, artifact_sha256, hf_repo, hf_revision, artifact_path, artifact_bytes,
              quant_method, quant_bits, display_name, note, requested_from,
              created_at, resolved_at, resolution
       FROM model_requests
       ORDER BY resolved_at NULLS FIRST, created_at ASC, id ASC`,
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      id: Number(row.id),
      artifactSha256: row.artifact_sha256 as string,
      hfRepo: row.hf_repo as string,
      hfRevision: row.hf_revision as string,
      artifactPath: row.artifact_path as string,
      artifactBytes: Number(row.artifact_bytes),
      quantMethod: row.quant_method as string,
      quantBits: Number(row.quant_bits),
      displayName: row.display_name as string,
      note: row.note as string,
      requestedFrom: row.requested_from as string,
      createdAt: (row.created_at as Date).getTime(),
      resolvedAt: row.resolved_at ? (row.resolved_at as Date).getTime() : null,
      resolution: (row.resolution as "" | "promoted" | "rejected") ?? "",
    }));
  }

  async resolveModelRequest(id: number, resolution: "promoted" | "rejected"): Promise<boolean> {
    const rows = await this.query(
      `UPDATE model_requests
         SET resolved_at = now(), resolution = $2
       WHERE id = $1 AND resolved_at IS NULL
       RETURNING id`,
      [id, resolution],
    );
    return rows.length > 0;
  }

  async pushNotice(target: { all: true } | { leaseId: string }, notice: Notice): Promise<void> {
    if ("all" in target) {
      // Write-time prune (T5): leases dead for over a day will never
      // heartbeat to drain a queue — clear their existing rows AND exclude
      // them from the fan-out.
      await this.query(
        `DELETE FROM peer_notices WHERE lease_id IN (
           SELECT lease_id FROM peer_leases
           WHERE expires_at < now() - make_interval(secs => $1)
         )`,
        [Math.floor(EXPIRED_ROW_GRACE_MS / 1000)],
      );
      await this.query(
        `INSERT INTO peer_notices (lease_id, notice)
         SELECT lease_id, $1::jsonb FROM peer_leases
         WHERE expires_at >= now() - make_interval(secs => $2)`,
        [JSON.stringify(notice), Math.floor(EXPIRED_ROW_GRACE_MS / 1000)],
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
    // Bounded mailboxes (parity with MemoryStore): a TTL sweep plus a
    // per-mailbox cap — rows aimed at peers that never drain must not
    // grow the table unboundedly (write-time pruning; reads drain-once).
    await this.query(
      `DELETE FROM rendezvous WHERE received_at < now() - make_interval(secs => $1)`,
      [Math.floor(RENDEZVOUS_TTL_MS / 1000)],
    );
    await this.query(
      `DELETE FROM rendezvous WHERE ctid IN (
         SELECT ctid FROM (
           SELECT ctid, row_number() OVER (
             PARTITION BY to_peer_id ORDER BY received_at DESC
           ) AS rn
           FROM rendezvous WHERE to_peer_id = $1
         ) ranked WHERE rn > $2
       )`,
      [item.toPeerId, RENDEZVOUS_MAX_PER_MAILBOX],
    );
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

  async recordGrantedAccounting(rows: GrantedAccountingRow[]): Promise<void> {
    if (rows.length === 0) return;
    const values: unknown[] = [];
    const tuples = rows.map((r, i) => {
      const b = i * 7;
      values.push(
        r.receiptDigest, r.peerId, r.installationId, r.profileId,
        r.endpointSeconds, r.verifiedTokens, PgStore.iso(r.grantedAt),
      );
      return `($${b + 1}, $${b + 2}, $${b + 3}, $${b + 4}, $${b + 5}, $${b + 6}, $${b + 7})`;
    });
    await this.query(
      `INSERT INTO granted_accounting
         (receipt_digest, peer_id, installation_id, profile_id,
          endpoint_seconds, verified_tokens, granted_at)
       VALUES ${tuples.join(", ")}
       ON CONFLICT (receipt_digest, peer_id) DO NOTHING`,
      values,
    );
  }

  async recordConsumption(row: { installationId: string; receiptDigest: string; tokens: number; at: number }): Promise<void> {
    await this.query(
      `INSERT INTO consumed_accounting (installation_id, receipt_digest, tokens, consumed_at)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT (installation_id, receipt_digest) DO NOTHING`,
      [row.installationId, row.receiptDigest, row.tokens, PgStore.iso(row.at)],
    );
  }

  async consumedInWindow(installationId: string, windowSecs: number): Promise<number> {
    const res = (await this.query(
      `SELECT COALESCE(SUM(tokens), 0) AS total
         FROM consumed_accounting
        WHERE installation_id = $1 AND consumed_at > now() - make_interval(secs => $2)`,
      [installationId, windowSecs],
    )) as Record<string, unknown>[];
    return Number(res[0]?.total ?? 0);
  }

  async servedThroughputInWindow(
    peerId: string,
    windowSecs: number,
  ): Promise<{ tokens: number; seconds: number }> {
    const res = (await this.query(
      `SELECT COALESCE(SUM(verified_tokens), 0) AS tokens,
              COALESCE(SUM(endpoint_seconds), 0) AS seconds
         FROM granted_accounting
        WHERE peer_id = $1 AND granted_at > now() - make_interval(secs => $2)`,
      [peerId, windowSecs],
    )) as Record<string, unknown>[];
    const r = res[0] ?? {};
    return { tokens: Number(r.tokens ?? 0), seconds: Number(r.seconds ?? 0) };
  }

  async consumedTokensDecayed(
    installationId: string,
    halfLifeSecs: number,
  ): Promise<{ decayed: number; raw: number; rows: number }> {
    const res = (await this.query(
      `SELECT
         COALESCE(SUM(tokens * pow(0.5, extract(epoch FROM (now() - consumed_at)) / $2)), 0) AS decayed,
         COALESCE(SUM(tokens), 0) AS raw,
         COUNT(*) AS rows
       FROM consumed_accounting WHERE installation_id = $1`,
      [installationId, halfLifeSecs],
    )) as Record<string, unknown>[];
    const r = res[0] ?? {};
    return {
      decayed: Number(r.decayed ?? 0),
      raw: Number(r.raw ?? 0),
      rows: Number(r.rows ?? 0),
    };
  }

  async grantedCounters(
    peerId: string,
    halfLifeSecs: number,
  ): Promise<{
    endpointSecondsDecayed: number;
    verifiedTokensDecayed: number;
    endpointSecondsRaw: number;
    verifiedTokensRaw: number;
    rows: number;
  }> {
    const res = (await this.query(
      `SELECT
         COALESCE(SUM(endpoint_seconds * pow(0.5, extract(epoch FROM (now() - granted_at)) / $2)), 0) AS decayed_s,
         COALESCE(SUM(verified_tokens * pow(0.5, extract(epoch FROM (now() - granted_at)) / $2)), 0) AS decayed_t,
         COALESCE(SUM(endpoint_seconds), 0) AS raw_s,
         COALESCE(SUM(verified_tokens), 0) AS raw_t,
         COUNT(*)::int AS rows
       FROM granted_accounting WHERE peer_id = $1`,
      [peerId, halfLifeSecs],
    )) as Array<Record<string, number | null>>;
    const r = res[0] ?? {};
    return {
      endpointSecondsDecayed: Number(r.decayed_s ?? 0),
      verifiedTokensDecayed: Number(r.decayed_t ?? 0),
      endpointSecondsRaw: Number(r.raw_s ?? 0),
      verifiedTokensRaw: Number(r.raw_t ?? 0),
      rows: Number(r.rows ?? 0),
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
    // Write-time prune (T5): write-only rows with a 10-minute TTL.
    await this.query(`DELETE FROM session_authorizations WHERE expires_at < now()`, []);
    await this.query(
      `INSERT INTO session_authorizations (session_id, profile_id, mode, peer_ids, expires_at)
       VALUES ($1, $2, $3, $4, $5)`,
      [rec.sessionId, rec.profileId, rec.mode, JSON.stringify(rec.peerIds), PgStore.iso(rec.expiresAt)],
    );
  }

  // -- website accounts (migration 0008) ----------------------------------------

  private static rowToAccountUser(row: Record<string, unknown>): AccountUserRecord {
    return {
      userId: row.user_id as string,
      email: row.email as string,
      passwordHash: row.password_hash as string,
      displayName: row.display_name as string,
      createdAt: (row.created_at as Date).getTime(),
    };
  }

  private static rowToDeviceAuth(row: Record<string, unknown>): DeviceAuthRecord {
    return {
      deviceCode: row.device_code as string,
      installationId: row.installation_id as string,
      userCode: row.user_code as string,
      verifyUrl: row.verify_url as string,
      approved: row.approved as boolean,
      expiresAt: (row.expires_at as Date).getTime(),
      claimedByUserId: (row.claimed_by_user as string | null) ?? null,
      claimedLabel: (row.claimed_label as string | null) ?? null,
      claimedAt: row.claimed_at === null || row.claimed_at === undefined ? null : (row.claimed_at as Date).getTime(),
      claimConfirmedAt:
        row.claim_confirmed_at === null || row.claim_confirmed_at === undefined
          ? null
          : (row.claim_confirmed_at as Date).getTime(),
    };
  }

  async createAccountUser(rec: AccountUserRecord): Promise<void> {
    // A duplicate email raises Pg 23505 here — the route checked first and
    // maps this throw to 409 email_taken.
    await this.query(
      `INSERT INTO account_users (user_id, email, password_hash, display_name, created_at)
       VALUES ($1, $2, $3, $4, $5)`,
      [rec.userId, rec.email, rec.passwordHash, rec.displayName, PgStore.iso(rec.createdAt)],
    );
  }

  async getAccountUserByEmail(email: string): Promise<AccountUserRecord | null> {
    const rows = await this.query(
      `SELECT user_id, email, password_hash, display_name, created_at
       FROM account_users WHERE email = $1`,
      [email],
    );
    const row = rows[0];
    return row ? PgStore.rowToAccountUser(row as Record<string, unknown>) : null;
  }

  async getAccountUserById(userId: string): Promise<AccountUserRecord | null> {
    const rows = await this.query(
      `SELECT user_id, email, password_hash, display_name, created_at
       FROM account_users WHERE user_id = $1`,
      [userId],
    );
    const row = rows[0];
    return row ? PgStore.rowToAccountUser(row as Record<string, unknown>) : null;
  }

  async setAccountUserPassword(userId: string, passwordHash: string, _now: number): Promise<boolean> {
    const rows = await this.query(
      `UPDATE account_users SET password_hash = $2 WHERE user_id = $1 RETURNING user_id`,
      [userId, passwordHash],
    );
    if (rows.length === 0) return false;
    // Kill every live session: a reset must invalidate stolen cookies too.
    await this.deleteAllUserSessions(userId);
    return true;
  }

  async createUserSession(rec: UserSessionRecord): Promise<void> {
    // Write-time prune (T5), CENSUS-SAFE mirror of createSession: expired
    // rows are deleted EXCEPT the newest per user (one tombstone each).
    await this.query(
      `DELETE FROM user_sessions WHERE ctid IN (
         SELECT ctid FROM (
           SELECT ctid,
                  ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY expires_at DESC) AS rn
           FROM user_sessions WHERE expires_at < now() - make_interval(secs => $1)
         ) ranked WHERE rn > 1
       )`,
      [Math.floor(EXPIRED_ROW_GRACE_MS / 1000)],
    );
    await this.query(
      `INSERT INTO user_sessions (token, user_id, expires_at) VALUES ($1, $2, $3)`,
      [rec.token, rec.userId, PgStore.iso(rec.expiresAt)],
    );
  }

  async getUserSession(token: string): Promise<UserSessionRecord | null> {
    const rows = await this.query(
      `SELECT token, user_id, created_at, expires_at FROM user_sessions WHERE token = $1`,
      [token],
    );
    const row = rows[0] as { token: string; user_id: string; created_at: Date; expires_at: Date } | undefined;
    if (!row) return null;
    return {
      token: row.token,
      userId: row.user_id,
      createdAt: row.created_at.getTime(),
      expiresAt: row.expires_at.getTime(),
    };
  }

  async deleteUserSession(token: string): Promise<void> {
    await this.query(`DELETE FROM user_sessions WHERE token = $1`, [token]);
  }

  async deleteAllUserSessions(userId: string): Promise<void> {
    await this.query(`DELETE FROM user_sessions WHERE user_id = $1`, [userId]);
  }

  async listAccountMachines(userId: string): Promise<AccountMachineRecord[]> {
    const rows = await this.query(
      `SELECT user_id, installation_id, label, status, added_at, approved_at, last_seen_at, signaling_device_id
       FROM account_machines WHERE user_id = $1
       ORDER BY added_at DESC, installation_id ASC`,
      [userId],
    );
    return (rows as Record<string, unknown>[]).map((row) => ({
      userId: row.user_id as string,
      installationId: row.installation_id as string,
      label: (row.label as string | null) ?? null,
      status: row.status as AccountMachineStatus,
      addedAt: (row.added_at as Date).getTime(),
      approvedAt: row.approved_at === null ? null : (row.approved_at as Date).getTime(),
      lastSeenAt: row.last_seen_at === null ? null : (row.last_seen_at as Date).getTime(),
      signalingDeviceId: (row.signaling_device_id as string | null) ?? null,
    }));
  }

  async getActiveAccountForInstallation(installationId: string): Promise<{ userId: string } | null> {
    const rows = await this.query(
      `SELECT user_id FROM account_machines WHERE installation_id = $1 AND status = 'active' LIMIT 1`,
      [installationId],
    );
    const row = rows[0] as { user_id: string } | undefined;
    return row ? { userId: row.user_id } : null;
  }

  async bindMachineToAccount(rec: {
    userId: string;
    installationId: string;
    label: string | null;
    now: number;
  }): Promise<"created" | "already_bound"> {
    // Pre-check mirrors the partial unique index's rule; the index itself is
    // the race-time backstop (its 23505 maps to already_bound below).
    const active = await this.getActiveAccountForInstallation(rec.installationId);
    if (active && active.userId !== rec.userId) return "already_bound";
    const at = PgStore.iso(rec.now);
    try {
      await this.query(
        `INSERT INTO account_machines (user_id, installation_id, label, status, added_at, approved_at)
         VALUES ($1, $2, $3, 'active', $4, $4)
         ON CONFLICT (user_id, installation_id) DO UPDATE
           SET label = EXCLUDED.label, status = 'active', approved_at = EXCLUDED.approved_at,
               last_seen_at = NULL, signaling_device_id = NULL`,
        [rec.userId, rec.installationId, rec.label, at],
      );
      return "created";
    } catch (err) {
      if (typeof err === "object" && err !== null && (err as { code?: string }).code === "23505") {
        return "already_bound";
      }
      throw err;
    }
  }

  async revokeAccountMachine(userId: string, installationId: string): Promise<boolean> {
    const rows = await this.query(
      `UPDATE account_machines SET status = 'revoked'
        WHERE user_id = $1 AND installation_id = $2
        RETURNING installation_id`,
      [userId, installationId],
    );
    return rows.length > 0;
  }

  async setAccountMachineLabel(userId: string, installationId: string, label: string): Promise<boolean> {
    const rows = await this.query(
      `UPDATE account_machines SET label = $3
        WHERE user_id = $1 AND installation_id = $2
        RETURNING installation_id`,
      [userId, installationId, label],
    );
    return rows.length > 0;
  }

  async updateMachineContact(
    installationId: string,
    contact: { signalingDeviceId?: string | null; displayName?: string },
    now: number,
  ): Promise<boolean> {
    // The partial unique index guarantees at most one ACTIVE row per
    // installation — the heartbeat's touch targets exactly it. The signaling
    // mapping is PRESENT-means-set (a present NULL clears it: a sharing-off
    // machine reports null so the directory stops listing it as
    // controllable); ABSENT keeps the stored one (bare liveness payloads).
    // Label auto-populates only while NULL — owner renames always win.
    const rows = await this.query(
      `UPDATE account_machines SET
         last_seen_at = $2,
         signaling_device_id = CASE WHEN $5 THEN $3 ELSE signaling_device_id END,
         label = COALESCE(label, $4)
        WHERE installation_id = $1 AND status = 'active'
        RETURNING installation_id`,
      [
        installationId,
        PgStore.iso(now),
        contact.signalingDeviceId ?? null,
        contact.displayName ?? null,
        contact.signalingDeviceId !== undefined,
      ],
    );
    return rows.length > 0;
  }
}

// (Removed 2026-10-09, D16: the challenge prompt is no longer a fixed
// constant — it is built per challenge instance in lib/challenge.ts and
// never stored as a column. No import sites referenced the old re-export.)

// ---------------------------------------------------------------------------
// Factory

export function getStore(clock: () => number = Date.now): TrackerStore {
  const url = process.env.DATABASE_URL;
  if (url) return new PgStore(url, clock);
  return new MemoryStore(clock);
}
