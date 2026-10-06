# ADR-023: Community model requests (HF-pointer request queue)

Status: Accepted (2026-10-06) · Extends ADR-005 (immutability), ADR-011/022
(resolver + identity hashes), ADR-022's resolver pipeline (candidate →
promote).

## Context

The catalog is intentionally curated: only active-catalog profiles may
register (`POST /peers/register` rejects `unknown_profile`), every artifact
is pinned to a full HF commit + sha256, and promotion is owner-gated. Users
asked to "select from the full Hugging Face list". A free-for-all breaks the
exact-profile swarm rule (arbitrary picks = swarms of one, unreviewed
artifacts, engine-compat surprises). But the desktop already has everything
needed to *find* any GGUF via the public HF Hub API — search, file tree with
byte sizes and LFS sha256 per file.

## Decision

**Requests, not self-service.** Two mechanisms, both keeping promotion
owner-gated:

1. **Ladder growth (resolver, unchanged flow)** — the owner resolves popular
   models (first wave: the Qwen ladder for 16/32 GB-RAM machines) with
   `scripts/resolve-candidate.mjs` and promotes via the existing admin
   routes. No protocol change.

2. **Community request queue (this ADR)** — new public endpoint plus admin
   queue management:

   - `POST /api/v1/catalog/requests` — public, enrollment-grade rate limit
     (10/min/IP), body is exactly
     `{hf_repo, hf_revision, artifact_path, artifact_sha256, artifact_bytes,
     quant_method, quant_bits, display_name, note?}`.
   - `GET /api/v1/admin/catalog/requests` — the owner's queue (open first).
   - `POST /api/v1/admin/catalog/requests/resolve` —
     `{id, resolution: "promoted"|"rejected"}` closes the loop.

   The desktop fills the request body **from the HF Hub API** (models search
   → tree listing with `lfs.oid` sha256 and size) — the values are
   hub-verified pointers, not user free text beyond the bounded display name
   and optional 280-char note. The owner turns an accepted request into a
   candidate with the same resolver script (which re-derives every hash from
   the artifact bytes — client hints are display-only), then promotes.

### Content-blindness and safety

- The schema is GGUF-pointer-only: no field can carry prompts or
  completions (the strict-zod pattern of msp-v1 §3 applies — unknown keys
  rejected).
- Dedupe on `artifact_sha256` makes the queue idempotent and un-spammable
  per model; per-IP rate limit bounds spam across models.
- Unreviewed artifacts never serve: the request queue has no serving
  semantics; nothing bypasses candidates → promote.
- The requested repo/revision is recorded but the resolver re-resolves and
  re-hashes everything fail-closed before a candidate exists.

### Client surface (desktop)

Three new IPC commands in `crates/modelswarm-desktop` (all HTTP from the
Rust process; the webview CSP never gains a network origin):

- `hf_search_models(query)` — HF `/api/models?search=…&library=gguf`, sorted
  by downloads, bounded list.
- `hf_list_ggufs(repo)` — pinned revision sha + GGUF file list
  (path, bytes, sha256, quant parsed from the filename).
- `request_model(request)` — POST to the tracker's new endpoint; the UI
  reports `queued` / `already-requested` honestly.

## Consequences

- The tracker gains one table (`model_requests`, migration 0003) and three
  routes; the signed catalog envelope is untouched.
- The desktop UI gains a collapsible "search Hugging Face" panel under
  Models that files requests; nothing about start/stop/switch changes.
- "Full HF list" remains search-driven — no bulk mirror of the Hub, ever.
- Rejected requests stay recorded (audit trail of what the community wants
  and what the owner declined).
