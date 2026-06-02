//! `doctree-core` — the pure-Rust core of human-document-tree.
//!
//! This crate holds the deterministic, dependency-light heart of the pipeline:
//! the graph [`schema`] (node/edge types), the GBNF grammar that constrains LLM
//! output to that schema, and the deterministic structure walker that builds the
//! reproducible "spine" of the graph from a document. It has **no** native, LLM,
//! or GPU dependencies, so `cargo test -p doctree-core` runs anywhere — that
//! decoupling is the load-bearing invariant from ADR-0001.

pub mod build;
pub mod classify;
pub mod grammar;
pub mod schema;
pub mod walker;

pub use build::{build_sequence, BuildStep};
pub use classify::{
    classify_document, Classification, ClassificationSignals, DocumentClass, RecommendedPipeline,
};
pub use grammar::{graph_grammar, lint_gbnf, GRAPH_GBNF};
pub use schema::{Edge, EdgeKind, Graph, Node, NodeKind, Provenance, Span};
pub use walker::{walk, walk_with, WalkOptions};

/// Crate name surfaced for diagnostics / the engine-health command.
pub const CRATE_NAME: &str = "doctree-core";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_builds_and_test_harness_runs() {
        // A1 smoke test: proves the workspace, toolchain, and test harness work
        // before any external crate or schema is introduced.
        assert_eq!(CRATE_NAME, "doctree-core");
    }
}
