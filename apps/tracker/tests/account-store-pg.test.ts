// Pg-backed account store parity: the 0008 tables + the same write-time
// prune / exclusivity rules the MemoryStore suite asserts, against a REAL
// Postgres (DATABASE_URL; skipped otherwise). Mirror of pg-enrollment.test.ts.
import { execSync } from "node:child_process";
import { beforeAll, describe, expect, it } from "vitest";
import { PgStore, type AccountUserRecord } from "@/lib/store";
import { randomBytes } from "node:crypto";

const dbUrl = process.env.DATABASE_URL;

/** Unique-per-run fixture ids so repeated runs never collide. */
const uid = () => randomBytes(16).toString("hex");

function accountUser(overrides: Partial<AccountUserRecord> = {}): AccountUserRecord {
  const id = uid();
  return {
    userId: id,
    email: `${id}@example.com`,
    passwordHash: `scrypt$65536$8$1$${Buffer.from("salt").toString("base64")}$${Buffer.from("hash").toString("base64")}`,
    displayName: "Pg User",
    createdAt: 1_800_000_000_000,
    ...overrides,
  };
}

describe.skipIf(!dbUrl)("Pg account store", () => {
  let store: PgStore;
  let nowMs: number;

  beforeAll(() => {
    // The CI postgres job migrates before tests; local runs may not have.
    execSync("npm run db:migrate", {
      env: { ...process.env, DATABASE_URL: dbUrl },
      stdio: "pipe",
    });
    nowMs = 1_800_000_000_000;
    store = new PgStore(dbUrl!, () => nowMs);
  }, 60_000);

  it("createAccountUser + getByEmail/ById roundtrip; duplicate email throws", async () => {
    const user = accountUser({ email: `dup-${uid()}@example.com` });
    await store.createAccountUser(user);
    expect(await store.getAccountUserByEmail(user.email)).toMatchObject({
      userId: user.userId,
      email: user.email,
      displayName: user.displayName,
    });
    expect(await store.getAccountUserById(user.userId)).toMatchObject({ userId: user.userId });
    await expect(store.createAccountUser({ ...user, userId: uid() })).rejects.toThrow();
  });

  it("user_sessions prune-on-write keeps one expired tombstone per user", async () => {
    const user = accountUser();
    await store.createAccountUser(user);
    // Real-world past timestamps: the Pg prune compares against DB now().
    const base = Date.now();
    const oldestToken = `pg-old-${uid()}`;
    const olderToken = `pg-mid-${uid()}`;
    const newestToken = `pg-new-${uid()}`;
    const expired = (expiresAt: number, token: string) => ({ userId: user.userId, createdAt: base, expiresAt, token });
    await store.createUserSession(expired(base - 7 * 24 * 3600 * 1000, oldestToken));
    await store.createUserSession(expired(base - 5 * 24 * 3600 * 1000, olderToken));
    await store.createUserSession(expired(base - 3 * 24 * 3600 * 1000, newestToken));
    const live = `pg-live-${uid()}`;
    // This write prunes: both older expired rows go, the newest tombstone stays.
    await store.createUserSession({ userId: user.userId, createdAt: base, expiresAt: base + 30 * 24 * 3600 * 1000, token: live });

    expect(await store.getUserSession(oldestToken)).toBeNull();
    expect(await store.getUserSession(olderToken)).toBeNull();
    expect(await store.getUserSession(newestToken)).not.toBeNull();
    expect(await store.getUserSession(live)).not.toBeNull();
  });

  it("setAccountUserPassword swaps the hash AND kills the user's sessions; unknown user false", async () => {
    const user = accountUser();
    await store.createAccountUser(user);
    const liveToken = `pg-pw-${uid()}`;
    await store.createUserSession({ userId: user.userId, createdAt: nowMs, expiresAt: nowMs + 60_000, token: liveToken });
    expect(await store.setAccountUserPassword(user.userId, "scrypt$1$1$1$QQ$QQ", nowMs)).toBe(true);
    expect((await store.getAccountUserById(user.userId))?.passwordHash).toBe("scrypt$1$1$1$QQ$QQ");
    // A reset must invalidate live cookies too.
    expect(await store.getUserSession(liveToken)).toBeNull();
    expect(await store.setAccountUserPassword(uid(), "scrypt$1$1$1$QQ$QQ", nowMs)).toBe(false);
  });

  it("bindMachineToAccount exclusivity + revoke/rebind + label", async () => {
    const u1 = accountUser();
    const u2 = accountUser();
    await store.createAccountUser(u1);
    await store.createAccountUser(u2);
    // The FK target: account_machines.installation_id references installations.
    const installationId = uid();
    await store.upsertInstallation({ installationId, pubKeyB58: `pk${uid()}`.slice(0, 64), createdAt: nowMs });

    expect(await store.bindMachineToAccount({ userId: u1.userId, installationId, label: "One", now: nowMs })).toBe("created");
    expect(await store.getActiveAccountForInstallation(installationId)).toEqual({ userId: u1.userId });

    // Active under u1: u2 (and u1 after u2 takes over) cannot bind.
    expect(await store.bindMachineToAccount({ userId: u2.userId, installationId, label: "Two", now: nowMs })).toBe("already_bound");

    expect(await store.revokeAccountMachine(u1.userId, installationId)).toBe(true);
    expect(await store.getActiveAccountForInstallation(installationId)).toBeNull();

    expect(await store.bindMachineToAccount({ userId: u2.userId, installationId, label: "Two", now: nowMs })).toBe("created");
    expect(await store.getActiveAccountForInstallation(installationId)).toEqual({ userId: u2.userId });
    // u1's own re-bind is blocked while u2 is active (flipping it back would
    // create two active owners).
    expect(await store.bindMachineToAccount({ userId: u1.userId, installationId, label: "One again", now: nowMs })).toBe("already_bound");

    // The revoked row stays as history; u2's is active.
    const u1List = await store.listAccountMachines(u1.userId);
    expect(u1List).toHaveLength(1);
    expect(u1List[0]).toMatchObject({ installationId, status: "revoked", label: "One" });
    const u2List = await store.listAccountMachines(u2.userId);
    expect(u2List[0]).toMatchObject({ installationId, status: "active", label: "Two" });

    expect(await store.setAccountMachineLabel(u2.userId, installationId, "Renamed")).toBe(true);
    expect((await store.listAccountMachines(u2.userId))[0]?.label).toBe("Renamed");
    expect(await store.setAccountMachineLabel(uid(), installationId, "X")).toBe(false);

    // Machine-seen touch (migration 0009): only the ACTIVE row updates, the
    // same installation's revoked row keeps null, and unknown ids touch
    // nothing (the heartbeat route maps false to 404 unknown_machine).
    expect(await store.touchAccountMachineSeen(installationId, nowMs + 5_000)).toBe(true);
    expect((await store.listAccountMachines(u2.userId))[0]).toMatchObject({ lastSeenAt: nowMs + 5_000 });
    expect((await store.listAccountMachines(u1.userId))[0]).toMatchObject({ status: "revoked", lastSeenAt: null });
    expect(await store.touchAccountMachineSeen(uid(), nowMs)).toBe(false);

    // deleteUserSession removes the row.
    const token = `pg-del-${uid()}`;
    await store.createUserSession({ userId: u2.userId, createdAt: nowMs, expiresAt: nowMs + 60_000, token });
    await store.deleteUserSession(token);
    expect(await store.getUserSession(token)).toBeNull();
  });
});
