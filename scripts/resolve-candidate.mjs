// resolve-candidate — fail-closed catalog resolver (ADR-011 / ADR-022, Phase H).
//
// Given an HF GGUF repo + filename + quantization + the pinned engine
// (runtime-pins.json), resolves every identity hash from the actual artifact
// bytes and emits a schema-v2 ModelProfileManifest plus the derived msp1: id.
// Never guesses: any missing input aborts with a non-zero exit.
//
// All tokenizer/template/architecture hashes are derived from the GGUF file
// itself (the artifact a node actually possesses), per ADR-022:
//   - tokenizer_hash    = sha256(canonical_json({model, tokens, token_type,
//                        scores_bits}))  — GGUF f32 scores canonicalized as
//                        decimal u32 IEEE-754 bit patterns
//   - chat_template_hash= sha256(utf8(GGUF `tokenizer.chat_template` string))
//   - architecture_hash = sha256(canonical_json(subset of GGUF hyperparams;
//                        f32 fields as `_bits` u32, enumerated in ADR-022))
//   - runtime.build_hash= sha256 of the pinned engine zip (runtime-pins.json)
//
// Usage:
//   node scripts/resolve-candidate.mjs \
//     --repo Qwen/Qwen2.5-0.5B-Instruct-GGUF \
//     --file qwen2.5-0.5b-instruct-q4_k_m.gguf \
//     --quant-method q4_k_m --quant-bits 4 \
//     --display-name "Qwen2.5 0.5B Instruct (Q4_K_M)" \
//     [--cache-dir DIR]      // reuse/download the artifact here (default: tmp)
//     [--out catalog/candidate-profiles/<id>.json]
//
// Self-tests the canonical serialization + derivation against the frozen
// golden vectors (protocol/vectors/manifest-*.json) before trusting itself.

import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..");

// ---------- canonical JSON + derivation (mirrors validate-vectors.mjs) ----------

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

function sha256Hex(input) {
  return createHash("sha256").update(input).digest("hex");
}

function deriveProfileId(manifest) {
  return `msp1:${sha256Hex(Buffer.from(canonicalJson(manifest), "utf8"))}`;
}

function selfTestAgainstGoldenVectors() {
  const vectorsDir = join(repoRoot, "protocol", "vectors");
  const files = readdirSync(vectorsDir).filter((f) => f.endsWith(".json")).sort();
  if (files.length === 0) throw new Error("no golden vectors found — refusing to resolve");
  for (const file of files) {
    const doc = JSON.parse(readFileSync(join(vectorsDir, file), "utf8"));
    const derived = deriveProfileId(doc.manifest);
    if (derived !== doc.expected_profile_id) {
      throw new Error(`canonical self-test failed for ${file}: ${derived} != ${doc.expected_profile_id}`);
    }
  }
  return files.length;
}

// ---------- GGUF metadata parser (metadata section only; tensors skipped) ----------

const GGUF_VALUE_TYPES = new Set([...Array(13).keys()]); // 0..12 per GGUF v3

function parseGgufMetadata(buf) {
  if (buf.length < 12) throw new Error("file too small to be GGUF");
  if (buf.toString("ascii", 0, 4) !== "GGUF") throw new Error("not a GGUF file (bad magic)");
  let o = 4;
  const version = buf.readUInt32LE(o);
  o += 4;
  if (version !== 3) throw new Error(`unsupported GGUF version ${version} (only v3)`);
  const tensorCount = buf.readBigUInt64LE(o); // eslint-disable-line no-unused-vars
  o += 8;
  const kvCount = Number(buf.readBigUInt64LE(o));
  o += 8;

  const readString = () => {
    const n = Number(buf.readBigUInt64LE(o));
    o += 8;
    // Prefix parses must fail loudly, never clamp: a truncated string would
    // silently corrupt every identity hash derived from it.
    if (o + n > buf.length) throw new Error("metadata prefix too small — truncated string");
    // Lossy UTF-8 (U+FFFD per invalid sequence) — ADR-022 canonical string rule.
    const s = buf.toString("utf8", o, o + n);
    o += n;
    return s;
  };
  const readValue = (type) => {
    switch (type) {
      case 0: { const v = buf.readUInt8(o); o += 1; return v; }
      case 1: { const v = buf.readInt8(o); o += 1; return v; }
      case 2: { const v = buf.readUInt16LE(o); o += 2; return v; }
      case 3: { const v = buf.readInt16LE(o); o += 2; return v; }
      case 4: { const v = buf.readUInt32LE(o); o += 4; return v; }
      case 5: { const v = buf.readInt32LE(o); o += 4; return v; }
      case 6: { const bits = buf.readUInt32LE(o); o += 4; return bits; } // f32 → u32 bit pattern (ADR-022)
      case 7: { const v = buf.readUInt8(o) !== 0; o += 1; return v; }
      case 8: return readString();
      case 9: {
        const elementType = buf.readUInt32LE(o);
        o += 4;
        const n = Number(buf.readBigUInt64LE(o));
        o += 8;
        const arr = new Array(n);
        for (let i = 0; i < n; i += 1) arr[i] = readValue(elementType);
        return arr;
      }
      case 10: case 11: {
        const v = buf.readBigInt64LE(o); // i64/u64 never enter hashed objects; keep raw
        o += 8;
        return v;
      }
      case 12: { o += 8; return null; } // f64: never hashed; not representable canonically
      default: throw new Error(`unsupported GGUF value type ${type}`);
    }
  };

  const kv = new Map();
  for (let i = 0; i < kvCount; i += 1) {
    const key = readString();
    const type = buf.readUInt32LE(o); // gguf_metadata_kv_t.value_type is uint32
    o += 4;
    if (!GGUF_VALUE_TYPES.has(type)) throw new Error(`bad value type ${type} for key ${key}`);
    kv.set(key, readValue(type));
  }
  return kv;
}

