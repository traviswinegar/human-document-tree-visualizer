//! The GBNF grammar that constrains the LLM's semantic-extraction output to the
//! [`crate::schema`] shape.
//!
//! This is the architectural linchpin from ADR-0001: the grammar is the
//! *deterministic contract* that tames the non-deterministic model. The model is
//! forced to emit `{ "nodes": [...], "edges": [...] }` where every node `kind`
//! and edge `kind` is one of an enumerated set and every value is a JSON string
//! — so the output deserializes into [`crate::schema::Graph`] *by construction*,
//! never "usually parses".
//!
//! The grammar constrains the **semantic subset** only: the LLM produces
//! `character / place / concept / event / object / group` nodes and the semantic
//! edge kinds. Structural nodes (sentences, clauses, …) come from the
//! deterministic walker, not the model. Semantic edges may reference structural
//! node ids (for coreference linking onto the spine); the grammar permits any
//! string id and Rust validates referential integrity post-parse.
//!
//! Actual *compilation* of this grammar happens in the LLM crate via
//! `InferenceEngine::complete_with_grammar` (the gated B-stream). Here we keep
//! the grammar dependency-free and lint it deterministically — [`lint_gbnf`]
//! catches the most common authoring bug (a reference to an undefined rule), so
//! `cargo test -p doctree-core` guards the grammar with no model in the loop.

use std::collections::HashSet;

/// The GBNF grammar for the semantic-extraction `{nodes,edges}` output.
///
/// llama.cpp GBNF dialect: `*` `+` `?` repetition, `|` alternation, `(...)`
/// grouping, `[...]` char classes, `"..."` terminals (inner `"` escaped as
/// `\"`), `#` line comments. Repetition-count syntax (`{n}`) is intentionally
/// avoided for maximum compatibility — the unicode escape is spelled out as four
/// `hex` references instead.
pub const GRAPH_GBNF: &str = r##"# human-document-tree — semantic extraction grammar
# Output deserializes into doctree_core::schema::Graph by construction.
root ::= ws graph ws

graph ::= "{" ws "\"nodes\"" ws ":" ws nodelist ws "," ws "\"edges\"" ws ":" ws edgelist ws "}"

nodelist ::= "[" ws ( node ( ws "," ws node )* )? ws "]"
node ::= "{" ws "\"id\"" ws ":" ws string ws "," ws "\"kind\"" ws ":" ws nodekind ws "," ws "\"label\"" ws ":" ws string ws "}"

edgelist ::= "[" ws ( edge ( ws "," ws edge )* )? ws "]"
edge ::= "{" ws "\"source\"" ws ":" ws string ws "," ws "\"target\"" ws ":" ws string ws "," ws "\"kind\"" ws ":" ws edgekind ws "}"

nodekind ::= "\"character\"" | "\"place\"" | "\"concept\"" | "\"event\"" | "\"object\"" | "\"group\""
edgekind ::= "\"mentions\"" | "\"interacts_with\"" | "\"located_in\"" | "\"relates_to\"" | "\"causes\"" | "\"precedes\""

string ::= "\"" char* "\""
char ::= [^"\\] | "\\" ( ["\\/bfnrt] | "u" hex hex hex hex )
hex ::= [0-9a-fA-F]
ws ::= ([ \t\n] ws)?
"##;

/// The semantic node `kind` tags the grammar permits (mirrors the semantic arm
/// of [`crate::schema::NodeKind`]). Used by tests to keep grammar and schema in
/// lockstep.
pub const SEMANTIC_NODE_KINDS: &[&str] =
    &["character", "place", "concept", "event", "object", "group"];

/// The semantic edge `kind` tags the grammar permits.
pub const SEMANTIC_EDGE_KINDS: &[&str] = &[
    "mentions",
    "interacts_with",
    "located_in",
    "relates_to",
    "causes",
    "precedes",
];

/// Returns the grammar text. (Function form for callers that prefer it over the
/// const, e.g. the LLM crate handing it to `complete_with_grammar`.)
pub fn graph_grammar() -> &'static str {
    GRAPH_GBNF
}

