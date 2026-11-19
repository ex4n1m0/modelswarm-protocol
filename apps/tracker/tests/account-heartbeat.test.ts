// Machine heartbeat (migration 0009 layer): the signed liveness touch at
// POST /api/v1/account/machines/{installationId}/heartbeat — bind-then-beat
// happy path with lastSeenAt surfacing in the machines list, envelope/path
// id mismatch, the unbound and revoked 404s, session expiry via the guard,
// and the frozen exactly-{} body. Same rig as tests/account.test.ts: real
// route handlers with Request objects against MemoryStore via makeRig.

import { beforeEach, describe, expect, it } from "vitest";
import * as signupRoute from "@/app/api/v1/account/signup/route";
import * as machinesRoute from "@/app/api/v1/account/machines/route";
import * as approveRoute from "@/app/api/v1/account/devices/approve/route";
import * as revokeRoute from "@/app/api/v1/account/machines/[installationId]/revoke/route";
import * as machineHeartbeatRoute from "@/app/api/v1/account/machines/[installationId]/heartbeat/route";
import * as deviceStart from "@/app/api/v1/auth/device/start/route";
import * as deviceComplete from "@/app/api/v1/auth/device/complete/route";
import { SESSION_TTL_MS } from "@/lib/constants";
import type { KeyPair } from "@/lib/crypto";
import { keyPairIds, makeKeypair, makeRig, signedRequest } from "./helpers";

let rig: ReturnType<typeof makeRig>;
let ipCounter = 1;

