//! The document-graph schema — the narrative ontology.
//!
//! A [`Graph`] is `{ nodes, edges }`. This is the deterministic contract the
//! whole app revolves around:
//!
//! * the **structure walker** emits structural nodes/edges (the spine);
//! * the **LLM semantic layer** emits semantic nodes/edges, constrained by a
//!   GBNF grammar to deserialize straight into these types;
//! * the **frontend** renders `{ nodes, links }` (edges → links) and colors by
//!   [`NodeKind`] / [`Provenance`].
//!
//! Optional fields use `#[serde(default)]` so a *partial* LLM fragment (id +
//! kind + label for nodes; source + target + kind for edges) deserializes
//! cleanly — the grammar can emit a minimal shape while the deterministic layer
//! populates the richer fields (spans, text).

use serde::{Deserialize, Serialize};

/// What a node represents. Serializes to a stable snake_case tag that the GBNF
/// grammar enumerates verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    // --- structural (deterministic spine) ---
    /// A titled/numbered division (chapter, part, heading-delimited block).
    Section,
    /// A paragraph.
    Paragraph,
    /// A sentence.
    Sentence,
    /// A clause within a sentence (rule-based split).
    Clause,
    /// A quoted span (dialogue or block quote).
    Quote,
    /// A citation / footnote / cross-reference marker.
    Reference,
    /// A salient repeated term / keyword (from frequency + co-occurrence).
    Term,

    // --- semantic (LLM, narrative ontology) ---
    /// A person/agent in the narrative.
    Character,
    /// A location/setting.
    Place,
    /// An abstract idea, theme, or motif.
    Concept,
    /// Something that happens (action, occurrence) in the narrative.
    Event,
    /// A concrete object/artifact of narrative significance.
    Object,
    /// A collection of characters (family, faction, organization).
    Group,
}

impl NodeKind {
    /// `true` for the structural kinds the deterministic walker produces.
    pub fn is_structural(self) -> bool {
        matches!(
            self,
            NodeKind::Section
                | NodeKind::Paragraph
                | NodeKind::Sentence
                | NodeKind::Clause
                | NodeKind::Quote
                | NodeKind::Reference
                | NodeKind::Term
        )
    }

    /// `true` for the semantic kinds the LLM layer produces.
    pub fn is_semantic(self) -> bool {
        !self.is_structural()
    }
}

/// How two nodes relate. Serializes to a stable snake_case tag enumerated by the
/// GBNF grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Containment in the structural hierarchy (clause ∈ sentence ∈ paragraph …).
    PartOf,
    /// Sequential/temporal ordering (sentence→sentence, event→event).
    Precedes,
    /// A passage mentions an entity/term.
    Mentions,
    /// A reference node points at what it cites.
    References,
    /// A quote is attributed to / quotes a source.
    Quotes,
    /// Two terms co-occur (weighted by frequency).
    CoOccursWith,
    /// Two characters interact.
    InteractsWith,
    /// An entity/event is located in a place.
    LocatedIn,
    /// Generic semantic association (concept↔concept, etc.).
    RelatesTo,
    /// One event causes another.
    Causes,
    /// Embedding-similarity link (weighted by cosine similarity).
    SimilarTo,
}

/// Which layer produced an element — drives frontend coloring (deterministic
/// spine vs inferred semantics vs embedding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Deterministic structure walker.
    Structural,
    /// LLM semantic extraction.
    Semantic,
    /// Embedding similarity.
    Embedding,
}

impl Default for Provenance {
    /// Partial JSON we deserialize comes from the LLM layer, so an omitted
    /// `provenance` defaults to [`Provenance::Semantic`]. The deterministic
    /// walker always sets `Structural` explicitly in code.
    fn default() -> Self {
        Provenance::Semantic
    }
}

/// A half-open character range `[start, end)` into the source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

/// A graph node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Stable identifier. Deterministic for spine nodes (e.g. `"sent:12"`,
    /// `"clause:12.1"`); slugified for semantic nodes (e.g. `"char:elizabeth"`).
    pub id: String,
    pub kind: NodeKind,
    /// Human-readable display label.
    pub label: String,
    /// Underlying span text, for structural nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Character range into the source document (provenance for navigation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(default)]
    pub provenance: Provenance,
}

impl Node {
    /// Construct a structural node (provenance = `Structural`).
    pub fn structural(
        id: impl Into<String>,
        kind: NodeKind,
        label: impl Into<String>,
    ) -> Self {
        Node {
            id: id.into(),
            kind,
            label: label.into(),
            text: None,
            span: None,
            provenance: Provenance::Structural,
        }
    }

