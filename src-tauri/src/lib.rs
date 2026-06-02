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

use doctree_core::{build_sequence, walk_with, BuildStep, Graph, WalkOptions};
use serde::{Deserialize, Serialize};

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

/// Launch the desktop app. Called by the thin `main.rs` (and by the mobile
/// entry point Tauri generates).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![walk_document, build_steps])
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
}
