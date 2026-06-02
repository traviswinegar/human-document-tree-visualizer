//! Verifies the shared frontend fixture is schema-valid. This fixture is the
//! render target the 3D graph is built against (ADR-0001 decoupling), so it must
//! always deserialize and validate against the canonical schema.

use doctree_core::schema::{Graph, NodeKind, Provenance};

const SAMPLE: &str = include_str!("../../../fixtures/sample-narrative.graph.json");

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
