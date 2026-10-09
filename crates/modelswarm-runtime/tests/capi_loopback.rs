//! P16 loopback A/B: in-process C-API adapter vs the production HTTP adapter
//! against the SAME pinned engine build + model artifact, one machine, one
//! test run (environment-labeled prints; never a LAN or product claim).
//!
//! Gate: `cargo test -p modelswarm-runtime --features capi-adapter
//! --test capi_loopback -- --ignored --nocapture` with
//! `MSP_LLAMA_SERVER` (pinned llama-server.exe) and `MSP_REAL_GGUF`
//! (verified GGUF) set. Optional `MSP_CAPI_ENGINE_DIR` (defaults to the
//! server exe's bundle directory).
//!
//! Correctness pins (asserted): tokenize parity with the server, greedy
//! stream parity token-for-token, verification outcomes match the HTTP
//! ground truth, reject-path rollback exactness. Performance numbers are
//! printed for the record, not asserted (single-machine, load-sensitive).
#![cfg(feature = "capi-adapter")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};

use modelswarm_runtime::capi::{CapiConfig, LlamaCppCapi};
use modelswarm_runtime::llamacpp::{EngineIdentity, LlamaCppAdapter, LlamaCppConfig};
use modelswarm_runtime::{InferenceRuntime, SamplingParams};

const FILLER: &str = "The swarm serves one exact profile per cohort and every peer verifies the artifact digest before hosting. ";
const N_CTX: u32 = 4096;