    /// Construct a semantic node (provenance = `Semantic`).
    pub fn semantic(
        id: impl Into<String>,
        kind: NodeKind,
        label: impl Into<String>,
    ) -> Self {
        Node {
            id: id.into(),
            kind,
            label: label.into(),
            text: None,
            span: None,
            provenance: Provenance::Semantic,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// A graph edge (directed: `source` → `target`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Strength for weighted edges (co-occurrence, similarity) in `[0, 1]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    #[serde(default)]
    pub provenance: Provenance,
}

impl Edge {
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        kind: EdgeKind,
        provenance: Provenance,
    ) -> Self {
        Edge {
            id: None,
            source: source.into(),
            target: target.into(),
            kind,
            label: None,
            weight: None,
            provenance,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = Some(weight);
        self
    }
}

/// The document graph: `{ nodes, edges }`. This exact shape is what the GBNF
/// grammar constrains the LLM to emit, and what the frontend renders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

/// An edge whose endpoint(s) reference no existing node — surfaced by
/// [`Graph::validate`].
#[derive(Debug, Clone, PartialEq)]
pub struct DanglingEdge {
    pub edge_index: usize,
    pub missing_source: bool,
    pub missing_target: bool,
}

impl Graph {
    pub fn new() -> Self {
        Graph::default()
    }

    pub fn push_node(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub fn push_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    pub fn contains_node(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    /// Every edge whose `source` or `target` does not name an existing node.
    /// Empty result ⇒ referentially valid.
    pub fn validate(&self) -> Vec<DanglingEdge> {
        use std::collections::HashSet;
        let ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let mut dangling = Vec::new();
        for (i, e) in self.edges.iter().enumerate() {
            let missing_source = !ids.contains(e.source.as_str());
            let missing_target = !ids.contains(e.target.as_str());
            if missing_source || missing_target {
                dangling.push(DanglingEdge {
                    edge_index: i,
                    missing_source,
                    missing_target,
                });
            }
        }
        dangling
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// Merge another graph into this one. Nodes are deduplicated by `id`
    /// (existing nodes win — the deterministic spine is authoritative over an
    /// LLM fragment that references the same id). Edges are appended.
    pub fn merge(&mut self, other: Graph) {
        for node in other.nodes {
            if !self.contains_node(&node.id) {
                self.nodes.push(node);
            }
        }
        self.edges.extend(other.edges);
    }

    /// Drop every edge whose `source` or `target` does not name an existing
    /// node, returning how many were removed. This is the post-merge safety net
    /// for the semantic layer (B3): the GBNF grammar lets the LLM emit edges to
    /// *any* string id, so a model can reference an entity it never declared (or
    /// a spine id that isn't in the fragment). Pruning after [`merge`] keeps the
    /// combined graph referentially valid by construction, so the build stream
    /// and the renderer never see a half-wired edge.
    pub fn prune_dangling_edges(&mut self) -> usize {
        use std::collections::HashSet;
        let ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let before = self.edges.len();
        self.edges
            .retain(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()));
        before - self.edges.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_kind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(NodeKind::Character).unwrap(),
            serde_json::json!("character")
        );
        assert_eq!(
            serde_json::to_value(NodeKind::Reference).unwrap(),
            serde_json::json!("reference")
        );
    }

    #[test]
    fn edge_kind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(EdgeKind::InteractsWith).unwrap(),
            serde_json::json!("interacts_with")
        );
        assert_eq!(
            serde_json::to_value(EdgeKind::CoOccursWith).unwrap(),
            serde_json::json!("co_occurs_with")
        );
    }

    #[test]
    fn structural_vs_semantic_partition_is_total() {
        let all = [
            NodeKind::Section,
            NodeKind::Paragraph,
            NodeKind::Sentence,
            NodeKind::Clause,
            NodeKind::Quote,
            NodeKind::Reference,
            NodeKind::Term,
            NodeKind::Character,
            NodeKind::Place,
            NodeKind::Concept,
            NodeKind::Event,
            NodeKind::Object,
            NodeKind::Group,
        ];
        for k in all {
            assert_ne!(k.is_structural(), k.is_semantic(), "{k:?} must be exactly one");
        }
        assert!(NodeKind::Sentence.is_structural());
        assert!(NodeKind::Character.is_semantic());
    }

    #[test]
    fn graph_round_trips_through_json() {
        let mut g = Graph::new();
        g.push_node(
            Node::structural("sent:1", NodeKind::Sentence, "It was a dark night.")
                .with_text("It was a dark night.")
                .with_span(Span::new(0, 20)),
        );
        g.push_node(Node::semantic("char:watson", NodeKind::Character, "Watson"));
        g.push_edge(
            Edge::new("sent:1", "char:watson", EdgeKind::Mentions, Provenance::Structural),
        );

        let json = serde_json::to_string(&g).unwrap();
        let back: Graph = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }

    #[test]
    fn partial_llm_fragment_deserializes_with_defaults() {
        // The minimal shape the GBNF grammar will emit: no provenance, no span,
        // no text on nodes; no id/label/weight on edges.
        let raw = r#"{
            "nodes": [
                {"id": "char:elizabeth", "kind": "character", "label": "Elizabeth Bennet"},
                {"id": "char:darcy", "kind": "character", "label": "Mr. Darcy"}
            ],
            "edges": [
                {"source": "char:elizabeth", "target": "char:darcy", "kind": "interacts_with"}
            ]
        }"#;
        let g: Graph = serde_json::from_str(raw).unwrap();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        // Omitted provenance defaults to Semantic (LLM origin).
        assert_eq!(g.nodes[0].provenance, Provenance::Semantic);
        assert_eq!(g.edges[0].provenance, Provenance::Semantic);
        assert!(g.nodes[0].span.is_none());
        assert!(g.edges[0].id.is_none());
        assert!(g.is_valid());
    }

