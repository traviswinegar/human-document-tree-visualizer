//! `doctree-llm` — the gated, native local-LLM layer of human-document-tree.
//!
//! This crate isolates everything that needs a native build (llama.cpp via
//! `momusdev_llm`, optionally CUDA/Vulkan/LanceDB) behind the `llm` feature.
//! With **default features** it pulls in **zero** native dependencies and still
//! compiles + tests, so `cargo test` and the whole default workspace build run
//! anywhere. That decoupling is the load-bearing invariant from ADR-0001: a
//! failed native/GPU build must never block the deterministic Stream-A pipeline.
//!
//! The semantic layer constrains LLM output to the canonical graph schema using
//! the GBNF grammar that lives in [`doctree_core`] — see [`graph_extraction_grammar`].

/// Whether this build includes the native LLM engine (the `llm` feature).
/// Lets callers (and tests) branch without `cfg!` scattered everywhere.
pub const LLM_ENABLED: bool = cfg!(feature = "llm");

/// Environment variable that overrides the GGUF model path at runtime. The
/// model currently lives under another app's AppData, so the path is never
/// hardcoded as the permanent answer — it is resolved from here.
pub const MODEL_PATH_ENV: &str = "DOCTREE_MODEL_PATH";

/// The GBNF grammar that constrains LLM extraction output to the canonical
/// graph schema's semantic subset. Sourced from [`doctree_core`] so the grammar
/// and the schema can never drift apart (ADR-0002).
pub fn graph_extraction_grammar() -> &'static str {
    doctree_core::GRAPH_GBNF
}

/// Soft cap on the anchored-document body inside an extraction prompt, in bytes.
/// A single qwen3-4b context is ~4096 tokens (~16 KB); leaving headroom for the
/// instruction preamble and the model's own output, ~8 KB of source keeps the
/// whole exchange inside one window. Documents larger than this are truncated
/// for now — proper chunked extraction is a follow-up (see BUILD_LOG backlog).
pub const PROMPT_DOC_BUDGET_BYTES: usize = 8_192;

/// Build the grammar-constrained extraction prompt for a document, given its
/// deterministic spine [`doctree_core::Graph`] (B3).
///
/// The document is presented as its **anchored** form: each section/sentence
/// node is rendered as `[<id>] <text>` in document order. This does double duty
/// — it is the readable text *and* the map of stable spine ids the model can
/// attach `mentions` edges to, so the semantic layer grounds onto the spine
/// instead of floating free. Pure and native-free: prompt construction needs no
/// model, so it is fully unit-testable without the `llm` feature.
///
/// The prompt is wrapped in **ChatML** — a `system` turn (the instructions), a
/// `user` turn (the anchored document), and an *open* `assistant` turn that ends
/// the string. This mirrors `momusdev_llm`'s proven `extract_command` pattern and
/// is load-bearing for the grammar: priming an open assistant turn makes the
/// model commit to its first JSON token immediately instead of warming up with
/// prose/whitespace. Together with the now whitespace-free [`graph_extraction_grammar`]
/// it cures the "0 output bytes" stall (the model could otherwise satisfy the old
/// `root ::= ws graph ws` grammar by emitting newlines until the budget ran out).
pub fn build_extraction_prompt(spine: &doctree_core::Graph) -> String {
    use doctree_core::NodeKind;

    // Anchorable nodes are the ones the walker gives real text + a span:
    // sections (headings) and sentences. They tile the document in order.
    let mut anchored: Vec<(usize, &str, &str)> = spine
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Section | NodeKind::Sentence))
        .filter_map(|n| {
            let text = n.text.as_deref()?;
            let start = n.span.map(|s| s.start).unwrap_or(usize::MAX);
            Some((start, n.id.as_str(), text))
        })
        .collect();
    anchored.sort_by_key(|(start, _, _)| *start);

    let mut body = String::new();
    let mut truncated = false;
    for (_, id, text) in &anchored {
        // One line per anchor; collapse internal newlines so the [id] prefix
        // stays meaningful and the model reads one unit per line.
        let line_text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let line = format!("[{id}] {line_text}\n");
        if body.len() + line.len() > PROMPT_DOC_BUDGET_BYTES {
            truncated = true;
            break;
        }
        body.push_str(&line);
    }
    if truncated {
        body.push_str("[...document truncated...]\n");
    }

    let node_kinds = doctree_core::grammar::SEMANTIC_NODE_KINDS.join("|");
    let edge_kinds = doctree_core::grammar::SEMANTIC_EDGE_KINDS.join("|");

    let instructions = format!(
        "You are a precise literary-analysis engine. Read the DOCUMENT and extract its \
semantic graph: the characters, places, concepts, events, objects, and groups in the \
narrative, plus the relationships between them.\n\n\
Output a single JSON object of the form {{\"nodes\": [...], \"edges\": [...]}}.\n\
- node: {{\"id\": \"<kind>:<slug>\", \"kind\": \"<{node_kinds}>\", \"label\": \"<display name>\"}} \
where <slug> is a short lowercase identifier (e.g. \"char:mara\", \"place:cove\", \"concept:betrayal\").\n\
- edge: {{\"source\": \"<id>\", \"target\": \"<id>\", \"kind\": \"<{edge_kinds}>\"}}.\n\
- Ground entities in the text: when a sentence introduces or refers to an entity, add a \
\"mentions\" edge from that sentence's bracketed anchor id (e.g. \"sent:3\") to the entity id.\n\
- Reuse the same entity id everywhere that entity appears; do not duplicate it.\n\
- Extract only what the text supports — do not invent entities or relationships."
    );

    // ChatML envelope with an OPEN assistant turn. The open turn primes the model
    // to emit its JSON answer immediately (mirrors momusdev_llm's extract_command),
    // which — paired with the whitespace-free grammar — prevents the leading-
    // whitespace stall. `body` already ends in a newline, so the user turn closes
    // cleanly as `…\n<|im_end|>`.
    format!(
        "<|im_start|>system\n{instructions}\n<|im_end|>\n\
<|im_start|>user\nDOCUMENT (each line is prefixed with its anchor id):\n{body}<|im_end|>\n\
<|im_start|>assistant\n"
    )
}

