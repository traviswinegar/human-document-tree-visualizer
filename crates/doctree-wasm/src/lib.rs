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

use doctree_core::{
    build_sequence, decode_graph, decode_text, encode, token_stats, walk_with, Graph, WalkOptions,
};
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

/// Reconstruct the original document from `(text, graph)` via the reversible
/// tokenizer (ADR-00013) → JSON `{text, byteExact, tokens, docBytes, vocabSize}`.
///
/// `byteExact` is the pinned text-round-trip invariant evaluated live in the
/// browser: it is `true` iff `decode_text(encode(text, graph)) == text`.
/// Host-testable core for the `reconstructDocument` export.
fn reconstruct_document_json(text: &str, graph_json: &str) -> Result<String, String> {
    let graph: Graph = serde_json::from_str(graph_json).map_err(|e| e.to_string())?;
    let tokens = encode(text, &graph);
    let recovered = decode_text(&tokens).map_err(|e| e.to_string())?;
    let st = token_stats(text, &tokens);
    serde_json::to_string(&serde_json::json!({
        "text": recovered,
        "byteExact": recovered == text,
        "tokens": st.tokens,
        "docBytes": st.doc_bytes,
        "vocabSize": st.vocab_size,
    }))
    .map_err(|e| e.to_string())
}

/// Tokenize `(text, graph)` into the reversible integer-id stream (the "token
/// list to pass into a model") → JSON `{ids, tokens, docBytes, vocabSize}`.
/// Host-testable core for the `tokenizeGraph` export.
fn tokenize_graph_json(text: &str, graph_json: &str) -> Result<String, String> {
    let graph: Graph = serde_json::from_str(graph_json).map_err(|e| e.to_string())?;
    let tokens = encode(text, &graph);
    let st = token_stats(text, &tokens);
    serde_json::to_string(&serde_json::json!({
        "ids": tokens.ids(),
        "tokens": st.tokens,
        "docBytes": st.doc_bytes,
        "vocabSize": st.vocab_size,
    }))
    .map_err(|e| e.to_string())
}

/// Round-trip the graph through the token stream and return the decoded
/// [`Graph`] as JSON. Proves the graph-round-trip projection in the browser:
/// the result equals the input graph (ADR-00013). Host-testable core for the
/// `reconstructGraph` export.
fn reconstruct_graph_json(text: &str, graph_json: &str) -> Result<String, String> {
    let graph: Graph = serde_json::from_str(graph_json).map_err(|e| e.to_string())?;
    let tokens = encode(text, &graph);
    let decoded = decode_graph(&tokens).map_err(|e| e.to_string())?;
    serde_json::to_string(&decoded).map_err(|e| e.to_string())
}

