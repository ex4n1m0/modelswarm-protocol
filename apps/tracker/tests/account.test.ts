// Website accounts (migration 0008 layer): signup/login/logout/me, session
// expiry, Bearer auth, device approve + binding exclusivity, admin password
// reset, the "account" rate bucket, and scrypt unit tests. Drives the real
// route handlers with Request objects against MemoryStore via makeRig.

import { beforeEach, describe, expect, it } from "vitest";
import * as signupRoute from "@/app/api/v1/account/signup/route";
import * as loginRoute from "@/app/api/v1/account/login/route";
import * as logoutRoute from "@/app/api/v1/account/logout/route";
import * as meRoute from "@/app/api/v1/account/me/route";
import * as machinesRoute from "@/app/api/v1/account/machines/route";
import * as approveRoute from "@/app/api/v1/account/devices/approve/route";
import * as revokeRoute from "@/app/api/v1/account/machines/[installationId]/revoke/route";
import * as labelRoute from "@/app/api/v1/account/machines/[installationId]/label/route";
import * as resetRoute from "@/app/api/v1/admin/account/reset-password/route";
import * as deviceStart from "@/app/api/v1/auth/device/start/route";
import * as deviceComplete from "@/app/api/v1/auth/device/complete/route";
import * as claimConfirmRoute from "@/app/api/v1/auth/device/claim-confirm/route";
import * as revokeAllRoute from "@/app/api/v1/account/sessions/revoke-all/route";
import * as directoryRoute from "@/app/api/v1/account/directory/route";
import { hashPassword, verifyPassword } from "@/lib/passwords";
import { USER_SESSION_TTL_MS } from "@/lib/constants";
import { keyPairIds, makeKeypair, makeRig, signedRequest } from "./helpers";
import { DEVICE_CODE_TTL_MS } from "@/lib/constants";

let rig: ReturnType<typeof makeRig>;
let ipCounter = 1;

beforeEach(() => {
  // Fresh limiter per test; signup open unless a test says otherwise.
  rig = makeRig({ accountSignupOpen: true });
  ipCounter = 1;
});

const nextIp = () => `198.51.100.${(ipCounter++ % 250) + 1}`;