// ---------------------------------------------------------------------------
// Document-class confirmation (Phase 6 #6, the deferred ADR-0005 step) — pure,
// native-free prompt/grammar/parse helpers. The deterministic `classify_document`
// gate (doctree-core) owns the verdict; when it lands a low-confidence prose call
// (`Classification::warrants_confirmation`), the gated command layer asks the
// model — using THIS grammar + prompt — to confirm or correct the class, then
// folds the answer back with `Classification::with_confirmed_class`. Everything
// here is plain strings: no model, so it's exercised by the default `cargo test`.
// ---------------------------------------------------------------------------

/// The GBNF grammar that constrains a class-confirmation completion to **exactly
/// one** bare class tag. `Unknown` is deliberately absent: the model is only ever
/// asked to adjudicate the `Narrative`↔`Expository` prose boundary (the
/// deterministic gate owns "too short to tell"), and `Structured` is offered only
/// as an escape hatch for a document the surface features misread as prose.
///
/// Whitespace-free for the same reason as [`graph_extraction_grammar`]: paired
/// with an open assistant turn it makes the model commit to its answer token
/// immediately instead of stalling on leading whitespace.
pub fn classification_grammar() -> &'static str {
    "root ::= \"narrative\" | \"expository\" | \"structured\"\n"
}

/// Soft cap on the document excerpt inside a confirmation prompt, in bytes. The
/// class is a whole-document judgement that a representative opening already
/// answers, so a small excerpt keeps the exchange fast and well inside context.
pub const CONFIRM_DOC_BUDGET_BYTES: usize = 4_096;

/// Build the grammar-constrained class-confirmation prompt: show the model the
/// deterministic verdict it's checking plus a document excerpt, and ask for a
/// single corrected/confirmed class tag. Pure and native-free (no model needed to
/// construct it), so it's unit-tested without the `llm` feature.
///
/// Wrapped in the same ChatML envelope with an open assistant turn as
/// [`build_extraction_prompt`], so — paired with [`classification_grammar`] — the
/// model emits its one-word answer immediately.
pub fn build_confirmation_prompt(
    text: &str,
    current: doctree_core::DocumentClass,
) -> String {
    // A representative excerpt: the opening, whitespace-collapsed, byte-budgeted
    // on a char boundary so multi-byte text never splits mid-codepoint.
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut excerpt = collapsed.as_str();
    if excerpt.len() > CONFIRM_DOC_BUDGET_BYTES {
        let mut end = CONFIRM_DOC_BUDGET_BYTES;
        while end > 0 && !excerpt.is_char_boundary(end) {
            end -= 1;
        }
        excerpt = &excerpt[..end];
    }

    // The deterministic verdict, by its serialized snake_case tag, so the model
    // knows what it's being asked to confirm or override.
    let current_tag = match current {
        doctree_core::DocumentClass::Narrative => "narrative",
        doctree_core::DocumentClass::Expository => "expository",
        doctree_core::DocumentClass::Structured => "structured",
        doctree_core::DocumentClass::Unknown => "unknown",
    };

    let instructions = format!(
        "You are a precise document classifier. Decide what kind of document the \
EXCERPT is, choosing exactly one of:\n\
- narrative: prose fiction — characters, dialogue, recounted past-tense action.\n\
- expository: non-fiction prose — essays, articles, manuals, documentation.\n\
- structured: heading/list/code/table reference material, not flowing prose.\n\n\
A fast heuristic classifier guessed \"{current_tag}\" but was not confident. \
Confirm that class if it fits, or correct it. Answer with the single class word only."
    );

    format!(
        "<|im_start|>system\n{instructions}\n<|im_end|>\n\
<|im_start|>user\nEXCERPT:\n{excerpt}\n<|im_end|>\n\
<|im_start|>assistant\n"
    )
}

