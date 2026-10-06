# ADR-022: Real model-profile registration (GGUF-anchored identity hashes)

Status: Accepted (Phase H, 2026-10-05) · Extends ADR-011 (which deferred the
exact hash inputs to "the resolver spec, Phase B" — this is that spec) and
ADR-008 (engine pinning). First real profile registered under it:
`msp1:eb0a0d21a61daa85e9c4f382c45ebcc13c6d346cc5e402e22a4b4e38bcd8120c`
(Qwen2.5-0.5B-Instruct Q4_K_M).

## Context

Schema v2 manifests hash `tokenizer_hash`, `chat_template_hash`, and
`architecture_hash`, but the only real HF artifacts we register are GGUF-only
repos (e.g. `Qwen/Qwen2.5-0.5B-Instruct-GGUF`) that carry **no**
`tokenizer.json` / `config.json` sidecars. ADR-011's fail-closed rule forbids
guessing hash inputs, so the resolver must draw every identity hash from a
source that (a) actually exists in the artifact set and (b) a node can
re-verify from what it possesses.

## Decision

**All identity hashes derive from the GGUF artifact itself** — the file the
node downloads and serves from. The runtime that executes it (llama.cpp)
loads its tokenizer, chat template, and architecture from exactly this file,
so GGUF-anchored hashes pin the behavior that matters. Sibling HF files are
never hashed.

### 1. GGUF metadata reading (resolver + Rust re-verification)

GGUF v3 only. Layout per the gguf spec: magic `GGUF`, `version` u32, then
u64 `tensor_count`, u64 `metadata_kv_count`, then KVs of
`[u64 key_len | key utf8 | u32 value_type | value]`. Strings decode as
lossy UTF-8 (U+FFFD per invalid sequence); the Rust reader uses
`from_utf8_lossy`. Tokenizer byte-fallback tokens are single bytes, where
Node and Rust replacement semantics coincide.

### 2. `tokenizer_hash`

sha256 of canonical JSON (msp-v1 §2.2) of the object with exactly these
GGUF-sourced members:

```json
{
  "add_bos_token": <bool tokenizer.ggml.add_bos_token>,
  "bos_token_id":  <u32 tokenizer.ggml.bos_token_id>,
  "eos_token_id":  <u32 tokenizer.ggml.eos_token_id>,
  "merges":        [string … tokenizer.ggml.merges],
  "model":         "<tokenizer.ggml.model>",
  "padding_token_id": <u32 tokenizer.ggml.padding_token_id>,
  "pre":           "<tokenizer.ggml.pre>",
  "token_type":    [i32 … tokenizer.ggml.token_type],
  "tokens":        [string … tokenizer.ggml.tokens]
}
```

Everything that affects tokenize/detokenize. `tokenizer.ggml.scores` is
deliberately absent: modern BPE conversions do not emit it (fail-closed
would otherwise reject every current llama.cpp GGUF).

### 3. `chat_template_hash`

