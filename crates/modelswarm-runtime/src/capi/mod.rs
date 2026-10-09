//! `LlamaCppCapi`: IN-PROCESS runtime over the pinned llama.cpp C API (P16
//! spike, feature `capi-adapter`, OFF by default).
//!
//! Where the production [`crate::LlamaCppAdapter`] speaks HTTP to the pinned
//! `llama-server` sidecar (ADR-002/021), this research adapter loads the
//! *same pinned binaries* in-process (`llama.dll` from the hash-verified
//! engine bundle — see [`ffi`]) and drives the public C API directly. It is
//! the cooperative-mode research path the 9.6 pass-1 dry run justified:
//! per-round prefix re-post over the request/reply wire measured 1.07–3.36×
//! slower than the independently executed fastest single
//! (`experiments/reports/PASS-1-LOOPBACK-DRYRUN.md`).
//!
//! Capabilities this adapter exists to prove (all measured in the loopback
//! test, environment-labeled):
//!
//! - **Prefix/KV reuse across rounds** — the context retains its KV; each
//!   round decodes only the *delta* tokens. No prefix is ever re-posted.
//!   Rollback (draft rejection, divergence) uses `llama_memory_seq_rm`.
//! - **Logits access** — every batch row's logits are readable
//!   (`llama_get_logits_ith`), which exact speculative verification needs.
//! - **Batched linear draft verification** — k draft tokens verified in ONE
//!   `llama_decode` call (logits on every row) against the same sampler
//!   chain the pinned server uses, then a single KV rollback to the
//!   rejection point. Tree verification (branching drafts in one call) is
//!   NOT implemented: the C API exposes the primitives (`seq_cp`,
//!   multi-sequence batches) but no tree-attention mask construction; linear
//!   verify is the documented fallback (honest spike scope).
//!
//! Trait conformance: implements the unchanged [`crate::InferenceRuntime`]
//! shape (ADR-019), overrides `propose` (greedy self-continuation, ADR-021 —
//! now KV-cheap), and adds verification as an inherent method
//! (`verify_drafts`) rather than a trait change; folding it into the trait is
//! the scheduler's gated follow-up decision.
//!
//! Research-only posture: production serving stays on the HTTP adapter; the
//! feature is off by default and the node never enables it.

mod ffi;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;

pub use ffi::LlamaLib;
use ffi::{ContextPtr, Logits};

use crate::{
    DecodeEvent, DecodeEventStream, Handle, KvCommitment, RuntimeDescriptor, RuntimeError,
    RuntimeMetrics, SamplingParams, TaskId,
};

/// Budget for one `propose` window (matches the HTTP adapter's default
/// request timeout class).
const PROPOSE_BUDGET: Duration = Duration::from_secs(60);

/// Builder-style configuration for [`LlamaCppCapi`].
#[derive(Debug, Clone)]
pub struct CapiConfig {
    /// Hash-verified engine bundle directory containing `llama.dll` (or the
    /// per-OS C-API library) and its ggml dependencies.
    pub engine_dir: PathBuf,
    /// Path to the verified GGUF artifact (sha-verified upstream by the
    /// artifact manager; this adapter re-derives nothing).
    pub model_path: PathBuf,
    /// Context window (prompt + generation budget); mirrors the server's
    /// `-c` so loopback comparisons see identical capacity.
    pub n_ctx: u32,
    /// Compute threads for decode; `None` = engine default (what
    /// `llama-server` uses when `-t` is not passed).
    pub n_threads: Option<u32>,
    /// Pinned-engine identity for `id()`.
    pub engine: Option<crate::llamacpp::EngineIdentity>,
}

impl CapiConfig {
    /// Configuration with the defaults used by the node's engine spec
    /// (`ctx = 4096`, threads = engine default).
    pub fn new(engine_dir: impl Into<PathBuf>, model_path: impl Into<PathBuf>) -> Self {
        Self {
            engine_dir: engine_dir.into(),
            model_path: model_path.into(),
            n_ctx: 4096,
            n_threads: None,
            engine: None,
        }
    }

    /// Attaches the pinned-engine identity.
    pub fn with_engine(mut self, engine: crate::llamacpp::EngineIdentity) -> Self {
        self.engine = Some(engine);
        self
    }
}

