//! Dual-pipeline benchmark (#20) — the **user's** desktop measurement run.
//!
//! Runs one document through *both* extraction strategies and benchmarks them at
//! every stage:
//!
//! 1. **Tier-1** — the deterministic spine (the walker alone). Always available,
//!    sub-millisecond.
//! 2. **Hybrid** — the spine plus the grammar-constrained LLM semantic layer
//!    (B3): prompt assembly → constrained extraction → merge onto the spine.
//!
//! The comparison machinery itself ([`graph_metrics`], [`compare_pipelines`]) is
//! pure and lives in `doctree-core`, fully unit-tested headlessly. What *needs* a
//! live model — running the hybrid lane end-to-end and timing a real CPU decode —
//! can only happen on the user's desktop, so it lives here as an `#[ignore]`d,
//! `--features llm`-gated integration test (same shape as `llm_roundtrip.rs`).
//! Without the feature this file compiles to zero tests, so the native-free
//! `cargo test` is unaffected.
//!
//! To run it on a machine with the model on disk:
//!
//! ```text
//! set DOCTREE_MODEL_PATH=C:\...\qwen3-4b-q4km.gguf
//! cargo test -p doctree-tauri --features llm --test benchmark_roundtrip -- --ignored --nocapture
//! ```
#![cfg(feature = "llm")]

use std::time::Instant;

use doctree_core::{build_sequence, compare_pipelines, PipelineRun};
use doctree_llm::{build_extraction_prompt, Engine, LlmConfig};
use doctree_tauri_lib::{llm::merge_semantic_onto_spine, walk_document_impl};

/// A short narrative with enough entities/relationships that the hybrid lane has
/// something real to extract over the structural spine.
const DOC: &str = "# The Cove\n\nMara met the inspector Vane at the cove. \
    They argued about the missing ship. The storm had taken it. \
    Vane suspected the harbor master, who had vanished the same night.";

/// The headline #20 acceptance: walk the document into its Tier-1 spine, run the
/// full hybrid pipeline, time each stage, and assert the hybrid lane *earns* its
/// latency by adding semantic content the spine can't produce. Prints a per-stage
/// report (`--nocapture`) so the latency/quality trade-off is legible.
#[test]
#[ignore = "loads a multi-GB GGUF model; run with --features llm -- --ignored --nocapture"]
fn benchmarks_tier1_against_hybrid_per_stage() {
    // --- Tier-1 lane: deterministic spine only (the segmentation stage) ---
    let t_walk = Instant::now();
    let spine = walk_document_impl(DOC, None);
    let walk_ms = t_walk.elapsed().as_millis() as u64;
    let tier1 = PipelineRun::measured("tier1", &spine, walk_ms);

    // --- Hybrid lane: spine + grammar-constrained LLM semantics ---
    // Stage: anchored prompt assembly (instant, pure).
    let t_prompt = Instant::now();
    let prompt = build_extraction_prompt(&spine);
    let prompt_ms = t_prompt.elapsed().as_millis() as u64;

    // Stage: grammar-constrained extraction (the expensive one — real CPU decode).
    let engine = Engine::load(&LlmConfig::from_env()).expect("load model from DOCTREE_MODEL_PATH");
    let t_extract = Instant::now();
    let json = engine
        .extract_graph_json(&prompt)
        .expect("grammar-constrained extraction")
        .text;
    let extract_ms = t_extract.elapsed().as_millis() as u64;

    // Stage: parse the fragment + merge onto the spine (instant, pure).
    let t_merge = Instant::now();
    let fragment: doctree_core::Graph =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("not schema JSON: {e}\n{json}"));
    let hybrid_graph = merge_semantic_onto_spine(spine.clone(), fragment);
    let merge_ms = t_merge.elapsed().as_millis() as u64;

    // The hybrid lane pays the shared walk plus its own three stages.
    let hybrid_ms = walk_ms + prompt_ms + extract_ms + merge_ms;
    let hybrid = PipelineRun::measured("hybrid", &hybrid_graph, hybrid_ms);

    let comparison = compare_pipelines(tier1, hybrid);

    // --- Per-stage report ---
    eprintln!("\n=== dual-pipeline benchmark (#20) ===");
    eprintln!(
        "stages (ms): walk={walk_ms}  prompt={prompt_ms}  extract={extract_ms}  merge={merge_ms}"
    );
    eprintln!(
        "tier1  : {} nodes / {} edges (valid={}, density={:.2})",
        comparison.tier1.metrics.nodes,
        comparison.tier1.metrics.edges,
        comparison.tier1.metrics.valid,
        comparison.tier1.metrics.edge_density,
    );
    eprintln!(
        "hybrid : {} nodes / {} edges (valid={}, density={:.2})",
        comparison.hybrid.metrics.nodes,
        comparison.hybrid.metrics.edges,
        comparison.hybrid.metrics.valid,
        comparison.hybrid.metrics.edge_density,
    );
    eprintln!("tier1  node kinds: {:?}", comparison.tier1.metrics.node_kinds);
    eprintln!("hybrid node kinds: {:?}", comparison.hybrid.metrics.node_kinds);
    eprintln!("hybrid provenance: {:?}", comparison.hybrid.metrics.provenance);
    eprintln!(
        "deltas : +{} nodes, +{} edges, {:.1}x latency ({} → {} ms)",
        comparison.node_delta,
        comparison.edge_delta,
        comparison.latency_ratio,
        comparison.tier1.elapsed_ms,
        comparison.hybrid.elapsed_ms,
    );
    eprintln!(
        "steps  : tier1={}  hybrid={}",
        build_sequence(&spine).len(),
        build_sequence(&hybrid_graph).len(),
    );

    // --- Assertions: the hybrid lane must actually earn its latency ---
    assert!(comparison.hybrid.metrics.valid, "hybrid graph must be valid");
    assert!(
        comparison.hybrid.metrics.nodes >= comparison.tier1.metrics.nodes,
        "hybrid never drops spine nodes"
    );
    assert!(
        hybrid_graph.nodes.iter().any(|n| n.kind.is_semantic()),
        "hybrid must add at least one semantic node, got: {json}"
    );
    assert!(
        comparison.node_delta > 0,
        "hybrid should add semantic nodes over the bare spine (got delta {})",
        comparison.node_delta
    );
}