// ---------- fail-closed extractors ----------

const need = (kv, key, check) => {
  if (!kv.has(key)) throw new Error(`fail-closed: GGUF metadata is missing required key "${key}"`);
  const v = kv.get(key);
  if (!check(v)) throw new Error(`fail-closed: key "${key}" has unexpected value ${JSON.stringify(v)?.slice(0, 60)}`);
  return v;
};
const isU32 = (v) => typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= 0xffffffff;
const isBool = (v) => typeof v === "boolean";
const isStr = (v) => typeof v === "string";
const isI32Array = (v) => Array.isArray(v) && v.every((x) => Number.isInteger(x));
const isU32Array = (v) => Array.isArray(v) && v.every((x) => Number.isInteger(x) && x >= 0 && x <= 0xffffffff);
const isStrArray = (v) => Array.isArray(v) && v.every((x) => typeof x === "string");

// llama.cpp LLAMA_FTYPE enum (llama.h) for the quantizations we register.
const FTYPE_TO_QUANT = new Map([
  [2, ["q4_0", 4]], [7, ["q8_0", 8]], [8, ["q5_0", 5]], [9, ["q5_1", 5]],
  [10, ["q2_k", 2]], [11, ["q3_k_s", 3]], [12, ["q3_k_m", 3]], [13, ["q3_k_l", 3]],
  [14, ["q4_k_s", 4]], [15, ["q4_k_m", 4]], [16, ["q5_k_s", 5]], [17, ["q5_k_m", 5]],
  [18, ["q6_k", 6]],
]);

// ---------- args ----------

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const flag = argv[i];
    if (!flag.startsWith("--")) throw new Error(`unexpected argument ${flag}`);
    out[flag.slice(2)] = argv[i + 1];
  }
  for (const required of ["repo", "file", "quant-method", "quant-bits", "display-name"]) {
    if (!out[required]) throw new Error(`missing --${required}`);
  }
  return out;
}

// ---------- main ----------

const args = parseArgs(process.argv.slice(2));
const vectorsChecked = selfTestAgainstGoldenVectors();
console.error(`self-test: canonical derivation matches ${vectorsChecked} golden vectors`);

// 1. Resolve the pinned revision from the HF API (never a branch name).
const api = await fetch(`https://huggingface.co/api/models/${args.repo}`);
if (!api.ok) throw new Error(`HF api ${api.status} for ${args.repo}`);
const model = await api.json();
const revision = model.sha;
if (!/^[0-9a-f]{40}$/.test(revision ?? "")) throw new Error(`HF api returned no 40-hex sha (${revision})`);
console.error(`revision: ${args.repo}@${revision}`);