/// Outcome of one exact speculative verification round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    /// Draft tokens accepted (a prefix of `draft`), token-exact under the
    /// declared sampling contract.
    pub accepted: Vec<u32>,
    /// The verifier's own next token: the replacement at the rejection
    /// point, or the bonus token after full acceptance (`None` only when
    /// generation ended at an EOG token).
    pub bonus: Option<u32>,
}

/// Per-handle decode state: the engine context plus the adapter's
/// bookkeeping of what is in its KV (the token `trail`) and which KV
/// position the current logits rows end at. Invariant: `trail` holds
/// exactly the tokens decoded into sequence 0; sampled-but-not-decoded
/// tokens are never in it.
struct HandleState {
    ctx: ContextPtr,
    trail: Vec<u32>,
    /// KV position of the last output row of the most recent decode
    /// (`None` when no logits are live). Rows are only valid until the next
    /// decode, exactly like the engine's own output buffer.
    last_output_pos: Option<u32>,
}

struct Shared {
    metrics: RwLock<RuntimeMetrics>,
    next_handle: AtomicU64,
    handles: Mutex<HashMap<u64, Arc<Mutex<HandleState>>>>,
    /// TaskId of the most recent `cancel` call (0 = none); decode loops
    /// observe it between steps (cooperative, deadline-class cancellation —
    /// the same approximation as the HTTP adapter's abort-on-disconnect).
    cancelled: AtomicU64,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            metrics: RwLock::new(RuntimeMetrics::default()),
            next_handle: AtomicU64::new(1),
            handles: Mutex::new(HashMap::new()),
            cancelled: AtomicU64::new(0),
        }
    }
}

/// In-process [`crate::InferenceRuntime`] over the pinned llama.cpp C API.
///
/// Construction loads the pinned C-API library, initializes the backend
/// (process-global, once), loads the model and runs an ABI canary against
/// the engine's default parameters. Each `load()` creates a fresh context
/// (own KV) sharing the one model.
pub struct LlamaCppCapi {
    lib: Arc<LlamaLib>,
    model: ffi::ModelPtr,
    vocab: *const ffi::LlamaVocab,
    eos_id: Option<u32>,
    n_ctx: u32,
    config: CapiConfig,
    shared: Arc<Shared>,
}

// `Send`/`Sync` are implemented in `ffi.rs` (the crate's single unsafe
// scope): the adapter's only raw pointer is `vocab`, derived from the owned
// `ModelPtr` and freed together with it.

/// Process-global backend bring-up guard: `llama_backend_init` once, then
/// `ggml_backend_load_all_from_path(<bundle dir>)` once (modern llama.cpp
/// refuses to load a model until the bundle's ggml-cpu backends are
/// registered — the server does this internally at startup).
static BACKEND_READY: OnceLock<Result<(), String>> = OnceLock::new();

impl LlamaCppCapi {
    /// Loads the pinned library + model and runs the ABI canary.
    pub fn new(config: CapiConfig) -> Result<Self, RuntimeError> {
        let err = |message: String| RuntimeError::Http(format!("capi-adapter: {message}"));
        let lib = Arc::new(
            LlamaLib::load(&config.engine_dir)
                .map_err(|e| err(format!("pinned library load: {e}")))?,
        );
        let backend_ready = BACKEND_READY.get_or_init(|| {
            lib.backend_init();
            lib.load_bundle_backends()
                .map_err(|e| format!("backend registry: {e}"))
        });
        backend_ready.as_ref().map_err(|e| err(e.clone()))?;

        // ABI canary: the default-params struct round-trip must land on the
        // documented b11407 shape. A future pin that changes the layout
        // surfaces HERE as a construction error, never as silent corruption.
        let defaults = lib.context_default_params();
        if defaults.n_batch < 256
            || defaults.n_ctx == 0
            || defaults.n_seq_max != 1
            || defaults.n_threads < -1
        {
            return Err(err(format!(
                "ABI canary failed: b11407 context defaults look wrong \
                 (n_ctx={}, n_batch={}, n_seq_max={}, n_threads={})",
                defaults.n_ctx, defaults.n_batch, defaults.n_seq_max, defaults.n_threads
            )));
        }

        let model = lib
            .load_model(&config.model_path, lib.model_default_params())
            .map_err(|e| err(format!("model load: {e}")))?;
        let vocab = lib.vocab(&model);
        let n_vocab = lib.vocab_n_tokens(vocab);
        if n_vocab == 0 {
            return Err(err("model reports an empty vocabulary".into()));
        }
        lib.set_cached_vocab_n_tokens(n_vocab);
        let eos_id = {
            let eos = lib.vocab_eos(vocab);
            (eos >= 0).then_some(eos as u32)
        };
        Ok(Self {
            lib,
            model,
            vocab,
            eos_id,
            n_ctx: config.n_ctx,
            config,
            shared: Arc::new(Shared::default()),
        })
    }