sha256 of the raw UTF-8 bytes of the `tokenizer.chat_template` string
(ADR-011's rule, applied to the GGUF-embedded template).

### 4. `architecture_hash`

sha256 of canonical JSON of the subset with exactly these members (GGUF key
names, `{arch}` = `general.architecture`):

| Member | GGUF key |
|---|---|
| `general.architecture` | string |
| `attention.head_count` | u32 `{arch}.attention.head_count` |
| `attention.head_count_kv` | u32 `{arch}.attention.head_count_kv` |
| `attention.layer_norm_rms_epsilon_bits` | f32 → bits (rule 5) |
| `block_count` | u32 `{arch}.block_count` |
| `context_length` | u32 `{arch}.context_length` |
| `embedding_length` | u32 `{arch}.embedding_length` |
| `feed_forward_length` | u32 `{arch}.feed_forward_length` |
| `file_type` | u32 `general.file_type` |
| `rope.freq_base_bits` | f32 → bits (rule 5) |

`{arch}.tie_word_embeddings` and `{arch}.vocab_size` are **deliberately not
hashed**: llama.cpp's qwen2 converter emits neither; vocab identity is pinned
by the tokenizer arrays inside `tokenizer_hash`. Any listed key missing from
an artifact aborts the resolver (fail-closed, ADR-011).

### 5. f32 canonicalization

Canonical JSON forbids floats. GGUF f32 members are hashed as the **decimal
u32 of their IEEE-754 bit pattern** (member names carry a `_bits` suffix).
Deterministic across languages, no float-formatting drift.

### 6. Engine pin → `runtime.build_hash`

`runtime-pins.json` pins one llama.cpp CPU-x64 release: `tag`, `zip_url`,
`zip_sha256`, per-file sha256 for every zip member, and the installer
`bundle[]` subset. `manifest.runtime.build_hash = zip_sha256` — one digest
transitively pins every engine binary. `runtime.version = tag` (e.g.
`b11407`). Installers verify extracted files against `files{}`; nodes can
re-verify their engine directory the same way (ADR-008: the engine ships in
the installer; **model weights never do** — first-run download only).

### 7. Artifact URL derivation (no schema change)

Download URLs are derived, not stored:
`https://huggingface.co/{hf_repo}/resolve/{hf_revision}/{artifact_hashes[].path}`.
Schema v2 is unchanged, so no new golden-vector obligations arise from URLs.

### 8. Cross-language parity gates

- The resolver self-tests its canonical serialization against every
  `protocol/vectors/manifest-*.json` before resolving anything.
- Real profiles are added as golden vectors (the Qwen profile is
  `manifest-qwen25-05b-q4km-real.json`), gated by the Node validator and the
  Rust `modelswarm-types` derivation test.
- The Rust artifact subsystem (Phase H2) re-derives `tokenizer_hash` /
`chat_template_hash` / `architecture_hash` from GGUF bytes and must equal the
manifest values — asserted offline in unit tests with a synthetic GGUF and
against the real artifact in the env-gated real-model CI job.

### 9. Quantization cross-check

`general.file_type` must map through the resolver's LLAMA_FTYPE table to the
declared `quantization {method, bits}` (e.g. 15 → `q4_k_m`/4) — mismatch
aborts.

## Consequences

+ Identity is verifiable from possession alone (chunk challenges can probe
  the same bytes the hashes came from).
+ Works for every GGUF-only repo; no dependence on sidecar files staying in
  sync with the quantized artifact.
− The subset is llama.cpp-conversion-shaped; a non-llama.cpp GGUF producer
  with different metadata key coverage would need the subset extended (new
  ADR or a v2 subset hash — a manifest change means a new profile id anyway).
− Rust-side GGUF metadata parsing becomes node code (~150 lines, no deps).

## Amendment (2026-10-06): absent tokenizer keys hash as null

Qwen3.5-family GGUF conversions (unsloth, lmstudio-community — every ungated
source checked) omit `tokenizer.ggml.add_bos_token`; the original spec's
fail-closed `need()` therefore rejected the whole line. Absence is not an
anomaly to refuse — it is tokenizer identity like any value.

**Rule:** within the `tokenizer_hash` object, `add_bos_token` and
`padding_token_id` are **nullable** — when the GGUF omits the key, the
canonical object carries `null` (JSON null, canonical-stable). No default
is ever guessed. Every other member remains fail-closed on absence, and a
present-but-wrong-type key still errors.

- Backward compatible: artifacts that carry the keys hash byte-identically
  to before (all 14 promoted profiles re-verified unchanged by the
  golden-vector gate).
- Parity lock: `gguf.rs::absent_tokenizer_keys_hash_as_null_amendment`
  pins the exact digest of a null-carrying tokenizer object against the
  node resolver's canonical derivation
  (`02400c13…86e20`).
- Scope note: this amendment does not extend to `architecture_hash` or
  `chat_template_hash` inputs — those keys have no observed-absent case;
  new absent-key cases get their own amendment with evidence.

## Amendment 2 (2026-10-06, later the same day): absent `bos_token_id` hashes as null

Evidence: `unsloth/Qwen3.5-9B-GGUF@3885219b` (Qwen3.5-9B Q4_K_M, the
RTX-3080-tier artifact) omits `tokenizer.ggml.bos_token_id` as well.
Same rule, same reasoning as Amendment 1: absence is identity-relevant
and hashes as `null`; never an error, never a guessed default.

- `tokenizer.ggml.bos_token_id` is now nullable in both derivations
  (`gguf.rs`, `scripts/resolve-candidate.mjs`).
- `tokenizer.ggml.eos_token_id` stays REQUIRED: decode termination
  (`TokenVocab::eos_id`) depends on it, so tolerance there would let an
  unservable artifact into the catalog.
- Parity lock: `absent_bos_token_id_hashes_as_null_amendment_2` pins
  digest `d525b0cf…79e900` (absent add_bos + padding + bos together).
- Shipped with the promotion of
  `msp1:d45c55cce8469410708595a01672f4cacf1c842328a64b93266661954c5fbc80`
  (artifact sha256 `9b86850a…77db`, catalogVersion 34).
