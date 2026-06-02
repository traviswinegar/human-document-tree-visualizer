//! Tauri command layer — the desktop shell's bridge from a document string to
//! the frontend's 3D graph (A8).
//!
//! The flow is deliberately thin and pure: a document `text` (plus optional
//! tunables) goes into [`doctree_core::walk_with`], producing the deterministic
//! spine [`Graph`], and `build_steps` additionally turns that graph into the
//! ordered [`BuildStep`] stream the animated build replays. **No LLM, no GPU,
//! no native deps** — this is the structure-only path that ADR-0001 requires the
//! default build to stand on. The semantic layer (B-phases) merges onto this
//! spine later; it never replaces it.
//!
//! The command bodies delegate to free functions ([`walk_document_impl`],
//! [`build_steps_impl`]) that take no `Window`/`AppHandle`, so the real work is
//! unit-testable with `cargo test -p doctree-tauri` and the `#[tauri::command]`
//! wrappers stay trivial.

use doctree_core::{
    build_sequence, classify_document as core_classify, walk_with, BuildStep,
    ClassificationSignals, DocumentClass, Graph, RecommendedPipeline, WalkOptions,
};
use serde::{Deserialize, Serialize};

/// The gated local-LLM command layer (B2/B3). Always present in the source; the
/// native engine inside it is compiled only under the `llm` feature. Public so
/// the pure pieces (e.g. `merge_semantic_onto_spine`) are reachable from the
/// crate's integration tests.
pub mod llm;

/// Frontend-supplied walk tunables. Mirrors [`WalkOptions`]; every field is
/// optional so the frontend can send `{}` (or omit the argument) and get the
/// conservative defaults. Kept as its own type rather than reusing
/// `WalkOptions` directly because the core struct is not `Deserialize` (and the
/// core crate must stay serde-light at its boundary).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkParams {
    pub min_term_freq: Option<usize>,
    pub min_clause_words: Option<usize>,
    pub max_terms: Option<usize>,
}

impl From<WalkParams> for WalkOptions {
    fn from(p: WalkParams) -> Self {
        let d = WalkOptions::default();
        WalkOptions {
            min_term_freq: p.min_term_freq.unwrap_or(d.min_term_freq),
            min_clause_words: p.min_clause_words.unwrap_or(d.min_clause_words),
            max_terms: p.max_terms.unwrap_or(d.max_terms),
        }
    }
}

/// Walk `text` into its deterministic spine [`Graph`]. Pure; no Tauri state.
pub fn walk_document_impl(text: &str, params: Option<WalkParams>) -> Graph {
    let opts: WalkOptions = params.unwrap_or_default().into();
    walk_with(text, &opts)
}

/// Walk `text`, then order it into the animated-build [`BuildStep`] stream.
/// Pure; no Tauri state.
pub fn build_steps_impl(text: &str, params: Option<WalkParams>) -> Vec<BuildStep> {
    build_sequence(&walk_document_impl(text, params))
}

/// Tauri command: document → spine graph.
#[tauri::command]
fn walk_document(text: String, params: Option<WalkParams>) -> Graph {
    walk_document_impl(&text, params)
}

/// Tauri command: document → ordered build steps for the animated build.
#[tauri::command]
fn build_steps(text: String, params: Option<WalkParams>) -> Vec<BuildStep> {
    build_steps_impl(&text, params)
}

// ---------------------------------------------------------------------------
// B5 — document-type detection runtime gate.
//
// `doctree_core::classify_document` decides *what kind* of document this is and
// recommends the ideal extraction pipeline from the document type alone. Here we
// reconcile that ideal with the native features this particular build actually
// compiled in (LLM under `llm`, embedder under `vectordb`) and report back the
// concrete Tauri command the frontend should invoke. The recommendation is pure
// domain logic in core; the capability reconciliation is a command-layer concern
// because only this layer knows what was compiled. Both halves are pure and
// unit-tested headlessly — no model is consulted to classify or to route.
// ---------------------------------------------------------------------------

/// Which native semantic features this binary was compiled with. Drives how a
/// recommended pipeline is degraded to what the build can actually run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Compiled with the `llm` feature (local grammar-constrained inference).
    pub llm: bool,
    /// Compiled with the `vectordb` feature (embedding similarity + search).
    pub vectordb: bool,
}