    #[test]
    fn validate_flags_dangling_edges() {
        let mut g = Graph::new();
        g.push_node(Node::semantic("a", NodeKind::Concept, "A"));
        g.push_edge(Edge::new("a", "ghost", EdgeKind::RelatesTo, Provenance::Semantic));
        let dangling = g.validate();
        assert_eq!(dangling.len(), 1);
        assert_eq!(dangling[0].edge_index, 0);
        assert!(!dangling[0].missing_source);
        assert!(dangling[0].missing_target);
        assert!(!g.is_valid());
    }

    #[test]
    fn merge_dedupes_nodes_by_id_existing_wins() {
        let mut spine = Graph::new();
        spine.push_node(Node::structural("char:elizabeth", NodeKind::Term, "Elizabeth"));

        let mut fragment = Graph::new();
        // Same id, richer semantic label — spine must win (authoritative).
        fragment.push_node(Node::semantic(
            "char:elizabeth",
            NodeKind::Character,
            "Elizabeth Bennet",
        ));
        fragment.push_node(Node::semantic("char:darcy", NodeKind::Character, "Mr. Darcy"));
        fragment.push_edge(Edge::new(
            "char:elizabeth",
            "char:darcy",
            EdgeKind::InteractsWith,
            Provenance::Semantic,
        ));

        spine.merge(fragment);
        assert_eq!(spine.nodes.len(), 2, "duplicate id merged, new id added");
        let liz = spine.nodes.iter().find(|n| n.id == "char:elizabeth").unwrap();
        assert_eq!(liz.kind, NodeKind::Term, "existing spine node wins");
        assert_eq!(spine.edges.len(), 1);
        assert!(spine.is_valid());
    }

    #[test]
    fn prune_dangling_edges_drops_only_unwired_edges() {
        // Simulates a merged graph after the LLM layer: a spine sentence, two
        // semantic entities, and three edges — one valid spine→entity mention,
        // one valid entity↔entity, and one pointing at an entity that was never
        // declared (a hallucinated id the grammar happily allowed).
        let mut g = Graph::new();
        g.push_node(Node::structural("sent:1", NodeKind::Sentence, "Mara met Vane."));
        g.push_node(Node::semantic("char:mara", NodeKind::Character, "Mara"));
        g.push_node(Node::semantic("char:vane", NodeKind::Character, "Vane"));
        g.push_edge(Edge::new("sent:1", "char:mara", EdgeKind::Mentions, Provenance::Semantic));
        g.push_edge(Edge::new(
            "char:mara",
            "char:vane",
            EdgeKind::InteractsWith,
            Provenance::Semantic,
        ));
        g.push_edge(Edge::new(
            "char:mara",
            "char:ghost", // never declared
            EdgeKind::InteractsWith,
            Provenance::Semantic,
        ));
        assert!(!g.is_valid(), "the ghost edge makes it invalid pre-prune");

        let removed = g.prune_dangling_edges();
        assert_eq!(removed, 1, "exactly the ghost edge is dropped");
        assert_eq!(g.edges.len(), 2);
        assert!(g.is_valid(), "valid by construction after pruning");
        // The two good edges survive in order.
        assert_eq!(g.edges[0].target, "char:mara");
        assert_eq!(g.edges[1].target, "char:vane");
    }

    #[test]
    fn prune_is_a_noop_on_a_valid_graph() {
        let mut g = Graph::new();
        g.push_node(Node::semantic("a", NodeKind::Concept, "A"));
        g.push_node(Node::semantic("b", NodeKind::Concept, "B"));
        g.push_edge(Edge::new("a", "b", EdgeKind::RelatesTo, Provenance::Semantic));
        assert_eq!(g.prune_dangling_edges(), 0);
        assert_eq!(g.edges.len(), 1);
    }
}
