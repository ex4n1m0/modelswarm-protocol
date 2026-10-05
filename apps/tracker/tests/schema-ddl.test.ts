// A1 (no-DB path), A2 (forbidden column names + no unbounded text), A3
// (timestamptz everywhere) — asserted by parsing migrations/0001_init.sql.

import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const ddlPath = join(here, "..", "migrations", "0001_init.sql");

interface Column {
  table: string;
  name: string;
  type: string;
}

function parseColumns(sql: string): Column[] {
  const columns: Column[] = [];
  let table: string | null = null;
  let depth = 0;
  for (const rawLine of sql.split(/\r?\n/)) {
    const line = rawLine.replace(/--.*$/, "").trim();
    if (line.length === 0) continue;
    const createMatch = /^CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_]+)/i.exec(line);
    if (createMatch) {
      table = createMatch[1]!;
      depth = 1; // the "(" is on the same line in our DDL
      continue;
    }
    if (table && depth === 1) {
      if (/^\);/.test(line)) {
        table = null;
        continue;
      }
      // column definitions start with an identifier + type; skip constraints
      const colMatch = /^([a-z_][a-z0-9_]*)\s+([a-z_]+(?:\s*\([^)]*\))?)/i.exec(line);
      if (colMatch && !/^(PRIMARY|UNIQUE|CHECK|CONSTRAINT|FOREIGN|EXCLUDE)$/i.test(colMatch[1]!)) {
        columns.push({ table: table!, name: colMatch[1]!.toLowerCase(), type: colMatch[2]!.replace(/\s+/g, "") });
      }
    }
  }
  return columns;
}

const columns = parseColumns(readFileSync(ddlPath, "utf8"));

const FORBIDDEN_NAMES = [
  "prompt",
  "completion",
  "messages",
  "conversation",
  "content",
  "history",
  "hf_token",
  "access_token",
  "api_key",
  "secret",
] as const;

describe("A2 structural privacy assertions over the DDL", () => {
  it("parses the DDL with a sane column count", () => {
    expect(columns.length).toBeGreaterThan(60);
    const tables = new Set(columns.map((c) => c.table));
    for (const required of [
      "users",
      "installations",
      "peer_keys",
      "model_profiles",
      "license_acceptances",
      "peer_leases",
      "peer_observations",
      "hosting_challenges",
      "capability_tokens",
      "job_receipts",
      "blocked_peers",
      "release_channels",
      "nonces",
      "rate_counters",
      "rendezvous",
      "session_authorizations",
    ]) {
      expect(tables.has(required), `missing table ${required}`).toBe(true);
    }
  });

  it("no column name anywhere contains a forbidden token", () => {
    const offenders = columns.filter((c) =>
      FORBIDDEN_NAMES.some((bad) => c.name.includes(bad)),
    );
    expect(offenders.map((c) => `${c.table}.${c.name}`)).toEqual([]);
  });

  it("no unbounded text columns (text without length) exist", () => {
    // The tracker DDL uses varchar(n) for every string column and jsonb only
    // for protocol-defined structured metadata (manifests, profiles lists,
    // notices, rosters). No plain `text` column is allowed.
    const offenders = columns.filter((c) => /^text$/i.test(c.type));
    expect(offenders.map((c) => `${c.table}.${c.name}:${c.type}`)).toEqual([]);
  });

  it("jsonb columns are the enumerated structured-metadata set only", () => {
    const jsonb = columns.filter((c) => c.type.startsWith("jsonb"));
    const allowed = new Set([
      "model_profiles.manifest",
      "model_profiles.provenance",
      "peer_leases.profiles",
      "peer_leases.addresses",
      "peer_notices.notice",
      "session_authorizations.peer_ids",
    ]);
    const unexpected = jsonb.filter((c) => !allowed.has(`${c.table}.${c.name}`));
    expect(unexpected.map((c) => `${c.table}.${c.name}`)).toEqual([]);
  });
});

describe("A3 timestamps are timestamptz UTC", () => {
  it("every *_at column is timestamptz", () => {
    const at = columns.filter((c) => c.name.endsWith("_at"));
    expect(at.length).toBeGreaterThan(10);
    const offenders = at.filter((c) => c.type !== "timestamptz");
    expect(offenders.map((c) => `${c.table}.${c.name}:${c.type}`)).toEqual([]);
  });

  it("no timestamp column escapes the *_at naming convention", () => {
    const offenders = columns.filter((c) => c.type === "timestamptz" && !c.name.endsWith("_at"));
    expect(offenders.map((c) => `${c.table}.${c.name}`)).toEqual([]);
  });
});

describe("A1 migration runner (no DATABASE_URL path)", () => {
  it("skips cleanly and exits 0 without DATABASE_URL", () => {
    const out = execFileSync("node", [join(here, "..", "scripts", "migrate.mjs")], {
      env: { ...process.env, DATABASE_URL: "" },
      encoding: "utf8",
    });
    expect(out.trim()).toBe("skipped (no DATABASE_URL)");
  });
});
