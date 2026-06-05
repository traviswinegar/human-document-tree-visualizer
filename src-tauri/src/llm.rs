//! Gated local-model command layer — the ADR-0001 acceptance surface for the
//! semantic layers: CPU inference (B2), grammar-constrained extraction merged
//! onto the spine (B3), and embedding similarity + search (B4).
//!
//! These commands are **always registered** so the frontend has one stable IPC
//! surface regardless of how the desktop shell was built. The native models,
//! however, only exist under Cargo features: inference under `llm` (cascades
//! `doctree-llm/llm` → `momusdev_llm/inference` → llama.cpp) and the embedder
//! under `vectordb` (`doctree-llm/vectordb` → fastembed/ONNX). The two are
//! independent — a build can carry one, both, or neither. On a default
//! (native-free) build every command resolves to a clear, actionable error stub
//! and [`llm_status`] reports `enabled: false`; nothing here pulls in a native
//! dep. That is the load-bearing decoupling invariant from ADR-0001 — a failed
//! native/GPU build must never block the Stream-A structure pipeline, yet the
//! same binary can be rebuilt with the features to light up the semantic layers
//! without the frontend or the walker changing at all.
//!
//! Loading a multi-GB GGUF model is slow, so the engine is **lazily** loaded on
//! the first [`llm_complete`] call (not at app startup) and held behind a
//! `Mutex` in Tauri-managed state — llama.cpp serialises decodes on one model
//! anyway, and inference itself runs on a blocking thread so the UI never
//! stalls.

use doctree_core::{BuildStep, Graph};
use doctree_llm::{LlmConfig, LLM_ENABLED, MODEL_PATH_ENV};
use serde::Serialize;

// `build_sequence` only feeds the gated build-step commands; importing it on the
// native-free build would be an unused import.
#[cfg(any(feature = "llm", feature = "vectordb"))]
use doctree_core::build_sequence;

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

/// Attach embedding-similarity edges to a graph (B4). Pure: given each node's
/// embedding vector, derive the [`doctree_core::EdgeKind::SimilarTo`] edges and
/// append them. The edges only reference ids that came *from* this graph, so the
/// result stays referentially valid by construction — no prune needed. Native-
/// free, so the similarity-augmentation contract is unit-tested headlessly even
/// though the vectors that feed it come from the desktop-only embedder.
pub fn attach_similarity_edges(
    mut graph: Graph,
    embedded: &[(String, Vec<f32>)],
    opts: &doctree_llm::SimilarityOptions,
) -> Graph {
    for edge in doctree_llm::similarity_edges(embedded, opts) {
        graph.push_edge(edge);
    }
    graph
}

/// One semantic-search result, flattened for the frontend (camelCase JSON): the
/// matched node's id, display label, kind, and its similarity score.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub id: String,
    pub label: String,
    /// The node kind (serialises to its snake_case tag, e.g. `"character"`).
    pub kind: doctree_core::NodeKind,
    pub score: f32,
}

/// Resolve ranked [`doctree_llm::SearchHit`]s against a graph into frontend DTOs,
/// attaching each hit's label and kind. Hits whose node is absent from the graph
/// are dropped (defensive — the ids come from the same graph). Pure.
pub fn to_search_hits(graph: &Graph, hits: Vec<doctree_llm::SearchHit>) -> Vec<SearchHitDto> {
    hits.into_iter()
        .filter_map(|h| {
            graph.nodes.iter().find(|n| n.id == h.id).map(|n| SearchHitDto {
                id: h.id,
                label: n.label.clone(),
                kind: n.kind,
                score: h.score,
            })
        })
        .collect()
}

/// One cross-document RAG search result, flattened for the frontend (camelCase
/// JSON): which document and node matched, the node kind tag, the stored text,
/// and a `[0,1]` similarity score. Native-free — mirrors [`doctree_llm::RagHit`]
/// so the IPC type exists on every build (the gated command fills it; the stub
/// returns an actionable error).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagHitDto {
    pub doc_id: String,
    pub node_id: String,
    pub kind: String,
    pub text: String,
    pub score: f32,
}