/// Parse a model's class-confirmation answer into a [`doctree_core::DocumentClass`].
/// [`classification_grammar`] already guarantees a clean tag, but this is tolerant
/// of surrounding whitespace/case and stray text so a free (ungrammared) fallback
/// completion still parses. Returns `None` if no known prose/structured tag is
/// present (`Unknown` is never a confirmation answer — see the grammar).
pub fn parse_confirmed_class(answer: &str) -> Option<doctree_core::DocumentClass> {
    let a = answer.trim().to_lowercase();
    // Prefer an exact one-word answer (the grammared path); else look for the
    // first tag mentioned (a tolerant fallback for an unconstrained completion).
    if a == "narrative" {
        return Some(doctree_core::DocumentClass::Narrative);
    }
    if a == "expository" {
        return Some(doctree_core::DocumentClass::Expository);
    }
    if a == "structured" {
        return Some(doctree_core::DocumentClass::Structured);
    }
    [
        ("narrative", doctree_core::DocumentClass::Narrative),
        ("expository", doctree_core::DocumentClass::Expository),
        ("structured", doctree_core::DocumentClass::Structured),
    ]
    .into_iter()
    .filter_map(|(tag, class)| a.find(tag).map(|pos| (pos, class)))
    .min_by_key(|(pos, _)| *pos)
    .map(|(_, class)| class)
}

// ---------------------------------------------------------------------------
// Embedding similarity (B4) — pure, native-free.
//
// The embedding *model* needs a native build (fastembed/ONNX, behind the
// `vectordb` feature), but everything that turns embedding vectors into graph
// structure — cosine similarity, top-k neighbour selection, query ranking — is
// plain arithmetic. Keeping it native-free means the real B4 logic is exercised
// by the default `cargo test`, exactly like `build_extraction_prompt` (B3); the
// gated `Embedder` only supplies the vectors.
// ---------------------------------------------------------------------------

/// Environment variable pointing at the on-disk cache for the embedding model
/// (the all-MiniLM-L6-v2 ONNX files). If unset, [`embed_cache_dir`] falls back
/// to a stable per-user temp directory; pre-populate it for fully offline use.
pub const EMBED_CACHE_ENV: &str = "DOCTREE_EMBED_CACHE";

/// Resolve the embedding-model cache directory: the [`EMBED_CACHE_ENV`] env var
/// if set and non-empty, else a stable per-user temp subdirectory. Native-free,
/// so the Tauri layer can name the cache location without the `vectordb` build.
pub fn embed_cache_dir() -> String {
    std::env::var(EMBED_CACHE_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("doctree-embed-cache")
                .to_string_lossy()
                .into_owned()
        })
}

/// Tunables for deriving [`doctree_core::EdgeKind::SimilarTo`] edges from node
/// embeddings.
#[derive(Debug, Clone, Copy)]
pub struct SimilarityOptions {
    /// Minimum cosine similarity for an edge to be considered (in `[-1, 1]`).
    pub min_similarity: f32,
    /// Cap on neighbours kept per node (its strongest `top_k`). Bounds the edge
    /// count to ~`top_k · n` instead of `n²`, so the graph stays legible.
    pub top_k: usize,
}

impl Default for SimilarityOptions {
    fn default() -> Self {
        // all-MiniLM-L6-v2 puts unrelated text around 0.2–0.4 and clearly
        // related text at 0.6+, so 0.6 keeps only meaningful links; four
        // neighbours per node shows clusters without hairballing.
        Self {
            min_similarity: 0.6,
            top_k: 4,
        }
    }
}

/// Cosine similarity of two equal-length vectors. Returns `0.0` for a length
/// mismatch, an empty input, or a zero-magnitude vector (no direction ⇒ no
/// similarity) — so it never yields `NaN`.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// One semantic-search result: a node id and its cosine similarity to the query.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub score: f32,
}

/// Rank embedded nodes by cosine similarity to `query`, best first, keeping at
/// most `top_k`. Ties break by id so the ordering is deterministic. Pure.
pub fn rank_by_similarity(
    query: &[f32],
    embedded: &[(String, Vec<f32>)],
    top_k: usize,
) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = embedded
        .iter()
        .map(|(id, v)| SearchHit {
            id: id.clone(),
            score: cosine_similarity(query, v),
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(top_k);
    hits
}

/// Which node kinds carry enough standalone meaning to embed. Structural
/// sub-units (paragraph/clause/quote/reference) are skipped — they are spans of
/// their parent sentence and only add noise to similarity. Sections, sentences,
/// salient terms, and every semantic entity are embedded.
pub fn is_embeddable_kind(kind: doctree_core::NodeKind) -> bool {
    use doctree_core::NodeKind;
    matches!(
        kind,
        NodeKind::Section | NodeKind::Sentence | NodeKind::Term
    ) || kind.is_semantic()
}

/// The `(id, text)` pairs to embed for a graph, in node order. A node's text is
/// its span `text` when present (sentences/sections), else its `label` (terms
/// and semantic entities). Nodes whose resolved text is empty are skipped. Pure
/// and deterministic, so the gated embedder only has to turn strings into
/// vectors.
pub fn graph_embedding_inputs(graph: &doctree_core::Graph) -> Vec<(String, String)> {
    graph
        .nodes
        .iter()
        .filter(|n| is_embeddable_kind(n.kind))
        .filter_map(|n| {
            let text = n
                .text
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| n.label.trim());
            (!text.is_empty()).then(|| (n.id.clone(), text.to_string()))
        })
        .collect()
}

