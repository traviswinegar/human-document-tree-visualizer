//! corpus-export — Phase 8 / [ADR-00015] M1/M3.
//!
//! Walk a corpus of `.md` files (curated to authored prose) and export the source
//! data for the three tokenizer-bench arms:
//!
//! * **arm C (byte+graph) → `arm_c.u16` + `arm_c.body`**: the **arm-C-lite** stream —
//!   the document bytes interleaved with structural boundary + kind markers *only*
//!   (`NODE_OPEN<kind>` / `NODE_CLOSE`). It carries **no labels, ids, text fields, or
//!   trailer**. This is deliberate: the full lossless `doctree_core::encode` emits each
//!   spanned node's `label` (which the walker sets to the node's *own text*) as a byte
//!   run immediately before the identical body bytes — a causal LM trivially copies it,
//!   leaking the text and invalidating the comparison (discovered in M3). `arm_c.body`
//!   is a `u8`-per-token mask (1 = a document body byte, scored for bits-per-byte;
//!   0 = a structural marker, context only). Lossy by design — this is the LM
//!   representation, not the reversible artifact (that remains `doctree_core::encode`).
//! * **arms A (BPE) & B (byte) → `corpus.txt`**: the exact concatenated source text.
//! * `manifest.json`: per-doc `{path, bytes, c_tokens}` + totals + a deterministic
//!   doc-based train/val split.
//!
//! Native-free (std + doctree-core). Graphs come from the **deterministic** walker.
//! Curation excludes the AI chat logs, templates, and Obsidian internals (ADR-00015 §4).
//!
//! ```text
//! cargo run -p corpus-export --release -- "C:\Writing Vault" research/tokenizer-bench/data
//! ```
//!
//! [ADR-00015]: ../../docs/adr/ADR-00015-graph-tokenizer-training-experiment.md

use std::fs;
use std::path::{Path, PathBuf};

use doctree_core::{walk, Graph, NodeKind};
use serde::Serialize;

/// Top-level vault folders excluded from the authored-prose corpus (ADR-00015 §4):
/// AI chat logs (not authored prose; would confound the comparison) and templates
/// (boilerplate). Obsidian internals are dropped separately via the dot-dir rule.
const DEFAULT_EXCLUDED_DIRS: &[&str] = &["_Gemini Chats", "_Templates"];

// arm-C-lite vocabulary: 0..=255 literal bytes, then one NODE_OPEN id per NodeKind
// (256 + kind index, 13 kinds → 256..=268), then NODE_CLOSE (269). 270 total.
const LITE_OPEN_BASE: u16 = 256;
const LITE_NODE_CLOSE: u16 = 269;
const LITE_VOCAB: u32 = 270;

fn kind_index(k: NodeKind) -> u16 {
    match k {
        NodeKind::Section => 0,
        NodeKind::Paragraph => 1,
        NodeKind::Sentence => 2,
        NodeKind::Clause => 3,
        NodeKind::Quote => 4,
        NodeKind::Reference => 5,
        NodeKind::Term => 6,
        NodeKind::Character => 7,
        NodeKind::Place => 8,
        NodeKind::Concept => 9,
        NodeKind::Event => 10,
        NodeKind::Object => 11,
        NodeKind::Group => 12,
    }
}

/// Arm-C-lite encoder: document bytes + structural boundary/kind markers, leak-free.
/// Returns `(ids, is_body)` of equal length; `is_body[i] == 1` for a document body
/// byte (id < 256), `0` for a `NODE_OPEN<kind>` / `NODE_CLOSE` marker (id >= 256).
fn encode_arm_c_lite(text: &str, graph: &Graph) -> (Vec<u16>, Vec<u8>) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut opens_at: Vec<Vec<NodeKind>> = vec![Vec::new(); len + 1];
    let mut closes_at: Vec<usize> = vec![0; len + 1];
    for node in &graph.nodes {
        if let Some(span) = node.span {
            let s = span.start.min(len);
            let e = span.end.min(len).max(s);
            opens_at[s].push(node.kind);
            closes_at[e] += 1;
        }
    }
    let mut ids: Vec<u16> = Vec::new();
    let mut is_body: Vec<u8> = Vec::new();
    for i in 0..=len {
        for _ in 0..closes_at[i] {
            ids.push(LITE_NODE_CLOSE);
            is_body.push(0);
        }
        for &k in &opens_at[i] {
            ids.push(LITE_OPEN_BASE + kind_index(k));
            is_body.push(0);
        }
        if i < len {
            ids.push(bytes[i] as u16);
            is_body.push(1);
        }
    }
    (ids, is_body)
}

#[derive(Serialize)]
struct DocRecord {
    path: String,
    bytes: usize,
    c_tokens: usize,
}

#[derive(Serialize)]
struct Manifest {
    /// Arm-C-lite vocabulary size (256 bytes + 13 NODE_OPEN kinds + NODE_CLOSE).
    vocab_size_arm_c: u32,
    doc_count: usize,
    /// Total source-text bytes == `corpus.txt` length == arm-C body-token count.
    total_bytes: usize,
    total_c_tokens: usize,
    /// Deterministic split: the last `val_doc_count` docs (in `docs` order) are val.
    val_doc_count: usize,
    docs: Vec<DocRecord>,
}

