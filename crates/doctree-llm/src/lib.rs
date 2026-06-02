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

    format!(
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
- Extract only what the text supports — do not invent entities or relationships.\n\n\
DOCUMENT (each line is prefixed with its anchor id):\n{body}"
    )
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
}