/// Decode a raw token-id stream (a `.dttok` file's `ids`) back to BOTH projections —
/// the byte-exact document and the exact graph — from the **ids alone** (not a
/// re-encoded pair), so importing a token file reconstructs the doc and tree
/// (ADR-00018). JSON `{text, graph, tokens}`. Host-testable core for `decodeTokens`.
fn decode_tokens_json(ids_json: &str) -> Result<String, String> {
    let ids: Vec<u32> = serde_json::from_str(ids_json).map_err(|e| e.to_string())?;
    let tokens = doctree_core::Tokens(ids);
    let text = decode_text(&tokens).map_err(|e| e.to_string())?;
    let graph = decode_graph(&tokens).map_err(|e| e.to_string())?;
    serde_json::to_string(&serde_json::json!({
        "text": text,
        "graph": graph,
        "tokens": tokens.len(),
    }))
    .map_err(|e| e.to_string())
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

/// Reconstruct the original document from a graph (the **"Reconstruct
/// document"** button), as a JSON string
/// (`{"text":…,"byteExact":true,"tokens":…,"docBytes":…,"vocabSize":…}`).
#[wasm_bindgen(js_name = reconstructDocument)]
pub fn reconstruct_document(text: &str, graph_json: &str) -> Result<String, JsValue> {
    reconstruct_document_json(text, graph_json).map_err(|e| JsValue::from_str(&e))
}

/// Tokenize `(text, graph)` into the reversible integer-id stream, as a JSON
/// string (`{"ids":[…],"tokens":…,"docBytes":…,"vocabSize":…}`).
#[wasm_bindgen(js_name = tokenizeGraph)]
pub fn tokenize_graph(text: &str, graph_json: &str) -> Result<String, JsValue> {
    tokenize_graph_json(text, graph_json).map_err(|e| JsValue::from_str(&e))
}

/// Round-trip a graph through the token stream, returning the decoded graph as
/// a JSON string (`{"nodes":[…],"edges":[…]}`) — the graph-round-trip projection.
#[wasm_bindgen(js_name = reconstructGraph)]
pub fn reconstruct_graph(text: &str, graph_json: &str) -> Result<String, JsValue> {
    reconstruct_graph_json(text, graph_json).map_err(|e| JsValue::from_str(&e))
}

/// Decode a `.dttok` file's `ids` (a JSON array of token ids) back to the document
/// and graph, as a JSON string (`{"text":…,"graph":{…},"tokens":…}`).
#[wasm_bindgen(js_name = decodeTokens)]
pub fn decode_tokens(ids_json: &str) -> Result<String, JsValue> {
    decode_tokens_json(ids_json).map_err(|e| JsValue::from_str(&e))
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

    /// The browser-facing text round-trip: reconstruct returns the byte-exact
    /// original and reports `byteExact: true` (ADR-00013 pinned invariant,
    /// evaluated through the WASM JSON boundary).
    #[test]
    fn reconstruct_document_json_is_byte_exact() {
        let graph_json = walk_document_json(DOC, &WalkOptions::default()).unwrap();
        let out = reconstruct_document_json(DOC, &graph_json).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["text"], DOC);
        assert_eq!(v["byteExact"], true);
        // It is a fidelity tool, not a codec: the stream carries ≥ one token per byte.
        assert!(v["tokens"].as_u64().unwrap() >= v["docBytes"].as_u64().unwrap());
        assert!(v["docBytes"].as_u64().unwrap() >= DOC.len() as u64);
    }

    /// Text round-trip does not depend on graph correctness: an empty graph
    /// still reconstructs the document byte-exactly (the body carries the bytes).
    #[test]
    fn reconstruct_document_json_is_independent_of_graph() {
        let empty = r#"{"nodes":[],"edges":[]}"#;
        let out = reconstruct_document_json(DOC, empty).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["text"], DOC);
        assert_eq!(v["byteExact"], true);
    }

    /// `tokenizeGraph` yields the integer-id stream and consistent stats.
    #[test]
    fn tokenize_graph_json_emits_in_vocab_ids() {
        let graph_json = walk_document_json(DOC, &WalkOptions::default()).unwrap();
        let out = tokenize_graph_json(DOC, &graph_json).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let ids = v["ids"].as_array().expect("an array of ids");
        assert!(!ids.is_empty());
        assert_eq!(ids.len() as u64, v["tokens"].as_u64().unwrap());
        let vocab = v["vocabSize"].as_u64().unwrap();
        assert!(ids.iter().all(|i| i.as_u64().unwrap() < vocab));
    }

    /// The graph round-trip projection through the WASM boundary: decoding the
    /// stream reproduces the input graph exactly.
    #[test]
    fn reconstruct_graph_json_round_trips_exactly() {
        let graph_json = walk_document_json(DOC, &WalkOptions::default()).unwrap();
        let out = reconstruct_graph_json(DOC, &graph_json).unwrap();
        // serde round-trips to identical JSON values (field order is fixed by serde).
        let original: Value = serde_json::from_str(&graph_json).unwrap();
        let decoded: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(decoded, original);
    }

    /// ADR-00018: decoding from the `ids` ALONE recovers the byte-exact document AND
    /// the exact graph — the `.dttok` file round-trip (not a re-encoded pair).
    #[test]
    fn decode_tokens_json_round_trips_from_ids_alone() {
        let graph_json = walk_document_json(DOC, &WalkOptions::default()).unwrap();
        let tok: Value =
            serde_json::from_str(&tokenize_graph_json(DOC, &graph_json).unwrap()).unwrap();
        let ids_json = serde_json::to_string(&tok["ids"]).unwrap();
        let out: Value = serde_json::from_str(&decode_tokens_json(&ids_json).unwrap()).unwrap();
        assert_eq!(out["text"].as_str().unwrap(), DOC, "doc recovered byte-exact from ids");
        let original: Value = serde_json::from_str(&graph_json).unwrap();
        assert_eq!(out["graph"], original, "graph recovered exactly from ids");
    }

    /// Malformed graph JSON surfaces as an `Err`, not a panic.
    #[test]
    fn reconstruct_document_json_rejects_bad_graph_json() {
        assert!(reconstruct_document_json(DOC, "{ not json").is_err());
    }
}
