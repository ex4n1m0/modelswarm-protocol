# ADR-005: Model profiles pin immutable HF revisions and digests

Status: Accepted (Phase 0)

## Context

If "Qwen3 4B Q4_K_M" silently means "whatever `main` holds today," two peers
with the same label can hold different weights, chat templates, or tokenizers —
incompatible replicas that fragment the swarm and quietly change model
behavior. Hugging Face allows downloads pinned to a full commit hash.

## Decision

Every catalog profile records the **full 40-character commit hash, exact
filename, byte size, and SHA-256 digest** of the approved artifact
(`catalog/schema.json`). Profile IDs are immutable: any change to repo,
revision, file, digest, quantization, context policy, or runtime floor creates
a **new `ModelProfileId`** (separate swarm). Nodes verify digests after
download and before serving. Updates are staged: `candidate` → test-node
validation → new immutable id published → background download → `active` flip
once enough hosts are ready → old id `deprecated`. Nothing ever resolves to a
branch or tag.

## Consequences

+ Swarms are homogeneous by construction; the acceptance matrix's
  "same name, different hash → never share a swarm" holds mechanically.
+ Reproducible audits: any node's artifact is byte-identical.
+ HF `main` moving under us changes nothing until an admin promotes.
− Real update churn (storage of old + new artifacts during transition).
− Admin procedure required for every model refresh (intentional).

## Alternatives rejected

- **Track `main`**: silent swarm splits — the exact failure this ADR prevents.
- **Tag pinning**: tags are mutable in HF repos; only commit hashes are not.
