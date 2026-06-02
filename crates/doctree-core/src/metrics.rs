//! Pipeline metrics — the genre-agnostic, native-free measuring tape for #20.
//!
//! The product extracts a document two ways: a deterministic **Tier-1 spine**
//! (the walker, always available) and a **hybrid** path that layers the gated
//! LLM semantics (and, optionally, embedding similarity) on top. To know whether
//! the expensive path *earns its keep* we have to compare the two — not just at
//! the finish line, but per stage: how many nodes/edges each produces, of which
//! kinds, from which layer, how referentially clean the result is, and how long
//! it took.
//!
//! Measuring a [`Graph`] is pure and dependency-free, so it lives here in
//! `doctree-core` and is fully testable headlessly. The *running* of the gated
//! LLM lane (and the wall-clock timing of a live model) happens in the command
//! layer / an `#[ignore]`d desktop harness — this module only consumes the two
//! finished graphs plus their elapsed times and reports the comparison. Same
//! split as ADR-0004/0005: pure domain logic in core, capability-bound work in
//! the command layer.
//!
//! Histograms key on the schema enums via [`BTreeMap`] so the ordering is
//! deterministic (enum declaration order) and the serialized JSON is stable for
//! the frontend — same graph in ⇒ same report out.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::{EdgeKind, Graph, NodeKind, Provenance};

/// A structural snapshot of one extracted [`Graph`]: counts, per-kind and
/// per-provenance histograms, referential validity, and mean connectivity.
///
/// Serializes camelCase for the frontend; the histogram *keys* are the schema
/// enums' snake_case tags (`character`, `part_of`, `semantic`, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMetrics {
    /// Total node count.
    pub nodes: usize,
    /// Total edge count.
    pub edges: usize,
    /// `true` when every edge's endpoints name existing nodes (no danglers).
    pub valid: bool,
    /// Mean connectivity: edges per node (`edges / nodes`), `0.0` for an empty
    /// graph. Not graph-theoretic density (`|E| / |V|(|V|-1)`), which is
    /// vanishingly small for a sparse document graph and far less legible — this
    /// is the average-degree reading a reader actually wants.
    pub edge_density: f32,
    /// How many nodes of each [`NodeKind`] (zero-count kinds omitted).
    pub node_kinds: BTreeMap<NodeKind, usize>,
    /// How many edges of each [`EdgeKind`] (zero-count kinds omitted).
    pub edge_kinds: BTreeMap<EdgeKind, usize>,
    /// How many elements (nodes + edges) came from each [`Provenance`] layer.
    pub provenance: BTreeMap<Provenance, usize>,
}

/// Measure `graph` into a [`GraphMetrics`]. Pure: same graph ⇒ same metrics.
pub fn graph_metrics(graph: &Graph) -> GraphMetrics {
    let mut node_kinds: BTreeMap<NodeKind, usize> = BTreeMap::new();
    let mut edge_kinds: BTreeMap<EdgeKind, usize> = BTreeMap::new();
    let mut provenance: BTreeMap<Provenance, usize> = BTreeMap::new();

    for node in &graph.nodes {
        *node_kinds.entry(node.kind).or_insert(0) += 1;
        *provenance.entry(node.provenance).or_insert(0) += 1;
    }
    for edge in &graph.edges {
        *edge_kinds.entry(edge.kind).or_insert(0) += 1;
        *provenance.entry(edge.provenance).or_insert(0) += 1;
    }

    let nodes = graph.nodes.len();
    let edges = graph.edges.len();
    let edge_density = if nodes == 0 {
        0.0
    } else {
        edges as f32 / nodes as f32
    };

    GraphMetrics {
        nodes,
        edges,
        valid: graph.is_valid(),
        edge_density,
        node_kinds,
        edge_kinds,
        provenance,
    }
}