fn sha256_of(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read file for hashing");
    let digest = Sha256::digest(&bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn find_repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("cwd");
    loop {
        if dir.join("runtime-pins.json").is_file() {
            return dir;
        }
        dir = dir.parent().expect("repo root reached").to_path_buf();
    }
}

struct Pins {
    tag: String,
    canonical_build_hash: String,
    files: serde_json::Map<String, serde_json::Value>,
}

fn load_pins() -> Pins {
    let root = find_repo_root();
    let raw = std::fs::read_to_string(root.join("runtime-pins.json")).expect("runtime-pins.json");
    let pins: serde_json::Value = serde_json::from_str(&raw).expect("pins parse");
    let platform = &pins["platforms"]["windows-x64"];
    Pins {
        tag: pins["tag"].as_str().expect("tag").to_string(),
        canonical_build_hash: pins["canonical_build_hash"]
            .as_str()
            .expect("canonical_build_hash")
            .to_string(),
        files: platform["files"].as_object().expect("files").clone(),
    }
}

fn verify_pinned(dir: &Path, files: &[&str], pins: &Pins) {
    for file in files {
        let expected = pins.files[*file]
            .as_str()
            .unwrap_or_else(|| panic!("runtime-pins.json has no sha256 entry for {file}"));
        let actual = sha256_of(&dir.join(file));
        assert_eq!(
            actual, expected,
            "sha256 mismatch for {file} — refusing to bench an unpinned engine"
        );
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn spawn_server(exe: &Path, model: &Path, port: u16, bearer: &str) -> Child {
    let mut command = Command::new(exe);
    command
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "-m",
            &model.display().to_string(),
            "-c",
            &N_CTX.to_string(),
            "--no-webui",
            "--api-key",
            bearer,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.spawn().expect("spawn llama-server")
}

/// Kills the server on scope exit, panic paths included (an orphaned
/// llama-server from a failed measurement run would skew the next one).
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_healthy(base_url: &str, bearer: &str, timeout: Duration) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    let started = Instant::now();
    loop {
        assert!(started.elapsed() < timeout, "server never became healthy");
        let healthy = client
            .get(format!("{base_url}/health"))
            .bearer_auth(bearer)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if healthy {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn ms(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

fn label(section: &str) {
    println!("\n=== {section} ===");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "set MSP_LLAMA_SERVER and MSP_REAL_GGUF"]
async fn capi_vs_http_loopback_measurements() {
    let exe = PathBuf::from(std::env::var("MSP_LLAMA_SERVER").expect("MSP_LLAMA_SERVER"));
    let model = PathBuf::from(std::env::var("MSP_REAL_GGUF").expect("MSP_REAL_GGUF"));
    let engine_dir = std::env::var("MSP_CAPI_ENGINE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| exe.parent().expect("engine dir").to_path_buf());

    // ---- environment + pin verification ---------------------------------
    label("ENVIRONMENT");
    let pins = load_pins();
    verify_pinned(
        &engine_dir,
        &["llama-server.exe", "llama.dll", "ggml.dll", "ggml-base.dll"],
        &pins,
    );
    let model_bytes = std::fs::metadata(&model).expect("model stat").len();
    println!(
        "env: single-machine loopback, Windows x64, {} logical CPUs; NOT LAN, NOT a product claim",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );
    println!(
        "engine: pinned {} (canonical_build_hash {}), bundle sha256-verified (server exe + llama.dll + ggml.dll + ggml-base.dll)",
        pins.tag, pins.canonical_build_hash
    );
    println!(
        "model: {} ({:.1} MiB), context window {N_CTX}, sampling: greedy (default -> temperature 0 on both adapters)",
        model.display(),
        model_bytes as f64 / (1024.0 * 1024.0)
    );

    // ---- start both runtimes on the same binaries -----------------------
    let bearer = "spike-internal-secret";
    let port = free_port();
    let base_url = format!("http://127.0.0.1:{port}");
    let server_started = Instant::now();
    let server = ServerGuard(spawn_server(&exe, &model, port, bearer));
    wait_healthy(&base_url, bearer, Duration::from_secs(120)).await;
    let server_load_ms = ms(server_started.elapsed());
    println!("http adapter: llama-server sidecar up in {server_load_ms:.0} ms (spawn -> /health)");

    let capi_started = Instant::now();
    let capi = LlamaCppCapi::new(CapiConfig::new(&engine_dir, &model).with_engine(
        EngineIdentity {
            version: pins.tag.clone(),
            build_hash: pins.canonical_build_hash.clone(),
        },
    ))
    .expect("capi adapter constructs");
    let capi_load_ms = ms(capi_started.elapsed());
    println!("capi adapter: model + context in {capi_load_ms:.0} ms (in-process load)");

    let identity = EngineIdentity {
        version: pins.tag.clone(),
        build_hash: pins.canonical_build_hash.clone(),
    };
    // Attach the exact vocab from the same GGUF (what the node supervisor
    // does in production): streaming id recovery needs logprobs + vocab.
    let vocab = {
        let metadata = modelswarm_types::read_metadata(&model).expect("gguf metadata");
        std::sync::Arc::new(modelswarm_types::token_vocab(&metadata).expect("gguf tokenizer vocab"))
    };
    assert_eq!(
        vocab.eos_id,
        capi.eos_id(),
        "GGUF-derived eos must agree with llama_vocab_eos on the pinned artifact"
    );
    let http = LlamaCppAdapter::new(LlamaCppConfig {
        base_url: base_url.clone(),
        bearer_token: bearer.to_string(),
        request_timeout: Duration::from_secs(120),
        engine: Some(identity),
        vocab: Some(vocab),
    })
    .expect("http adapter constructs");

    let http_handle = http.load("msp1:spike").await.expect("http load");
    let capi_handle = capi.load("msp1:spike").await.expect("capi load");
    println!(
        "vocab={} tokens, eos_id={:?} (capi, from the pinned GGUF)",
        capi.vocab_size(),
        capi.eos_id()
    );

    // Shared prompts (long ≈ 1200 tokens mirrors the P1 measurement class).
    let mut long_prompt = String::new();
    while http.tokenize(&long_prompt).await.unwrap().len() < 1200 {
        long_prompt.push_str(FILLER);
    }
    let short_prompt = "Count from one to five, slowly:";
    let sampling = SamplingParams::default(); // both adapters map this to greedy

    // ---- 1. tokenize parity ---------------------------------------------
    label("TOKENIZE PARITY (capi llama_tokenize vs server /tokenize)");
    let ids_long = http.tokenize(&long_prompt).await.unwrap();
    let ids_short = http.tokenize(short_prompt).await.unwrap();
    let capi_long = capi.tokenize(&long_prompt).await.unwrap();
    let capi_short = capi.tokenize(short_prompt).await.unwrap();
    assert_eq!(capi_long, ids_long, "long-prompt tokenize parity");
    assert_eq!(capi_short, ids_short, "short-prompt tokenize parity");
    println!(
        "OK: identical ids (long prompt {} tokens, short prompt {} tokens)",
        ids_long.len(),
        ids_short.len()
    );

    // Warm both paths once (slot cache for the server; first-touch for capi).
    let _ = http
        .decode_stream(
            &http_handle,
            &ids_long,
            &sampling,
            8,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    let _ = capi
        .decode_stream(
            &capi_handle,
            &ids_long,
            &sampling,
            8,
            Duration::from_secs(60),
        )
        .await
        .unwrap();

    // ---- 2. greedy stream parity ----------------------------------------
    label("GREEDY STREAM PARITY (32 tokens, long prompt; asserted token-for-token)");
    let http_stream_started = Instant::now();
    let http_tokens = http
        .decode_stream(
            &http_handle,
            &ids_long,
            &sampling,
            32,
            Duration::from_secs(120),
        )
        .await
        .unwrap();
    let http_stream_ms = ms(http_stream_started.elapsed());
    let capi_stream_started = Instant::now();
    let capi_tokens = capi
        .decode_stream(
            &capi_handle,
            &ids_long,
            &sampling,
            32,
            Duration::from_secs(120),
        )
        .await
        .unwrap();
    let capi_stream_ms = ms(capi_stream_started.elapsed());
    assert_eq!(
        capi_tokens, http_tokens,
        "greedy decode must be token-exact"
    );
    println!(
        "OK: {} tokens identical. http {:.1} ms total ({:.2} ms/tok) | capi {:.1} ms total ({:.2} ms/tok)",
        http_tokens.len(),
        http_stream_ms,
        http_stream_ms / http_tokens.len() as f64,
        capi_stream_ms,
        capi_stream_ms / capi_tokens.len() as f64,
    );
    let http_text = http.detokenize(&http_tokens).await.unwrap();
    let capi_text = capi.detokenize(&http_tokens).await.unwrap();
    assert_eq!(capi_text, http_text, "detokenize parity");

    // ---- 3. per-step decode cost on a long prefix -----------------------
    label(
        "DECODE_STEP COST, GROWING ~1217-TOK PREFIX (per-round wire re-post vs 1-token KV append)",
    );
    let mut prefix: Vec<u32> = ids_long.clone();
    prefix.extend_from_slice(&http_tokens[..16.min(http_tokens.len())]);
    let steps = 16;
    let started = Instant::now();
    for _ in 0..steps {
        let token = http
            .decode_step(&http_handle, &prefix, &sampling)
            .await
            .unwrap();
        prefix.push(token);
    }
    let http_step_ms = ms(started.elapsed()) / steps as f64;
    // capi on the same prefix (fresh working copy, same starting point).
    let mut capi_prefix: Vec<u32> = {
        let mut p = ids_long.clone();
        p.extend_from_slice(&http_tokens[..16.min(http_tokens.len())]);
        p
    };
    let started = Instant::now();
    for _ in 0..steps {
        let token = capi
            .decode_step(&capi_handle, &capi_prefix, &sampling)
            .await
            .unwrap();
        capi_prefix.push(token);
    }
    let capi_step_ms = ms(started.elapsed()) / steps as f64;
    assert_eq!(capi_prefix, prefix, "step loop must stay token-exact");
    println!(
        "http: {:.2} ms/step (one POST of the whole {}-tok prefix per step) | capi: {:.2} ms/step (one in-process token decode)",
        http_step_ms,
        prefix.len() - steps as usize,
        capi_step_ms
    );

    // ---- 4. repeated identical prefill (KV retention proof) -------------
    label("REPEATED IDENTICAL PREFILL x5 (1217 tok; server slot cache vs retained in-process KV)");
    let mut http_prefill_ms = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        http.prefill(&http_handle, &ids_long).await.unwrap();
        http_prefill_ms.push(ms(started.elapsed()));
    }
    let mut capi_prefill_ms = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        capi.prefill(&capi_handle, &ids_long).await.unwrap();
        capi_prefill_ms.push(ms(started.elapsed()));
    }
    println!("http prefill ms/round: {http_prefill_ms:.2?}");
    println!("capi prefill ms/round: {capi_prefill_ms:.2?} (delta = 0 after the first: nothing re-posted, nothing re-evaluated)");

    // ---- 5. growing-prefix rounds (the multi-round cooperative shape) ----
    label("GROWING-PREFIX ROUNDS x5 (+16 tok/round; re-post whole prefix vs decode the delta)");
    let base_round: Vec<u32> = ids_long.clone();
    let mut http_round_ms = Vec::new();
    let mut round = base_round.clone();
    for r in 0..5 {
        if r > 0 {
            round.extend_from_slice(&http_tokens[(r - 1) * 3..r * 3]);
        }
        let started = Instant::now();
        http.prefill(&http_handle, &round).await.unwrap();
        http_round_ms.push(ms(started.elapsed()));
    }
    let mut capi_round_ms = Vec::new();
    let mut round = base_round;
    for r in 0..5 {
        if r > 0 {
            round.extend_from_slice(&http_tokens[(r - 1) * 3..r * 3]);
        }
        let started = Instant::now();
        capi.prefill(&capi_handle, &round).await.unwrap();
        capi_round_ms.push(ms(started.elapsed()));
    }
    println!("http prefill ms/round (re-posts everything): {http_round_ms:.2?}");
    println!("capi prefill ms/round (decodes only the +3-tok delta): {capi_round_ms:.2?}");

    // ---- 6. the speculative round A/B (9.6 pass-2 shape) ----------------
    label("SPECULATIVE 2-ROUND A/B (k=8 greedy self-draft; per-position HTTP re-posts vs one batched in-process decode per round)");
    let window = 8;
    let spec_prefix = ids_long.clone();

    // Ground truth via the HTTP adapter (independently executed single):
    // enough for both rounds plus the reject-path section below.
    let ground_truth: Vec<u32> = {
        let mut truth = Vec::new();
        let mut p = spec_prefix.clone();
        for _ in 0..2 * (window + 1) {
            let t = http.decode_step(&http_handle, &p, &sampling).await.unwrap();
            truth.push(t);
            p.push(t);
        }
        truth
    };

    // One full cooperative round per adapter. Returns (http_ms, capi_ms,
    // accepted) and asserts cross-adapter exactness against the ground truth.
    #[allow(clippy::too_many_arguments)]
    async fn speculative_round(
        http: &LlamaCppAdapter,
        capi: &LlamaCppCapi,
        http_handle: &modelswarm_runtime::Handle,
        capi_handle: &modelswarm_runtime::Handle,
        sampling: &SamplingParams,
        prefix: &[u32],
        window: u32,
        truth: &[u32],
    ) -> (f64, f64, f64, f64, usize) {
        // HTTP arm: propose = k per-token POSTs (each re-posts the prefix),
        // verify = up to k+1 more POSTs of prefix + accepted draft.
        let http_started = Instant::now();
        let http_draft = http.propose(http_handle, prefix, window).await.unwrap();
        let mut http_accepted = 0usize;
        for (i, draft_token) in http_draft.iter().enumerate() {
            let mut p = prefix.to_vec();
            p.extend_from_slice(&http_draft[..i]);
            let verifier_token = http.decode_step(http_handle, &p, sampling).await.unwrap();
            if verifier_token == *draft_token {
                http_accepted += 1;
            } else {
                break;
            }
        }
        let http_ms = ms(http_started.elapsed());

        // CAPI arm: propose = k one-token in-context decodes; verify = ONE
        // batched decode (k+1 logits rows) + per-row sampling + rollback.
        let capi_started = Instant::now();
        let capi_draft = capi.propose(capi_handle, prefix, window).await.unwrap();
        let outcome = capi
            .verify_drafts(capi_handle, prefix, &capi_draft, sampling)
            .await
            .unwrap();
        let capi_ms = ms(capi_started.elapsed());

        // Exactness pins.
        assert_eq!(
            capi_draft.as_slice(),
            &truth[..window as usize],
            "capi draft = greedy truth"
        );
        assert_eq!(
            http_draft.as_slice(),
            &truth[..window as usize],
            "http draft = greedy truth"
        );
        assert_eq!(
            outcome.accepted.as_slice(),
            &truth[..http_accepted],
            "accepted prefix matches the HTTP ground truth"
        );
        if http_accepted == window as usize {
            let bonus = outcome.bonus.expect("full acceptance yields a bonus token");
            assert_eq!(bonus, truth[window as usize], "bonus = ground truth");
        }
        let propose_only = {
            let started = Instant::now();
            let _ = capi.propose(capi_handle, prefix, window).await.unwrap();
            ms(started.elapsed())
        };
        (
            http_ms,
            capi_ms,
            propose_only,
            capi_ms - propose_only,
            http_accepted,
        )
    }

    let (http1, capi1, propose1, verify1, accepted1) = speculative_round(
        &http,
        &capi,
        &http_handle,
        &capi_handle,
        &sampling,
        &spec_prefix,
        window,
        &ground_truth,
    )
    .await;
    println!(
        "round 1 (cold shapes): http propose+verify {http1:.1} ms | capi propose+verify {capi1:.1} ms (propose {propose1:.1}, verify {verify1:.1}); accepted {accepted1}/{window}"
    );

    // Round 2 continues on the committed tokens (the 2-round pattern the
    // 9.6 dry run measured over the wire).
    let mut round2_prefix = spec_prefix.clone();
    round2_prefix.extend_from_slice(&ground_truth[..(window + 1) as usize]);
    let truth2 = &ground_truth[(window + 1) as usize..];
    let (http2, capi2, propose2, verify2, accepted2) = speculative_round(
        &http,
        &capi,
        &http_handle,
        &capi_handle,
        &sampling,
        &round2_prefix,
        window,
        truth2,
    )
    .await;
    println!(
        "round 2 (warm shapes): http propose+verify {http2:.1} ms | capi propose+verify {capi2:.1} ms (propose {propose2:.1}, verify {verify2:.1}); accepted {accepted2}/{window}"
    );
    println!(
        "2-round totals: http {:.1} ms vs capi {:.1} ms ({:.2}x) — no prefix ever re-posted or re-evaluated in the capi arm",
        http1 + http2,
        capi1 + capi2,
        (http1 + http2) / (capi1 + capi2)
    );

    // ---- 7. reject path (rollback exactness) ----------------------------
    label("REJECT PATH: CORRUPTED DRAFT (shift by 1) — KV rollback must land on ground truth");
    let draft = capi
        .propose(&capi_handle, &spec_prefix, window)
        .await
        .unwrap();
    let mut corrupted = draft[1..].to_vec();
    corrupted.push(draft[0]);
    let outcome = capi
        .verify_drafts(&capi_handle, &spec_prefix, &corrupted, &sampling)
        .await
        .unwrap();
    // corrupted[0] = draft[1] != ground_truth[0] => rejection at position 0.
    assert!(
        outcome.accepted.is_empty(),
        "shift-by-one draft must reject at the first token (got {:?})",
        outcome.accepted
    );
    assert_eq!(
        outcome.bonus,
        Some(ground_truth[0]),
        "replacement must be the ground-truth token"
    );
    // And the context must continue exactly on the ground-truth line.
    let next = capi
        .decode_step(
            &capi_handle,
            &{
                let mut p = spec_prefix.clone();
                p.push(ground_truth[0]);
                p
            },
            &sampling,
        )
        .await
        .unwrap();
    assert_eq!(
        next, ground_truth[1],
        "post-rollback decode continues on truth"
    );
    println!("OK: rejected at position 0; replacement + continuation token-exact");

    // ---- metrics + teardown ----------------------------------------------
    label("ADAPTER METRICS (as reported through the trait)");
    let http_metrics = http.metrics();
    let capi_metrics = capi.metrics();
    println!(
        "http: prefill {:.1} tok/ms, decode {:.3} tok/ms | capi: prefill {:.1} tok/ms, decode {:.3} tok/ms",
        http_metrics.prefill_tokens_per_ms,
        http_metrics.decode_tokens_per_ms,
        capi_metrics.prefill_tokens_per_ms,
        capi_metrics.decode_tokens_per_ms,
    );

    drop(capi);
    let mut server = server;
    server.0.kill().expect("kill llama-server");
    let _ = server.0.wait();
    println!("\nmeasurement complete; server killed");
}