/// Curation predicate (ADR-00015 §4): a path is authored prose iff it is a `.md`
/// file under no excluded top-level folder, with no dot-component anywhere (Obsidian
/// internals). `rel` is relative to the corpus root.
fn is_authored_prose(rel: &Path, excluded: &[&str]) -> bool {
    if !rel.extension().is_some_and(|e| e == "md") {
        return false;
    }
    if rel
        .components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    {
        return false;
    }
    if let Some(first) = rel.components().next() {
        let top = first.as_os_str().to_string_lossy();
        if excluded.iter().any(|e| *e == top) {
            return false;
        }
    }
    true
}

/// Recursively collect every `.md` path under `dir`, skipping dot-directories.
fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if ft.is_dir() {
            if name.starts_with('.') {
                continue;
            }
            collect_md(&path, out)?;
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(corpus_dir) = args.next() else {
        eprintln!("usage: corpus-export <corpus_dir> [out_dir]");
        std::process::exit(2);
    };
    let out_dir = args.next().unwrap_or_else(|| "data".to_string());
    let corpus_dir = PathBuf::from(corpus_dir);
    let out_dir = PathBuf::from(out_dir);
    fs::create_dir_all(&out_dir)?;

    let mut all_md = Vec::new();
    collect_md(&corpus_dir, &mut all_md)?;
    all_md.sort(); // deterministic order

    let mut corpus_txt = String::new();
    let mut arm_c: Vec<u8> = Vec::new();
    let mut arm_c_body: Vec<u8> = Vec::new();
    let mut docs: Vec<DocRecord> = Vec::new();
    let mut total_c_tokens = 0usize;

    for path in &all_md {
        let rel = path.strip_prefix(&corpus_dir).unwrap_or(path);
        if !is_authored_prose(rel, DEFAULT_EXCLUDED_DIRS) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue; // skip non-UTF-8 / unreadable
        };
        let (ids, body) = encode_arm_c_lite(&text, &walk(&text));
        for &id in &ids {
            arm_c.extend_from_slice(&id.to_le_bytes());
        }
        arm_c_body.extend_from_slice(&body);
        corpus_txt.push_str(&text);
        total_c_tokens += ids.len();
        docs.push(DocRecord {
            path: rel.to_string_lossy().replace('\\', "/"),
            bytes: text.len(),
            c_tokens: ids.len(),
        });
    }

    let val_doc_count = if docs.is_empty() { 0 } else { (docs.len() / 10).max(1) };
    let manifest = Manifest {
        vocab_size_arm_c: LITE_VOCAB,
        doc_count: docs.len(),
        total_bytes: corpus_txt.len(),
        total_c_tokens,
        val_doc_count,
        docs,
    };

    fs::write(out_dir.join("arm_c.u16"), &arm_c)?;
    fs::write(out_dir.join("arm_c.body"), &arm_c_body)?;
    fs::write(out_dir.join("corpus.txt"), corpus_txt.as_bytes())?;
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
    )?;

    println!(
        "exported {} docs · {} text bytes · {} arm-C-lite tokens (vocab {}) → {}",
        manifest.doc_count,
        manifest.total_bytes,
        manifest.total_c_tokens,
        LITE_VOCAB,
        out_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curation_keeps_authored_prose_and_drops_the_rest() {
        let ex = DEFAULT_EXCLUDED_DIRS;
        assert!(is_authored_prose(Path::new("_Books/ch01.md"), ex));
        assert!(is_authored_prose(Path::new("_Short Stories/tale.md"), ex));
        assert!(is_authored_prose(Path::new("Campaigns/session-3.md"), ex));
        assert!(is_authored_prose(Path::new("_Poetry/verse.md"), ex));
        assert!(!is_authored_prose(Path::new("_Gemini Chats/log.md"), ex));
        assert!(!is_authored_prose(Path::new("_Templates/daily.md"), ex));
        assert!(!is_authored_prose(Path::new(".obsidian/plugin.md"), ex));
        assert!(!is_authored_prose(Path::new(".trash/deleted.md"), ex));
        assert!(!is_authored_prose(Path::new("_Books/cover.png"), ex));
        assert!(!is_authored_prose(Path::new("Untitled.base"), ex));
    }

    #[test]
    fn arm_c_lite_body_recovers_source_and_leaks_no_text() {
        // The body positions, read as bytes, must reproduce the source EXACTLY, and
        // every source byte appears exactly once (no label/text duplication = no leak).
        let doc = "# Title\n\nMara met Vane by the river. \"Hello,\" she said.\n\nThen on.\n";
        let (ids, body) = encode_arm_c_lite(doc, &walk(doc));
        assert_eq!(ids.len(), body.len());
        let recovered: Vec<u8> = ids
            .iter()
            .zip(&body)
            .filter(|(_, b)| **b == 1)
            .map(|(id, _)| *id as u8)
            .collect();
        assert_eq!(recovered, doc.as_bytes(), "body positions must reproduce the source");
        assert_eq!(
            body.iter().filter(|b| **b == 1).count(),
            doc.len(),
            "each source byte appears exactly once — no duplication/leak"
        );
    }

    #[test]
    fn arm_c_lite_markers_are_nonbody_and_ids_in_vocab() {
        let doc = "Some text with Ünïcödé, punctuation, and a quote: \"yes.\"\n\nA second paragraph.";
        let (ids, body) = encode_arm_c_lite(doc, &walk(doc));
        for (id, b) in ids.iter().zip(&body) {
            assert!((u32::from(*id)) < LITE_VOCAB, "id in arm-C-lite vocab");
            if *b == 0 {
                assert!(*id >= LITE_OPEN_BASE, "markers are >= 256");
            } else {
                assert!(*id < LITE_OPEN_BASE, "body bytes are < 256");
            }
        }
    }
}