impl Capabilities {
    /// The capabilities actually compiled into *this* build. `cfg!` resolves to a
    /// constant per build, so this stays native-free (no model, no managed state).
    pub fn compiled() -> Self {
        Capabilities {
            llm: cfg!(feature = "llm"),
            vectordb: cfg!(feature = "vectordb"),
        }
    }

    /// A hypothetical fully-featured build — the yardstick for "was this routing
    /// degraded?".
    fn full() -> Self {
        Capabilities { llm: true, vectordb: true }
    }
}

/// The concrete build path chosen for a document, naming the Tauri command the
/// frontend should call. Serializes to a snake_case tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedPipeline {
    /// `semantic_build_steps` — spine + LLM narrative semantics (hybrid).
    SemanticBuild,
    /// `embedded_build_steps` — spine + embedding-similarity edges.
    EmbeddedBuild,
    /// `build_steps` — deterministic spine only.
    StructuralBuild,
}

impl ResolvedPipeline {
    /// The registered Tauri command that runs this pipeline.
    pub fn command(self) -> &'static str {
        match self {
            ResolvedPipeline::SemanticBuild => "semantic_build_steps",
            ResolvedPipeline::EmbeddedBuild => "embedded_build_steps",
            ResolvedPipeline::StructuralBuild => "build_steps",
        }
    }
}

/// Degrade the *ideal* pipeline for a document's class to what `caps` can run:
/// a narrative doc wants the LLM hybrid, but on a build without `llm` it falls
/// back to similarity (if `vectordb`) and ultimately to the always-available
/// deterministic spine. Pure and total over every (recommendation, caps) pair.
pub fn resolve_pipeline(recommended: RecommendedPipeline, caps: Capabilities) -> ResolvedPipeline {
    match recommended {
        RecommendedPipeline::NarrativeHybrid => {
            if caps.llm {
                ResolvedPipeline::SemanticBuild
            } else if caps.vectordb {
                ResolvedPipeline::EmbeddedBuild
            } else {
                ResolvedPipeline::StructuralBuild
            }
        }
        RecommendedPipeline::StructuralPlusSimilarity => {
            if caps.vectordb {
                ResolvedPipeline::EmbeddedBuild
            } else {
                ResolvedPipeline::StructuralBuild
            }
        }
        RecommendedPipeline::StructuralOnly => ResolvedPipeline::StructuralBuild,
    }
}

/// The full B5 routing verdict for the frontend: the document class + confidence
/// + the signals that produced it, the ideal pipeline, the pipeline actually
/// resolved against this build's capabilities, the command to call, and whether
/// the ideal had to be degraded (so the UI can explain "rebuild with `--features
/// llm` for the richer extraction").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Routing {
    pub class: DocumentClass,
    pub confidence: f32,
    pub signals: ClassificationSignals,
    pub recommended_pipeline: RecommendedPipeline,
    pub resolved_pipeline: ResolvedPipeline,
    /// The registered Tauri command the frontend should invoke to build the doc.
    pub command: &'static str,
    /// True when the ideal pipeline for this class isn't compiled into this build.
    pub downgraded: bool,
    pub capabilities: Capabilities,
}

/// Classify `text` and resolve the routing against this build's capabilities.
/// Pure; native-free; no managed state.
pub fn classify_document_impl(text: &str) -> Routing {
    let c = core_classify(text);
    let recommended = c.recommended_pipeline();
    let caps = Capabilities::compiled();
    let resolved = resolve_pipeline(recommended, caps);
    // "Downgraded" iff a fully-featured build would have resolved differently.
    let downgraded = resolve_pipeline(recommended, Capabilities::full()) != resolved;
    Routing {
        class: c.class,
        confidence: c.confidence,
        signals: c.signals,
        recommended_pipeline: recommended,
        resolved_pipeline: resolved,
        command: resolved.command(),
        downgraded,
        capabilities: caps,
    }
}

/// Tauri command: classify a document and tell the frontend which build command
/// to call for it. Always available (native-free, every build).
#[tauri::command]
fn classify_document(text: String) -> Routing {
    classify_document_impl(&text)
}