    /// The model's EOS token id from the pinned GGUF vocabulary.
    pub fn eos_id(&self) -> Option<u32> {
        self.eos_id
    }

    /// Vocabulary size (logits row width).
    pub fn vocab_size(&self) -> u32 {
        self.lib.vocab_n_tokens(self.vocab)
    }

    fn context(&self, handle: &Handle) -> Result<Arc<Mutex<HandleState>>, RuntimeError> {
        self.shared
            .handles
            .lock()
            .map_err(|_| RuntimeError::Http("capi-adapter: state lock poisoned".into()))?
            .get(&handle.handle_id())
            .cloned()
            .ok_or(RuntimeError::NotLoaded(handle.handle_id()))
    }

    fn lock_poisoned() -> RuntimeError {
        RuntimeError::Http("capi-adapter: state lock poisoned".into())
    }

    /// True when the sampled token ends generation (EOS/EOT class).
    fn is_eog(&self, token: u32) -> bool {
        self.lib.vocab_is_eog(self.vocab, token)
    }

    /// Synchronizes the context's KV to `target`: rolls back to the longest
    /// common prefix (`llama_memory_seq_rm`) and decodes only the delta.
    /// This is the structural fix for the per-round prefix re-post cost —
    /// the prefix is never evaluated, serialized or shipped twice.
    ///
    /// When `keep_last_logits` is set, the final position's logits are live
    /// on return; if the target is already the trail but its tail logits are
    /// stale, the last token alone is rolled back and re-evaluated (a
    /// single-token cost, never a prefix cost).
    fn sync(
        &self,
        state: &mut HandleState,
        target: &[u32],
        keep_last_logits: bool,
    ) -> Result<(), RuntimeError> {
        if target.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "sync of an empty token span".to_string(),
            ));
        }
        if target.len() as u64 > u64::from(self.n_ctx) {
            return Err(RuntimeError::Api {
                status: 400,
                code: "prompt_too_long".to_string(),
                message: format!(
                    "{} tokens exceed the context window of {}",
                    target.len(),
                    self.n_ctx
                ),
            });
        }
        let common = common_prefix_len(&state.trail, target);
        if common < state.trail.len() {
            let ok = self.lib.memory_seq_rm(&state.ctx, common as ffi::LlamaPos);
            if !ok {
                return Err(RuntimeError::Api {
                    status: 500,
                    code: "kv_rollback_refused".to_string(),
                    message: format!("llama_memory_seq_rm refused rollback to position {common}"),
                });
            }
            state.trail.truncate(common);
        }
        if common < target.len() {
            let started = Instant::now();
            let delta = &target[common..];
            let mode = if keep_last_logits {
                Logits::Last
            } else {
                Logits::None
            };
            self.decode_into(state, delta, mode)?;
            state.trail.extend_from_slice(delta);
            self.record_prefill(delta.len() as u64, started.elapsed());
            return Ok(());
        }
        // trail == target already.
        if keep_last_logits && state.last_output_pos != Some((target.len() - 1) as u32) {
            let cut = (target.len() - 1) as ffi::LlamaPos;
            let ok = self.lib.memory_seq_rm(&state.ctx, cut);
            if !ok {
                return Err(RuntimeError::Api {
                    status: 500,
                    code: "kv_rollback_refused".to_string(),
                    message: format!("tail-logits rollback to {cut} refused"),
                });
            }
            state.trail.truncate(target.len() - 1);
            let tail = target[target.len() - 1];
            self.decode_into(state, std::slice::from_ref(&tail), Logits::Last)?;
            state.trail.push(tail);
        }
        Ok(())
    }

    /// One `llama_decode` call; updates `last_output_pos` per the mode.
    fn decode_into(
        &self,
        state: &mut HandleState,
        tokens: &[u32],
        mode: Logits,
    ) -> Result<(), RuntimeError> {
        if tokens.is_empty() {
            return Ok(());
        }
        let start = state.trail.len() as u32;
        self.lib
            .decode(&mut state.ctx, tokens, mode)
            .map_err(|rc| RuntimeError::Api {
                status: 500,
                code: "decode_failed".to_string(),
                message: format!("llama_decode rc={rc}"),
            })?;
        state.last_output_pos = match mode {
            Logits::None => None,
            _ => Some(start + tokens.len() as u32 - 1),
        };
        Ok(())
    }

    /// Samples the next token given that the context is synced to `prefix`.
    /// The sampled token is NOT added to the trail (it has not been decoded).
    fn sample_next(
        &self,
        state: &mut HandleState,
        prefix: &[u32],
        sampling: &SamplingParams,
    ) -> Result<u32, RuntimeError> {
        self.sync(state, prefix, true)?;
        self.sample_row(state, -1, sampling)
    }

    /// Decodes `token` (one position) and samples its logits row. Returns
    /// the NEXT token (not yet decoded). Used by stream steps + propose.
    fn step_sampled(&self, state: &mut HandleState, token: u32) -> Result<u32, RuntimeError> {
        let started = Instant::now();
        self.decode_into(state, std::slice::from_ref(&token), Logits::Last)?;
        state.trail.push(token);
        let next = self.sample_row(state, -1, &SamplingParams::default())?;
        self.record_decode(1, started.elapsed());
        Ok(next)
    }

    fn sample_row(
        &self,
        state: &mut HandleState,
        row: i32,
        sampling: &SamplingParams,
    ) -> Result<u32, RuntimeError> {
        let mut chain = self
            .lib
            .build_chain(sampling)
            .map_err(|e| RuntimeError::Http(format!("capi-adapter: sampler: {e}")))?;
        Ok(self.lib.sample(&mut chain, &mut state.ctx, row))
    }

    fn record_prefill(&self, tokens: u64, elapsed: Duration) {
        let micros = elapsed.as_micros();
        if tokens > 0 && micros > 0 {
            if let Ok(mut metrics) = self.shared.metrics.write() {
                metrics.prefill_tokens_per_ms = tokens as f64 / (micros as f64 / 1000.0);
            }
        }
    }

    fn record_decode(&self, tokens: u64, elapsed: Duration) {
        let micros = elapsed.as_micros();
        if tokens > 0 && micros > 0 {
            if let Ok(mut metrics) = self.shared.metrics.write() {
                metrics.decode_tokens_per_ms = tokens as f64 / (micros as f64 / 1000.0);
            }
        }
    }

    /// EXACT SPECULATIVE VERIFICATION (the cooperative capability; inherent
    /// method — see the module docs). Verifies `draft` continuing `prefix`
    /// in ONE batched decode:
    ///
    /// 1. sync the KV to `prefix` **without** its last token,
    /// 2. one `llama_decode` batch of `[prefix_last, draft…]` with logits on
    ///    every row — row `i` is the target distribution at draft position
    ///    `i` (the final row additionally yields the bonus token),
    /// 3. accept `draft[i]` while the sampler chain (the same chain the
    ///    pinned server uses) would sample it from row `i`,
    /// 4. on the first rejection roll the KV back to the rejection point
    ///    (`llama_memory_seq_rm`); the replacement token is sampled from the
    ///    rejecting row.
    ///
    /// The verifier context is left at `prefix + accepted` with the
    /// replacement/bonus token sampled but NOT decoded — the caller's next
    /// round feeds it back through `prefix` and the KV grows by the delta,
    /// exactly like every other sync.
    pub async fn verify_drafts(
        &self,
        handle: &Handle,
        prefix: &[u32],
        draft: &[u32],
        sampling: &SamplingParams,
    ) -> Result<VerifyOutcome, RuntimeError> {
        if prefix.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "verify_drafts with empty prefix; prefill first".to_string(),
            ));
        }
        if prefix.len() + draft.len() + 1 >= self.n_ctx as usize {
            return Err(RuntimeError::Api {
                status: 400,
                code: "prompt_too_long".to_string(),
                message: format!(
                    "prefix {} + draft {} exceed the context window of {}",
                    prefix.len(),
                    draft.len(),
                    self.n_ctx
                ),
            });
        }
        let started = Instant::now();
        let context = self.context(handle)?;
        let mut state = context.lock().map_err(|_| Self::lock_poisoned())?;
        let outcome = self.verify_locked(&mut state, prefix, draft, sampling)?;
        self.record_decode(outcome.accepted.len() as u64 + 1, started.elapsed());
        Ok(outcome)
    }

    fn verify_locked(
        &self,
        state: &mut HandleState,
        prefix: &[u32],
        draft: &[u32],
        sampling: &SamplingParams,
    ) -> Result<VerifyOutcome, RuntimeError> {
        if draft.is_empty() {
            // Degenerate round: just produce the verifier's next token.
            let token = self.sample_next(state, prefix, sampling)?;
            return Ok(VerifyOutcome {
                accepted: Vec::new(),
                bonus: (!self.is_eog(token)).then_some(token),
            });
        }
        // 1. KV to prefix minus its last token (no logits rows needed).
        self.sync(state, &prefix[..prefix.len() - 1], false)?;
        // 2. One batched decode: [prefix_last, draft…] with all rows live.
        let mut batch: Vec<u32> = Vec::with_capacity(draft.len() + 1);
        batch.push(prefix[prefix.len() - 1]);
        batch.extend_from_slice(draft);
        self.decode_into(state, &batch, Logits::All)?;
        state.trail.clear();
        state.trail.extend_from_slice(prefix);
        state.trail.extend_from_slice(draft);

        // 3. Accept while the target's own sampler choice matches the draft.
        let mut chain = self
            .lib
            .build_chain(sampling)
            .map_err(|e| RuntimeError::Http(format!("capi-adapter: sampler: {e}")))?;
        let mut accepted = 0usize;
        let mut replacement: Option<u32> = None;
        for (i, draft_token) in draft.iter().enumerate() {
            let verifier_token = self.lib.sample(&mut chain, &mut state.ctx, i as i32);
            if verifier_token == *draft_token {
                accepted += 1;
            } else {
                replacement = Some(verifier_token);
                break;
            }
        }
        if accepted == draft.len() {
            // Full acceptance: bonus token from the final row (not decoded).
            let bonus = self
                .lib
                .sample(&mut chain, &mut state.ctx, draft.len() as i32);
            if self.is_eog(bonus) {
                return Ok(VerifyOutcome {
                    accepted: draft.to_vec(),
                    bonus: None,
                });
            }
            return Ok(VerifyOutcome {
                accepted: draft.to_vec(),
                bonus: Some(bonus),
            });
        }
        // 4. Roll back to the rejection point.
        let keep = prefix.len() + accepted;
        let rollback_ok = self.lib.memory_seq_rm(&state.ctx, keep as ffi::LlamaPos);
        if !rollback_ok {
            return Err(RuntimeError::Api {
                status: 500,
                code: "kv_rollback_refused".to_string(),
                message: format!("verify rollback to {keep} refused"),
            });
        }
        state.trail.truncate(keep);
        state.last_output_pos = None;
        let replacement = replacement.expect("rejection path sets a replacement");
        if self.is_eog(replacement) {
            return Ok(VerifyOutcome {
                accepted: draft[..accepted].to_vec(),
                bonus: None,
            });
        }
        Ok(VerifyOutcome {
            accepted: draft[..accepted].to_vec(),
            bonus: Some(replacement),
        })
    }
}