/// Lint a GBNF grammar for the single most common authoring error: a reference
/// to a rule that is never defined (a typo silently makes the grammar
/// unsatisfiable). Also requires a `root` rule.
///
/// This is a *structural* linter, not a full GBNF parser: it strips terminals
/// (`"..."`), char classes (`[...]`), and `#` comments, then treats each
/// non-blank line as either a definition (`name ::= rhs`) or an alternation
/// continuation, collecting defined vs. referenced rule names. That assumption
/// (one rule definition begins per line) holds for [`GRAPH_GBNF`].
///
/// Returns `Ok(())` if clean, or `Err(messages)` listing each problem.
pub fn lint_gbnf(src: &str) -> Result<(), Vec<String>> {
    let stripped = strip_terminals_classes_comments(src);

    let mut defined: HashSet<String> = HashSet::new();
    let mut referenced: Vec<String> = Vec::new();

    for raw_line in stripped.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(pos) = line.find("::=") {
            let lhs = line[..pos].trim();
            if is_ident(lhs) {
                defined.insert(lhs.to_string());
            }
            collect_idents(&line[pos + 3..], &mut referenced);
        } else {
            // Alternation continuation line: every identifier is a reference.
            collect_idents(line, &mut referenced);
        }
    }

    let mut errors = Vec::new();
    if !defined.contains("root") {
        errors.push("grammar has no `root` rule".to_string());
    }
    let mut seen_missing = HashSet::new();
    for name in &referenced {
        if !defined.contains(name) && seen_missing.insert(name.clone()) {
            errors.push(format!("undefined rule reference: `{name}`"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Replace `"..."` terminals, `[...]` char classes, and `# ...` comments with a
/// space, respecting backslash escapes. Newlines are preserved so the caller can
/// still process line by line.
fn strip_terminals_classes_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '"' => {
                while let Some(n) = chars.next() {
                    if n == '\\' {
                        chars.next();
                    } else if n == '"' {
                        break;
                    }
                }
                out.push(' ');
            }
            '[' => {
                while let Some(n) = chars.next() {
                    if n == '\\' {
                        chars.next();
                    } else if n == ']' {
                        break;
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Pull GBNF rule identifiers (`[A-Za-z][A-Za-z0-9_-]*`) out of an RHS fragment
/// that has already had terminals and char classes stripped.
fn collect_idents(s: &str, out: &mut Vec<String>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let d = bytes[i] as char;
                if d.is_ascii_alphanumeric() || d == '_' || d == '-' {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(s[start..i].to_string());
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_lints_clean() {
        // The real grammar must have a root and no dangling rule references.
        if let Err(errors) = lint_gbnf(GRAPH_GBNF) {
            panic!("GRAPH_GBNF failed lint: {errors:?}");
        }
    }

    #[test]
    fn grammar_has_root_and_top_level_shape() {
        assert!(GRAPH_GBNF.contains("root ::="));
        assert!(GRAPH_GBNF.contains("\\\"nodes\\\""));
        assert!(GRAPH_GBNF.contains("\\\"edges\\\""));
    }

    #[test]
    fn grammar_enumerates_every_semantic_kind() {
        for k in SEMANTIC_NODE_KINDS {
            let terminal = format!("\\\"{k}\\\"");
            assert!(
                GRAPH_GBNF.contains(&terminal),
                "grammar is missing node kind {k}"
            );
        }
        for k in SEMANTIC_EDGE_KINDS {
            let terminal = format!("\\\"{k}\\\"");
            assert!(
                GRAPH_GBNF.contains(&terminal),
                "grammar is missing edge kind {k}"
            );
        }
    }

    #[test]
    fn grammar_kinds_are_valid_schema_variants() {
        // Every kind the grammar can emit must deserialize into the schema enum —
        // this is the lockstep guarantee between grammar and schema.
        use crate::schema::{EdgeKind, NodeKind};
        for k in SEMANTIC_NODE_KINDS {
            let v: NodeKind = serde_json::from_value(serde_json::json!(k))
                .unwrap_or_else(|_| panic!("node kind {k} not in schema"));
            assert!(v.is_semantic(), "{k} must be a semantic node kind");
        }
        for k in SEMANTIC_EDGE_KINDS {
            let _v: EdgeKind = serde_json::from_value(serde_json::json!(k))
                .unwrap_or_else(|_| panic!("edge kind {k} not in schema"));
        }
    }

    #[test]
    fn lint_catches_undefined_reference() {
        let broken = r#"root ::= thing ws
thing ::= "x" missing_rule
ws ::= ([ \t] ws)?
"#;
        let err = lint_gbnf(broken).expect_err("should flag the undefined rule");
        assert!(
            err.iter().any(|e| e.contains("missing_rule")),
            "expected undefined-reference error, got {err:?}"
        );
    }

    #[test]
    fn lint_requires_root() {
        let no_root = r#"thing ::= "x"
"#;
        let err = lint_gbnf(no_root).expect_err("should require root");
        assert!(err.iter().any(|e| e.contains("root")));
    }

    #[test]
    fn lint_ignores_kinds_inside_terminals() {
        // The kind words live inside "..." terminals; the linter must not treat
        // them as rule references (stripping terminals is what prevents that).
        assert!(lint_gbnf(GRAPH_GBNF).is_ok());
    }
}