beforeEach(() => {
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

async function createAccount(email: string): Promise<{ userId: string; token: string }> {
  const res = await signupRoute.POST(
    jsonPost("/api/v1/account/signup", { email, password: "correct-horse-battery", displayName: "Test User" }, { "x-forwarded-for": nextIp() }),
  );
  if (res.status !== 201) throw new Error(`signup failed: ${res.status} ${await res.text()}`);
  const body = (await res.json()) as { userId: string };
  return { userId: body.userId, token: sessionTokenOf(res) };
}

/** Full device enrollment (start -> approve via store hook -> complete) that
 *  KEEPS the pairing userCode, so the account approve route can bind it. */
async function enrollMachine(): Promise<{ keys: KeyPair; installationId: string; userCode: string; session: string }> {
  const keys = makeKeypair();
  const { installationId, pubKeyB58 } = keyPairIds(keys);
  const startRes = await deviceStart.POST(
    jsonPost("/api/v1/auth/device/start", { installationId, pubKey: pubKeyB58 }, { "x-forwarded-for": nextIp() }),
  );
  if (startRes.status !== 200) throw new Error(`device/start failed: ${startRes.status} ${await startRes.text()}`);
  const start = (await startRes.json()) as { deviceCode: string; userCode: string };
  await rig.store.approveDevice(start.deviceCode);
  const completeRes = await deviceComplete.POST(
    signedRequest(keys, installationId, "", "/api/v1/auth/device/complete", JSON.stringify({ deviceCode: start.deviceCode }), rig.ctx.now, { ip: nextIp() }),
  );
  if (completeRes.status !== 200) throw new Error(`device/complete failed: ${completeRes.status} ${await completeRes.text()}`);
  const session = (await completeRes.json()) as { token: string };
  return { keys, installationId, userCode: start.userCode, session: session.token };
}

interface Enrolled {
  keys: KeyPair;
  installationId: string;
  userCode: string;
  session: string;
}

/** Signed heartbeat POST for `pathId` (defaults to the caller's own id). */
async function beat(m: Enrolled, pathId = m.installationId, body = "{}"): Promise<Response> {
  const req = signedRequest(
    m.keys,
    m.installationId,
    m.session,
    `/api/v1/account/machines/${pathId}/heartbeat`,
    body,
    rig.ctx.now,
  );
  return machineHeartbeatRoute.POST(req, { params: Promise.resolve({ installationId: pathId }) });
}

async function bind(acctToken: string, m: Enrolled): Promise<void> {
  const res = await approveRoute.POST(
    jsonPost("/api/v1/account/devices/approve", { userCode: m.userCode }, { ...withCookie(acctToken), "x-forwarded-for": nextIp() }),
  );
  if (res.status !== 201) throw new Error(`approve failed: ${res.status} ${await res.text()}`);
}

interface ListedMachine {
  installationId: string;
  status: string;
  lastSeenAt: string | null;
}

async function listedMachines(token: string): Promise<ListedMachine[]> {
  const res = await machinesRoute.GET(
    new Request("http://tracker.local/api/v1/account/machines", { headers: withCookie(token) }),
  );
  expect(res.status).toBe(200);
  return ((await res.json()) as { machines: ListedMachine[] }).machines;
}

// ---------------------------------------------------------------------------

describe("machine heartbeat", () => {
  it("bind then heartbeat -> 200 seenAt; lastSeenAt surfaces in store and the machines list", async () => {
    const acct = await createAccount("hb-owner@example.com");
    const machine = await enrollMachine();
    await bind(acct.token, machine);

    // Bound but never seen: null in both views (0009's nullable column).
    expect((await rig.store.listAccountMachines(acct.userId))[0]!.lastSeenAt).toBeNull();
    expect((await listedMachines(acct.token))[0]!.lastSeenAt).toBeNull();

    const res = await beat(machine);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { seenAt: string };
    expect(body.seenAt).toBe(new Date(rig.nowMs).toISOString());

    // Store: epoch ms; API list: ISO of the same instant.
    expect((await rig.store.listAccountMachines(acct.userId))[0]!.lastSeenAt).toBe(rig.nowMs);
    const listed = (await listedMachines(acct.token))[0]!;
    expect(listed.lastSeenAt).not.toBeNull();
    expect(Date.parse(listed.lastSeenAt!)).toBe(rig.nowMs);
    expect(Date.parse(listed.lastSeenAt!)).toBeGreaterThan(0);
  });

  it("envelope installationId != path id -> 404 unknown_machine, nothing touched", async () => {
    const acct = await createAccount("hb-mismatch@example.com");
    const machine = await enrollMachine();
    const other = await enrollMachine();
    await bind(acct.token, machine);

    // Valid signature + valid session, but the path names another machine.
    const res = await beat(machine, other.installationId);
    expect(res.status).toBe(404);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");
    expect((await rig.store.listAccountMachines(acct.userId))[0]!.lastSeenAt).toBeNull();
  });

  it("enrolled but unbound installation -> 404 unknown_machine", async () => {
    await createAccount("hb-unbound@example.com");
    const machine = await enrollMachine();
    const res = await beat(machine);
    expect(res.status).toBe(404);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");
  });

  it("revoked machine -> 404; the row keeps its last touch", async () => {
    const acct = await createAccount("hb-revoked@example.com");
    const machine = await enrollMachine();
    await bind(acct.token, machine);
    expect((await beat(machine)).status).toBe(200);

    const revokeRes = await revokeRoute.POST(
      new Request(`http://tracker.local/api/v1/account/machines/${machine.installationId}/revoke`, {
        method: "POST",
        headers: { "content-type": "application/json", ...withCookie(acct.token) },
      }),
      { params: Promise.resolve({ installationId: machine.installationId }) },
    );
    expect(revokeRes.status).toBe(200);

    const after = await beat(machine);
    expect(after.status).toBe(404);
    expect(((await after.json()) as { error: { code: string } }).error.code).toBe("unknown_machine");
    const row = (await rig.store.listAccountMachines(acct.userId))[0]!;
    expect(row.status).toBe("revoked");
    expect(row.lastSeenAt).toBe(rig.nowMs); // frozen at the last successful touch
  });

  it("expired device session -> 401 via the guard, lastSeenAt untouched", async () => {
    const acct = await createAccount("hb-expired@example.com");
    const machine = await enrollMachine();
    await bind(acct.token, machine);
    rig.advance(SESSION_TTL_MS + 1);
    const res = await beat(machine);
    expect(res.status).toBe(401);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("unauthorized");
    expect((await rig.store.listAccountMachines(acct.userId))[0]!.lastSeenAt).toBeNull();
  });

  it("the frozen body is exactly {} — anything else is 400 invalid_body", async () => {
    const acct = await createAccount("hb-body@example.com");
    const machine = await enrollMachine();
    await bind(acct.token, machine);
    const res = await beat(machine, machine.installationId, JSON.stringify({ leaseId: "0".repeat(32) }));
    expect(res.status).toBe(400);
    expect(((await res.json()) as { error: { code: string } }).error.code).toBe("invalid_body");
    expect((await rig.store.listAccountMachines(acct.userId))[0]!.lastSeenAt).toBeNull();
  });
});