impl From<doctree_llm::RagHit> for RagHitDto {
    fn from(h: doctree_llm::RagHit) -> Self {
        Self {
            doc_id: h.doc_id,
            node_id: h.node_id,
            kind: h.kind,
            text: h.text,
            score: h.score,
        }
    }
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

#[cfg(any(feature = "llm", feature = "vectordb"))]
use std::sync::{Arc, Mutex};

/// Tauri-managed handle to the lazily-loaded native models. Each `Option` is a
/// lazy slot — `None` until the first call loads it; the `Mutex` serialises both
/// the load and the model's compute; the `Arc` lets an async command clone a
/// `'static` handle to hand to the blocking thread without borrowing managed
/// state across an await.
///
/// The two slots are independently feature-gated: the inference engine exists
/// under `llm`, the embedder under `vectordb`. A build can carry one, both, or
/// (in the default native-free build) neither — in which case this struct isn't
/// compiled and nothing is managed.
#[cfg(any(feature = "llm", feature = "vectordb"))]
#[derive(Default)]
pub struct LlmState {
    #[cfg(feature = "llm")]
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
    #[cfg(feature = "vectordb")]
    embedder: Arc<Mutex<Option<doctree_llm::Embedder>>>,
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

/// Run a grammar-constrained class confirmation on a blocking thread, returning
/// the model's single-word class answer. The grammar
/// ([`doctree_llm::classification_grammar`]) forces the answer to one of the
/// three prose/structured tags, so the text is trivially parseable downstream.
#[cfg(feature = "llm")]
fn confirm_class_blocking(
    engine: Arc<Mutex<Option<doctree_llm::Engine>>>,
    prompt: String,
) -> Result<String, String> {
    let mut guard = engine.lock().expect("llm engine mutex poisoned");
    let completion = ensure_loaded(&mut guard)?
        .complete_with_grammar(&prompt, doctree_llm::classification_grammar())
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
    // 1. Deterministic spine (pure, fast) and the per-window extraction prompts.
    //    `chunk_spine` covers the WHOLE document in batch-safe windows (ADR-00017),
    //    not just the opening. `DOCTREE_MAX_CHUNKS` caps how many windows we extract
    //    to bound interactive latency (default: the whole document).
    let spine = crate::walk_document_impl(&text, params);
    let mut prompts = doctree_llm::chunk_spine(&spine);
    if let Some(max) = std::env::var("DOCTREE_MAX_CHUNKS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        if max > 0 && prompts.len() > max {
            prompts.truncate(max);
        }
    }
    let total = prompts.len();

    // 2. Extract each window on a blocking thread, canonicalize its entity ids so the
    //    same entity collapses across windows under merge, and merge onto the
    //    accumulating graph. A window that fails (join error, non-schema JSON) is
    //    SKIPPED with a log line — one bad chunk never sinks the whole pass.
    let mut merged = spine;
    for (i, prompt) in prompts.into_iter().enumerate() {
        let engine = state.engine.clone();
        let json = match tauri::async_runtime::spawn_blocking(move || extract_blocking(engine, prompt))
            .await
        {
            Ok(Ok(json)) => json,
            Ok(Err(e)) => {
                eprintln!("[semantic] chunk {}/{total} extraction failed, skipped: {e}", i + 1);
                continue;
            }
            Err(e) => {
                eprintln!("[semantic] chunk {}/{total} task join failed, skipped: {e}", i + 1);
                continue;
            }
        };
        match serde_json::from_str::<Graph>(&json) {
            Ok(mut fragment) => {
                doctree_core::canonicalize_semantic_ids(&mut fragment);
                merged = merge_semantic_onto_spine(merged, fragment);
            }
            Err(e) => {
                let head: String = json.chars().take(160).collect();
                eprintln!(
                    "[semantic] chunk {}/{total} not schema JSON, skipped ({e}); starts: {head:?}",
                    i + 1
                );
            }
        }
    }

