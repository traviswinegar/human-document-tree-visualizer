//! Gated local-LLM command layer — B2, the ADR-0001 acceptance round-trip.
//!
//! These commands are **always registered** so the frontend has one stable IPC
//! surface regardless of how the desktop shell was built. The native engine,
//! however, only exists under the `llm` Cargo feature (which cascades
//! `doctree-llm/llm` → `momusdev_llm/inference` → llama.cpp). On a default
//! (native-free) build, [`llm_complete`] returns a clear, actionable error and
//! [`llm_status`] reports `enabled: false`; nothing here pulls in a native dep.
//! That is the load-bearing decoupling invariant from ADR-0001 — a failed
//! native/GPU build must never block the Stream-A structure pipeline, yet the
//! same binary can be rebuilt with `--features llm` to light up CPU inference
//! without the frontend or the walker changing at all.
//!
//! Loading a multi-GB GGUF model is slow, so the engine is **lazily** loaded on
//! the first [`llm_complete`] call (not at app startup) and held behind a
//! `Mutex` in Tauri-managed state — llama.cpp serialises decodes on one model
//! anyway, and inference itself runs on a blocking thread so the UI never
//! stalls.

use doctree_llm::{LlmConfig, LLM_ENABLED, MODEL_PATH_ENV};
use serde::Serialize;

/// What the frontend needs to decide whether to offer LLM-backed features:
/// whether this binary embeds the engine, and whether a model file is actually
/// present at the resolved path. Pure — needs neither a loaded model nor any
/// managed state, so it answers instantly on every build.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStatus {
    /// Was this binary compiled with the `llm` feature (native engine present)?
    pub enabled: bool,
    /// The env var consulted for the model path, so the UI can name it.
    pub model_path_env: String,
    /// The resolved model path (explicit or from the env var), or `None`.
    pub model_path: Option<String>,
    /// Does a file actually exist at `model_path`?
    pub model_present: bool,
}

/// Compute the LLM capability snapshot. Pure; unit-testable without a model.
pub fn llm_status_impl() -> LlmStatus {
    let model_path = LlmConfig::from_env().resolve_model_path().ok();
    let model_present = model_path
        .as_deref()
        .map(|p| std::path::Path::new(p).is_file())
        .unwrap_or(false);
    LlmStatus {
        enabled: LLM_ENABLED,
        model_path_env: MODEL_PATH_ENV.to_string(),
        model_path,
        model_present,
    }
}

/// One completion, flattened for the frontend (camelCase JSON). Native-free —
/// mirrors [`doctree_llm::Completion`] so the IPC type never depends on the
/// engine actually being compiled in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionDto {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub inference_ms: u64,
    /// Which backend produced this. Always `"cpu"` for now — GPU is still
    /// blocked on this machine (see BUILD_LOG Catch-all).
    pub origin: &'static str,
}

impl From<doctree_llm::Completion> for CompletionDto {
    fn from(c: doctree_llm::Completion) -> Self {
        Self {
            text: c.text,
            prompt_tokens: c.prompt_tokens,
            completion_tokens: c.completion_tokens,
            inference_ms: c.inference_ms,
            origin: "cpu",
        }
    }
}

/// Tauri command: report whether this build can do local inference, and whether
/// a model is on disk. Always available (every build).
#[tauri::command]
pub fn llm_status() -> LlmStatus {
    llm_status_impl()
}

// ---------------------------------------------------------------------------
// Native path — only compiled under the `llm` feature. Defined directly in this
// module (not a submodule) so the `#[tauri::command]`-generated helper items
// resolve at `llm::llm_complete`, where `generate_handler!` expects them.
// ---------------------------------------------------------------------------

#[cfg(feature = "llm")]
use std::sync::{Arc, Mutex};

/// Tauri-managed handle to the (lazily loaded) CPU inference engine. The
/// `Option` is the lazy slot — `None` until the first completion loads the
/// model; the `Mutex` serialises both the load and llama.cpp's decodes; the
/// `Arc` lets the async command clone a `'static` handle to hand to the blocking
/// thread without borrowing managed state across an await.
#[cfg(feature = "llm")]
#[derive(Default)]
pub struct LlmState {
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
}

