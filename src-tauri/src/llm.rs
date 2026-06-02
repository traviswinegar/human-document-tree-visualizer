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

use doctree_core::{build_sequence, BuildStep, Graph};
use doctree_llm::{LlmConfig, LLM_ENABLED, MODEL_PATH_ENV};
use serde::Serialize;

/// Merge an LLM semantic fragment onto the deterministic spine and guarantee the
/// result is referentially valid (B3).
///
/// The spine is **authoritative** — its nodes win id collisions, so a model that
/// re-labels a spine `term:` as a `character:` cannot overwrite the structural
/// truth — and any edge the model emitted to a node that exists in neither the
/// spine nor the fragment is pruned. The output is therefore valid by
/// construction, so the build stream and the renderer never see a half-wired
/// edge. Pure: no model, no feature gate, so the merge contract is unit-tested
/// headlessly even though the extraction that feeds it is desktop-only.
pub fn merge_semantic_onto_spine(mut spine: Graph, fragment: Graph) -> Graph {
    spine.merge(fragment);
    spine.prune_dangling_edges();
    spine
}

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
/// `Option` is the lazy slot — `None` until the first call loads the model; the
/// `Mutex` serialises both the load and llama.cpp's decodes; the `Arc` lets an
/// async command clone a `'static` handle to hand to the blocking thread without
/// borrowing managed state across an await.
#[cfg(feature = "llm")]
#[derive(Default)]
pub struct LlmState {
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
}

/// Lazily load the model into the engine slot the first time it's needed, then
/// hand back a reference. Shared by every blocking inference path (completion
/// and extraction). CPU-heavy first call — may take minutes to load a multi-GB
/// model. Errors flatten to `String` so the crate needs no `anyhow` dep and the
/// result crosses Tauri's IPC boundary directly.
#[cfg(feature = "llm")]
fn ensure_loaded(slot: &mut Option<doctree_llm::Engine>) -> Result<&doctree_llm::Engine, String> {
    if slot.is_none() {
        *slot = Some(doctree_llm::Engine::load(&LlmConfig::from_env()).map_err(|e| e.to_string())?);
    }
    Ok(slot.as_ref().expect("engine loaded above"))
}

/// Run a free-form completion on a blocking thread.
#[cfg(feature = "llm")]
fn complete_blocking(
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
    prompt: String,
) -> Result<doctree_llm::Completion, String> {
    let mut guard = engine.lock().expect("llm engine mutex poisoned");
    ensure_loaded(&mut guard)?
        .complete(&prompt)
        .map_err(|e| e.to_string())
}

