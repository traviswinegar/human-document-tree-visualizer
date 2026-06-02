//! Build ordering — the authoritative event stream for the animated build.
//!
//! The frontend grows the graph one element at a time and can replay it; that
//! requires a **deterministic ordering** of "add this node" / "add this edge"
//! steps. This module is the source of truth for that order (the TypeScript
//! `buildSequence` in `src/build-player.ts` mirrors it, the way `src/types.ts`
//! mirrors [`crate::schema`]).
//!
//! Order rule: **preserve the input node order** (the walker already emits its
//! spine in document order — "from word one"), and reveal each edge the instant
//! both of its endpoints have appeared. Edges whose endpoints never both appear
//! (dangling refs) are flushed at the end so the replayed graph is identical to
//! the source graph. Same graph in ⇒ same sequence out, which is what makes
//! replay faithful.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::schema::{Edge, Graph, Node};

/// One step of the animated build. Serializes to the tagged shape the frontend
/// consumes: `{"kind":"node","node":{…}}` / `{"kind":"edge","edge":{…}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuildStep {
    Node { node: Node },
    Edge { edge: Edge },
}

/// Deterministic build order for `graph`: nodes in their existing order, each
/// edge emitted as soon as both endpoints are present, danglers flushed last.
pub fn build_sequence(graph: &Graph) -> Vec<BuildStep> {
    let mut present: HashSet<&str> = HashSet::with_capacity(graph.nodes.len());
    let mut emitted = vec![false; graph.edges.len()];
    let mut seq: Vec<BuildStep> = Vec::with_capacity(graph.nodes.len() + graph.edges.len());

    for node in &graph.nodes {
        present.insert(node.id.as_str());
        seq.push(BuildStep::Node { node: node.clone() });
        for (i, edge) in graph.edges.iter().enumerate() {
            if !emitted[i]
                && present.contains(edge.source.as_str())
                && present.contains(edge.target.as_str())
            {
                seq.push(BuildStep::Edge { edge: edge.clone() });
                emitted[i] = true;
            }
        }
    }

    for (i, edge) in graph.edges.iter().enumerate() {
        if !emitted[i] {
            seq.push(BuildStep::Edge { edge: edge.clone() });
        }
    }

    seq
}

impl BuildStep {
    /// Borrow the node this step adds, if it is a node step.
    pub fn as_node(&self) -> Option<&Node> {
        match self {
            BuildStep::Node { node } => Some(node),
            BuildStep::Edge { .. } => None,
        }
    }

    /// Borrow the edge this step adds, if it is an edge step.
    pub fn as_edge(&self) -> Option<&Edge> {
        match self {
            BuildStep::Edge { edge } => Some(edge),
            BuildStep::Node { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{EdgeKind, NodeKind, Provenance};
    use crate::walker::walk;

    fn fixture_graph() -> Graph {
        let mut g = Graph::new();
        g.push_node(Node::structural("sec:1", NodeKind::Section, "Chapter One"));
        g.push_node(Node::structural("sent:1", NodeKind::Sentence, "A."));
        g.push_node(Node::structural("sent:2", NodeKind::Sentence, "B."));
        g.push_node(Node::semantic("char:mara", NodeKind::Character, "Mara"));
        g.push_edge(Edge::new("sec:1", "sent:1", EdgeKind::PartOf, Provenance::Structural));
        g.push_edge(Edge::new("sent:1", "sent:2", EdgeKind::Precedes, Provenance::Structural));
        g.push_edge(Edge::new("sent:1", "char:mara", EdgeKind::Mentions, Provenance::Semantic));
        g
    }

    #[test]
    fn sequence_is_deterministic() {
        let g = fixture_graph();
        assert_eq!(build_sequence(&g), build_sequence(&g));
    }

    #[test]
    fn every_step_accounts_for_exactly_the_graph() {
        let g = fixture_graph();
        let seq = build_sequence(&g);
        let nodes = seq.iter().filter(|s| s.as_node().is_some()).count();
        let edges = seq.iter().filter(|s| s.as_edge().is_some()).count();
        assert_eq!(nodes, g.nodes.len());
        assert_eq!(edges, g.edges.len());
    }

    #[test]
    fn no_edge_precedes_its_endpoints() {
        let g = fixture_graph();
        let mut present: HashSet<&str> = HashSet::new();
        for step in &build_sequence(&g) {
            match step {
                BuildStep::Node { node } => {
                    present.insert(node.id.as_str());
                }
                BuildStep::Edge { edge } => {
                    assert!(
                        present.contains(edge.source.as_str()),
                        "edge source {} revealed before its node",
                        edge.source
                    );
                    assert!(
                        present.contains(edge.target.as_str()),
                        "edge target {} revealed before its node",
                        edge.target
                    );
                }
            }
        }
    }

    #[test]
    fn preserves_input_node_order() {
        let g = fixture_graph();
        let seq = build_sequence(&g);
        let order: Vec<&str> = seq
            .iter()
            .filter_map(|s| s.as_node().map(|n| n.id.as_str()))
            .collect();
        assert_eq!(order, ["sec:1", "sent:1", "sent:2", "char:mara"]);
    }

    #[test]
    fn serializes_to_frontend_tagged_shape() {
        let step = BuildStep::Node {
            node: Node::structural("sent:1", NodeKind::Sentence, "Hi."),
        };
        let v = serde_json::to_value(&step).unwrap();
        assert_eq!(v["kind"], "node");
        assert_eq!(v["node"]["id"], "sent:1");

        let estep = BuildStep::Edge {
            edge: Edge::new("a", "b", EdgeKind::PartOf, Provenance::Structural),
        };
        let ev = serde_json::to_value(&estep).unwrap();
        assert_eq!(ev["kind"], "edge");
        assert_eq!(ev["edge"]["source"], "a");
    }

    #[test]
    fn walked_document_yields_valid_nontrivial_sequence() {
        let doc = "# Chapter One\n\nThe keeper found a letter. The inspector arrived.\n\nThey argued about the ship. The ship was missing.";
        let g = walk(doc);
        assert!(g.is_valid(), "spine must be referentially valid");
        let seq = build_sequence(&g);
        assert_eq!(
            seq.iter().filter(|s| s.as_node().is_some()).count(),
            g.nodes.len()
        );
        assert_eq!(
            seq.iter().filter(|s| s.as_edge().is_some()).count(),
            g.edges.len()
        );
        // first step is a node (you can't wire an edge into an empty graph)
        assert!(seq.first().unwrap().as_node().is_some());
        // and no edge precedes its endpoints
        let mut present: HashSet<&str> = HashSet::new();
        for step in &seq {
            match step {
                BuildStep::Node { node } => {
                    present.insert(node.id.as_str());
                }
                BuildStep::Edge { edge } => {
                    assert!(present.contains(edge.source.as_str()));
                    assert!(present.contains(edge.target.as_str()));
                }
            }
        }
    }
}
