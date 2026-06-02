//! WebAssembly bindings for `doctree-core` (ADR-0003).
//!
//! This crate is the **browser** half of the walker bridge: the public web build
//! runs the *same* pure-Rust deterministic walker as the desktop shell, compiled
//! to `wasm32-unknown-unknown` instead of going over Tauri IPC. There is no
//! reimplementation — these functions are a thin shim over
//! [`doctree_core::walk_with`] + [`doctree_core::build_sequence`].
//!
//! The exports return a **JSON string** (not a structured-cloned JS object) so
//! the payload is byte-for-byte the tagged shape the Tauri `build_steps` command
//! serializes (`[{"kind":"node","node":{…}}, …]`); the frontend then `JSON.parse`s
//! both engines identically. The real serialization lives in the private
//! `*_json` helpers so it is host-testable with `cargo test -p doctree-wasm`.

use doctree_core::{build_sequence, walk_with, WalkOptions};
use wasm_bindgen::prelude::*;

/// Build [`WalkOptions`] from optional frontend tunables, defaulting any unset.
fn walk_options(
    min_term_freq: Option<u32>,
    min_clause_words: Option<u32>,
    max_terms: Option<u32>,
) -> WalkOptions {
    let d = WalkOptions::default();
    WalkOptions {
        min_term_freq: min_term_freq.map(|v| v as usize).unwrap_or(d.min_term_freq),
        min_clause_words: min_clause_words
            .map(|v| v as usize)
            .unwrap_or(d.min_clause_words),
        max_terms: max_terms.map(|v| v as usize).unwrap_or(d.max_terms),
    }
}

/// Walk `text` → ordered build steps → JSON string. Host-testable core.
fn build_steps_json(text: &str, opts: &WalkOptions) -> Result<String, serde_json::Error> {
    serde_json::to_string(&build_sequence(&walk_with(text, opts)))
}

/// Walk `text` → spine graph → JSON string. Host-testable core.
fn walk_document_json(text: &str, opts: &WalkOptions) -> Result<String, serde_json::Error> {
    serde_json::to_string(&walk_with(text, opts))
}

/// Install the panic hook so a Rust panic surfaces as `console.error` with a
/// readable message instead of an opaque `unreachable executed`.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Walk a document into the ordered animated-build steps, as a JSON string
/// (`[{"kind":"node","node":{…}}|{"kind":"edge","edge":{…}}, …]`).
#[wasm_bindgen(js_name = buildSteps)]
pub fn build_steps(
    text: &str,
    min_term_freq: Option<u32>,
    min_clause_words: Option<u32>,
    max_terms: Option<u32>,
) -> Result<String, JsValue> {
    let opts = walk_options(min_term_freq, min_clause_words, max_terms);
    build_steps_json(text, &opts).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Walk a document into its spine [`doctree_core::Graph`], as a JSON string
/// (`{"nodes":[…],"edges":[…]}`).
#[wasm_bindgen(js_name = walkDocument)]
pub fn walk_document(
    text: &str,
    min_term_freq: Option<u32>,
    min_clause_words: Option<u32>,
    max_terms: Option<u32>,
) -> Result<String, JsValue> {
    let opts = walk_options(min_term_freq, min_clause_words, max_terms);
    walk_document_json(text, &opts).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use doctree_core::build_sequence;
    use serde_json::Value;

    const DOC: &str = "# Chapter One\n\nThe keeper found a letter. The inspector arrived.\n\nThey argued about the ship. The ship was missing.";

    #[test]
    fn walk_options_defaults_when_unset() {
        let o = walk_options(None, None, None);
        let d = WalkOptions::default();
        assert_eq!(o.min_term_freq, d.min_term_freq);
        assert_eq!(o.min_clause_words, d.min_clause_words);
        assert_eq!(o.max_terms, d.max_terms);
    }

    #[test]
    fn walk_options_applies_overrides() {
        let o = walk_options(Some(5), None, Some(9));
        assert_eq!(o.min_term_freq, 5);
        assert_eq!(o.max_terms, 9);
        assert_eq!(o.min_clause_words, WalkOptions::default().min_clause_words);
    }

    #[test]
    fn build_steps_json_matches_core_sequence() {
        let opts = WalkOptions::default();
        let json = build_steps_json(DOC, &opts).unwrap();
        let from_wasm: Value = serde_json::from_str(&json).unwrap();
        // Same authoritative ordering as the core, serialized through the same path.
        let from_core = serde_json::to_value(build_sequence(&walk_with(DOC, &opts))).unwrap();
        assert_eq!(from_wasm, from_core);
    }

    #[test]
    fn build_steps_json_is_frontend_tagged_shape() {
        let json = build_steps_json(DOC, &WalkOptions::default()).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        let arr = v.as_array().expect("an array of steps");
        assert!(!arr.is_empty());
        // First step is a node (can't wire an edge into an empty graph).
        assert_eq!(arr[0]["kind"], "node");
        assert!(arr[0]["node"]["id"].is_string());
    }

    #[test]
    fn walk_document_json_is_valid_graph_shape() {
        let json = walk_document_json(DOC, &WalkOptions::default()).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert!(v["nodes"].as_array().is_some_and(|a| !a.is_empty()));
        assert!(v["edges"].as_array().is_some());
    }

    #[test]
    fn empty_document_is_handled() {
        let json = build_steps_json("", &WalkOptions::default()).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert!(v.as_array().is_some());
    }
}