/// One pipeline's result: a human label, the wall-clock it took to produce the
/// graph, and the [`GraphMetrics`] of what it produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRun {
    /// Identifies the lane in a report, e.g. `"tier1"` or `"hybrid"`.
    pub label: String,
    /// Wall-clock milliseconds the lane took (caller-supplied; timing the gated
    /// LLM is the command layer's job, not core's).
    pub elapsed_ms: u64,
    /// What the lane produced.
    pub metrics: GraphMetrics,
}

impl PipelineRun {
    /// Build a run by measuring `graph` under `label`, recording `elapsed_ms`.
    pub fn measured(label: impl Into<String>, graph: &Graph, elapsed_ms: u64) -> Self {
        PipelineRun {
            label: label.into(),
            elapsed_ms,
            metrics: graph_metrics(graph),
        }
    }
}

/// A side-by-side of the deterministic Tier-1 lane against the hybrid lane: each
/// run plus the headline deltas a reader scans first — how much semantic content
/// the hybrid added and what it cost in latency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineComparison {
    /// The deterministic spine-only lane (baseline).
    pub tier1: PipelineRun,
    /// The richer lane (spine + LLM semantics, ± embeddings).
    pub hybrid: PipelineRun,
    /// `hybrid.nodes - tier1.nodes` (signed: hybrid usually adds entities).
    pub node_delta: i64,
    /// `hybrid.edges - tier1.edges` (signed).
    pub edge_delta: i64,
    /// `hybrid.elapsed_ms / tier1.elapsed_ms`, how many times slower the hybrid
    /// lane ran. The baseline is floored at 1 ms so a sub-millisecond Tier-1
    /// timing never divides by zero.
    pub latency_ratio: f32,
}