    // 3. Ordered build steps for the animated hybrid build.
    Ok(build_sequence(&merged))
}

/// Tauri command: **LLM-confirmed routing** (B5 / Phase 6 #6 — the deferred
/// ADR-0005 step). Run the deterministic classifier, and *only* when its verdict
/// is a low-confidence prose call ([`doctree_core::Classification::warrants_confirmation`])
/// ask the already-loaded model to confirm or correct the `Narrative`↔`Expository`
/// class before routing. A confident verdict — or a `Structured`/`Unknown` one —
/// skips the model entirely and routes exactly like `classify_document`.
///
/// The confirmed class re-resolves its pipeline through the same
/// [`crate::routing_from_classification`] logic as the first pass, so the returned
/// [`crate::Routing`] is shape- and rule-identical to the deterministic one — only
/// the `class`/`confidence` may differ. Inference is offloaded to a blocking
/// thread so the webview never stalls; classification and routing are pure.
#[cfg(feature = "llm")]
#[tauri::command]
pub async fn confirm_classification(
    text: String,
    state: tauri::State<'_, LlmState>,
) -> Result<crate::Routing, String> {
    // 1. Deterministic verdict first (pure, fast).
    let verdict = doctree_core::classify_document(&text);

    // 2. Only a low-confidence prose call is worth the model's opinion; every
    //    other verdict routes exactly like the native-free `classify_document`.
    if !verdict.warrants_confirmation() {
        return Ok(crate::routing_from_classification(verdict));
    }

    // 3. Ask the model for one corrected/confirmed class tag, on a blocking
    //    thread. `verdict.class` is the guess we're asking it to adjudicate.
    let prompt = doctree_llm::build_confirmation_prompt(&text, verdict.class);
    let engine = state.engine.clone();
    let answer = tauri::async_runtime::spawn_blocking(move || confirm_class_blocking(engine, prompt))
        .await
        .map_err(|e| format!("confirmation task failed to join: {e}"))??;

    // 4. Fold the answer back in — keeping the deterministic verdict if the model
    //    produced no recognizable tag — then route through the shared logic.
    let confirmed = match doctree_llm::parse_confirmed_class(&answer) {
        Some(class) => verdict.with_confirmed_class(class),
        None => verdict,
    };
    Ok(crate::routing_from_classification(confirmed))
}

// ---------------------------------------------------------------------------
// Native embedding path — only compiled under the `vectordb` feature. Embeddings
// (fastembed/ONNX) are independent of llama.cpp inference, so this whole block
// can be present without `llm` (similarity + search on the deterministic spine)
// or alongside it (similarity over the full hybrid graph).
// ---------------------------------------------------------------------------

/// Lazily load the embedding model into its slot on first use, then hand back a
/// reference. The model is cached under [`doctree_llm::embed_cache_dir`]; the
/// first call may download it. Errors flatten to `String` for the IPC boundary.
#[cfg(feature = "vectordb")]
fn ensure_embedder_loaded(
    slot: &mut Option<doctree_llm::Embedder>,
) -> Result<&doctree_llm::Embedder, String> {
    if slot.is_none() {
        let cache = doctree_llm::embed_cache_dir();
        *slot = Some(doctree_llm::Embedder::load(&cache).map_err(|e| e.to_string())?);
    }
    Ok(slot.as_ref().expect("embedder loaded above"))
}

/// Embed a batch of texts on a blocking thread (lazy-loading the model first).
/// One vector per input, in order.
#[cfg(feature = "vectordb")]
fn embed_batch_blocking(
    embedder: Arc<Mutex<Option<doctree_llm::Embedder>>>,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    let mut guard = embedder.lock().expect("embedder mutex poisoned");
    ensure_embedder_loaded(&mut guard)?
        .embed(texts)
        .map_err(|e| e.to_string())
}

/// Tauri command: the **embedding build** (B4). Walk the document into its
/// deterministic spine, embed its content nodes, derive similarity edges between
/// the nearest ones, and return the ordered [`BuildStep`] stream for the
/// augmented graph — so the frontend animates the structural build now woven
/// with embedding-similarity links (rendered like the other weighted edges).
///
/// Embedding is CPU-heavy, so it runs on a blocking thread; the walk and the
/// edge derivation are instant and pure.
#[cfg(feature = "vectordb")]
#[tauri::command]
pub async fn embedded_build_steps(
    text: String,
    params: Option<crate::WalkParams>,
    state: tauri::State<'_, LlmState>,
) -> Result<Vec<BuildStep>, String> {
    // 1. Deterministic spine, then the (id, text) pairs worth embedding.
    let spine = crate::walk_document_impl(&text, params);
    let inputs = doctree_llm::graph_embedding_inputs(&spine);
    if inputs.is_empty() {
        return Ok(build_sequence(&spine)); // nothing to embed → plain spine
    }
    let (ids, texts): (Vec<String>, Vec<String>) = inputs.into_iter().unzip();

    // 2. Embed the node texts on a blocking thread.
    let embedder = state.embedder.clone();
    let vectors = tauri::async_runtime::spawn_blocking(move || embed_batch_blocking(embedder, texts))
        .await
        .map_err(|e| format!("embedding task failed to join: {e}"))??;

    // 3. Pair ids with vectors, derive similarity edges, append (all pure).
    let embedded: Vec<(String, Vec<f32>)> = ids.into_iter().zip(vectors).collect();
    let augmented =
        attach_similarity_edges(spine, &embedded, &doctree_llm::SimilarityOptions::default());

    // 4. Ordered build steps for the animated, similarity-augmented build.
    Ok(build_sequence(&augmented))
}

/// Tauri command: **semantic search** (B4). Walk the document, embed both its
/// content nodes and the `query`, and return the nodes ranked by cosine
/// similarity to the query (best first, capped at `top_k`). The frontend uses
/// this to jump to the most relevant node for a free-text query, beyond the
/// literal substring search the structural build already offers.
#[cfg(feature = "vectordb")]
#[tauri::command]
pub async fn semantic_search(
    query: String,
    text: String,
    params: Option<crate::WalkParams>,
    top_k: Option<usize>,
    state: tauri::State<'_, LlmState>,
) -> Result<Vec<SearchHitDto>, String> {
    let graph = crate::walk_document_impl(&text, params);
    let inputs = doctree_llm::graph_embedding_inputs(&graph);
    if query.trim().is_empty() || inputs.is_empty() {
        return Ok(Vec::new());
    }
    let (ids, mut texts): (Vec<String>, Vec<String>) = inputs.into_iter().unzip();
    // Embed nodes + query in one batched model call; the query is the last row.
    texts.push(query);

    let embedder = state.embedder.clone();
    let mut vectors =
        tauri::async_runtime::spawn_blocking(move || embed_batch_blocking(embedder, texts))
            .await
            .map_err(|e| format!("embedding task failed to join: {e}"))??;

    let query_vec = vectors.pop().ok_or("embedder returned no query vector")?;
    let embedded: Vec<(String, Vec<f32>)> = ids.into_iter().zip(vectors).collect();
    let hits = doctree_llm::rank_by_similarity(&query_vec, &embedded, top_k.unwrap_or(10));
    Ok(to_search_hits(&graph, hits))
}

/// Tauri command: **index a document into the cross-document corpus** (Phase 6 #7
/// / ADR-00010). Walk the document into its deterministic spine, embed its content
/// nodes, and persist each `(doc_id, node_id)` row into the on-disk LanceDB corpus
/// (momusdev_llm's `VectorStore`, consumed read-only). Returns how many rows were
/// written. Unlike the in-memory B4 search, this corpus survives restarts and
/// spans every indexed document, so [`rag_search`] can retrieve across the whole
/// library.
///
/// Embedding *and* the store's own Tokio `block_on` must run off the webview's
/// async event loop — and the store's runtime must not nest inside Tauri's async
/// runtime — so the whole embed→open→index sequence runs on one blocking thread.
#[cfg(feature = "vectordb")]
#[tauri::command]
pub async fn rag_index_document(
    doc_id: String,
    text: String,
    params: Option<crate::WalkParams>,
    state: tauri::State<'_, LlmState>,
) -> Result<usize, String> {
    // 1. Deterministic spine, then the (id, text) pairs worth embedding (pure).
    let spine = crate::walk_document_impl(&text, params);
    let inputs = doctree_llm::graph_embedding_inputs(&spine);
    if inputs.is_empty() {
        return Ok(0); // nothing embeddable → nothing persisted
    }
    let (ids, texts): (Vec<String>, Vec<String>) = inputs.into_iter().unzip();

    // 2. Embed, then persist — all on ONE blocking thread. `RagStore` owns its own
    //    Tokio runtime and drives LanceDB with `block_on`, which would panic if it
    //    nested inside Tauri's async worker; `spawn_blocking` gives it a plain
    //    thread, so opening the store here (not on the async path) is correct.
    let embedder = state.embedder.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let vectors = embed_batch_blocking(embedder, texts)?;
        let dim = vectors
            .first()
            .map(Vec::len)
            .ok_or("embedder returned no vectors")?;
        let embedded: Vec<(String, Vec<f32>)> = ids.into_iter().zip(vectors).collect();
        let records = doctree_llm::graph_rag_records(&spine, &doc_id, &embedded);
        let store = doctree_llm::RagStore::open(&doctree_llm::rag_db_dir(), dim)
            .map_err(|e| e.to_string())?;
        store.index(&records).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("corpus indexing task failed to join: {e}"))?
}