/// Run a grammar-constrained graph extraction on a blocking thread, returning
/// the raw (schema-conformant) JSON the model produced.
#[cfg(feature = "llm")]
fn extract_blocking(
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
    prompt: String,
) -> Result<String, String> {
    let mut guard = engine.lock().expect("llm engine mutex poisoned");
    let completion = ensure_loaded(&mut guard)?
        .extract_graph_json(&prompt)
        .map_err(|e| e.to_string())?;
    Ok(completion.text)
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

/// Tauri command: the **hybrid build** (B3). Walk the document into its
/// deterministic spine, ask the grammar-constrained model for the semantic
/// layer, merge it onto the spine (pruning any dangling edges), and return the
/// ordered [`BuildStep`] stream for the *whole* merged graph — so the frontend
/// animates the same way it does the structural build, now with the LLM's
/// semantic nodes and (flowing-particle) semantic edges woven in.
///
/// The spine walk is instant; only the extraction is slow, so the whole command
/// is async with the inference offloaded to a blocking thread.
#[cfg(feature = "llm")]
#[tauri::command]
pub async fn semantic_build_steps(
    text: String,
    params: Option<crate::WalkParams>,
    state: tauri::State<'_, LlmState>,
) -> Result<Vec<BuildStep>, String> {
    // 1. Deterministic spine (pure, fast) and the anchored extraction prompt.
    let spine = crate::walk_document_impl(&text, params);
    let prompt = doctree_llm::build_extraction_prompt(&spine);

    // 2. Grammar-constrained extraction on a blocking thread.
    let engine = state.engine.clone();
    let json = tauri::async_runtime::spawn_blocking(move || extract_blocking(engine, prompt))
        .await
        .map_err(|e| format!("extraction task failed to join: {e}"))??;

    // 3. Parse the fragment, merge onto the spine, prune danglers (all pure).
    let fragment: Graph =
        serde_json::from_str(&json).map_err(|e| format!("LLM output was not schema JSON: {e}"))?;
    let merged = merge_semantic_onto_spine(spine, fragment);

    // 4. Ordered build steps for the animated hybrid build.
    Ok(build_sequence(&merged))
}

/// Native-free stand-in: same IPC contract (`{ prompt }` in, `CompletionDto`
/// out), but this build has no engine, so it returns an actionable error
/// instead of doing inference. Keeps the command registered on every build.
#[cfg(not(feature = "llm"))]
#[tauri::command]
pub fn llm_complete(prompt: String) -> Result<CompletionDto, String> {
    let _ = &prompt; // contract parity: the frontend still sends `prompt`
    Err(no_llm_error())
}

/// Native-free stand-in for the hybrid build: same IPC contract (`{ text,
/// params }` in, `BuildStep[]` out) but no engine, so it returns an actionable
/// error. The frontend should fall back to the structural `build_steps`.
#[cfg(not(feature = "llm"))]
#[tauri::command]
pub fn semantic_build_steps(
    text: String,
    params: Option<crate::WalkParams>,
) -> Result<Vec<BuildStep>, String> {
    let _ = (&text, &params); // contract parity with the gated signature
    Err(no_llm_error())
}

/// The shared "no engine in this build" message, pointing at the fix.
#[cfg(not(feature = "llm"))]
fn no_llm_error() -> String {
    format!(
        "this desktop build has no local LLM. Rebuild the shell with `--features llm` \
         and set {MODEL_PATH_ENV} to a .gguf model to enable CPU inference. \
         The deterministic structure pipeline (walk/build) works without it."
    )
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
    fn merge_semantic_onto_spine_is_authoritative_and_valid() {
        use doctree_core::{Edge, EdgeKind, Node, NodeKind, Provenance};

        // A minimal spine: one sentence, plus a term the walker already found.
        let mut spine = Graph::new();
        spine.push_node(Node::structural("sent:1", NodeKind::Sentence, "Mara met Vane."));
        spine.push_node(Node::structural("term:mara", NodeKind::Term, "mara"));

        // The LLM fragment: re-labels term:mara as a Character (must NOT win),
        // adds a new Character, a grounded mention from the real sentence, an
        // entity↔entity edge, and a hallucinated edge to an undeclared id.
        let mut fragment = Graph::new();
        fragment.push_node(Node::semantic("term:mara", NodeKind::Character, "Mara (LLM)"));
        fragment.push_node(Node::semantic("char:vane", NodeKind::Character, "Vane"));
        fragment.push_edge(Edge::new("sent:1", "char:vane", EdgeKind::Mentions, Provenance::Semantic));
        fragment.push_edge(Edge::new(
            "term:mara",
            "char:vane",
            EdgeKind::InteractsWith,
            Provenance::Semantic,
        ));
        fragment.push_edge(Edge::new(
            "char:vane",
            "char:ghost", // never declared anywhere
            EdgeKind::InteractsWith,
            Provenance::Semantic,
        ));

        let merged = merge_semantic_onto_spine(spine, fragment);

        // Spine wins the id collision: term:mara stays a structural Term.
        let mara = merged.nodes.iter().find(|n| n.id == "term:mara").unwrap();
        assert_eq!(mara.kind, NodeKind::Term);
        assert_eq!(mara.provenance, Provenance::Structural);
        // The genuinely new semantic entity was added.
        assert!(merged.nodes.iter().any(|n| n.id == "char:vane"));
        // Three fragment edges in, the ghost one pruned ⇒ two survive.
        assert_eq!(merged.edges.len(), 2);
        assert!(merged.is_valid(), "merged hybrid graph is valid by construction");
        assert!(!merged.edges.iter().any(|e| e.target == "char:ghost"));
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