/// Longest common prefix length of two token slices.
fn common_prefix_len(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

#[async_trait]
impl crate::InferenceRuntime for LlamaCppCapi {
    fn id(&self) -> RuntimeDescriptor {
        match &self.config.engine {
            Some(engine) => {
                RuntimeDescriptor::new("llama.cpp-capi", &engine.version, &engine.build_hash)
                    .expect("pinned descriptor is valid")
            }
            // Dev/test setups without a pin report the same zero placeholder
            // class as the HTTP adapter; never a claimed build.
            None => RuntimeDescriptor::new("llama.cpp-capi", "adapter-capi-v1", "0".repeat(64))
                .expect("static descriptor is valid"),
        }
    }

    async fn load(&self, profile_id: &str) -> Result<Handle, RuntimeError> {
        if profile_id.is_empty() {
            return Err(RuntimeError::InvalidProfile(profile_id.to_string()));
        }
        let handle_id = self.shared.next_handle.fetch_add(1, Ordering::Relaxed);
        let mut params = self.lib.context_default_params();
        params.n_ctx = self.n_ctx;
        if let Some(threads) = self.config.n_threads {
            params.n_threads = threads as i32;
            params.n_threads_batch = threads as i32;
        }
        let ctx = self
            .lib
            .init_context(&self.model, params)
            .map_err(|e| RuntimeError::Http(format!("capi-adapter: context: {e}")))?;
        // ABI canary #2: the engine must hand back exactly the context size
        // we wrote at offset 0 of the params struct.
        if self.lib.n_ctx(&ctx) != self.n_ctx {
            return Err(RuntimeError::Http(format!(
                "capi-adapter: ABI canary failed — requested n_ctx {} but the engine allocated {}",
                self.n_ctx,
                self.lib.n_ctx(&ctx)
            )));
        }
        let state = HandleState {
            ctx,
            trail: Vec::new(),
            last_output_pos: None,
        };
        self.shared
            .handles
            .lock()
            .map_err(|_| Self::lock_poisoned())?
            .insert(handle_id, Arc::new(Mutex::new(state)));
        Ok(Handle::new(profile_id, handle_id))
    }

    async fn tokenize(&self, text: &str) -> Result<Vec<u32>, RuntimeError> {
        // add_special=true / parse_special=true matches the pinned server's
        // /tokenize behavior (pinned token-for-token by the loopback test).
        self.lib
            .tokenize(self.vocab, text, true, true)
            .map_err(|e| RuntimeError::Http(format!("capi-adapter: tokenize: {e}")))
    }

    async fn detokenize(&self, ids: &[u32]) -> Result<String, RuntimeError> {
        self.lib
            .detokenize(self.vocab, ids)
            .map_err(|e| RuntimeError::Http(format!("capi-adapter: detokenize: {e}")))
    }

    async fn prefill(&self, handle: &Handle, ids: &[u32]) -> Result<KvCommitment, RuntimeError> {
        if ids.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "prefill of empty token span".to_string(),
            ));
        }
        let context = self.context(handle)?;
        let mut state = context.lock().map_err(|_| Self::lock_poisoned())?;
        self.sync(&mut state, ids, false)?;
        Ok(KvCommitment::new(0, ids))
    }

    async fn decode_step(
        &self,
        handle: &Handle,
        prefix: &[u32],
        sampling: &SamplingParams,
    ) -> Result<u32, RuntimeError> {
        if prefix.is_empty() {
            return Err(RuntimeError::MalformedResponse(
                "decode_step with empty prefix; prefill first".to_string(),
            ));
        }
        let started = Instant::now();
        let context = self.context(handle)?;
        let mut state = context.lock().map_err(|_| Self::lock_poisoned())?;
        let token = self.sample_next(&mut state, prefix, sampling)?;
        self.record_decode(1, started.elapsed());
        Ok(token)
    }

    /// Incremental in-process decode: sync once, then one-token decode
    /// steps. Each engine call blocks for ~one token of compute — spike
    /// posture (research adapter, bench-driven), with a yield between steps
    /// so a single-threaded executor is not starved.
    fn decode_stream_events<'a>(
        &'a self,
        handle: &'a Handle,
        prefix: &'a [u32],
        sampling: &'a SamplingParams,
        max_tokens: u32,
        deadline: Duration,
    ) -> DecodeEventStream<'a> {
        let context = match self.context(handle) {
            Ok(context) => context,
            Err(e) => {
                return Box::pin(futures_util::stream::once(async move { Err(e) }));
            }
        };
        struct StreamState {
            context: Arc<Mutex<HandleState>>,
            /// Full working prefix (prompt + produced tokens).
            working: Vec<u32>,
            /// Last sampled token (not yet decoded into the KV).
            last_token: Option<u32>,
            sampling: SamplingParams,
            produced: u32,
            started: Instant,
            done: bool,
        }
        let state = StreamState {
            context,
            working: prefix.to_vec(),
            last_token: None,
            sampling: *sampling,
            produced: 0,
            started: Instant::now(),
            done: max_tokens == 0 || prefix.is_empty(),
        };
        Box::pin(futures_util::stream::unfold(
            state,
            move |mut state| async move {
                if state.done || state.produced >= max_tokens {
                    return None;
                }
                if state.started.elapsed() >= deadline {
                    state.done = true;
                    return Some((
                        Err(RuntimeError::Timeout {
                            after_ms: crate::duration_ms_u64(deadline),
                        }),
                        state,
                    ));
                }
                if let Some(task) = self.cancelled_task() {
                    state.done = true;
                    return Some((Err(RuntimeError::Cancelled(task)), state));
                }
                let outcome = {
                    let Ok(mut handle_state) = state.context.lock() else {
                        state.done = true;
                        return Some((
                            Err(RuntimeError::Http(
                                "capi-adapter: state lock poisoned".into(),
                            )),
                            state,
                        ));
                    };
                    match state.last_token {
                        // First event: sync (delta decode) + sample.
                        None => {
                            self.sample_next(&mut handle_state, &state.working, &state.sampling)
                        }
                        // Subsequent: one-token decode + sample.
                        Some(last) => self.step_sampled(&mut handle_state, last),
                    }
                };
                let token = match outcome {
                    Ok(token) => token,
                    Err(e) => {
                        state.done = true;
                        return Some((Err(e), state));
                    }
                };
                if self.is_eog(token) {
                    return None;
                }
                state.working.push(token);
                state.last_token = Some(token);
                state.produced += 1;
                tokio::task::yield_now().await;
                Some((Ok(DecodeEvent::Token(token)), state))
            },
        ))
    }

    fn metrics(&self) -> RuntimeMetrics {
        self.shared.metrics.read().map(|m| *m).unwrap_or_default()
    }

    fn cancel(&self, task: TaskId) -> Result<(), RuntimeError> {
        self.shared.cancelled.store(task.0, Ordering::Relaxed);
        Ok(())
    }

    /// Greedy self-continuation (ADR-021 unforked subset) — now KV-cheap:
    /// the prefix is synced once and every proposal is a one-token decode.
    async fn propose(
        &self,
        handle: &Handle,
        prefix: &[u32],
        window: u32,
    ) -> Result<Vec<u32>, RuntimeError> {
        if window == 0 || prefix.is_empty() {
            return Ok(Vec::new());
        }
        let started = Instant::now();
        let context = self.context(handle)?;
        let mut state = context.lock().map_err(|_| Self::lock_poisoned())?;
        let default = SamplingParams::default();
        let mut draft = Vec::with_capacity(window as usize);
        let mut current = self.sample_next(&mut state, prefix, &default)?;
        while draft.len() < window as usize {
            if self.is_eog(current) || started.elapsed() >= PROPOSE_BUDGET {
                break;
            }
            draft.push(current);
            if draft.len() == window as usize {
                break;
            }
            current = self.step_sampled(&mut state, current)?;
        }
        Ok(draft)
    }
}

impl LlamaCppCapi {
    fn cancelled_task(&self) -> Option<TaskId> {
        let raw = self.shared.cancelled.swap(0, Ordering::Relaxed);
        (raw > 0).then_some(TaskId(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_prefix_lengths() {
        assert_eq!(common_prefix_len(&[1, 2, 3], &[1, 2, 4]), 2);
        assert_eq!(common_prefix_len(&[1, 2], &[1, 2, 3, 4]), 2);
        assert_eq!(common_prefix_len(&[], &[1]), 0);
        assert_eq!(common_prefix_len(&[5], &[5]), 1);
    }

    #[test]
    fn config_defaults_mirror_the_engine_spec() {
        let config = CapiConfig::new("engine", "model.gguf");
        assert_eq!(config.n_ctx, 4096);
        assert!(config.n_threads.is_none());
        assert!(config.engine.is_none());
    }
}