// 2. Artifact bytes: cache dir if present, else download once.
const cacheDir = args["cache-dir"] ?? join(tmpdir(), "msp-resolve-candidate");
await mkdir(cacheDir, { recursive: true });
const artifactPath = join(cacheDir, args.file);
if (!statSync(artifactPath, { throwIfNoEntry: false })) {
  const url = `https://huggingface.co/${args.repo}/resolve/${revision}/${args.file}`;
  console.error(`downloading ${url}`);
  const res = await fetch(url);
  if (!res.ok || !res.body) throw new Error(`artifact download failed: HTTP ${res.status}`);
  const { Readable } = await import("node:stream");
  const { createWriteStream } = await import("node:fs");
  await new Promise((resolve, reject) => {
    const sink = createWriteStream(artifactPath);
    Readable.fromWeb(res.body).pipe(sink);
    sink.on("finish", resolve);
    sink.on("error", reject);
  });
}
// Stream-hash + prefix-parse: artifacts at or above ~4 GB (everything from
// the 7B tier up) exceed Node's single-Buffer cap, so whole-file reads can
// never work for the full ladder. sha256 is chunk-streamed; the GGUF
// metadata section sits at the file start and is parsed from a prefix that
// grows on demand (readString guards against truncation — a short prefix is
// always a retry, never a wrong hash).
const artifactBytes = statSync(artifactPath).size;
const artifactSha = await (async () => {
  const { createReadStream } = await import("node:fs");
  const hash = createHash("sha256");
  await new Promise((resolve, reject) => {
    const src = createReadStream(artifactPath, { highWaterMark: 1 << 23 });
    src.on("data", (chunk) => hash.update(chunk));
    src.on("end", resolve);
    src.on("error", reject);
  });
  return hash.digest("hex");
})();
console.error(`artifact: ${args.file} ${artifactBytes} bytes sha256=${artifactSha}`);

// 3. GGUF metadata → identity hashes (ADR-022), fail-closed on the prefix.
const { open } = await import("node:fs/promises");
let kv = null;
for (let prefixLen = 64 << 20; prefixLen <= 1 << 30 && !kv; prefixLen *= 2) {
  const fh = await open(artifactPath, "r");
  try {
    const prefix = Buffer.alloc(prefixLen);
    const { bytesRead } = await fh.read(prefix, 0, Math.min(prefixLen, artifactBytes), 0);
    const view = prefix.subarray(0, bytesRead);
    try {
      kv = parseGgufMetadata(view);
    } catch (e) {
      // Whole file already in view → a real parse error, not a short prefix.
      if (artifactBytes <= bytesRead) throw e;
      if (!/prefix too small|out of range|out_of_bounds/i.test(String(e?.message ?? e))) throw e;
      console.error(`metadata prefix of ${bytesRead} bytes was short — retrying with a larger one`);
    }
  } finally {
    await fh.close();
  }
}
if (!kv) throw new Error("GGUF metadata section exceeds the 1 GB prefix cap");
const arch = need(kv, "general.architecture", isStr);

const tokens = need(kv, "tokenizer.ggml.tokens", isStrArray);
const tokenType = need(kv, "tokenizer.ggml.token_type", isI32Array);
const tokenizerModel = need(kv, "tokenizer.ggml.model", isStr);
if (tokens.length !== tokenType.length) {
  throw new Error(`tokenizer arrays disagree: ${tokens.length}/${tokenType.length}`);
}

// ADR-022 subset: exactly the identity-bearing GGUF keys llama.cpp artifacts
// actually carry for this architecture. qwen2 GGUFs have no tie_word_embeddings
// / vocab_size keys — vocab identity is pinned by the tokenizer arrays hashed
// into tokenizer_hash, so those keys are deliberately absent from this subset.
const archSubset = {
  "general.architecture": arch,
  "attention.head_count": need(kv, `${arch}.attention.head_count`, isU32),
  "attention.head_count_kv": need(kv, `${arch}.attention.head_count_kv`, isU32),
  "attention.layer_norm_rms_epsilon_bits": need(kv, `${arch}.attention.layer_norm_rms_epsilon`, isU32),
  "block_count": need(kv, `${arch}.block_count`, isU32),
  "context_length": need(kv, `${arch}.context_length`, isU32),
  "embedding_length": need(kv, `${arch}.embedding_length`, isU32),
  "feed_forward_length": need(kv, `${arch}.feed_forward_length`, isU32),
  "file_type": need(kv, "general.file_type", isU32),
  "rope.freq_base_bits": need(kv, `${arch}.rope.freq_base`, isU32),
};

// Everything that affects tokenize/detokenize behavior, from the GGUF itself.
// (No scores key: modern BPE conversions don't emit tokenizer.ggml.scores.)
// ADR-022 amendment (2026-10-06): keys a GGUF may legitimately omit
// (add_bos_token, padding_token_id — both absent in Qwen3.5-family
// conversions) hash as null. Absence is identity; never a guessed default.
const optionalBool = (key) => {
  if (!kv.has(key)) return null;
  const v = kv.get(key);
  if (typeof v !== "boolean") throw new Error(`fail-closed: key "${key}" has unexpected value ${JSON.stringify(v)}`);
  return v;
};
const optionalU32 = (key) => {
  if (!kv.has(key)) return null;
  const v = kv.get(key);
  if (!isU32(v)) throw new Error(`fail-closed: key "${key}" has unexpected value ${JSON.stringify(v)}`);
  return v;
};
const tokenizerObject = {
  add_bos_token: optionalBool("tokenizer.ggml.add_bos_token"),
  bos_token_id: optionalU32("tokenizer.ggml.bos_token_id"),
  eos_token_id: need(kv, "tokenizer.ggml.eos_token_id", isU32),
  merges: need(kv, "tokenizer.ggml.merges", isStrArray),
  model: tokenizerModel,
  padding_token_id: optionalU32("tokenizer.ggml.padding_token_id"),
  pre: need(kv, "tokenizer.ggml.pre", isStr),
  token_type: tokenType,
  tokens,
};
const chatTemplate = need(kv, "tokenizer.chat_template", isStr);

