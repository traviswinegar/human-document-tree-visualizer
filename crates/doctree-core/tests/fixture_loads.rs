//! Verifies the shared frontend fixture is schema-valid. This fixture is the
//! render target the 3D graph is built against (ADR-0001 decoupling), so it must
//! always deserialize and validate against the canonical schema.

use doctree_core::schema::{Graph, NodeKind, Provenance};
use doctree_core::walker;

const SAMPLE: &str = include_str!("../../../fixtures/sample-narrative.graph.json");
const SAMPLE_TEXT: &str = include_str!("../../../fixtures/sample-narrative.txt");

#[test]
fn sample_narrative_fixture_is_schema_valid() {
    let graph: Graph = serde_json::from_str(SAMPLE).expect("fixture must deserialize");

    assert!(!graph.nodes.is_empty(), "fixture has nodes");
    assert!(!graph.edges.is_empty(), "fixture has edges");

    // No edge may reference a missing node — the frontend would render dangling links.
    let dangling = graph.validate();
    assert!(dangling.is_empty(), "fixture has dangling edges: {dangling:?}");

    // Node ids are unique (the merge/dedupe and the frontend both assume this).
    let mut ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    let unique = ids.len();
    ids.dedup();
    assert_eq!(unique, ids.len(), "fixture node ids must be unique");

    // The fixture exercises both layers (structural spine + semantic overlay) so
    // the renderer can prove color-by-provenance against it.
    assert!(graph
        .nodes
        .iter()
        .any(|n| n.provenance == Provenance::Structural && n.kind.is_structural()));
    assert!(graph
        .nodes
        .iter()
        .any(|n| n.provenance == Provenance::Semantic && n.kind == NodeKind::Character));
}

#[test]
fn walking_the_sample_text_yields_a_valid_nontrivial_spine() {
    let g = walker::walk(SAMPLE_TEXT);

    // The walker should find the heading as a section.
    assert!(g
        .nodes
        .iter()
        .any(|n| n.kind == NodeKind::Section && n.label.contains("Keeper")));

    // Multiple paragraphs and a healthy number of sentences.
    assert!(g.nodes.iter().filter(|n| n.kind == NodeKind::Paragraph).count() >= 4);
    assert!(g.nodes.iter().filter(|n| n.kind == NodeKind::Sentence).count() >= 8);

    // The dialogue line is a detected quote.
    assert!(g.nodes.iter().any(|n| n.kind == NodeKind::Quote));

    // The "[3]" marker is a detected reference.
    assert!(g
        .nodes
        .iter()
        .any(|n| n.kind == NodeKind::Reference && n.text.as_deref() == Some("[3]")));

    // Recurring nouns (ship, cove, harbor, letter, cormorant) become terms.
    let terms: Vec<&str> = g
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Term)
        .map(|n| n.label.as_str())
        .collect();
    assert!(terms.contains(&"ship"), "expected 'ship' term in {terms:?}");
    assert!(terms.contains(&"cove"), "expected 'cove' term in {terms:?}");

    // Whole spine is referentially valid and purely structural.
    assert!(g.validate().is_empty());
    assert!(g.nodes.iter().all(|n| n.kind.is_structural()));
}
