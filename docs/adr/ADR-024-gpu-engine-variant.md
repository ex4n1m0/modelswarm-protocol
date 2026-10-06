# ADR-024: GPU engine variant (Vulkan) with honest CPU fallback

- Status: accepted (2026-10-06)
- Owners: Runtime Engineer (engine + pins), Windows Product Engineer (bundling,
  UI disclosure), Test and Release (CI gates)
- Reviewers: Security (pin verification surface unchanged), Protocol Architect
  (identity semantics below)

## Context

Every ModelSwarm desktop bundle ships one pinned llama.cpp engine
(`runtime-pins.json`, tag b11407). The Windows/macOS/Linux CPU builds
(from the official ggml-org release archives) never touch a GPU: on the
owner's target hardware — a RTX 3080 10 GB in a 32 GB RAM PC — the
Qwen ladder's larger profiles (14B+) decode at a few tokens per second
even though the GPU sits idle. macOS already computes on GPU via the
Metal backend bundled in the pinned macos-arm64 archive; Windows and
Linux do not.

Two official GPU builds exist for tag b11407: CUDA (per-toolkit builds)
and Vulkan (single driver-only build covering NVIDIA, AMD, and Intel).

## Decision

1. **Vulkan variant first.** `runtime-pins.json` gains
   `platforms["windows-x64"].variants.vulkan` — the b11407
   `win-vulkan-x64` archive (sha256
   `a56279698e17484bfb55eaf98c16e4b7414481675ac94b309b5372aa73ae6f2a`),
   a 23-file bundle verified exactly like the canonical set. One build
   covers every GPU vendor with no toolkit prerequisite — the Vulkan
   loader ships inside every modern NVIDIA/AMD/Intel driver. A CUDA
   variant may be pinned later as another entry if benchmarks demand it;
   nothing in this ADR blocks that.
2. **Identity is unchanged by the backend.** The wire runtime descriptor
   stays `{ name: "llama.cpp", build_hash: canonical_build_hash }` on
   every backend. The canonical anchor remains the CPU windows-x64
   archive sha. Rationale: v0.x serving routes whole requests to one
   peer; backends never exchange tokens, so cross-backend FP differences
   cannot corrupt a swarm result. The backend is disclosed locally
   (telemetry `engine.started {backend}`, UI "Compute" line) — it is an
   execution detail, not a swarm-visible runtime change.
   **Forward constraint:** when cooperative inference phases begin
   (P0+), backend MUST become part of verifier-class eligibility —
   lossless verification cannot mix backends. This ADR explicitly does
   not grant permission for cross-backend token-level cooperation.
3. **GPU preferred, fallback honest.** When the desktop finds an
   `engine-vulkan/` bundle beside the CPU engine it starts it first.
   **No `-ngl` is passed** (amended 2026-10-06 night): b11407's
   auto-fit ABORTS when a user-pinned layer count cannot fit FREE VRAM
   (`failed to fit params to free device memory: n_gpu_layers already
   set by user to 999` — observed with Qwen3.8-27B on a 16 GB 5080);
   left unset the engine auto-fits: full offload when it fits, partial
   when it doesn't (measured: the 27B loads in ~20 s with warm shader
   cache). Any pin-verification, spawn, or health failure — now
   fail-fast, the supervisor flags a dead child instead of burning the
   startup window — logs `engine.gpu_fallback {variant, reason}` (warn)
   and starts the canonical CPU engine instead. There is no silent
   downgrade: the fallback is a telemetry event and the UI shows the
   backend actually serving. On machines without a Vulkan driver the
   variant fails fast at startup and the CPU engine serves, visibly.
4. **Bundling.** The Windows NSIS bundle ships BOTH engine sets
   (`engine/` + `engine-vulkan/`, tauri resources). Linux and macOS
   bundles ship the canonical engine only (the resource dir contains a
   placeholder README so the glob stays non-empty); the node resolves a
   GPU binary only on Windows layouts, so other OSes are unchanged.
   CI verifies every variant file before packaging AND requires
   `engine-vulkan/llama-server.exe` inside the built setup.exe (the
   v0.2.2 engine-less-installer lesson, now applied to the variant).
5. **Weights never move.** The variant changes only the compute engine
   binaries; artifact acquisition, profile identity, and the no-
   redistribution rule are untouched.

## Measured (honest) results

RTX 5080 Laptop (16 GB) + 24-thread CPU, b11407, `llama-bench`
(pp512/tg128, defaults; 27B single rep — noted, not hidden):

| Model | CPU tg128 | Vulkan tg128 (ngl 99) | Vulkan pp512 |
|---|---|---|---|
| Qwen2.5-0.5B Q4_K_M | 126.6 ± 11.8 tok/s | **517.0 ± 23.3 tok/s (4.1×)** | — |
| Qwen3.8-27B Q4_K_M (15.65 GiB) | 4.20 tok/s | **8.25 tok/s (2.0×)** | 522 tok/s |

The 27B fits fully in 16 GB VRAM (tight: 15.65 GiB weights); on the
owner's 10 GB RTX 3080 the same profile offloads partially
(`-ngl 999` degrades gracefully — measured ngl 40 on the 5080: 7.6 tok/s,
still 1.8× CPU). Decode is the honest headline: Vulkan is not CUDA — a
CUDA build would likely push 27B decode further; that comparison is a
follow-up, published whatever it shows.

## Consequences

- Installer grows ~45 MB compressed (second engine set) on Windows only.
- Nodes report identical swarm identity on CPU and GPU — the census
  cannot (and, in v0.x, must not) distinguish them; local disclosure
  carries the truth.
- `EngineSpec` gains `gpu_layers` + `backend`; `NodeConfig` gains
  `engine_gpu_binary`; `NodeHandle` exposes `engine_backend` for the UI.
- Every variant file is inside the fail-closed pin verification path:
  a tampered variant cannot launch and cannot bypass to serving.