function jsonPost(path: string, body: unknown, headers: Record<string, string> = {}): Request {
  return new Request(`http://tracker.local${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
}

function withCookie(token: string): Record<string, string> {
  return { cookie: `msw_session=${token}` };
}

function sessionTokenOf(res: Response): string {
  const setCookie = res.headers.get("set-cookie") ?? "";
  const match = /msw_session=([0-9a-f]+)/.exec(setCookie);
  expect(match, `set-cookie carried a session token: ${setCookie}`).not.toBeNull();
  return match![1]!;
}

interface Account {
  userId: string;
  token: string;
  email: string;
}

async function createAccount(email: string, password = "correct-horse-battery", displayName = "Test User"): Promise<Account> {
  const res = await signupRoute.POST(
    jsonPost("/api/v1/account/signup", { email, password, displayName }, { "x-forwarded-for": nextIp() }),
  );
  if (res.status !== 201) throw new Error(`signup failed: ${res.status} ${await res.text()}`);
  const body = (await res.json()) as { userId: string };
  return { userId: body.userId, token: sessionTokenOf(res), email };
}

/** device/start (no completion needed — binding only needs the pending
 *  pairing code). Pass `keys` to re-enroll the SAME installation. */
async function startDeviceEnrollment(
  keys = makeKeypair(),
): Promise<{ installationId: string; userCode: string; deviceCode: string; keys: ReturnType<typeof makeKeypair> }> {
  const { installationId, pubKeyB58 } = keyPairIds(keys);
  const res = await deviceStart.POST(
    new Request("http://tracker.local/api/v1/auth/device/start", {
      method: "POST",
      headers: { "content-type": "application/json", "x-forwarded-for": nextIp() },
      body: JSON.stringify({ installationId, pubKey: pubKeyB58 }),
    }),
  );
  expect(res.status).toBe(200);
  const body = (await res.json()) as { userCode: string; deviceCode: string };
  return { installationId, userCode: body.userCode, deviceCode: body.deviceCode, keys };
}

/** The full claim handshake as the real parties run it: the account user
 *  claims the userCode at the dashboard (202, no bind yet), and the DEVICE
 *  confirms with its key (the only step that can complete the binding). */
async function claimAndConfirm(
  acctToken: string,
  enrollment: { installationId: string; userCode: string; deviceCode: string; keys: ReturnType<typeof makeKeypair> },
  label?: string,
): Promise<void> {
  const claimRes = await approveRoute.POST(
    jsonPost(
      "/api/v1/account/devices/approve",
      { userCode: enrollment.userCode, ...(label !== undefined ? { label } : {}) },
      { ...withCookie(acctToken), "x-forwarded-for": nextIp() },
    ),
  );
  expect(claimRes.status).toBe(202);
  const confirmRes = await claimConfirmRoute.POST(
    signedRequest(
      enrollment.keys,
      enrollment.installationId,
      "",
      "/api/v1/auth/device/claim-confirm",
      JSON.stringify({ deviceCode: enrollment.deviceCode, accept: true }),
      rig.ctx.now,
      { ip: nextIp() },
    ),
  );
  expect(confirmRes.status).toBe(200);
}

// ---------------------------------------------------------------------------
// scrypt unit tests

describe("passwords: scrypt hash/verify", () => {
  it("roundtrip verifies; wrong password fails; malformed stored never throws", async () => {
    const stored = await hashPassword("correct-horse-battery");
    expect(stored).toMatch(/^scrypt\$65536\$8\$1\$[A-Za-z0-9+/=]+\$[A-Za-z0-9+/=]+$/);
    expect(await verifyPassword("correct-horse-battery", stored)).toBe(true);
    expect(await verifyPassword("correct-horse-battery ", stored)).toBe(false);
    for (const bad of ["", "plain", "scrypt$1$2", "scrypt$a$b$c$d$e", "scrypt$0$8$1$AA$AA", `${stored.slice(0, -4)}!!!`]) {
      await expect(verifyPassword("x", bad)).resolves.toBe(false);
    }
  });

  it("salts are random (two hashes of one password differ)", async () => {
    const a = await hashPassword("same-password-1");
    const b = await hashPassword("same-password-1");
    expect(a).not.toBe(b);
  });
});

// ---------------------------------------------------------------------------
// signup / me

describe("signup + me", () => {
  it("happy path: 201, HttpOnly/Secure/SameSite=Lax cookie, token never in the body; me returns the user", async () => {
    const res = await signupRoute.POST(
      jsonPost("/api/v1/account/signup", { email: "Ada@Example.COM ", password: "correct-horse-battery", displayName: " Ada " }, { "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(201);
    const setCookie = res.headers.get("set-cookie") ?? "";
    expect(setCookie).toContain("msw_session=");
    expect(setCookie).toContain("HttpOnly");
    expect(setCookie).toContain("Secure");
    expect(setCookie).toContain("SameSite=Lax");
    expect(setCookie).toContain("Path=/");
    expect(setCookie).toContain(`Max-Age=${USER_SESSION_TTL_MS / 1000}`);

    const text = await res.text();
    const token = sessionTokenOf(res);
    expect(text.includes(token)).toBe(false);
    const body = JSON.parse(text) as { userId: string; email: string; displayName: string; signedIn: boolean };
    expect(body.userId).toMatch(/^[0-9a-f]{32}$/);
    expect(body.email).toBe("ada@example.com"); // trimmed + lowercased
    expect(body.displayName).toBe("Ada");
    expect(body.signedIn).toBe(true);

    const meRes = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me", { headers: withCookie(token) }));
    expect(meRes.status).toBe(200);
    const me = (await meRes.json()) as { userId: string; email: string; displayName: string; createdAt: string };
    expect(me.userId).toBe(body.userId);
    expect(me.email).toBe("ada@example.com");
    expect(me.displayName).toBe("Ada");
    expect(Date.parse(me.createdAt)).toBe(rig.nowMs);
  });

  it("signup closed by default -> 403 signup_closed; open via context override", async () => {
    const closedRig = makeRig();
    expect(closedRig.ctx.accountSignupOpen).toBe(false);
    const res = await signupRoute.POST(
      jsonPost("/api/v1/account/signup", { email: "x@example.com", password: "correct-horse-battery", displayName: "X" }, { "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(403);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("signup_closed");
  });

  it("duplicate email (any case) -> 409 email_taken", async () => {
    await createAccount("dup@example.com");
    const res = await signupRoute.POST(
      jsonPost("/api/v1/account/signup", { email: "DUP@example.com", password: "correct-horse-battery", displayName: "Second" }, { "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(409);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("email_taken");
  });

  it("password too short -> 400 invalid_body", async () => {
    const res = await signupRoute.POST(
      jsonPost("/api/v1/account/signup", { email: "short@example.com", password: "short", displayName: "S" }, { "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
  });

  it("session-mint failure after user creation -> 201 signedIn:false, no cookie; login still works", async () => {
    // Simulate createUserSession throwing once after the user row landed.
    const realMint = rig.store.createUserSession.bind(rig.store);
    let throwOnce = true;
    (
      rig.store as unknown as { createUserSession: typeof rig.store.createUserSession }
    ).createUserSession = async (rec) => {
      if (throwOnce) {
        throwOnce = false;
        throw new Error("session mint failed");
      }
      return realMint(rec);
    };
    const res = await signupRoute.POST(
      jsonPost("/api/v1/account/signup", { email: "partial@example.com", password: "correct-horse-battery", displayName: "Partial" }, { "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(201);
    expect(res.headers.get("set-cookie")).toBeNull();
    const body = (await res.json()) as { userId: string; signedIn: boolean };
    expect(body.signedIn).toBe(false);
    expect(await rig.store.getAccountUserByEmail("partial@example.com")).not.toBeNull();

    // The account exists and logs in normally (the wrapper now delegates).
    const loginRes = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "partial@example.com", password: "correct-horse-battery" }, { "x-forwarded-for": nextIp() }),
    );
    expect(loginRes.status).toBe(200);
    expect(loginRes.headers.get("set-cookie")).toContain("msw_session=");
  });
});

// ---------------------------------------------------------------------------
// login / logout / expiry / Bearer

describe("login + logout + expiry", () => {
  it("login happy path sets the cookie; wrong password and unknown email are byte-identical 401s", async () => {
    const acct = await createAccount("login@example.com");

    const okRes = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "LOGIN@example.com", password: "correct-horse-battery" }, { "x-forwarded-for": nextIp() }),
    );
    expect(okRes.status).toBe(200);
    expect(okRes.headers.get("set-cookie")).toContain("msw_session=");
    const okBody = (await okRes.json()) as { userId: string; displayName: string };
    expect(okBody.userId).toBe(acct.userId);
    expect(okBody.displayName).toBe("Test User");

    const wrongRes = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "login@example.com", password: "wrong-password-1" }, { "x-forwarded-for": nextIp() }),
    );
    const unknownRes = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "ghost@example.com", password: "wrong-password-1" }, { "x-forwarded-for": nextIp() }),
    );
    expect(wrongRes.status).toBe(401);
    expect(unknownRes.status).toBe(401);
    // byte-identical bodies — no user enumeration
    const wrongText = await wrongRes.text();
    const unknownText = await unknownRes.text();
    expect(unknownText).toBe(wrongText);
    expect((JSON.parse(wrongText) as { error: { code: string } }).error.code).toBe("invalid_credentials");
  });

  it("logout deletes the session and clears the cookie; me -> 401 afterwards", async () => {
    const acct = await createAccount("logout@example.com");
    const res = await logoutRoute.POST(
      new Request("http://tracker.local/api/v1/account/logout", {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(acct.token) },
      }),
    );
    expect(res.status).toBe(200);
    expect(((await res.json()) as { ok: boolean }).ok).toBe(true);
    expect(res.headers.get("set-cookie")).toContain("Max-Age=0");

    const meRes = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me", { headers: withCookie(acct.token) }));
    expect(meRes.status).toBe(401);
  });

  it("me without a session -> 401 unauthorized", async () => {
    const res = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me"));
    expect(res.status).toBe(401);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unauthorized");
  });

  it("session expiry is row-aging: advancing the clock past the TTL kills the session", async () => {
    const acct = await createAccount("expire@example.com");
    const before = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me", { headers: withCookie(acct.token) }));
    expect(before.status).toBe(200);
    rig.advance(USER_SESSION_TTL_MS + 1);
    const after = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me", { headers: withCookie(acct.token) }));
    expect(after.status).toBe(401);
  });

  it("Bearer header authenticates without a cookie", async () => {
    const acct = await createAccount("bearer@example.com");
    const res = await meRoute.GET(
      new Request("http://tracker.local/api/v1/account/me", { headers: { authorization: `Bearer ${acct.token}` } }),
    );
    expect(res.status).toBe(200);
    const machinesRes = await machinesRoute.GET(
      new Request("http://tracker.local/api/v1/account/machines", { headers: { authorization: `Bearer ${acct.token}` } }),
    );
    expect(machinesRes.status).toBe(200);
  });
});

// ---------------------------------------------------------------------------
// device approve + machine binding

describe("account devices/approve + machines", () => {
  it("end-to-end: claim + device confirm binds the installation; re-claim from another account -> 409; revoke frees it", async () => {
    const owner = await createAccount("owner@example.com");
    const rival = await createAccount("rival@example.com");
    const first = await startDeviceEnrollment();
    const { installationId, keys } = first;

    // The dashboard claim alone does NOT bind — 202, pending device confirm.
    const claimRes = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: first.userCode, label: "Machine" }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    expect(claimRes.status).toBe(202);
    const claimBody = (await claimRes.json()) as { status: string; installationId: string; label: string | null; confirmBy: string };
    expect(claimBody).toMatchObject({ status: "claim_pending", installationId, label: "Machine" });
    expect(Date.parse(claimBody.confirmBy)).toBeGreaterThan(rig.nowMs);
    expect((await rig.store.listAccountMachines(owner.userId))).toHaveLength(0);

    // The device confirms — only this step creates the binding.
    const confirmRes = await claimConfirmRoute.POST(
      signedRequest(keys, installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: first.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(confirmRes.status).toBe(200);

    const listRes = await machinesRoute.GET(new Request("http://tracker.local/api/v1/account/machines", { headers: withCookie(owner.token) }));
    expect(listRes.status).toBe(200);
    const list = (await listRes.json()) as { machines: Array<{ installationId: string; label: string | null; status: string; addedAt: string; approvedAt: string | null }> };
    expect(list.machines).toHaveLength(1);
    expect(list.machines[0]).toMatchObject({ installationId, label: "Machine", status: "active" });
    expect(Date.parse(list.machines[0]!.addedAt)).toBe(rig.nowMs);
    expect(list.machines[0]!.approvedAt).not.toBeNull();

    // A second enrollment of the SAME installation, claimed by the rival:
    // the installation is active under the owner -> already_bound.
    const second = await startDeviceEnrollment(keys);
    expect(second.installationId).toBe(installationId);
    const clashRes = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: second.userCode, label: "Stolen" }, { ...withCookie(rival.token), "x-forwarded-for": nextIp() }),
    );
    expect(clashRes.status).toBe(409);
    expect(((await clashRes.json()) as { error: { code: string } }).error.code).toBe("already_bound");

    // Revoke -> the row stays (status revoked) and the rival can bind now.
    const revokeRes = await revokeRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${installationId}/revoke`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(owner.token) },
      }),
      { params: Promise.resolve({ installationId }) },
    );
    expect(revokeRes.status).toBe(200);

    const afterRevoke = await machinesRoute.GET(new Request("http://tracker.local/api/v1/account/machines", { headers: withCookie(owner.token) }));
    const ownerList = ((await afterRevoke.json()) as { machines: Array<{ status: string }> }).machines;
    expect(ownerList[0]!.status).toBe("revoked");

    const third = await startDeviceEnrollment(keys);
    await claimAndConfirm(rival.token, third, "Rival Machine");
    const rivalList = ((await (await machinesRoute.GET(new Request("http://tracker.local/api/v1/account/machines", { headers: withCookie(rival.token) }))).json()) as { machines: Array<{ installationId: string; status: string; label: string | null }> }).machines;
    expect(rivalList).toHaveLength(1);
    expect(rivalList[0]).toMatchObject({ installationId, status: "active", label: "Rival Machine" });
  });

  it("unknown code -> the admin approve route's 404 shape; approve without session -> 401", async () => {
    const acct = await createAccount("unknown-code@example.com");
    const res = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: "ZZZZZZ99" }, { ...withCookie(acct.token), "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(404);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unknown_request");

    const anon = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: "AB2CDEFG" }, { "x-forwarded-for": nextIp() }),
    );
    expect(anon.status).toBe(401);
  });

  it("expired pairing code -> the same 404 unknown_request (no code-liveness oracle)", async () => {
    const acct = await createAccount("expired-code@example.com");
    const { userCode } = await startDeviceEnrollment();
    rig.advance(DEVICE_CODE_TTL_MS + 1);
    const res = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode }, { ...withCookie(acct.token), "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(404);
    const body = (await res.json()) as { error: { code: string; message: string } };
    expect(body.error.code).toBe("unknown_request");
    expect(body.error.message).toBe("no pending device with that code");
  });

  it("label rename works and 404s for machines you do not own", async () => {
    const acct = await createAccount("rename@example.com");
    const other = await createAccount("other@example.com");
    const enrollment = await startDeviceEnrollment();
    const { installationId } = enrollment;
    await claimAndConfirm(acct.token, enrollment, "Workstation");

    const labelRes = await labelRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${installationId}/label`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(acct.token) },
        body: JSON.stringify({ label: "Living room PC" }),
      }),
      { params: Promise.resolve({ installationId }) },
    );
    expect(labelRes.status).toBe(200);
    const list = ((await (await machinesRoute.GET(new Request("http://tracker.local/api/v1/account/machines", { headers: withCookie(acct.token) }))).json()) as { machines: Array<{ label: string }> }).machines;
    expect(list[0]!.label).toBe("Living room PC");

    const notOwned = await labelRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${installationId}/label`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(other.token) },
        body: JSON.stringify({ label: "Not Mine" }),
      }),
      { params: Promise.resolve({ installationId }) },
    );
    expect(notOwned.status).toBe(404);
    expect(((await notOwned.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");

    const missingId = "x".repeat(20);
    const revokeMissing = await revokeRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${missingId}/revoke`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(other.token) },
      }),
      { params: Promise.resolve({ installationId: missingId }) },
    );
    expect(revokeMissing.status).toBe(404);
    expect(((await revokeMissing.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");

    // Path-param shape gate: >64-char garbage would 500 on Pg's varchar(64)
    // without it — must be the same 404 unknown_machine.
    const garbageId = "x".repeat(100);
    const garbageRes = await revokeRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${garbageId}/revoke`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(acct.token) },
      }),
      { params: Promise.resolve({ installationId: garbageId }) },
    );
    expect(garbageRes.status).toBe(404);
    expect(((await garbageRes.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");
  });
});

