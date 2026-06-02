//! Live embedding round-trip — the **user's** B4 acceptance test.
//!
//! These are `#[ignore]`d because they load the all-MiniLM-L6-v2 ONNX model
//! (downloaded + cached on first run) and run real embedding inference; they
//! cannot run in the headless CI/build environment. To run them on a machine
//! that can reach the model cache:
//!
//! ```text
//! :: optional — point at a pre-populated cache to stay fully offline
//! set DOCTREE_EMBED_CACHE=C:\...\fastembed-cache
//! cargo test -p doctree-tauri --features vectordb -- --ignored --nocapture
//! ```
//!
//! The whole file is gated on `--features vectordb`; without it the crate
//! compiles to zero tests here, so a missing native ONNX runtime never breaks
//! the default `cargo test`. Embeddings are independent of the `llm` feature —
//! this needs no llama.cpp build.
#![cfg(feature = "vectordb")]

use doctree_llm::{cosine_similarity, Embedder};

/// The bare B4 acceptance: load the embedding model and turn text into a vector
/// of the expected dimensionality.
#[test]
#[ignore = "loads/downloads the ONNX embedding model; run with --features vectordb -- --ignored"]
fn embedder_returns_a_384d_vector() {
    let embedder = Embedder::load(&doctree_llm::embed_cache_dir()).expect("load embedding model");
    assert_eq!(embedder.dimension(), 384, "all-MiniLM-L6-v2 is 384-dim");
    let v = embedder.embed_one("the lighthouse keeper watched the storm").expect("embed");
    assert_eq!(v.len(), 384);
    assert!(v.iter().any(|&x| x != 0.0), "vector must not be all zeros");
}

/// The semantic property B4 relies on: paraphrases embed closer together than
/// unrelated sentences. If this holds, the similarity-edge threshold is
/// meaningful and `semantic_search` ranks sensibly.
#[test]
#[ignore = "loads/downloads the ONNX embedding model; run with --features vectordb -- --ignored"]
fn related_text_is_closer_than_unrelated_text() {
    let embedder = Embedder::load(&doctree_llm::embed_cache_dir()).expect("load embedding model");
    let v = embedder
        .embed(vec![
            "The detective questioned the suspect about the theft.".to_string(),
            "The investigator interrogated the man regarding the robbery.".to_string(),
            "Photosynthesis converts sunlight into chemical energy in plants.".to_string(),
        ])
        .expect("embed batch");
    let related = cosine_similarity(&v[0], &v[1]);
    let unrelated = cosine_similarity(&v[0], &v[2]);
    eprintln!("related {related:.3}  vs  unrelated {unrelated:.3}");
    assert!(
        related > unrelated,
        "paraphrase ({related:.3}) must be closer than off-topic ({unrelated:.3})"
    );
}

/// The full B4 build: walk a document into its spine, embed its content nodes,
/// derive similarity edges, and confirm the augmented graph stays valid and
/// gains embedding-provenance links that order into a build stream. This mirrors
/// what `embedded_build_steps` runs at runtime, exercised without the window.
#[test]
#[ignore = "loads/downloads the ONNX embedding model; run with --features vectordb -- --ignored"]
fn similarity_edges_augment_the_spine() {
    use doctree_core::{build_sequence, Provenance};
    use doctree_llm::{graph_embedding_inputs, SimilarityOptions};
    use doctree_tauri_lib::{llm::attach_similarity_edges, walk_document_impl};

    let doc = "# The Cove\n\nThe keeper watched the storm roll in. \
               A storm was coming over the cove. The inspector studied the tide charts.";
    let spine = walk_document_impl(doc, None);
    let spine_edges = spine.edges.len();

    let inputs = graph_embedding_inputs(&spine);
    assert!(!inputs.is_empty(), "spine must have content nodes to embed");
    let (ids, texts): (Vec<String>, Vec<String>) = inputs.into_iter().unzip();

    let embedder = Embedder::load(&doctree_llm::embed_cache_dir()).expect("load embedding model");
    let vectors = embedder.embed(texts).expect("embed node texts");
    let embedded: Vec<(String, Vec<f32>)> = ids.into_iter().zip(vectors).collect();

    let augmented = attach_similarity_edges(spine, &embedded, &SimilarityOptions::default());
    assert!(augmented.is_valid(), "augmented graph must be valid by construction");
    assert!(
        augmented.edges.len() >= spine_edges,
        "augmentation never drops spine edges"
    );
    let sim_edges = augmented
        .edges
        .iter()
        .filter(|e| e.provenance == Provenance::Embedding)
        .count();
    assert!(
        sim_edges > 0,
        "expected at least one embedding-similarity edge (the two storm sentences)"
    );
    let steps = build_sequence(&augmented);
    assert!(!steps.is_empty());
    eprintln!(
        "embedding → {} nodes / {} edges ({} similarity) / {} steps",
        augmented.nodes.len(),
        augmented.edges.len(),
        sim_edges,
        steps.len()
    );
}
