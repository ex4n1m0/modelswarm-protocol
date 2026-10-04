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
const files = readdirSync(vectorsDir).filter((f) => f.endsWith(".json")).sort();
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

process.exit(failures > 0 ? 1 : 0);
