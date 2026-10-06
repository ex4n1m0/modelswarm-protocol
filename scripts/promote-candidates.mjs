// promote-candidates — publish + promote every candidate profile (ADR-005/023).
//
// The owner half of the catalog pipeline: resolve-candidate.mjs produces
// reviewed candidates in catalog/candidate-profiles/; this script POSTs each
// to /admin/catalog/candidates and immediately /admin/catalog/promote.
// Immutability is enforced server-side: a different manifest under an
// existing profileId 400s and this run stops fail-closed.
//
// Usage:
//   ADMIN_TOKEN=… node scripts/promote-candidates.mjs \
//     [--tracker https://modelswarm.deepflux.space] \
//     [--only qwen3]      # substring filter on display_name
//     [--dry]             # show what would run, touch nothing

import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..");
const dir = join(repoRoot, "catalog", "candidate-profiles");

const argv = process.argv.slice(2);
const flag = (name) => {
  const at = argv.indexOf(`--${name}`);
  return at === -1 ? undefined : argv[at + 1];
};
const tracker = (flag("tracker") ?? "https://modelswarm.deepflux.space").replace(/\/$/, "");
const only = flag("only")?.toLowerCase();
const dry = argv.includes("--dry");
const token = process.env.ADMIN_TOKEN;
if (!token && !dry) {
  console.error("ADMIN_TOKEN env is required (the tracker's X-MSP-Admin).");
  process.exit(2);
}

const records = readdirSync(dir)
  .filter((f) => f.endsWith(".json"))
  .map((f) => JSON.parse(readFileSync(join(dir, f), "utf8")))
  .filter((r) => !only || r.display_name.toLowerCase().includes(only))
  .sort((a, b) => a.display_name.localeCompare(b.display_name));

if (records.length === 0) {
  console.error("no candidate profiles matched.");
  process.exit(2);
}

async function call(path, body) {
  const res = await fetch(`${tracker}${path}`, {
    method: body ? "POST" : "GET",
    headers: { "content-type": "application/json", "x-msp-admin": token },
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = await res.json().catch(() => ({}));
  return { status: res.status, json };
}

let failures = 0;
for (const rec of records) {
  const label = `${rec.display_name} (${rec.profile_id.slice(0, 18)}…)`;
  if (dry) {
    console.log(`dry: would candidates+promote ${label}`);
    continue;
  }
  const posted = await call("/api/v1/admin/catalog/candidates", {
    manifest: rec.manifest,
    display_name: rec.display_name,
    status: "candidate",
    provenance: rec.provenance,
  });
  if (posted.status === 201) {
    console.log(`candidate ✓  ${label}`);
  } else if (posted.json?.error?.code === "invalid_body" && /already exists/.test(posted.json?.error?.message ?? "")) {
    console.log(`candidate =  ${label} (same manifest already published)`);
  } else {
    console.error(`candidate ✗  ${label}: ${posted.status} ${JSON.stringify(posted.json).slice(0, 160)}`);
    failures += 1;
    continue;
  }
  const promoted = await call("/api/v1/admin/catalog/promote", { profileId: rec.profile_id });
  if (promoted.status === 200) {
    console.log(`promoted  ✓  ${label}`);
  } else {
    console.error(`promoted  ✗  ${label}: ${promoted.status} ${JSON.stringify(promoted.json).slice(0, 160)}`);
    failures += 1;
  }
}

if (failures > 0) process.exit(1);
console.log(`done: ${records.length} profile(s) live at ${tracker} (verify: GET /api/v1/stats).`);