/// Lazily load the model (first call) and run a free-form completion. Runs on a
/// blocking thread — CPU-heavy, and may take minutes to first-load a multi-GB
/// model. Errors are flattened to `String` here so the crate needs no `anyhow`
/// dependency and the result crosses Tauri's IPC boundary directly.
#[cfg(feature = "llm")]
fn complete_blocking(
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
    prompt: String,
) -> Result<doctree_llm::Completion, String> {
    let mut guard = engine.lock().expect("llm engine mutex poisoned");
    if guard.is_none() {
        let loaded = doctree_llm::Engine::load(&LlmConfig::from_env()).map_err(|e| e.to_string())?;
        *guard = Some(loaded);
    }
    // Loaded just above (or on a prior call); the slot is always `Some` here.
    guard
        .as_ref()
        .expect("engine loaded above")
        .complete(&prompt)
        .map_err(|e| e.to_string())
}

/// Tauri command: free-form CPU completion. Async so it never blocks the
/// webview's event loop — the actual inference is offloaded to a blocking
/// thread, and only the small text result crosses back.
#[cfg(feature = "llm")]
#[tauri::command]
pub async fn llm_complete(
    prompt: String,
    state: tauri::State<'_, LlmState>,
) -> Result<CompletionDto, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || complete_blocking(engine, prompt))
        .await
        .map_err(|e| format!("inference task failed to join: {e}"))?
        .map(CompletionDto::from)
}

/// Native-free stand-in: same IPC contract (`{ prompt }` in, `CompletionDto`
/// out), but this build has no engine, so it returns an actionable error
/// instead of doing inference. Keeps the command registered on every build.
#[cfg(not(feature = "llm"))]
#[tauri::command]
pub fn llm_complete(prompt: String) -> Result<CompletionDto, String> {
    let _ = &prompt; // contract parity: the frontend still sends `prompt`
    Err(format!(
        "this desktop build has no local LLM. Rebuild the shell with `--features llm` \
         and set {MODEL_PATH_ENV} to a .gguf model to enable CPU inference. \
         The deterministic structure pipeline (walk/build) works without it."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_enabled_tracks_the_compiled_feature() {
        // The decoupling guarantee, observable through the IPC surface: the
        // status flag is exactly whether the engine was compiled in.
        assert_eq!(llm_status_impl().enabled, doctree_llm::LLM_ENABLED);
    }

    #[test]
    fn status_names_the_model_path_env() {
        assert_eq!(llm_status_impl().model_path_env, MODEL_PATH_ENV);
    }

    #[test]
    fn status_present_implies_a_path() {
        // model_present can only be true if there is a path to test, and that
        // path must point at a real file.
        let s = llm_status_impl();
        match s.model_path.as_deref() {
            Some(p) if s.model_present => assert!(std::path::Path::new(p).is_file()),
            None => assert!(!s.model_present, "no path ⇒ nothing present"),
            _ => {}
        }
    }

    #[test]
    fn status_serializes_camel_case_for_the_frontend() {
        let v = serde_json::to_value(llm_status_impl()).unwrap();
        for key in ["enabled", "modelPathEnv", "modelPath", "modelPresent"] {
            assert!(v.get(key).is_some(), "missing JSON key {key}");
        }
        // snake_case must NOT leak through.
        assert!(v.get("model_path").is_none());
    }

    #[test]
    fn completion_dto_maps_every_field_and_tags_cpu() {
        let c = doctree_llm::Completion {
            text: "ok".into(),
            prompt_tokens: 7,
            completion_tokens: 3,
            inference_ms: 1234,
        };
        let dto = CompletionDto::from(c);
        assert_eq!(dto.text, "ok");
        assert_eq!(dto.prompt_tokens, 7);
        assert_eq!(dto.completion_tokens, 3);
        assert_eq!(dto.inference_ms, 1234);
        assert_eq!(dto.origin, "cpu");
        // and it round-trips to camelCase JSON
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["promptTokens"], 7);
        assert_eq!(v["completionTokens"], 3);
        assert_eq!(v["inferenceMs"], 1234);
    }
}
