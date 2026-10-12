//! The composed daemon's family-engine seam (ADR-032 light-up tier,
//! feature `verifydrafts`).
//!
//! The serve paths stay engine-agnostic behind
//! `Arc<dyn BatchVerifyEngine>` (the [`super::serve::serve_verifydrafts_session`]
//! argument — the existing TimingEngine seam's pattern: the loop wraps
//! whatever it is given and measures it). THIS type is the one place that
//! knows WHICH engine backs the family:
//!
//! - [`FamilyEngineSource::Shared`] — one fixed engine for every session:
//!   the mock/stub venue of the plain serve paths and every scripted test;
//! - [`FamilyEngineSource::Capi`] (only under `verifydrafts-capi`, the G4
//!   bridge) — Runtime's production [`CapiVerifyEngine`], constructed
//!   fresh per family session (one handle per session on the shared
//!   adapter; multiple engines may share an adapter — the
//!   one-bundle-per-process guard applies to the ADAPTER, ADR-031 §2) and
//!   PREFILLED with the pinned session prompt.
//!
//! # The prompt gap (recorded, out-of-band pinning)
//!
//! The family's wire and gate never carry the session PROMPT — the
//! coordinator's draft windows chain on the prefix hash, and the verifier
//! must already hold the same prompt in engine KV
//! ([`modelswarm_session::verifydrafts::BatchVerifyEngine`] has no
//! prefill surface; the wiring handoff's G3 records the offer's missing
//! profile/params binding — the prompt is the same family of gap). The
//! seam therefore pins the prompt from daemon configuration, exactly as
//! `VerifyDraftsServeConfig` pins the profile id and sampling parameters,
//! and the Capi variant commits it as the PREFILLED root at engine
//! construction — before any window can arrive.

use std::sync::Arc;

use modelswarm_runtime::RuntimeError;
use modelswarm_session::negotiation::EngineDisclosure;
use modelswarm_session::verifydrafts::BatchVerifyEngine;

/// Where the composed daemon's family sessions get their batch-verify
/// engine. Cloning the source is not needed: the composed loop holds one
/// and asks it for an engine per accepted family session
/// ([`FamilyEngineSource::engine_for`]).
pub enum FamilyEngineSource {
    /// One shared engine for every family session (the mock/stub venue of
    /// the plain serve paths and tests). A shared engine serializes its
    /// sessions through one `verify_window` — acceptable for scripted
    /// engines; the production venue constructs per-session engines.
    Shared(Arc<dyn BatchVerifyEngine>),
    /// The production engine: a fresh `CapiVerifyEngine` per family
    /// session, prefilled with `prompt_tokens` (the out-of-band pinned
    /// session prompt — see the module docs). Available only under the
    /// `verifydrafts-capi` feature (which enables Runtime's
    /// `capi-adapter` and the G4 bridge hosting the architect's trait for
    /// the engine).
    #[cfg(feature = "verifydrafts-capi")]
    Capi {
        /// The shared pinned C-API adapter (already constructed over the
        /// hash-verified engine bundle; one adapter per process).
        adapter: Arc<modelswarm_runtime::capi::LlamaCppCapi>,
        /// The backend family the bundle was staged as (E0: CPU ≠ Vulkan;
        /// supply what was staged and verified —
        /// `VerifyBackend::of_bundle` detects it from bundle contents).
        backend: modelswarm_runtime::capi::verify::VerifyBackend,
        /// The runtime handle label for per-session loads (local KV
        /// bookkeeping only).
        profile_id: String,
        /// The pinned session prompt (token ids), committed as the
        /// engine's PREFILLED root at construction. MUST be non-empty.
        prompt_tokens: Vec<u32>,
    },
}

impl FamilyEngineSource {
    /// Yields the engine for ONE family session, prefilled and ready for
    /// its first `verify_window`. Errors are local engine-loading
    /// failures (the composed loop refuses the session honestly — the
    /// coordinator falls back to single, never silently).
    pub async fn engine_for(&self) -> Result<Arc<dyn BatchVerifyEngine>, RuntimeError> {
        match self {
            Self::Shared(engine) => Ok(Arc::clone(engine)),
            #[cfg(feature = "verifydrafts-capi")]
            Self::Capi {
                adapter,
                backend,
                profile_id,
                prompt_tokens,
            } => {
                if prompt_tokens.is_empty() {
                    return Err(RuntimeError::MalformedResponse(
                        "family engine source configured with an empty pinned prompt".to_string(),
                    ));
                }
                let engine = super::engine_bridge::CapiBatchVerifyEngine::new(
                    Arc::clone(adapter),
                    *backend,
                    profile_id.clone(),
                    prompt_tokens.clone(),
                )
                .await?;
                Ok(Arc::new(engine))
            }
        }
    }

    /// The disclosure this source's engines will report (the Shared
    /// variant's own capabilities; the Capi variant's real geometry —
    /// `batch_verify: true`, the adapter's pinned verify window, the
    /// staged backend). Callers that need the exact per-session disclosure
    /// should use [`Self::engine_for`] and read the engine's own
    /// `capabilities()` — the authoritative form.
    pub fn disclosure(&self) -> EngineDisclosure {
        match self {
            Self::Shared(engine) => engine.capabilities(),
            #[cfg(feature = "verifydrafts-capi")]
            Self::Capi {
                adapter, backend, ..
            } => super::engine_bridge::capi_disclosure(
                modelswarm_runtime::capi::verify::VerifyEngineCaps {
                    backend: *backend,
                    batch_verify: true,
                    batch_window_max: adapter.verify_window_max(),
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelswarm_runtime::SamplingParams;
    use modelswarm_session::negotiation::Backend;

    /// A fixed-outcome engine (no state): full acceptance plus a bonus.
    struct FixedEngine;

    #[async_trait::async_trait]
    impl BatchVerifyEngine for FixedEngine {
        async fn verify_window(
            &self,
            window: &[u32],
            _params: &SamplingParams,
        ) -> Result<(u32, Option<u32>), RuntimeError> {
            Ok((window.len() as u32, Some(999)))
        }

        fn capabilities(&self) -> EngineDisclosure {
            EngineDisclosure {
                backend: Backend::Cpu,
                batch_verify: true,
                batch_window_max: 8,
            }
        }
    }

    /// The Shared seam: `engine_for` yields the injected engine (mock/stub
    /// venue), its capabilities flow unchanged, and its passes work
    /// through the yielded `Arc<dyn BatchVerifyEngine>`.
    #[tokio::test]
    async fn shared_source_yields_the_injected_engine() {
        let source = FamilyEngineSource::Shared(Arc::new(FixedEngine));
        assert_eq!(source.disclosure().batch_window_max, 8);
        let engine = source.engine_for().await.expect("shared engine yields");
        assert_eq!(engine.capabilities().batch_window_max, 8);
        assert!(engine.capabilities().batch_verify);
        assert!(engine.capabilities().is_valid_disclosure());
        let (accepted, correction) = engine
            .verify_window(&[1, 2, 3], &SamplingParams::default())
            .await
            .expect("pass through the seam");
        assert_eq!((accepted, correction), (3, Some(999)));
    }
}