/// Launch the desktop app. Called by the thin `main.rs` (and by the mobile
/// entry point Tauri generates).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // The lazily-loaded native models (inference engine under `llm`, embedder
    // under `vectordb`) only exist — and only need managing — when a feature
    // turns them on; the default build manages nothing native.
    #[cfg(any(feature = "llm", feature = "vectordb"))]
    let builder = builder.manage(llm::LlmState::default());
    builder
        // Every command is registered on every build; only their native model
        // is feature-gated (the others resolve to actionable-error stubs), so
        // the frontend's IPC surface stays stable regardless of how it was built.
        .invoke_handler(tauri::generate_handler![
            walk_document,
            build_steps,
            classify_document,
            llm::llm_status,
            llm::llm_complete,
            llm::semantic_build_steps,
            llm::embedded_build_steps,
            llm::semantic_search
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# Chapter One\n\nThe keeper found a letter. The inspector arrived.\n\nThey argued about the ship. The ship was missing.";

    #[test]
    fn walk_params_default_matches_walkoptions_default() {
        let from_params: WalkOptions = WalkParams::default().into();
        let direct = WalkOptions::default();
        assert_eq!(from_params.min_term_freq, direct.min_term_freq);
        assert_eq!(from_params.min_clause_words, direct.min_clause_words);
        assert_eq!(from_params.max_terms, direct.max_terms);
    }

    #[test]
    fn walk_params_overrides_apply() {
        let p = WalkParams {
            min_term_freq: Some(5),
            min_clause_words: None,
            max_terms: Some(7),
        };
        let opts: WalkOptions = p.into();
        assert_eq!(opts.min_term_freq, 5);
        assert_eq!(opts.max_terms, 7);
        // unset field falls back to the default
        assert_eq!(opts.min_clause_words, WalkOptions::default().min_clause_words);
    }

    #[test]
    fn walk_document_produces_valid_graph() {
        let g = walk_document_impl(DOC, None);
        assert!(g.is_valid(), "spine must be referentially valid");
        assert!(!g.nodes.is_empty());
    }

    #[test]
    fn walk_document_is_deterministic() {
        assert_eq!(walk_document_impl(DOC, None), walk_document_impl(DOC, None));
    }

    #[test]
    fn build_steps_account_for_exactly_the_graph() {
        let g = walk_document_impl(DOC, None);
        let seq = build_steps_impl(DOC, None);
        let nodes = seq.iter().filter(|s| s.as_node().is_some()).count();
        let edges = seq.iter().filter(|s| s.as_edge().is_some()).count();
        assert_eq!(nodes, g.nodes.len());
        assert_eq!(edges, g.edges.len());
    }

    #[test]
    fn build_steps_serialize_to_frontend_tagged_shape() {
        let seq = build_steps_impl(DOC, None);
        let first = seq.first().expect("non-empty sequence");
        let v = serde_json::to_value(first).unwrap();
        // first step must be a node (can't wire an edge into an empty graph)
        assert_eq!(v["kind"], "node");
        assert!(v["node"]["id"].is_string());
    }

    #[test]
    fn empty_document_yields_empty_graph_not_panic() {
        let g = walk_document_impl("", None);
        assert!(g.is_valid());
        assert!(build_steps_impl("", None).is_empty() || !g.nodes.is_empty());
    }

    // --- B5: document-type detection runtime gate ---------------------------

    const NONE: Capabilities = Capabilities { llm: false, vectordb: false };
    const VEC_ONLY: Capabilities = Capabilities { llm: false, vectordb: true };
    const LLM_ONLY: Capabilities = Capabilities { llm: true, vectordb: false };
    const BOTH: Capabilities = Capabilities { llm: true, vectordb: true };

    #[test]
    fn narrative_routing_degrades_with_capabilities() {
        use RecommendedPipeline::NarrativeHybrid;
        // Full build runs the LLM hybrid; lose `llm` and it falls back to
        // similarity; lose both and it lands on the deterministic spine.
        assert_eq!(resolve_pipeline(NarrativeHybrid, BOTH), ResolvedPipeline::SemanticBuild);
        assert_eq!(resolve_pipeline(NarrativeHybrid, LLM_ONLY), ResolvedPipeline::SemanticBuild);
        assert_eq!(resolve_pipeline(NarrativeHybrid, VEC_ONLY), ResolvedPipeline::EmbeddedBuild);
        assert_eq!(resolve_pipeline(NarrativeHybrid, NONE), ResolvedPipeline::StructuralBuild);
    }

    #[test]
    fn expository_routing_needs_only_the_embedder() {
        use RecommendedPipeline::StructuralPlusSimilarity as Sps;
        assert_eq!(resolve_pipeline(Sps, BOTH), ResolvedPipeline::EmbeddedBuild);
        assert_eq!(resolve_pipeline(Sps, VEC_ONLY), ResolvedPipeline::EmbeddedBuild);
        // No embedder ⇒ spine only (the LLM hybrid is the wrong ontology here).
        assert_eq!(resolve_pipeline(Sps, LLM_ONLY), ResolvedPipeline::StructuralBuild);
        assert_eq!(resolve_pipeline(Sps, NONE), ResolvedPipeline::StructuralBuild);
    }

    #[test]
    fn structural_only_always_resolves_to_the_spine() {
        for caps in [NONE, VEC_ONLY, LLM_ONLY, BOTH] {
            assert_eq!(
                resolve_pipeline(RecommendedPipeline::StructuralOnly, caps),
                ResolvedPipeline::StructuralBuild
            );
        }
    }

    #[test]
    fn resolved_commands_are_actually_registered() {
        // The routing must only ever name commands the handler registers, or the
        // frontend would invoke a non-existent command.
        let registered = ["build_steps", "semantic_build_steps", "embedded_build_steps"];
        for p in [
            ResolvedPipeline::SemanticBuild,
            ResolvedPipeline::EmbeddedBuild,
            ResolvedPipeline::StructuralBuild,
        ] {
            assert!(registered.contains(&p.command()), "unregistered command {}", p.command());
        }
    }

    #[test]
    fn classify_routes_narrative_and_flags_downgrade_on_a_lean_build() {
        // On the default (native-free) test build, capabilities are empty, so a
        // clearly-narrative document is recommended the hybrid but resolves to
        // the deterministic spine — and that degradation is flagged.
        let narrative = "Mara found the letter where Vane had left it. She read it twice, \
            then looked out at the dark harbor. \"The ship is gone,\" she said. He turned and \
            watched her but said nothing. They had waited for weeks, and the cove was empty.";
        let r = classify_document_impl(narrative);
        assert_eq!(r.class, DocumentClass::Narrative, "signals {:?}", r.signals);
        assert_eq!(r.recommended_pipeline, RecommendedPipeline::NarrativeHybrid);
        // Capabilities reflect how this test binary was compiled.
        assert_eq!(r.capabilities, Capabilities::compiled());
        assert_eq!(r.command, r.resolved_pipeline.command());
        // downgraded is true exactly when the lean build can't run the hybrid.
        assert_eq!(r.downgraded, !r.capabilities.llm);
        if !r.capabilities.llm {
            assert_eq!(r.resolved_pipeline, ResolvedPipeline::StructuralBuild);
            assert_eq!(r.command, "build_steps");
        }
    }

    #[test]
    fn classify_document_serializes_camel_case_with_snake_case_tags() {
        let structured = "# Installation\n\n- Install the toolchain\n- Clone the repository\n\
            - Run the build\n\n## Usage\n\n1. Open the file\n2. Select a command\n3. Save output";
        let r = classify_document_impl(structured);
        assert_eq!(r.class, DocumentClass::Structured);
        assert_eq!(r.recommended_pipeline, RecommendedPipeline::StructuralOnly);
        assert_eq!(r.resolved_pipeline, ResolvedPipeline::StructuralBuild);
        assert!(!r.downgraded, "structural-only is never a downgrade");

        let v = serde_json::to_value(&r).unwrap();
        // camelCase field names for the frontend.
        for key in [
            "class",
            "confidence",
            "signals",
            "recommendedPipeline",
            "resolvedPipeline",
            "command",
            "downgraded",
            "capabilities",
        ] {
            assert!(v.get(key).is_some(), "missing JSON key {key}");
        }
        // snake_case enum tags, camelCase nested signal keys.
        assert_eq!(v["class"], "structured");
        assert_eq!(v["recommendedPipeline"], "structural_only");
        assert_eq!(v["resolvedPipeline"], "structural_build");
        assert_eq!(v["command"], "build_steps");
        assert!(v["signals"]["wordCount"].is_number());
        assert!(v["signals"]["structureRatio"].is_number());
        assert!(v["capabilities"].get("vectordb").is_some());
    }
}
