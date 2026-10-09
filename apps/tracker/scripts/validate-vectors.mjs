// Golden-vector validator for ModelProfileManifest v2 (ADR-011).
// Validates every protocol/vectors/*.json against catalog/schema-v2.json and
// recomputes the derived ModelProfileId from the canonical serialization.
// The Rust modelswarm-types tests must consume the same fixtures and produce
// the same IDs (cross-language parity gate).
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..");
const schemaPath = join(repoRoot, "catalog", "schema-v2.json");
const vectorsDir = join(repoRoot, "protocol", "vectors");

// Canonical JSON per ADR-011 / msp-v1 §2.2: UTF-8, recursively sorted keys,
// no insignificant whitespace.
function canonicalize(value) {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(canonicalize);
  const out = {};
  for (const key of Object.keys(value).sort()) out[key] = canonicalize(value[key]);
  return out;
}

function canonicalJson(value) {
  return JSON.stringify(canonicalize(value));
}

function deriveProfileId(manifest) {
  const digest = createHash("sha256").update(canonicalJson(manifest), "utf8").digest("hex");
  return `msp1:${digest}`;
}

const ajv = new Ajv2020({ allErrors: true });
const schema = JSON.parse(readFileSync(schemaPath, "utf8"));
// Compile the full document so internal $refs (#/$defs/Hash32) resolve
// against the schema root, then target the manifest definition.
const validateManifest = ajv.compile({ ...schema, $ref: "#/$defs/ModelProfileManifest" });

let failures = 0;
// Manifest fixtures only (lease-hubkey-1.json is a signed-lease vector
// covered by the test suites, not the manifest schema).
const files = readdirSync(vectorsDir)
  .filter((f) => f.startsWith("manifest-") && f.endsWith(".json"))
  .sort();
if (files.length === 0) {
  console.error("FAIL no vector files found in", vectorsDir);
  process.exit(1);
}

for (const file of files) {
  const path = join(vectorsDir, file);
  const doc = JSON.parse(readFileSync(path, "utf8"));
  const problems = [];

  if (!validateManifest(doc.manifest)) {
    problems.push(`schema: ${ajv.errorsText(validateManifest.errors)}`);
  }
  const derived = deriveProfileId(doc.manifest);
  if (doc.expected_profile_id !== derived) {
    problems.push(`profile id: expected ${doc.expected_profile_id}, derived ${derived}`);
  }

  if (problems.length > 0) {
    failures += 1;
    console.error(`FAIL ${file}`);
    for (const p of problems) console.error(`     ${p}`);
  } else {
    console.log(`PASS ${file} ${derived}`);
  }
}

// ---------------------------------------------------------------------------
// D16 challenge-prompt vector (gate 2026-10-09 / ADR-028 §7): validate the
// pinned construction independently of the TS/Rust suites — shape checks,
// build() and parse() round-trips re-derived here from the frozen format.
// ---------------------------------------------------------------------------
{
  const path = join(vectorsDir, "challenge-prompt-1.json");
  const doc = JSON.parse(readFileSync(path, "utf8"));
  const problems = [];
  const idRe = /^[0-9a-f]{32}$/;
  const digestRe = /^sha256:[0-9a-f]{64}$/;

  const build = (id, digest) =>
    digest === null
      ? `ModelSwarm readiness challenge ${id}`
      : `ModelSwarm readiness challenge ${id}; greedy-canary ${digest}`;
  const parse = (prompt) => {
    const m = /^ModelSwarm readiness challenge ([0-9a-f]{32})(?:; greedy-canary (sha256:[0-9a-f]{64}))?$/.exec(prompt);
    return m ? { challengeId: m[1], canaryDigest: m[2] ?? null } : null;
  };

  if (!Array.isArray(doc.cases) || doc.cases.length < 2) {
    problems.push("cases: want an array with at least the base and pinned forms");
  } else {
    let sawBase = false;
    let sawPinned = false;
    for (const [i, c] of doc.cases.entries()) {
      if (!idRe.test(c.challenge_id ?? "")) problems.push(`case ${i}: bad challenge_id shape`);
      const digest = c.canary_digest ?? null;
      if (digest !== null && !digestRe.test(digest)) problems.push(`case ${i}: bad canary_digest shape`);
      if (digest === null) sawBase = true;
      else sawPinned = true;
      const expected = build(c.challenge_id, digest);
      if (c.expected_prompt !== expected) {
        problems.push(`case ${i}: expected_prompt does not match the frozen format`);
      }
      const parsed = parse(c.expected_prompt ?? "");
      if (!parsed || parsed.challengeId !== c.challenge_id || parsed.canaryDigest !== digest) {
        problems.push(`case ${i}: expected_prompt does not round-trip`);
      }
    }
    if (!sawBase || !sawPinned) problems.push("cases: must pin both the base and pinned-canary forms");
  }

  if (problems.length > 0) {
    failures += 1;
    console.error("FAIL challenge-prompt-1.json");
    for (const p of problems) console.error(`     ${p}`);
  } else {
    console.log("PASS challenge-prompt-1.json");
  }
}

process.exit(failures > 0 ? 1 : 0);
