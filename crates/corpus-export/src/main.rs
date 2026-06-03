//! corpus-export — Phase 8 / [ADR-00015] M1.
//!
//! Walk a corpus directory of `.md` files (curated to authored prose) and export
//! the source data for the three tokenizer-bench arms:
//!
//! * **arm C (byte+graph)** → `arm_c.u16`: `doctree_core::tokenizer::encode(doc,
//!   walk(doc))` token ids, little-endian `u16` (the vocabulary is 296 < 65536), the
//!   documents concatenated in manifest order.
//! * **arms A (BPE) & B (byte)** → `corpus.txt`: the *exact* concatenated source text
//!   (no separators, so its byte count matches the arm-C bodies). The Python side
//!   trains a BPE over it (A) or reads its bytes (B).
//! * `manifest.json`: per-doc `{path, bytes, c_tokens}` in order + totals + the
//!   arm-C vocab size + a deterministic doc-based train/val split, so the trainer
//!   reconstructs document boundaries in either stream.
//!
//! Native-free (std + doctree-core). Graphs come from the **deterministic** walker
//! (reproducible), never the LLM layer. Curation excludes the AI chat logs,
//! templates, and Obsidian internals (ADR-00015 §4).
//!
//! ```text
//! cargo run -p corpus-export --release -- "C:\Writing Vault" research/tokenizer-bench/data
//! ```
//!
//! [ADR-00015]: ../../docs/adr/ADR-00015-graph-tokenizer-training-experiment.md

use std::fs;
use std::path::{Path, PathBuf};

use doctree_core::{encode, walk, VOCAB_SIZE};
use serde::Serialize;

/// Top-level vault folders excluded from the authored-prose corpus (ADR-00015 §4):
/// AI chat logs (not authored prose; would confound the comparison) and templates
/// (boilerplate). Obsidian internals are dropped separately via the dot-dir rule.
const DEFAULT_EXCLUDED_DIRS: &[&str] = &["_Gemini Chats", "_Templates"];

#[derive(Serialize)]
struct DocRecord {
    path: String,
    bytes: usize,
    c_tokens: usize,
}

#[derive(Serialize)]
struct Manifest {
    /// Arm-C vocabulary size (the fixed `doctree-core` tokenizer vocab).
    vocab_size_arm_c: u32,
    doc_count: usize,
    /// Total source-text bytes == `corpus.txt` length == sum of arm-C document bodies.
    total_bytes: usize,
    total_c_tokens: usize,
    /// Deterministic split: the last `val_doc_count` docs (in `docs` order) are val.
    val_doc_count: usize,
    docs: Vec<DocRecord>,
}

/// Arm C: the graph-tokenizer stream for one document, as token ids.
fn encode_doc_ids(text: &str) -> Vec<u32> {
    encode(text, &walk(text)).ids().to_vec()
}

/// Curation predicate (ADR-00015 §4): a path counts as authored prose iff it is a
/// `.md` file, lives under no excluded top-level folder, and has no dot-component
/// anywhere (Obsidian internals like `.obsidian` / `.trash`). `rel` is relative to
/// the corpus root.
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

/// Recursively collect every `.md` path under `dir`, skipping dot-directories
/// (Obsidian internals) for speed. The authored-prose decision is made later by
/// [`is_authored_prose`] on the path relative to the corpus root.
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
        let ids = encode_doc_ids(&text);
        for &id in &ids {
            debug_assert!(id < VOCAB_SIZE, "encode emitted an out-of-vocab id");
            arm_c.extend_from_slice(&(id as u16).to_le_bytes());
        }
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
        vocab_size_arm_c: VOCAB_SIZE,
        doc_count: docs.len(),
        total_bytes: corpus_txt.len(),
        total_c_tokens,
        val_doc_count,
        docs,
    };

    fs::write(out_dir.join("arm_c.u16"), &arm_c)?;
    fs::write(out_dir.join("corpus.txt"), corpus_txt.as_bytes())?;
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
    )?;

    println!(
        "exported {} docs · {} text bytes · {} arm-C tokens (vocab {}) → {}",
        manifest.doc_count,
        manifest.total_bytes,
        manifest.total_c_tokens,
        VOCAB_SIZE,
        out_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use doctree_core::{decode_text, Tokens};

    #[test]
    fn curation_keeps_authored_prose_and_drops_the_rest() {
        let ex = DEFAULT_EXCLUDED_DIRS;
        // authored prose — kept
        assert!(is_authored_prose(Path::new("_Books/ch01.md"), ex));
        assert!(is_authored_prose(Path::new("_Short Stories/tale.md"), ex));
        assert!(is_authored_prose(Path::new("Campaigns/session-3.md"), ex));
        assert!(is_authored_prose(Path::new("_Poetry/verse.md"), ex));
        // excluded top-level folders — dropped
        assert!(!is_authored_prose(Path::new("_Gemini Chats/log.md"), ex));
        assert!(!is_authored_prose(Path::new("_Templates/daily.md"), ex));
        // Obsidian internals (dot-dirs anywhere) — dropped
        assert!(!is_authored_prose(Path::new(".obsidian/plugin.md"), ex));
        assert!(!is_authored_prose(Path::new(".trash/deleted.md"), ex));
        // non-markdown — dropped
        assert!(!is_authored_prose(Path::new("_Books/cover.png"), ex));
        assert!(!is_authored_prose(Path::new("Untitled.base"), ex));
    }

    #[test]
    fn arm_c_stream_round_trips_to_source_bytes() {
        // ADR-00015 pinned invariant: the exported arm-C ids reproduce the source
        // byte-exact (ADR-00013's guarantee, exercised on the exporter path).
        let doc = "# Title\n\nMara met Vane by the river. \"Hello,\" she said.\n\nThen they walked on.\n";
        let ids = encode_doc_ids(doc);
        let recovered = decode_text(&Tokens(ids)).expect("arm-C stream decodes");
        assert_eq!(recovered, doc, "arm-C stream must reproduce the source byte-exact");
    }

    #[test]
    fn arm_c_ids_pack_losslessly_into_u16() {
        let doc = "Some text with Ünïcödé, punctuation, and a quote: \"yes.\"\n\nA second paragraph.";
        for id in encode_doc_ids(doc) {
            assert!(id < VOCAB_SIZE, "every id is in-vocab (< {VOCAB_SIZE})");
            assert_eq!(id as u16 as u32, id, "id packs losslessly into u16");
        }
    }
}