// ---------------------------------------------------------------------------
// the claim handshake (gate (a), migration 0010)

describe("claim handshake", () => {
  /** Full enrollment WITH an MSP session (admin-approve hook like the
   *  heartbeat suite), so the claim tests can drive complete + directory. */
  async function enrollWithSession(keys = makeKeypair()): Promise<{
    installationId: string;
    userCode: string;
    deviceCode: string;
    keys: ReturnType<typeof makeKeypair>;
    session: string;
  }> {
    const e = await startDeviceEnrollment(keys);
    await rig.store.approveDevice(e.deviceCode);
    const res = await deviceComplete.POST(
      signedRequest(keys, e.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: e.deviceCode }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(res.status).toBe(200);
    return { ...e, session: ((await res.json()) as { token: string }).token };
  }

  it("an unconfirmed claim gates complete: claim_pending with the masked claimer; confirm releases the session", async () => {
    const owner = await createAccount("claimer@example.com");
    const machine = await startDeviceEnrollment();

    const claimRes = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: machine.userCode }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    expect(claimRes.status).toBe(202);

    // complete now reports the pending claim instead of minting a session.
    const pendingRes = await deviceComplete.POST(
      signedRequest(machine.keys, machine.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: machine.deviceCode }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(pendingRes.status).toBe(200);
    expect(await pendingRes.json()).toEqual({ status: "claim_pending", claimedBy: "c***@example.com" });

    // Confirm; the next complete mints the session normally.
    const confirmRes = await claimConfirmRoute.POST(
      signedRequest(machine.keys, machine.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: machine.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(confirmRes.status).toBe(200);
    const doneRes = await deviceComplete.POST(
      signedRequest(machine.keys, machine.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: machine.deviceCode }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(doneRes.status).toBe(200);
    expect(((await doneRes.json()) as { token: string }).token).toMatch(/^[0-9a-f]{64}$/);
  });

  it("a rival's pending claim blocks the owner's claim -> 409 already_claimed", async () => {
    const owner = await createAccount("real-owner@example.com");
    const rival = await createAccount("code-thief@example.com");
    const machine = await startDeviceEnrollment();

    // The leaked userCode lets the rival CLAIM — but only pending.
    const rivalRes = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: machine.userCode }, { ...withCookie(rival.token), "x-forwarded-for": nextIp() }),
    );
    expect(rivalRes.status).toBe(202);
    expect((await rig.store.listAccountMachines(rival.userId))).toHaveLength(0);

    const ownerRes = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: machine.userCode }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    expect(ownerRes.status).toBe(409);
    expect(((await ownerRes.json()) as { error: { code: string } }).error.code).toBe("already_claimed");
  });

  it("device denies the claim -> cleared, nothing bound, the right owner can re-claim the same code", async () => {
    const owner = await createAccount("denied-owner@example.com");
    const rival = await createAccount("denied-rival@example.com");
    const machine = await startDeviceEnrollment();

    await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: machine.userCode }, { ...withCookie(rival.token), "x-forwarded-for": nextIp() }),
    );
    const denyRes = await claimConfirmRoute.POST(
      signedRequest(machine.keys, machine.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: machine.deviceCode, accept: false }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(denyRes.status).toBe(200);
    expect((await rig.store.listAccountMachines(rival.userId))).toHaveLength(0);

    const reClaim = await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: machine.userCode }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    expect(reClaim.status).toBe(202);
    const confirmRes = await claimConfirmRoute.POST(
      signedRequest(machine.keys, machine.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: machine.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(confirmRes.status).toBe(200);
    expect((await rig.store.listAccountMachines(owner.userId)).find((m) => m.installationId === machine.installationId)?.status).toBe("active");
  });

  it("claim-confirm signed by a DIFFERENT installation's key -> 401; confirm with no pending claim -> 404", async () => {
    const owner = await createAccount("cc-edges@example.com");
    const machine = await startDeviceEnrollment();
    const stranger = await startDeviceEnrollment();

    await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: machine.userCode }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    // Stranger signs over the machine's deviceCode — the envelope's
    // installation is the stranger's, so the code lookup 401s.
    const forged = await claimConfirmRoute.POST(
      signedRequest(stranger.keys, stranger.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: machine.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(forged.status).toBe(401);

    // A fresh code with no claim: accepting has nothing to confirm.
    const noClaim = await claimConfirmRoute.POST(
      signedRequest(stranger.keys, stranger.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: stranger.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(noClaim.status).toBe(404);
    expect(((await noClaim.json()) as { error: { code: string } }).error.code).toBe("unknown_request");
  });
});

// ---------------------------------------------------------------------------
// sessions revoke-all (gate (b)) + the machine directory (P2)

describe("sessions revoke-all + account directory", () => {
  it("revoke-all deletes every session of the user (this browser included) and only theirs", async () => {
    const acct = await createAccount("everywhere@example.com");
    const other = await createAccount("other-device@example.com");
    // A second session for the same user (another browser): login again.
    const secondLogin = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "everywhere@example.com", password: "correct-horse-battery" }, { "x-forwarded-for": nextIp() }),
    );
    expect(secondLogin.status).toBe(200);
    const secondToken = sessionTokenOf(secondLogin);

    const res = await revokeAllRoute.POST(
      jsonPost("/api/v1/account/sessions/revoke-all", {}, { ...withCookie(acct.token), "x-forwarded-for": nextIp() }),
    );
    expect(res.status).toBe(200);
    expect(await rig.store.getUserSession(acct.token)).toBeNull();
    expect(await rig.store.getUserSession(secondToken)).toBeNull();
    expect(await rig.store.getUserSession(other.token)).not.toBeNull();
    // The cookie is cleared too.
    expect(res.headers.get("set-cookie")).toMatch(/msw_session=;.*Max-Age=0/);
  });

  it("directory: a bound machine sees its account's ACTIVE machines with signaling ids; unbound -> the same 404", async () => {
    const owner = await createAccount("dir-owner@example.com");
    const outsider = await createAccount("dir-outsider@example.com");

    // Machine A: enrolled + bound (label from the claim).
    const keysA = makeKeypair();
    const a = await startDeviceEnrollment(keysA);
    await rig.store.approveDevice(a.deviceCode);
    const completeA = await deviceComplete.POST(
      signedRequest(keysA, a.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: a.deviceCode }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(completeA.status).toBe(200);
    const sessionA = ((await completeA.json()) as { token: string }).token;
    await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: a.userCode, label: "Alpha" }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    const confirmA = await claimConfirmRoute.POST(
      signedRequest(keysA, a.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: a.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(confirmA.status).toBe(200);

    // Machine B: bound too, and it shared a signaling id via its heartbeat.
    const keysB = makeKeypair();
    const b = await startDeviceEnrollment(keysB);
    await rig.store.approveDevice(b.deviceCode);
    const completeB = await deviceComplete.POST(
      signedRequest(keysB, b.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: b.deviceCode }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(completeB.status).toBe(200);
    await approveRoute.POST(
      jsonPost("/api/v1/account/devices/approve", { userCode: b.userCode, label: "Beta" }, { ...withCookie(owner.token), "x-forwarded-for": nextIp() }),
    );
    await claimConfirmRoute.POST(
      signedRequest(keysB, b.installationId, "", "/api/v1/auth/device/claim-confirm", JSON.stringify({ deviceCode: b.deviceCode, accept: true }), rig.ctx.now, { ip: nextIp() }),
    );
    expect(await rig.store.updateMachineContact(b.installationId, { signalingDeviceId: "fedcba9876543210" }, rig.nowMs)).toBe(true);

    // A asks for the directory: both machines, Beta controllable, Alpha not.
    const dirRes = await directoryRoute.GET(
      signedRequest(keysA, a.installationId, sessionA, "/api/v1/account/directory", undefined, rig.ctx.now, { ip: nextIp() }),
    );
    expect(dirRes.status).toBe(200);
    const dir = (await dirRes.json()) as { machines: Array<{ installationId: string; label: string | null; signalingDeviceId: string | null }> };
    expect(dir.machines).toHaveLength(2);
    const beta = dir.machines.find((m) => m.installationId === b.installationId);
    const alpha = dir.machines.find((m) => m.installationId === a.installationId);
    expect(beta).toMatchObject({ label: "Beta", signalingDeviceId: "fedcba9876543210" });
    expect(alpha).toMatchObject({ label: "Alpha", signalingDeviceId: null });

    // Revoked machines drop out of the directory.
    await revokeRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${b.installationId}/revoke`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(owner.token) },
      }),
      { params: Promise.resolve({ installationId: b.installationId }) },
    );
    const afterRevoke = await directoryRoute.GET(
      signedRequest(keysA, a.installationId, sessionA, "/api/v1/account/directory", undefined, rig.ctx.now, { ip: nextIp() }),
    );
    expect(((await afterRevoke.json()) as { machines: string[] }).machines).toHaveLength(1);

    // An enrolled-but-unbound machine gets the same 404 as a total stranger.
    const unbound = await startDeviceEnrollment();
    await rig.store.approveDevice(unbound.deviceCode);
    const unboundComplete = await deviceComplete.POST(
      signedRequest(unbound.keys, unbound.installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: unbound.deviceCode }), rig.ctx.now, { ip: nextIp() }),
    );
    const unboundSession = ((await unboundComplete.json()) as { token: string }).token;
    const unboundDir = await directoryRoute.GET(
      signedRequest(unbound.keys, unbound.installationId, unboundSession, "/api/v1/account/directory", undefined, rig.ctx.now, { ip: nextIp() }),
    );
    expect(unboundDir.status).toBe(404);
    expect(((await unboundDir.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");
    void outsider;
  });
});

// ---------------------------------------------------------------------------
// admin reset + rate bucket

describe("admin reset-password", () => {
  it("changes the hash and kills live sessions: old password fails, pre-reset cookie dies, new works", async () => {
    const adminRig = makeRig({ accountSignupOpen: true, adminToken: "test-admin-token" });
    rig = adminRig;
    const acct = await createAccount("reset@example.com");

    // The cookie minted before the reset must die with the password.
    const meBefore = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me", { headers: withCookie(acct.token) }));
    expect(meBefore.status).toBe(200);

    const denied = await resetRoute.POST(jsonPost("/api/v1/admin/account/reset-password", { email: "reset@example.com", newPassword: "brand-new-password" }));
    expect(denied.status).toBe(403);

    const res = await resetRoute.POST(
      jsonPost("/api/v1/admin/account/reset-password", { email: "reset@example.com", newPassword: "brand-new-password" }, { "x-msp-admin": "test-admin-token" }),
    );
    expect(res.status).toBe(200);
    expect(((await res.json()) as { ok: boolean }).ok).toBe(true);

    const meAfter = await meRoute.GET(new Request("http://tracker.local/api/v1/account/me", { headers: withCookie(acct.token) }));
    expect(meAfter.status).toBe(401);

    const oldLogin = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "reset@example.com", password: "correct-horse-battery" }, { "x-forwarded-for": nextIp() }),
    );
    expect(oldLogin.status).toBe(401);
    const newLogin = await loginRoute.POST(
      jsonPost("/api/v1/account/login", { email: "reset@example.com", password: "brand-new-password" }, { "x-forwarded-for": nextIp() }),
    );
    expect(newLogin.status).toBe(200);

    const unknown = await resetRoute.POST(
      jsonPost("/api/v1/admin/account/reset-password", { email: "ghost@example.com", newPassword: "brand-new-password" }, { "x-msp-admin": "test-admin-token" }),
    );
    expect(unknown.status).toBe(404);
    expect(((await unknown.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");
  });
});

describe("token precedence and cookie parsing", () => {
  const DEAD = "0".repeat(64); // well-formed hex, no session row behind it

  it("garbage Bearer (not 64-hex) -> 401 (shape-gated before any store read)", async () => {
    const res = await meRoute.GET(
      new Request("http://tracker.local/api/v1/account/me", { headers: { authorization: "Bearer not-hex-at-all", "x-forwarded-for": nextIp() } }),
    );
    expect(res.status).toBe(401);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unauthorized");
  });

  it("stale cookie + valid Bearer authenticates via the Bearer", async () => {
    const acct = await createAccount("precedence@example.com");
    const res = await meRoute.GET(
      new Request("http://tracker.local/api/v1/account/me", {
        headers: { cookie: `msw_session=${DEAD}`, authorization: `Bearer ${acct.token}`, "x-forwarded-for": nextIp() },
      }),
    );
    expect(res.status).toBe(200);
  });

  it("cookie is read mid-header (a=1; msw_session=<tok>; b=2)", async () => {
    const acct = await createAccount("mid-cookie@example.com");
    const res = await meRoute.GET(
      new Request("http://tracker.local/api/v1/account/me", {
        headers: { cookie: `a=1; msw_session=${acct.token}; b=2`, "x-forwarded-for": nextIp() },
      }),
    );
    expect(res.status).toBe(200);
  });

  it("duplicate msw_session cookies: the LAST occurrence wins", async () => {
    const acct = await createAccount("last-wins@example.com");
    const lastValid = await meRoute.GET(
      new Request("http://tracker.local/api/v1/account/me", {
        headers: { cookie: `msw_session=${DEAD}; msw_session=${acct.token}`, "x-forwarded-for": nextIp() },
      }),
    );
    expect(lastValid.status).toBe(200);
    const lastDead = await meRoute.GET(
      new Request("http://tracker.local/api/v1/account/me", {
        headers: { cookie: `msw_session=${acct.token}; msw_session=${DEAD}`, "x-forwarded-for": nextIp() },
      }),
    );
    expect(lastDead.status).toBe(401);
  });
});

describe("content-type gate on the new account POST routes", () => {
  it("text/plain -> 400 invalid_body and nothing is stored; charset suffix is allowed", async () => {
    const plainPost = (path: string, body: unknown) =>
      new Request(`http://tracker.local${path}`, {
        method: "POST",
        headers: { "content-type": "text/plain", "x-forwarded-for": nextIp() },
        body: JSON.stringify(body),
      });
    const signupRes = await signupRoute.POST(
      plainPost("/api/v1/account/signup", { email: "ct@example.com", password: "correct-horse-battery", displayName: "CT" }),
    );
    expect(signupRes.status).toBe(400);
    expect(((await signupRes.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
    expect(await rig.store.getAccountUserByEmail("ct@example.com")).toBeNull();

    const loginRes = await loginRoute.POST(
      plainPost("/api/v1/account/login", { email: "x@example.com", password: "whatever-1" }),
    );
    expect(loginRes.status).toBe(400);
    expect(((await loginRes.json()) as { error: { code: string } }).error.code).toBe("invalid_body");

    // application/json; charset=utf-8 passes the gate.
    const charsetRes = await signupRoute.POST(
      new Request("http://tracker.local/api/v1/account/signup", {
        method: "POST",
        headers: { "content-type": "application/json; charset=utf-8", "x-forwarded-for": nextIp() },
        body: JSON.stringify({ email: "charset@example.com", password: "correct-horse-battery", displayName: "CS" }),
      }),
    );
    expect(charsetRes.status).toBe(201);
  });
});

describe("F6 account bucket: 20/min per IP", () => {
  it("21st signup attempt in the window from one IP -> 429 rate_limited", async () => {
    const ip = { "x-forwarded-for": "203.0.113.77" };
    let lastStatus = 0;
    // 20 rejected-before-parse bodies still consume the bucket (rate hits
    // BEFORE schema parse — mirror of the abuse suite's device/start test).
    for (let i = 0; i < 20; i += 1) {
      const res = await signupRoute.POST(jsonPost("/api/v1/account/signup", { nonsense: true }, ip));
      expect(res.status).toBe(400);
      await res.json();
      lastStatus = res.status;
    }
    const res = await signupRoute.POST(
      jsonPost("/api/v1/account/signup", { email: "21st@example.com", password: "correct-horse-battery", displayName: "Late" }, ip),
    );
    expect(res.status).toBe(429);
    expect(lastStatus).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("rate_limited");
    // the 429th created nothing
    expect(await rig.store.getAccountUserByEmail("21st@example.com")).toBeNull();
  });
});