/// Compare a Tier-1 run against a hybrid run, computing the headline deltas.
pub fn compare_pipelines(tier1: PipelineRun, hybrid: PipelineRun) -> PipelineComparison {
    let node_delta = hybrid.metrics.nodes as i64 - tier1.metrics.nodes as i64;
    let edge_delta = hybrid.metrics.edges as i64 - tier1.metrics.edges as i64;
    let latency_ratio = hybrid.elapsed_ms as f32 / tier1.elapsed_ms.max(1) as f32;
    PipelineComparison {
        tier1,
        hybrid,
        node_delta,
        edge_delta,
        latency_ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Edge, Node};

    /// Spine-only fixture: structural nodes + structural edges, all `Structural`.
    fn tier1_graph() -> Graph {
        let mut g = Graph::new();
        g.push_node(Node::structural("sec:1", NodeKind::Section, "Chapter One"));
        g.push_node(Node::structural("sent:1", NodeKind::Sentence, "A."));
        g.push_node(Node::structural("sent:2", NodeKind::Sentence, "B."));
        g.push_edge(Edge::new("sec:1", "sent:1", EdgeKind::PartOf, Provenance::Structural));
        g.push_edge(Edge::new("sent:1", "sent:2", EdgeKind::Precedes, Provenance::Structural));
        g
    }

    /// Hybrid fixture: the spine plus a semantic character + a `Mentions` edge.
    fn hybrid_graph() -> Graph {
        let mut g = tier1_graph();
        g.push_node(Node::semantic("char:mara", NodeKind::Character, "Mara"));
        g.push_edge(Edge::new("sent:1", "char:mara", EdgeKind::Mentions, Provenance::Semantic));
        g
    }

    #[test]
    fn metrics_count_kinds_and_provenance() {
        let m = graph_metrics(&hybrid_graph());
        assert_eq!(m.nodes, 4);
        assert_eq!(m.edges, 3);
        assert_eq!(m.node_kinds[&NodeKind::Sentence], 2);
        assert_eq!(m.node_kinds[&NodeKind::Section], 1);
        assert_eq!(m.node_kinds[&NodeKind::Character], 1);
        assert_eq!(m.edge_kinds[&EdgeKind::PartOf], 1);
        assert_eq!(m.edge_kinds[&EdgeKind::Mentions], 1);
        // provenance spans both nodes and edges: 3 structural nodes + 2
        // structural edges = 5 structural; 1 semantic node + 1 semantic edge = 2.
        assert_eq!(m.provenance[&Provenance::Structural], 5);
        assert_eq!(m.provenance[&Provenance::Semantic], 2);
        // a kind with no instances is simply absent, not present-with-zero.
        assert!(!m.node_kinds.contains_key(&NodeKind::Place));
    }

    #[test]
    fn metrics_are_deterministic() {
        let g = hybrid_graph();
        assert_eq!(graph_metrics(&g), graph_metrics(&g));
    }

    #[test]
    fn edge_density_is_edges_per_node_and_empty_is_zero() {
        let m = graph_metrics(&tier1_graph());
        // 2 edges / 3 nodes.
        assert!((m.edge_density - 2.0 / 3.0).abs() < 1e-6);
        assert!(m.valid);

        let empty = graph_metrics(&Graph::new());
        assert_eq!(empty.edge_density, 0.0);
        assert_eq!(empty.nodes, 0);
        assert!(empty.valid, "an empty graph has no dangling edges");
    }

    #[test]
    fn comparison_reports_deltas_and_latency_ratio() {
        let tier1 = PipelineRun::measured("tier1", &tier1_graph(), 4);
        let hybrid = PipelineRun::measured("hybrid", &hybrid_graph(), 40);
        let c = compare_pipelines(tier1, hybrid);
        assert_eq!(c.node_delta, 1, "hybrid adds one Character");
        assert_eq!(c.edge_delta, 1, "hybrid adds one Mentions edge");
        assert!((c.latency_ratio - 10.0).abs() < 1e-6, "40ms / 4ms = 10x");
    }

    #[test]
    fn latency_ratio_floors_a_sub_millisecond_baseline() {
        // Tier-1 can finish in well under a millisecond; the floor keeps the
        // ratio finite instead of dividing by zero.
        let tier1 = PipelineRun::measured("tier1", &tier1_graph(), 0);
        let hybrid = PipelineRun::measured("hybrid", &hybrid_graph(), 25);
        let c = compare_pipelines(tier1, hybrid);
        assert!(c.latency_ratio.is_finite());
        assert!((c.latency_ratio - 25.0).abs() < 1e-6, "25ms / max(0,1)ms = 25x");
    }

    #[test]
    fn metrics_serialize_camel_case_with_snake_case_histogram_keys() {
        let run = PipelineRun::measured("hybrid", &hybrid_graph(), 7);
        let v = serde_json::to_value(&run).unwrap();
        assert_eq!(v["label"], "hybrid");
        assert_eq!(v["elapsedMs"], 7);
        let m = &v["metrics"];
        // camelCase field names...
        assert_eq!(m["edgeDensity"].as_f64().unwrap(), 3.0 / 4.0);
        assert!(m["nodeKinds"].is_object());
        assert!(m["edgeKinds"].is_object());
        // ...with snake_case enum tags as the histogram keys.
        assert_eq!(m["nodeKinds"]["character"], 1);
        assert_eq!(m["edgeKinds"]["part_of"], 1);
        assert_eq!(m["provenance"]["structural"], 5);
    }

    #[test]
    fn comparison_serializes_camel_case() {
        let tier1 = PipelineRun::measured("tier1", &tier1_graph(), 4);
        let hybrid = PipelineRun::measured("hybrid", &hybrid_graph(), 40);
        let v = serde_json::to_value(compare_pipelines(tier1, hybrid)).unwrap();
        assert_eq!(v["nodeDelta"], 1);
        assert_eq!(v["edgeDelta"], 1);
        assert_eq!(v["latencyRatio"].as_f64().unwrap(), 10.0);
        assert_eq!(v["tier1"]["label"], "tier1");
        assert_eq!(v["hybrid"]["label"], "hybrid");
    }
}
