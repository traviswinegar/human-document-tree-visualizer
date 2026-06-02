//! Live CPU inference round-trip — the **user's** B2 acceptance test.
//!
//! These are `#[ignore]`d because they load a multi-GB GGUF model and run real
//! inference (minutes on CPU); they cannot run in CI or the headless build
//! environment. To run them on a machine with the model on disk:
//!
//! ```text
//! set DOCTREE_MODEL_PATH=C:\...\qwen3-4b-q4km.gguf
//! cargo test -p doctree-tauri --features llm -- --ignored --nocapture
//! ```
//!
//! The whole file is gated on `--features llm`; without it the crate compiles
//! to zero tests here (the default native-free build), so a missing C++/native
//! toolchain never breaks `cargo test`.
#![cfg(feature = "llm")]

use doctree_llm::{Engine, LlmConfig};

/// The bare ADR-0001 acceptance: a desktop-side call into `momusdev_llm` loads a
/// CPU model and returns generated text. (B2's headline.)
#[test]
#[ignore = "loads a multi-GB GGUF model; run with --features llm -- --ignored"]
fn freeform_completion_returns_text() {
    let engine =
        Engine::load(&LlmConfig::from_env()).expect("load model from DOCTREE_MODEL_PATH");
    let out = engine
        .complete("Reply with exactly one word: ok")
        .expect("completion succeeds");
    assert!(
        !out.text.trim().is_empty(),
        "model returned empty text: {out:?}"
    );
    eprintln!(
        "freeform → {:?}  ({} prompt / {} completion tok, {} ms)",
        out.text, out.prompt_tokens, out.completion_tokens, out.inference_ms
    );
}

/// The grammar-ceiling check flagged at A3: prove llama.cpp's GBNF parser
/// actually accepts the canonical schema grammar (the 6-alternative
/// `nodekind`/`edgekind` productions are the risk) and that constrained output
/// parses as the schema's `{nodes,edges}` shape. This is what B3 will build on.
#[test]
#[ignore = "loads a multi-GB GGUF model; run with --features llm -- --ignored"]
fn grammar_constrained_output_is_schema_shaped_json() {
    let engine = Engine::load(&LlmConfig::from_env()).expect("load model");
    let prompt = "Extract a small knowledge graph as JSON from this text. \
         Text: \"Mara met the inspector Vane at the cove. They argued about the missing ship.\"";
    let out = engine
        .extract_graph_json(prompt)
        .expect("grammar-constrained completion (proves the GBNF grammar compiles)");
    let v: serde_json::Value = serde_json::from_str(&out.text)
        .unwrap_or_else(|e| panic!("constrained output was not valid JSON: {e}\n---\n{}", out.text));
    assert!(
        v.get("nodes").is_some() && v.get("edges").is_some(),
        "schema requires top-level nodes+edges, got: {}",
        out.text
    );
    eprintln!("graph → {}  ({} ms)", out.text, out.inference_ms);
}

/// The full B3 hybrid build: walk a document into its spine, build the anchored
/// extraction prompt, run the grammar-constrained model, merge the fragment onto
/// the spine, and confirm the result is a valid graph with semantic content that
/// orders into a build stream. This is the end-to-end shape `semantic_build_steps`
/// runs at runtime, exercised here without the Tauri window.
#[test]
#[ignore = "loads a multi-GB GGUF model; run with --features llm -- --ignored"]
fn hybrid_extraction_merges_onto_the_spine() {
    use doctree_core::build_sequence;
    use doctree_llm::build_extraction_prompt;
    use doctree_tauri_lib::{llm::merge_semantic_onto_spine, walk_document_impl};

    let doc = "# The Cove\n\nMara met the inspector Vane at the cove. \
               They argued about the missing ship. The storm had taken it.";
    let spine = walk_document_impl(doc, None);
    let spine_nodes = spine.nodes.len();

    let prompt = build_extraction_prompt(&spine);
    let engine =
        doctree_llm::Engine::load(&doctree_llm::LlmConfig::from_env()).expect("load model");
    let json = engine
        .extract_graph_json(&prompt)
        .expect("grammar-constrained extraction")
        .text;
    let fragment: doctree_core::Graph =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("not schema JSON: {e}\n{json}"));

    let merged = merge_semantic_onto_spine(spine, fragment);
    assert!(merged.is_valid(), "hybrid graph must be valid by construction");
    assert!(
        merged.nodes.len() >= spine_nodes,
        "merge never drops spine nodes"
    );
    assert!(
        merged.nodes.iter().any(|n| n.kind.is_semantic()),
        "expected at least one LLM semantic node, got: {json}"
    );
    let steps = build_sequence(&merged);
    assert!(!steps.is_empty());
    eprintln!(
        "hybrid → {} nodes / {} edges / {} steps (spine had {})",
        merged.nodes.len(),
        merged.edges.len(),
        steps.len(),
        spine_nodes
    );
}