/// Derive undirected [`doctree_core::EdgeKind::SimilarTo`] edges (provenance
/// [`doctree_core::Provenance::Embedding`]) from node embeddings.
///
/// For each node only its strongest `top_k` neighbours above `min_similarity`
/// are kept; an edge survives if it is in *either* endpoint's top-k, so a
/// mutually-strong pair is emitted exactly once. Each edge is weighted by the
/// pair's cosine similarity. Output is sorted by `(source, target)` for a stable
/// ordering. Pure — it takes vectors and returns graph edges, no model.
pub fn similarity_edges(
    embedded: &[(String, Vec<f32>)],
    opts: &SimilarityOptions,
) -> Vec<doctree_core::Edge> {
    use doctree_core::{Edge, EdgeKind, Provenance};
    use std::collections::BTreeSet;

    let n = embedded.len();
    // Candidate neighbours per node: (similarity, other-index), only above the
    // threshold. Built once over the upper triangle, mirrored to both nodes.
    let mut per_node: Vec<Vec<(f32, usize)>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in (i + 1)..n {
            let s = cosine_similarity(&embedded[i].1, &embedded[j].1);
            if s >= opts.min_similarity {
                per_node[i].push((s, j));
                per_node[j].push((s, i));
            }
        }
    }

    // Keep each node's strongest `top_k`; union the surviving index pairs.
    let mut kept: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (i, cands) in per_node.iter_mut().enumerate() {
        cands.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        for (_, j) in cands.iter().take(opts.top_k) {
            kept.insert(if i < *j { (i, *j) } else { (*j, i) });
        }
    }

    let mut edges: Vec<Edge> = kept
        .into_iter()
        .map(|(lo, hi)| {
            let s = cosine_similarity(&embedded[lo].1, &embedded[hi].1);
            Edge::new(
                embedded[lo].0.clone(),
                embedded[hi].0.clone(),
                EdgeKind::SimilarTo,
                Provenance::Embedding,
            )
            .with_weight(s)
        })
        .collect();
    edges.sort_by(|a, b| a.source.cmp(&b.source).then(a.target.cmp(&b.target)));
    edges
}

/// Configuration for loading a local GGUF model.
///
/// Native-free: this struct and its builders exist regardless of the `llm`
/// feature, so the Tauri command layer can construct/validate config without a
/// native build. The fields map onto [`momusdev_llm::InferenceEngine::load`].
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Absolute path to a `.gguf` file. `None` means "resolve from the
    /// [`MODEL_PATH_ENV`] env var at load time".
    pub model_path: Option<String>,
    /// Context-window budget in tokens.
    pub max_tokens: usize,
    /// Inference threads. `0` = auto-detect (the engine caps it at 8).
    pub n_threads: u32,
    /// Transformer layers to offload to GPU. `0` = CPU-only, `999` = all.
    /// Ignored unless the binary was built with a GPU feature (`cuda`/`vulkan`).
    pub n_gpu_layers: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            max_tokens: 4096,
            n_threads: 0,
            // Offload everything by default; harmlessly ignored on CPU builds.
            n_gpu_layers: 999,
        }
    }
}

impl LlmConfig {
    /// A config whose `model_path` is taken from the [`MODEL_PATH_ENV`] env var
    /// (if set), with all other fields at their defaults.
    pub fn from_env() -> Self {
        Self {
            model_path: std::env::var(MODEL_PATH_ENV).ok().filter(|s| !s.is_empty()),
            ..Self::default()
        }
    }

    /// Set an explicit model path (overrides the env var).
    pub fn with_model_path(mut self, path: impl Into<String>) -> Self {
        self.model_path = Some(path.into());
        self
    }

    /// Set GPU offload layers (`0` = CPU, `999` = all).
    pub fn with_gpu_layers(mut self, layers: u32) -> Self {
        self.n_gpu_layers = layers;
        self
    }

    /// Resolve the effective model path: the explicit `model_path` if set,
    /// otherwise the [`MODEL_PATH_ENV`] env var. `Err` if neither is present.
    pub fn resolve_model_path(&self) -> anyhow::Result<String> {
        if let Some(p) = &self.model_path {
            return Ok(p.clone());
        }
        std::env::var(MODEL_PATH_ENV)
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no model path: set LlmConfig.model_path or the {MODEL_PATH_ENV} env var"
                )
            })
    }
}

/// The text + token metrics of one completion. Native-free mirror of
/// `momusdev_met::InferenceResult`, so this crate's public surface never leaks
/// sibling-crate types into the (default) decoupled build.
#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub inference_ms: u64,
}

// ---------------------------------------------------------------------------
// Native engine — only compiled under the `llm` feature.
// ---------------------------------------------------------------------------

#[cfg(feature = "llm")]
mod engine {
    use super::{Completion, LlmConfig};
    use anyhow::{Context, Result};
    use momusdev_llm::InferenceEngine;

    /// A loaded local model. Thin wrapper over [`InferenceEngine`] that maps
    /// results into this crate's native-free [`Completion`] type and wires the
    /// graph-extraction grammar in by default.
    pub struct Engine {
        inner: InferenceEngine,
        max_output_bytes: usize,
    }

