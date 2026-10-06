# Qwen catalog ladder + community model requests — review packet

Date: 2026-10-06 · Owner decision requested: promote the ladder (Track 1) and
deploy ADR-023 (Track 2) · Everything below is built, tested, and verified;
nothing is live yet.

## What was built

### Track 1 — Qwen ladder for 16/32 GB-RAM machines (candidates ready)

Owner steer (2026-10-06): *"all the most popular Qwen models that fit … 16 or
32 local ram … this solution is aiming at those with less hardware."* Nine
profiles resolved with `scripts/resolve-candidate.mjs` (ADR-022 hashes from
the GGUF bytes, engine pin `b11407`, quant cross-checked, schema-v2
validated). Every artifact sha256 independently re-verified against the HF
Hub LFS digest (**13/13 ✓** — ten ladder profiles plus the three existing).

| Model | Source repo | DL (GB) | ≈RAM to host | Tier | License (evidence) |
|---|---|---|---|---|---|
| Qwen2.5-1.5B-Instruct Q4_K_M | Qwen/Qwen2.5-1.5B-Instruct-GGUF | 1.12 | ~1.6 GB | 8 GB+ | Apache-2.0 (in-repo) |
| Qwen2.5-3B-Instruct Q4_K_M | Qwen/Qwen2.5-3B-Instruct-GGUF | 2.10 | ~2.6 GB | 8 GB+ | Apache-2.0 (in-repo) |
| Qwen3-1.7B Q4_K_M | ggml-org/Qwen3-1.7B-GGUF | 1.28 | ~1.8 GB | 8 GB+ | Apache-2.0 (upstream repo) |
| Qwen3-4B Q4_K_M | Qwen/Qwen3-4B-GGUF | 2.50 | ~2.9 GB | 8–16 GB | Apache-2.0 (in-repo) |
| Qwen2.5-7B-Instruct Q4_K_M | bartowski/Qwen2.5-7B-Instruct-GGUF † | 4.68 | ~5.1 GB | 16 GB | Qwen Research (upstream repo) |
| Qwen3-8B Q4_K_M | Qwen/Qwen3-8B-GGUF | 5.03 | ~5.5 GB | 16 GB | Apache-2.0 (in-repo) |
| Qwen2.5-14B-Instruct Q4_K_M | bartowski/Qwen2.5-14B-Instruct-GGUF † | 8.99 | ~9.4 GB | 16–32 GB | Qwen Research (upstream repo) |
| Qwen3-14B Q4_K_M | Qwen/Qwen3-14B-GGUF | 9.00 | ~9.5 GB | 16–32 GB | Apache-2.0 (in-repo) |
| Qwen3-30B-A3B Q4_K_M (MoE) | Qwen/Qwen3-30B-A3B-GGUF | 18.56 | ~19 GB | 32 GB | Apache-2.0 (in-repo) |
| Qwen2.5-32B-Instruct Q4_K_M | bartowski/Qwen2.5-32B-Instruct-GGUF † | 19.85 | ~20.3 GB | 32 GB | Qwen Research (upstream repo) |

† bartowski single-file builds used where the official repo ships **sharded**
Q4_K_M (our artifact pipeline pins exactly one file); provenance points at
the upstream Qwen license. Popularity: Qwen3-4B 551k and Qwen3-8B 527k
downloads (official repos), Qwen2.5-32B 542k (bartowski), Qwen2.5-14B 84k,
Qwen2.5-1.5B 302k — the "most popular" claim is from live Hub counters.

Honesty notes for review: the dense 32B will be slow on CPU (~1–2 tok/s) —
the 30B-A3B MoE (~3B active) is the recommended 32 GB flagship; recommend
test-loading the three biggest on a real node before promoting them (engine
`b11407` supports qwen3/qwen3moe arch, but a load test is cheap proof).

### Track 2 — community model requests (ADR-023, built + tested)

- `POST /api/v1/catalog/requests` — public, 10/min/IP, artifact-pointer-only
  strict schema (content-blind by construction), dedupe on sha256.
- `GET /api/v1/admin/catalog/requests` + `POST …/resolve` — the owner queue.
- Migration `0003_model_requests.sql`; store methods (Memory + Pg); 7 new
  tests; msp-v1 §3.1/§3.5 documented; ADR-023 written.
- Desktop: `hf_search_models` / `hf_list_ggufs` / `request_model` IPC +
  collapsible "more models — search Hugging Face" picker under Models (RAM
  estimate per file reuses `hardware_requirements`; requests report
  `queued ✓` / `already-requested`).

### Resolver fixes required for the ladder (kept fail-closed)

- Artifacts ≥4 GB exceeded Node's single-Buffer cap → stream-hashed sha256 +
  GGUF metadata parsed from a grow-on-demand prefix (truncation-guarded so a
  short prefix can never produce a wrong hash). Cross-checked vs HF LFS oid.
- `runtime-pins.json` renamed `zip_sha256` → `canonical_build_hash`; the
  resolver now reads the new key and refuses to run without a 64-hex pin.
- `--license-url` override added (bartowski mirrors → upstream license).

## Evidence

- Tracker: `npm run typecheck` ✓ · `npm run build` ✓ · `vitest run`
  102 passed / 2 skipped (pg suite needs DATABASE_URL) — incl. 7 new
  model-request tests.
- Desktop: `cargo fmt` ✓ · `cargo clippy --features tauri-shell -- -D
  warnings` ✓ · `cargo test --features tauri-shell` 5/5 (incl. quant-token
  and repo-id validation tests).
- Candidates: **13/13 sha256 matches vs HF `lfs.oid`** (all ten ladder runs
  exited OK; the 18.56 GB and 19.85 GB artifacts exercised the new streamed
  hashing path end-to-end).

## Owner commands (when you approve)

Publish candidates to the hub and promote (requires `ADMIN_TOKEN`; adjust
TRACKER for a staging pass):

```bash
cd "C:\ModelSwarm Protocol (MSP)"
TRACKER=https://modelswarm.deepflux.space
node -e '
const fs=require("fs");const dir="catalog/candidate-profiles";
const tracker=process.argv[1],token=process.env.ADMIN_TOKEN;
(async()=>{for(const f of fs.readdirSync(dir).filter(f=>f.endsWith(".json"))){
  const rec=JSON.parse(fs.readFileSync(dir+"/"+f,"utf8"));
  const r=await fetch(tracker+"/api/v1/admin/catalog/candidates",{method:"POST",
    headers:{"content-type":"application/json","x-msp-admin":token},
    body:JSON.stringify({manifest:rec.manifest,display_name:rec.display_name,status:"candidate",provenance:rec.provenance})});
  console.log(rec.display_name, r.status);
}})();' "$TRACKER"
# then promote each (profile ids print in catalog/candidate-profiles/*.json):
node -e '… POST /api/v1/admin/catalog/promote {profileId} …'
```

Then resolve any matching community requests via
`POST /api/v1/admin/catalog/requests/resolve`. Tracker deploy (Vercel CLI,
per the deploy-via-CLI rule) and a desktop release build remain owner-gated;
until the tracker is deployed the desktop picker's requests will 404 —
ship the two together.

## Not done (deliberately)

- The ≥7B profiles are **candidates only** — promoting is the owner's call
  after the load-test note above.
- "Full HF list" stays search-driven; no bulk mirror of the Hub (ADR-023).
- Start/stop/switch UX fixes (separate study,
  `docs/reviews/desktop-start-stop-ux-study-2026-10-06.md`) are untouched by
  this change.