/// Tauri command: **cross-document corpus search** (Phase 6 #7 / ADR-00010). Embed
/// the free-text `query` and return the nearest nodes across *every* indexed
/// document (best first, capped at `top_k`, default 10) via the LanceDB ANN query.
/// Each [`RagHitDto`] names its document + node so the frontend can jump across the
/// whole corpus — the persistent, cross-document complement to [`semantic_search`]'s
/// single-document in-memory rank. An empty query returns no hits.
#[cfg(feature = "vectordb")]
#[tauri::command]
pub async fn rag_search(
    query: String,
    top_k: Option<usize>,
    state: tauri::State<'_, LlmState>,
) -> Result<Vec<RagHitDto>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Embed the query, open the corpus at that vector's width, run the ANN query —
    // all on one blocking thread (same runtime-nesting reason as indexing above).
    let embedder = state.embedder.clone();
    let hits = tauri::async_runtime::spawn_blocking(move || {
        let mut vectors = embed_batch_blocking(embedder, vec![query])?;
        let query_vec = vectors.pop().ok_or("embedder returned no query vector")?;
        let store = doctree_llm::RagStore::open(&doctree_llm::rag_db_dir(), query_vec.len())
            .map_err(|e| e.to_string())?;
        store
            .search(&query_vec, top_k.unwrap_or(10))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("corpus search task failed to join: {e}"))??;
    Ok(hits.into_iter().map(RagHitDto::from).collect())
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

/// Native-free stand-in for LLM-confirmed routing: with no engine to consult,
/// it returns the deterministic routing **unchanged** (not an error). Confirmation
/// is a pure enhancement, never a hard dependency — so the default build still
/// answers this command; it simply can't second-guess a low-confidence prose
/// verdict. Same IPC contract (`{ text }` in, `Routing` out) as the gated form.
#[cfg(not(feature = "llm"))]
#[tauri::command]
pub fn confirm_classification(text: String) -> Result<crate::Routing, String> {
    Ok(crate::classify_document_impl(&text))
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

/// Native-free stand-in for the embedding build: same IPC contract (`{ text,
/// params }` in, `BuildStep[]` out) but no embedder, so it returns an actionable
/// error. The frontend should fall back to the structural `build_steps`.
#[cfg(not(feature = "vectordb"))]
#[tauri::command]
pub fn embedded_build_steps(
    text: String,
    params: Option<crate::WalkParams>,
) -> Result<Vec<BuildStep>, String> {
    let _ = (&text, &params); // contract parity with the gated signature
    Err(no_vectordb_error())
}

/// Native-free stand-in for semantic search: same IPC contract but no embedder,
/// so it returns an actionable error. The frontend should fall back to the
/// literal substring search the structural build already offers.
#[cfg(not(feature = "vectordb"))]
#[tauri::command]
pub fn semantic_search(
    query: String,
    text: String,
    params: Option<crate::WalkParams>,
    top_k: Option<usize>,
) -> Result<Vec<SearchHitDto>, String> {
    let _ = (&query, &text, &params, &top_k); // contract parity
    Err(no_vectordb_error())
}

/// Native-free stand-in for corpus indexing: same IPC contract (`{ docId, text,
/// params }` in, count out) but no embedder/store, so it returns an actionable
/// error. The corpus is a `vectordb`-only capability — there is no deterministic
/// fallback, so unlike confirmation this stub errors rather than no-ops.
#[cfg(not(feature = "vectordb"))]
#[tauri::command]
pub fn rag_index_document(
    doc_id: String,
    text: String,
    params: Option<crate::WalkParams>,
) -> Result<usize, String> {
    let _ = (&doc_id, &text, &params); // contract parity with the gated signature
    Err(no_vectordb_error())
}

/// Native-free stand-in for corpus search: same IPC contract (`{ query, topK }`
/// in, `RagHitDto[]` out) but no embedder/store, so it returns an actionable
/// error. The frontend should fall back to the single-document literal search.
#[cfg(not(feature = "vectordb"))]
#[tauri::command]
pub fn rag_search(query: String, top_k: Option<usize>) -> Result<Vec<RagHitDto>, String> {
    let _ = (&query, &top_k); // contract parity with the gated signature
    Err(no_vectordb_error())
}

/// The shared "no embedder in this build" message, pointing at the fix.
#[cfg(not(feature = "vectordb"))]
fn no_vectordb_error() -> String {
    format!(
        "this desktop build has no embedding model. Rebuild the shell with \
         `--features vectordb` (optionally set {} to a model cache dir) to enable \
         similarity edges + semantic search. The deterministic structure pipeline \
         (walk/build) works without it.",
        doctree_llm::EMBED_CACHE_ENV
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
    fn attach_similarity_edges_appends_valid_weighted_links() {
        use doctree_core::{EdgeKind, Node, NodeKind, Provenance};

        // A tiny spine: two near-identical sentences and one unrelated term.
        let mut g = Graph::new();
        g.push_node(Node::structural("sent:1", NodeKind::Sentence, "the cat sat"));
        g.push_node(Node::structural("sent:2", NodeKind::Sentence, "the cat sat down"));
        g.push_node(Node::structural("term:x", NodeKind::Term, "x"));
        let before = g.edges.len();

        // Synthetic vectors: sent:1 ≈ sent:2 (parallel), term:x orthogonal.
        let embedded = vec![
            ("sent:1".to_string(), vec![1.0, 0.0, 0.0]),
            ("sent:2".to_string(), vec![0.95, 0.05, 0.0]),
            ("term:x".to_string(), vec![0.0, 0.0, 1.0]),
        ];
        let out = attach_similarity_edges(g, &embedded, &doctree_llm::SimilarityOptions::default());

        // Exactly one similarity edge added, between the two close sentences.
        assert_eq!(out.edges.len(), before + 1);
        let e = out.edges.last().unwrap();
        assert_eq!(e.kind, EdgeKind::SimilarTo);
        assert_eq!(e.provenance, Provenance::Embedding);
        assert!(e.weight.unwrap() > 0.9);
        assert!(out.is_valid(), "similarity edges reference existing nodes");
    }

    #[test]
    fn to_search_hits_attaches_label_and_kind_and_serializes_camel_case() {
        use doctree_core::{Node, NodeKind};
        let mut g = Graph::new();
        g.push_node(Node::semantic("char:mara", NodeKind::Character, "Mara"));
        g.push_node(Node::structural("sent:1", NodeKind::Sentence, "Mara met Vane."));

        let hits = vec![
            doctree_llm::SearchHit { id: "char:mara".into(), score: 0.91 },
            doctree_llm::SearchHit { id: "ghost".into(), score: 0.5 }, // not in graph
        ];
        let dtos = to_search_hits(&g, hits);

        // The absent node is dropped; the present one carries label + kind.
        assert_eq!(dtos.len(), 1);
        assert_eq!(dtos[0].id, "char:mara");
        assert_eq!(dtos[0].label, "Mara");
        assert_eq!(dtos[0].kind, NodeKind::Character);

        let v = serde_json::to_value(&dtos[0]).unwrap();
        assert_eq!(v["id"], "char:mara");
        assert_eq!(v["label"], "Mara");
        assert_eq!(v["kind"], "character"); // NodeKind → snake_case tag
        assert!(v["score"].is_number());
    }

    #[cfg(not(feature = "llm"))]
    #[test]
    fn confirm_classification_stub_matches_deterministic_routing() {
        // On a native-free build there is no model to consult, so confirmation
        // must fall back to *exactly* the deterministic routing — never an error,
        // never a different verdict. (Under `llm` the command may instead consult
        // the model on a low-confidence prose call; that path is desktop-only.)
        let doc = "Mara found the letter where Vane had left it. She read it twice, \
            then looked out at the dark harbor. \"The ship is gone,\" she said. The cove was empty.";
        let confirmed = confirm_classification(doc.to_string()).expect("stub never errors");
        let deterministic = crate::classify_document_impl(doc);
        assert_eq!(
            serde_json::to_value(&confirmed).unwrap(),
            serde_json::to_value(&deterministic).unwrap()
        );
    }

    #[cfg(not(feature = "vectordb"))]
    #[test]
    fn rag_commands_are_actionable_errors_without_vectordb() {
        // The corpus store is a `vectordb`-only capability with no deterministic
        // fallback, so on a native-free build both commands must return the
        // actionable rebuild hint — never silently succeed, never panic.
        let idx = rag_index_document("docA".into(), "Mara met Vane.".into(), None);
        assert!(idx.unwrap_err().contains("--features vectordb"));
        let search = rag_search("ship".into(), Some(5));
        assert!(search.unwrap_err().contains("--features vectordb"));
    }

    #[test]
    fn rag_hit_dto_maps_every_field_and_serializes_camel_case() {
        // The DTO mirrors `doctree_llm::RagHit` one-to-one and serialises to the
        // camelCase the frontend reads (docId/nodeId, not doc_id/node_id).
        let hit = doctree_llm::RagHit {
            doc_id: "docA".into(),
            node_id: "char:mara".into(),
            kind: "character".into(),
            text: "Mara".into(),
            score: 0.87,
        };
        let dto = RagHitDto::from(hit);
        assert_eq!(dto.doc_id, "docA");
        assert_eq!(dto.node_id, "char:mara");
        assert_eq!(dto.kind, "character");
        assert_eq!(dto.text, "Mara");

        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["docId"], "docA");
        assert_eq!(v["nodeId"], "char:mara");
        assert_eq!(v["kind"], "character");
        assert_eq!(v["text"], "Mara");
        assert!(v["score"].is_number());
        // snake_case must NOT leak through.
        assert!(v.get("doc_id").is_none());
        assert!(v.get("node_id").is_none());
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