    impl Engine {
        /// Load a GGUF model per `config`. CPU-heavy and blocking — call from
        /// `tokio::task::spawn_blocking` in the Tauri layer.
        pub fn load(config: &LlmConfig) -> Result<Self> {
            let path = config.resolve_model_path()?;
            let inner = InferenceEngine::load(
                &path,
                config.max_tokens,
                config.n_threads,
                config.n_gpu_layers,
            )
            .with_context(|| format!("loading GGUF model at {path}"))?;
            // Budget completion bytes off the context window (~4 bytes/token).
            let max_output_bytes = config.max_tokens.saturating_mul(4).max(4096);
            Ok(Self {
                inner,
                max_output_bytes,
            })
        }

        /// Free-form completion with metrics.
        pub fn complete(&self, prompt: &str) -> Result<Completion> {
            // Field access (not a named type) keeps `momusdev_met` — only a
            // transitive dep — out of this crate's extern prelude.
            let r = self
                .inner
                .complete_with_metrics(prompt, self.max_output_bytes)?;
            Ok(Completion {
                text: r.text,
                prompt_tokens: r.metrics.prompt_tokens,
                completion_tokens: r.metrics.completion_tokens,
                inference_ms: r.metrics.inference_ms,
            })
        }

        /// Grammar-constrained completion (arbitrary GBNF).
        pub fn complete_with_grammar(&self, prompt: &str, grammar: &str) -> Result<Completion> {
            let r =
                self.inner
                    .complete_with_grammar(prompt, grammar, self.max_output_bytes)?;
            Ok(Completion {
                text: r.text,
                prompt_tokens: r.metrics.prompt_tokens,
                completion_tokens: r.metrics.completion_tokens,
                inference_ms: r.metrics.inference_ms,
            })
        }

        /// Grammar-constrained completion using the canonical graph-extraction
        /// grammar — output is guaranteed to be schema-conformant `{nodes,edges}`.
        pub fn extract_graph_json(&self, prompt: &str) -> Result<Completion> {
            self.complete_with_grammar(prompt, super::graph_extraction_grammar())
        }
    }
}

#[cfg(feature = "llm")]
pub use engine::Engine;

// ---------------------------------------------------------------------------
// Native embedder — only compiled under the `vectordb` feature (fastembed/ONNX).
// Independent of `llm`: a build can embed (for similarity edges + search)
// without compiling llama.cpp, and vice-versa.
// ---------------------------------------------------------------------------

#[cfg(feature = "vectordb")]
mod embedder {
    use anyhow::{Context, Result};
    use momusdev_llm::embeddings::{EmbedText, FastEmbedder};

    /// A loaded sentence-embedding model (all-MiniLM-L6-v2 via fastembed/ONNX),
    /// wrapping momusdev_llm's [`FastEmbedder`]. Maps the embedder's errors into
    /// this crate's `anyhow` surface and keeps the sibling type out of the public
    /// API. Send+Sync (the inner model sits behind a `Mutex`), so it lives
    /// happily in Tauri's managed state.
    pub struct Embedder {
        inner: FastEmbedder,
    }

    impl Embedder {
        /// Load the embedding model, caching its ONNX files under `cache_dir`.
        /// CPU-bound — and network-bound on the very first run if the model
        /// isn't cached yet — so call it from `spawn_blocking`.
        pub fn load(cache_dir: &str) -> Result<Self> {
            let inner = FastEmbedder::new(cache_dir)
                .with_context(|| format!("loading embedding model (cache dir: {cache_dir})"))?;
            Ok(Self { inner })
        }

        /// Embed a batch of texts; one vector per input, in order.
        pub fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            self.inner.embed(texts)
        }

        /// Embed a single text into its vector.
        pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
            self.inner
                .embed(vec![text.to_string()])?
                .pop()
                .context("embedder returned no vector for the query")
        }

        /// Output dimensionality (384 for MiniLM-L6-v2).
        pub fn dimension(&self) -> usize {
            self.inner.dimension()
        }
    }
}

