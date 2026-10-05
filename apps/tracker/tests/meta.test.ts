// H2 + repo hygiene: no inference runtime references, no background timers,
// protocol version parity across the three copies.

import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { MSP_PROTOCOL_VERSION } from "@/lib/version";
import { DEFAULT_RATE_LIMITS } from "@/lib/ratelimit";
import { LEASE_TTL_MS, TOKEN_GRACE_MS } from "@/lib/constants";

const here = dirname(fileURLToPath(import.meta.url));
const appRoot = join(here, "..");

function listFiles(dir: string, exts: string[]): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry === ".next" || entry === ".git") continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...listFiles(full, exts));
    else if (exts.some((e) => entry.endsWith(e))) out.push(full);
  }
  return out;
}

const sourceFiles = [
  ...listFiles(join(appRoot, "lib"), [".ts"]),
  ...listFiles(join(appRoot, "app"), [".ts", ".tsx"]),
  ...listFiles(join(appRoot, "scripts"), [".mjs"]),
  ...listFiles(join(appRoot, "migrations"), [".sql"]),
];

describe("H2 no inference runtime in the tracker source", () => {
  it("no file under lib/app/scripts/migrations references the runtime or model files", () => {
    expect(sourceFiles.length).toBeGreaterThan(20);
    const offenders = sourceFiles.filter((file) => {
      const src = readFileSync(file, "utf8");
      return /llama|gguf/i.test(src);
    });
    expect(offenders).toEqual([]);
  });

  it("no *.gguf artifacts anywhere under apps/tracker", () => {
    const artifacts = listFiles(appRoot, [".gguf", ".bin"]);
    expect(artifacts).toEqual([]);
  });

  it("no dependency name references the runtime", () => {
    const pkg = JSON.parse(readFileSync(join(appRoot, "package.json"), "utf8")) as {
      dependencies: Record<string, string>;
      devDependencies: Record<string, string>;
    };
    const names = [...Object.keys(pkg.dependencies), ...Object.keys(pkg.devDependencies)];
    expect(names.filter((n) => /llama|gguf/i.test(n))).toEqual([]);
  });
});

describe("D3 companion: no background timers/crons in tracker source", () => {
  it("no setTimeout/setInterval/setImmediate anywhere in lib/app", () => {
    const offenders = [...listFiles(join(appRoot, "lib"), [".ts"]), ...listFiles(join(appRoot, "app"), [".ts", ".tsx"])].filter(
      (file) => /\b(setTimeout|setInterval|setImmediate)\b/.test(readFileSync(file, "utf8")),
    );
    expect(offenders).toEqual([]);
  });
});

describe("protocol version parity", () => {
  it("lib/version.ts matches protocol/msp-v1.md", () => {
    const doc = readFileSync(join(here, "..", "..", "..", "protocol", "msp-v1.md"), "utf8");
    const match = /Protocol version constant:\s*`"([^"]+)"/.exec(doc);
    expect(match).not.toBeNull();
    expect(MSP_PROTOCOL_VERSION).toBe(match![1]);
    expect(MSP_PROTOCOL_VERSION).toBe("1");
  });

  it("frozen constants stay inside their bands", () => {
    expect(LEASE_TTL_MS).toBeGreaterThanOrEqual(60_000);
    expect(LEASE_TTL_MS).toBeLessThanOrEqual(90_000);
    expect(TOKEN_GRACE_MS).toBe(60_000);
    expect(DEFAULT_RATE_LIMITS.peerPerInstallation).toBe(60);
    expect(DEFAULT_RATE_LIMITS.unsignedPerIp).toBe(60);
    expect(DEFAULT_RATE_LIMITS.enrollPerIp).toBe(10);
    expect(DEFAULT_RATE_LIMITS.publicPerIp).toBe(120);
  });
});
