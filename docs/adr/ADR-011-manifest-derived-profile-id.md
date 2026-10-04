# ADR-011: Manifest-derived ModelProfileId and catalog schema v2

Status: Accepted (Phase A, 2026-10-04) · Amends ADR-005 (mechanism, not
principle) and supersedes `catalog/schema.json` v1's human-minted
`msp:<family>:<quant>:vN` identifiers. Schema v1 remains valid for the
`single`-mode catalog until v2 ships in Phase B; nothing has deployed, so no
migration of live data exists.

## Context

Human-minted IDs (`msp:qwen3-4b:q4_k_m:v1`) rely on an admin never making a
naming mistake and don't encode tokenizer/runtime/ABI identity the
cooperative modes need. Two peers with the same name but different chat
templates would be silently incompatible for speculation.

## Decision

### Manifest

`ModelProfileManifest` (schema: `catalog/schema-v2.json`) contains exactly
the identity-bearing fields from the revision: `schema_version`, `hf_repo`,
`hf_revision`, `artifact_hashes[]`, `tokenizer_hash`, `chat_template_hash`,
`architecture_hash`, `quantization`, `runtime {name, version, build_hash}`,
`decoding_abi_version`, `speculative_capabilities[]`. Plus UI-only wrapper
fields (`display_name`, `status`, `provenance`) that are **not** hashed.

### Canonical serialization (the only accepted form)

UTF-8 JSON; no insignificant whitespace; object keys sorted lexicographically
(recursive); integers decimal; strings as-is (no escaping changes beyond
JSON minimum); no trailing newline. Same rules as msp-v1 §2.2.

### Derivation

```text
manifest_digest = sha256(canonical_json(manifest_hash_fields))
ModelProfileId  = "msp1:" + hex(manifest_digest)
```

The ID is displayed/stored; `display_name` is free text for humans. Any
change to any hashed field yields a different ID and a different swarm.

### Possession (amends msp-v1 §3.3 challenges)

1. Full-artifact SHA-256 (existing).
2. Randomized chunk challenge: hub names offset+length; peer answers
   sha256 of that byte range — proves file access, not just a hash claim.
3. Live inference challenge (existing): proves the artifact executes.

### Hash inputs, fail-closed rules

`chat_template_hash` = sha256 of the tokenizer's chat template string;
`architecture_hash` = sha256 of the canonicalized architecture-relevant
subset of `config.json` (model_type, context/window fields, rope/attention
config keys — enumerated exactly in the resolver spec, Phase B). If any
input cannot be resolved from the HF revision, the resolver **fails closed**
— no guessed hashes, ever (original plan rule).

### Golden vectors

`protocol/vectors/manifest-*.json`: `{manifest, expected_profile_id}`.
`expected_profile_id` values are computed, never typed by hand. The same
fixtures are consumed by the Node validator (`apps/tracker/scripts/`)
today and by Rust `modelswarm-types` tests in Phase B (implementation of
the serializer/derivation is Phase B deliverable; Phase A ships schema +
vectors + validator).

## Consequences

+ Cooperative compatibility becomes mechanical: same ID ⇒ same tokenizer,
  template, runtime build, and decoding ABI.
+ No admin naming mistakes can split or merge swarms.
− Breaking change to frozen schema v1 (accepted pre-deployment).
− Resolver complexity grows (fail-closed hash evidence).