#[cfg(feature = "vectordb")]
pub use embedder::Embedder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_is_disabled_in_the_default_build() {
        // The decoupling guarantee: default features pull in no native engine.
        assert!(!LLM_ENABLED, "default build must not enable the `llm` feature");
    }

    #[test]
    fn config_defaults_are_sane() {
        let c = LlmConfig::default();
        assert_eq!(c.model_path, None);
        assert_eq!(c.max_tokens, 4096);
        assert_eq!(c.n_threads, 0, "0 == auto-detect threads");
        assert_eq!(c.n_gpu_layers, 999, "offload all layers when a GPU build");
    }

    #[test]
    fn builder_sets_path_and_gpu_layers() {
        let c = LlmConfig::default()
            .with_model_path("C:/models/qwen3-4b.gguf")
            .with_gpu_layers(0);
        assert_eq!(c.model_path.as_deref(), Some("C:/models/qwen3-4b.gguf"));
        assert_eq!(c.n_gpu_layers, 0);
        assert_eq!(c.resolve_model_path().unwrap(), "C:/models/qwen3-4b.gguf");
    }

    #[test]
    fn resolve_model_path_errors_when_unset() {
        // No explicit path and (assuming a clean test env) no env var → error,
        // rather than silently loading nothing.
        let c = LlmConfig {
            model_path: None,
            ..LlmConfig::default()
        };
        if std::env::var(MODEL_PATH_ENV).is_err() {
            assert!(c.resolve_model_path().is_err());
        }
    }

    #[test]
    fn extraction_prompt_anchors_spine_and_lists_kinds() {
        use doctree_core::{Graph, Node, NodeKind, Span};
        let mut spine = Graph::new();
        spine.push_node(
            Node::structural("sec:1", NodeKind::Section, "Chapter One")
                .with_text("Chapter One")
                .with_span(Span::new(0, 11)),
        );
        // Deliberately out of document order to prove the prompt sorts by span.
        spine.push_node(
            Node::structural("sent:2", NodeKind::Sentence, "Vane arrived.")
                .with_text("Vane arrived.")
                .with_span(Span::new(30, 43)),
        );
        spine.push_node(
            Node::structural("sent:1", NodeKind::Sentence, "Mara found a letter.")
                .with_text("Mara found a letter.")
                .with_span(Span::new(12, 32)),
        );
        // A term has no text/span ⇒ must NOT appear as an anchor line.
        spine.push_node(Node::structural("term:letter", NodeKind::Term, "letter"));

        let p = build_extraction_prompt(&spine);

        // Anchors present, and in document (span) order: sec:1 < sent:1 < sent:2.
        let i_sec = p.find("[sec:1]").expect("sec anchor");
        let i_s1 = p.find("[sent:1]").expect("sent:1 anchor");
        let i_s2 = p.find("[sent:2]").expect("sent:2 anchor");
        assert!(i_sec < i_s1 && i_s1 < i_s2, "anchors must be in span order");
        // Anchored text travels with its id.
        assert!(p.contains("[sent:1] Mara found a letter."));
        // Non-anchorable nodes are excluded.
        assert!(!p.contains("term:letter"));
        // Every grammar kind the model may emit is named in the instructions.
        for k in doctree_core::grammar::SEMANTIC_NODE_KINDS {
            assert!(p.contains(k), "prompt should list node kind {k}");
        }
        for k in doctree_core::grammar::SEMANTIC_EDGE_KINDS {
            assert!(p.contains(k), "prompt should list edge kind {k}");
        }
        // It must steer toward grounding via mentions edges.
        assert!(p.contains("mentions"));
    }

    #[test]
    fn extraction_prompt_is_chatml_with_open_assistant_turn() {
        // Regression guard for the "0 output bytes" stall: the prompt must wrap
        // the instructions/document in ChatML and END with an *open* assistant
        // turn, so the model commits to its first JSON token instead of warming
        // up with whitespace.
        use doctree_core::{Graph, Node, NodeKind, Span};
        let mut spine = Graph::new();
        spine.push_node(
            Node::structural("sent:1", NodeKind::Sentence, "Mara found a letter.")
                .with_text("Mara found a letter.")
                .with_span(Span::new(0, 20)),
        );
        let p = build_extraction_prompt(&spine);

        // Turns appear in order: system → user → assistant.
        let i_sys = p.find("<|im_start|>system").expect("system turn");
        let i_user = p.find("<|im_start|>user").expect("user turn");
        let i_asst = p.find("<|im_start|>assistant").expect("assistant turn");
        assert!(i_sys < i_user && i_user < i_asst, "turns must be in order");
        // The assistant turn is left OPEN — no content, no closing tag.
        assert!(
            p.trim_end().ends_with("<|im_start|>assistant"),
            "assistant turn must be open to prime immediate JSON output, got tail: {:?}",
            &p[p.len().saturating_sub(40)..]
        );
        // Instructions live in the system turn; the document in the user turn.
        let i_engine = p
            .find("precise literary-analysis engine")
            .expect("instructions");
        let i_doc = p.find("[sent:1] Mara found a letter.").expect("document");
        assert!(
            i_sys < i_engine && i_engine < i_user,
            "instructions belong to the system turn"
        );
        assert!(i_user < i_doc && i_doc < i_asst, "document belongs to the user turn");
    }

    #[test]
    fn extraction_prompt_respects_the_doc_budget() {
        use doctree_core::{Graph, Node, NodeKind, Span};
        let mut spine = Graph::new();
        // Many long sentences, well past the byte budget.
        for i in 0..2000 {
            let text = format!("Sentence number {i} carries a fair amount of filler text.");
            let start = i * 60;
            spine.push_node(
                Node::structural(format!("sent:{i}"), NodeKind::Sentence, text.clone())
                    .with_text(text)
                    .with_span(Span::new(start, start + 58)),
            );
        }
        let p = build_extraction_prompt(&spine);
        assert!(p.contains("[...document truncated...]"), "must mark truncation");
        // The preamble is small; total prompt stays near the doc budget + slack.
        assert!(
            p.len() < PROMPT_DOC_BUDGET_BYTES + 2_000,
            "prompt should be bounded by the budget, got {} bytes",
            p.len()
        );
    }

    // --- #6: class-confirmation prompt / grammar / parse (native-free) --------

    #[test]
    fn classification_grammar_offers_the_three_prose_tags_not_unknown() {
        let g = classification_grammar();
        assert!(g.contains("\"narrative\""));
        assert!(g.contains("\"expository\""));
        assert!(g.contains("\"structured\""));
        assert!(!g.contains("unknown"), "the model is never asked to say unknown");
        // Whitespace-free root (one alternation), like the extraction grammar.
        assert!(g.trim_start().starts_with("root ::="));
    }

    #[test]
    fn confirmation_prompt_is_chatml_states_the_guess_and_carries_the_excerpt() {
        use doctree_core::DocumentClass;
        let p = build_confirmation_prompt("Mara read the letter twice.", DocumentClass::Expository);
        // ChatML envelope with an OPEN assistant turn (commits the answer token).
        assert!(p.starts_with("<|im_start|>system\n"));
        assert!(p.trim_end().ends_with("<|im_start|>assistant"));
        // Names the heuristic's guess so the model knows what it's checking.
        assert!(p.contains("\"expository\""));
        // Carries the document text.
        assert!(p.contains("Mara read the letter twice."));
    }

    #[test]
    fn confirmation_prompt_budgets_the_excerpt_on_a_char_boundary() {
        // A long multi-byte document must be truncated without splitting a glyph.
        let doc = "é ".repeat(4_000); // ~12 KB, well past CONFIRM_DOC_BUDGET_BYTES
        let p = build_confirmation_prompt(&doc, doctree_core::DocumentClass::Narrative);
        // It built a valid UTF-8 string (the char-boundary walk worked) and stayed
        // bounded by the budget plus the small fixed preamble.
        assert!(p.len() < CONFIRM_DOC_BUDGET_BYTES + 2_000);
    }

    #[test]
    fn parse_confirmed_class_reads_clean_tags_and_tolerates_noise() {
        use doctree_core::DocumentClass;
        // Clean one-word answers (the grammared path).
        assert_eq!(parse_confirmed_class("narrative"), Some(DocumentClass::Narrative));
        assert_eq!(parse_confirmed_class("  Expository\n"), Some(DocumentClass::Expository));
        assert_eq!(parse_confirmed_class("STRUCTURED"), Some(DocumentClass::Structured));
        // Tolerant fallback: a sentence answer → the first tag mentioned.
        assert_eq!(
            parse_confirmed_class("This reads as narrative to me."),
            Some(DocumentClass::Narrative)
        );
        // No known tag → None (caller keeps the deterministic verdict).
        assert_eq!(parse_confirmed_class("I'm not sure"), None);
        assert_eq!(parse_confirmed_class("unknown"), None);
    }

    #[test]
    fn cosine_similarity_handles_identical_orthogonal_and_degenerate() {
        // Identical direction → 1, orthogonal → 0, opposite → -1.
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        // Magnitude is normalised out: a longer parallel vector is still 1.
        assert!((cosine_similarity(&[1.0, 0.0], &[5.0, 0.0]) - 1.0).abs() < 1e-6);
        // Degenerate inputs never NaN: zero vector, empty, length mismatch → 0.
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
    }

    #[test]
    fn rank_by_similarity_orders_best_first_and_truncates() {
        let embedded = vec![
            ("a".to_string(), vec![1.0, 0.0]),  // exact match
            ("b".to_string(), vec![0.0, 1.0]),  // orthogonal
            ("c".to_string(), vec![1.0, 1.0]),  // 45° → ~0.707
        ];
        let hits = rank_by_similarity(&[1.0, 0.0], &embedded, 2);
        assert_eq!(hits.len(), 2, "truncated to top_k");
        assert_eq!(hits[0].id, "a");
        assert_eq!(hits[1].id, "c");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn similarity_edges_link_only_pairs_above_threshold() {
        // a≈b (near-parallel), c orthogonal to both.
        let embedded = vec![
            ("a".to_string(), vec![1.0, 0.0, 0.0]),
            ("b".to_string(), vec![0.9, 0.1, 0.0]),
            ("c".to_string(), vec![0.0, 0.0, 1.0]),
        ];
        let edges = similarity_edges(&embedded, &SimilarityOptions::default());
        assert_eq!(edges.len(), 1, "only the a–b pair clears 0.6");
        let e = &edges[0];
        assert_eq!((e.source.as_str(), e.target.as_str()), ("a", "b"));
        assert_eq!(e.kind, doctree_core::EdgeKind::SimilarTo);
        assert_eq!(e.provenance, doctree_core::Provenance::Embedding);
        assert!(e.weight.unwrap() > 0.9, "weighted by cosine similarity");
    }

    #[test]
    fn similarity_edges_respect_top_k() {
        // Three vectors at 0°, 10°, 20° — all pairwise above 0.6, but a–c is the
        // weakest pair. With top_k = 1 each node keeps only its single strongest
        // neighbour, so a–c is dropped; a–b and b–c survive.
        let embedded = vec![
            ("a".to_string(), vec![1.0, 0.0]),
            ("b".to_string(), vec![0.9848, 0.1736]),
            ("c".to_string(), vec![0.9397, 0.3420]),
        ];
        let opts = SimilarityOptions {
            min_similarity: 0.6,
            top_k: 1,
        };
        let edges = similarity_edges(&embedded, &opts);
        assert_eq!(edges.len(), 2, "top_k=1 drops the weakest (a–c) pair");
        assert!(
            !edges
                .iter()
                .any(|e| e.source == "a" && e.target == "c"),
            "a–c is the weakest and must be pruned"
        );
        // With a generous top_k all three pairs come back.
        let all = similarity_edges(&embedded, &SimilarityOptions { min_similarity: 0.6, top_k: 8 });
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn graph_embedding_inputs_selects_content_nodes() {
        use doctree_core::{Graph, Node, NodeKind, Span};
        let mut g = Graph::new();
        g.push_node(
            Node::structural("sec:1", NodeKind::Section, "The Cove")
                .with_text("The Cove")
                .with_span(Span::new(0, 8)),
        );
        g.push_node(
            Node::structural("sent:1", NodeKind::Sentence, "Mara met Vane.")
                .with_text("Mara met Vane.")
                .with_span(Span::new(9, 23)),
        );
        g.push_node(Node::structural("term:cove", NodeKind::Term, "cove")); // label only
        g.push_node(Node::semantic("char:mara", NodeKind::Character, "Mara")); // label only
        // Excluded structural sub-units (no usable standalone meaning):
        g.push_node(Node::structural("para:1", NodeKind::Paragraph, "para"));
        g.push_node(Node::structural("clause:1.1", NodeKind::Clause, "Mara met Vane"));

        let inputs = graph_embedding_inputs(&g);
        let ids: Vec<&str> = inputs.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["sec:1", "sent:1", "term:cove", "char:mara"]);
        // Sentence uses its span text; term/character fall back to the label.
        assert_eq!(inputs[1].1, "Mara met Vane.");
        assert_eq!(inputs[2].1, "cove");
        assert_eq!(inputs[3].1, "Mara");
    }

    #[test]
    fn embed_cache_dir_prefers_the_env_var() {
        // Default (env unset) is a non-empty path under temp.
        if std::env::var(EMBED_CACHE_ENV).is_err() {
            assert!(embed_cache_dir().contains("doctree-embed-cache"));
        }
    }

    #[test]
    fn extraction_grammar_comes_from_core_and_is_nonempty() {
        // The semantic layer must constrain output to the SAME grammar the core
        // crate owns — proves schema/grammar can't drift (ADR-0002).
        let g = graph_extraction_grammar();
        assert!(!g.is_empty());
        assert_eq!(g, doctree_core::GRAPH_GBNF, "must be the core crate's grammar verbatim");
        // And it must actually be valid GBNF per the core linter.
        assert!(doctree_core::lint_gbnf(g).is_ok(), "extraction grammar must lint clean");
    }

    /// Live demonstration of the "0 output bytes" bug and its fix. Needs a
    /// multi-GB local GGUF, so it is `#[ignore]`d and only compiled under the
    /// `llm` feature. Run it explicitly once the fix is in:
    ///   `DOCTREE_MODEL_PATH=… cargo test -p doctree-llm --features llm -- --ignored live_extraction`
    /// Before the fix (whitespace grammar + raw prompt) this returned an empty
    /// string; after it (whitespace-free grammar + ChatML open assistant turn) it
    /// returns schema-valid `{nodes,edges}` JSON with at least one entity.
    #[cfg(feature = "llm")]
    #[test]
    #[ignore = "needs a local GGUF model; set DOCTREE_MODEL_PATH and run with --features llm -- --ignored"]
    fn live_extraction_yields_nonempty_schema_valid_graph() {
        use doctree_core::{Graph, Node, NodeKind, Span};

        let config = LlmConfig::from_env();
        if config.resolve_model_path().is_err() {
            eprintln!("skipping live_extraction: DOCTREE_MODEL_PATH not set");
            return;
        }
        let engine = Engine::load(&config).expect("load GGUF model");

        let mut spine = Graph::new();
        spine.push_node(
            Node::structural(
                "sent:1",
                NodeKind::Sentence,
                "Mara found a letter in the cove.",
            )
            .with_text("Mara found a letter in the cove.")
            .with_span(Span::new(0, 32)),
        );
        spine.push_node(
            Node::structural(
                "sent:2",
                NodeKind::Sentence,
                "She feared Vane would betray the crew.",
            )
            .with_text("She feared Vane would betray the crew.")
            .with_span(Span::new(33, 71)),
        );

        let prompt = build_extraction_prompt(&spine);
        let completion = engine.extract_graph_json(&prompt).expect("extraction call");

        // The exact failure this guards: the model used to fill the token budget
        // with whitespace, which strip_chatml_tokens trimmed to "".
        assert!(
            !completion.text.trim().is_empty(),
            "extraction must not be empty (the 0-byte whitespace-stall bug)"
        );
        // The grammar guarantees the bytes deserialize into the schema by
        // construction — prove it end to end.
        let graph: Graph = serde_json::from_str(completion.text.trim()).unwrap_or_else(|e| {
            panic!(
                "extraction must be schema-valid JSON: {e}\n--- raw output ---\n{}",
                completion.text
            )
        });
        assert!(
            !graph.nodes.is_empty(),
            "should extract at least one semantic entity from the passage"
        );
    }
}