const tokenizerHash = sha256Hex(Buffer.from(canonicalJson(tokenizerObject), "utf8"));
const chatTemplateHash = sha256Hex(Buffer.from(chatTemplate, "utf8"));
const architectureHash = sha256Hex(Buffer.from(canonicalJson(archSubset), "utf8"));
console.error(`hashes: tokenizer=${tokenizerHash.slice(0, 16)}… template=${chatTemplateHash.slice(0, 16)}… arch=${architectureHash.slice(0, 16)}… vocab=${tokens.length}`);

// 4. Quantization cross-check: GGUF file_type must agree with --quant-*.
const ftypeQuant = FTYPE_TO_QUANT.get(archSubset.file_type);
const quantMethod = args["quant-method"];
const quantBits = Number(args["quant-bits"]);
if (!ftypeQuant) throw new Error(`general.file_type=${archSubset.file_type} not in the known quant table — extend FTYPE_TO_QUANT first`);
if (ftypeQuant[0] !== quantMethod || ftypeQuant[1] !== quantBits) {
  throw new Error(`quantization mismatch: GGUF file_type says ${ftypeQuant[0]}:${ftypeQuant[1]}, args say ${quantMethod}:${quantBits}`);
}

// 5. Engine pin → runtime block.
const pins = JSON.parse(readFileSync(join(repoRoot, "runtime-pins.json"), "utf8"));
// runtime-pins.json went multi-platform: the cross-OS anchor is
// canonical_build_hash (the old single-zip `zip_sha256` key is gone).
const buildHash = pins.canonical_build_hash ?? pins.zip_sha256;
if (!/^[0-9a-f]{64}$/.test(buildHash ?? "")) {
  throw new Error("runtime-pins.json carries no 64-hex canonical build hash — refusing to resolve");
}
const runtime = {
  name: "llama.cpp",
  version: pins.tag,
  build_hash: buildHash,
};

// 6. Manifest + derived id + schema validation (Ajv from the tracker's deps).
const manifest = {
  schema_version: 2,
  hf_repo: args.repo,
  hf_revision: revision,
  artifact_hashes: [{ path: args.file, sha256: artifactSha }],
  tokenizer_hash: tokenizerHash,
  chat_template_hash: chatTemplateHash,
  architecture_hash: architectureHash,
  quantization: { method: quantMethod, bits: quantBits },
  runtime,
  decoding_abi_version: 1,
  speculative_capabilities: ["proposal_tokens"],
};
const profileId = deriveProfileId(manifest);

const require_ = createRequire(join(repoRoot, "apps", "tracker", "package.json"));
const Ajv2020 = require_("ajv/dist/2020.js");
const ajv = new Ajv2020({ allErrors: true });
const schema = JSON.parse(readFileSync(join(repoRoot, "catalog", "schema-v2.json"), "utf8"));
const validateManifest = ajv.compile({ ...schema, $ref: "#/$defs/ModelProfileManifest" });
if (!validateManifest(manifest)) throw new Error(`manifest fails schema-v2: ${ajv.errorsText(validateManifest.errors)}`);

const record = {
  manifest,
  profile_id: profileId,
  display_name: args["display-name"],
  status: "candidate",
  provenance: {
    resolvedBy: "scripts/resolve-candidate.mjs (Phase H, ADR-022)",
    resolvedAt: new Date().toISOString(),
    reviewedBy: "owner",
    licenseEvidenceUrl: args["license-url"] ?? `https://huggingface.co/${args.repo}/blob/${revision}/LICENSE`,
  },
};

const outPath = args.out ?? join(
  repoRoot, "catalog", "candidate-profiles",
  `${profileId.replace(/:/g, "_")}.json` // ':' is an NTFS ADS separator
);
writeFileSync(outPath, JSON.stringify(record, null, 2) + "\n");
console.error(`wrote ${outPath}`);
console.log(JSON.stringify({ profile_id: profileId, path: outPath, artifact_sha256: artifactSha }, null, 2));
