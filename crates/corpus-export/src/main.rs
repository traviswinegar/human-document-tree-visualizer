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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use doctree_core::{walk, EdgeKind, Graph, NodeKind};
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

// arm-D (anonymized coref) vocabulary: 0..=255 literal bytes, then up to MAX_COREF_SLOTS
// anonymized entity-mention markers (`COREF_BASE + slot`). This is Phase 11 (a) /
// ADR-00019: the non-leaky semantic-graph CONDITIONING test. Each distinct salient
// `Term` (the deterministic walker's coreference proxy) is assigned a stable, per-doc
// anonymized slot; a `COREF_BASE + slot` marker is emitted right before each occurrence
// of that term. The markers carry the entity's *recurring identity* (slot k here is the
// same entity as slot k 200 bytes ago) and NOTHING of its surface text — the word that
// follows is a normal body byte run, scored as usual. So the model can attend to the
// coref skeleton while bits-per-byte is measured over the document body only (the
// `is_body == 1` positions), exactly like arm-C-lite — but unlike arm-C-lite's kind-only
// boundaries (redundant with punctuation), the anonymized *identity* is non-redundant
// signal. Per-doc slots reset per document, so no global term→bytes mapping can leak
// across the train/val split (slot k in a val doc is a different word than slot k in a
// train doc). 64 slots keeps the embedding table within a hair of byte's 256.
const COREF_BASE: u16 = 256;
const MAX_COREF_SLOTS: u16 = 64;
const ARM_D_VOCAB: u32 = COREF_BASE as u32 + MAX_COREF_SLOTS as u32; // 320

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

/// Assign each salient `Term` node a stable, per-document anonymized coref slot.
///
/// The universe is the graph's `Term` nodes (already filtered by the walker's
/// `min_term_freq` + `max_terms`); slots are handed out by **mention frequency
/// descending, then name ascending** (deterministic), capped at [`MAX_COREF_SLOTS`] so
/// the most-recurring entities are the ones the model gets to track. Frequency comes
/// from the `Mentions` edges (sentence → `term:{name}`). Returns `name → slot`.
fn coref_slots(graph: &Graph) -> BTreeMap<String, u16> {
    // Every Term node's name (its label == the lowercased term the walker tokenized).
    let mut freq: BTreeMap<String, usize> = BTreeMap::new();
    for node in &graph.nodes {
        if node.kind == NodeKind::Term {
            freq.entry(node.label.clone()).or_insert(0);
        }
    }
    // Mention count per term (one Mentions edge per sentence that mentions it).
    for edge in &graph.edges {
        if edge.kind == EdgeKind::Mentions {
            if let Some(name) = edge.target.strip_prefix("term:") {
                if let Some(c) = freq.get_mut(name) {
                    *c += 1;
                }
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = freq.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(MAX_COREF_SLOTS as usize);
    ranked
        .into_iter()
        .enumerate()
        .map(|(i, (name, _))| (name, i as u16))
        .collect()
}

/// Per-byte-offset coref markers: `at[off]` lists the anonymized marker id(s) for any
/// salient term whose occurrence *starts* at byte `off`. Replicates the walker's word
/// split (runs of alphanumeric/`'`, trimmed of `'`, lowercased) for offset alignment;
/// the stopword/length/digit filters are applied implicitly by membership in
/// `name_to_slot` (only salient terms are keys), so they are not duplicated here.
fn coref_markers_at(text: &str, name_to_slot: &BTreeMap<String, u16>, len: usize) -> Vec<Vec<u16>> {
    let mut at: Vec<Vec<u16>> = vec![Vec::new(); len + 1];
    let mut cur = String::new();
    let mut start = 0usize;
    let mut in_tok = false;
    let flush = |cur: &mut String, start: usize, at: &mut Vec<Vec<u16>>| {
        if cur.is_empty() {
            return;
        }
        let token = cur.trim_matches('\'').to_lowercase();
        cur.clear();
        if let Some(&slot) = name_to_slot.get(&token) {
            at[start].push(COREF_BASE + slot);
        }
    };
    for (off, ch) in text.char_indices() {
        if ch.is_alphanumeric() || ch == '\'' {
            if !in_tok {
                start = off;
                in_tok = true;
            }
            cur.push(ch);
        } else {
            flush(&mut cur, start, &mut at);
            in_tok = false;
        }
    }
    flush(&mut cur, start, &mut at);
    at
}

/// Arm-D (anonymized coref) encoder: document bytes with an anonymized coref marker
/// emitted immediately before each salient-term occurrence. Returns `(ids, is_body)` of
/// equal length; `is_body[i] == 1` for a document body byte (scored for BPB), `0` for a
/// `COREF_BASE + slot` marker (conditioning context, never scored — no surface text, so
/// no leak). Body positions reproduce the source exactly, each byte once.
fn encode_arm_d_coref(text: &str, graph: &Graph) -> (Vec<u16>, Vec<u8>) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let slots = coref_slots(graph);
    let at = coref_markers_at(text, &slots, len);
    let mut ids: Vec<u16> = Vec::new();
    let mut is_body: Vec<u8> = Vec::new();
    for i in 0..=len {
        for &m in &at[i] {
            ids.push(m);
            is_body.push(0);
        }
        if i < len {
            ids.push(bytes[i] as u16);
            is_body.push(1);
        }
    }
    (ids, is_body)
}

/// The salient-term graph of a document — the **ground truth** for the (b) downstream
/// text→graph task (Phase 11 / ADR-00019). `nodes` = salient `Term` labels; `edges` =
/// unordered `CoOccursWith` pairs (term↔term). Bounded + deterministic, so a tiny model
/// has a tractable target; serialized compact, one line per doc, matching the Python
/// `graphmatch.parse_term_graph` shape `{"nodes":[str], "edges":[[a,b]]}`.
#[derive(Serialize)]
struct TermGraph {
    nodes: Vec<String>,
    edges: Vec<[String; 2]>,
}

fn term_graph_of(graph: &Graph) -> TermGraph {
    let mut nodes: Vec<String> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Term)
        .map(|n| n.label.clone())
        .collect();
    nodes.sort();
    nodes.dedup();
    let mut edges: Vec<[String; 2]> = Vec::new();
    for e in &graph.edges {
        if e.kind == EdgeKind::CoOccursWith {
            let a = e.source.strip_prefix("term:").unwrap_or(&e.source);
            let b = e.target.strip_prefix("term:").unwrap_or(&e.target);
            if a != b {
                let (x, y) = if a <= b { (a, b) } else { (b, a) };
                edges.push([x.to_string(), y.to_string()]);
            }
        }
    }
    edges.sort();
    edges.dedup();
    TermGraph { nodes, edges }
}

#[derive(Serialize)]
struct DocRecord {
    path: String,
    bytes: usize,
    c_tokens: usize,
    d_tokens: usize,
}

#[derive(Serialize)]
struct Manifest {
    /// Arm-C-lite vocabulary size (256 bytes + 13 NODE_OPEN kinds + NODE_CLOSE).
    vocab_size_arm_c: u32,
    /// Arm-D (anonymized coref) vocabulary size (256 bytes + 64 coref-slot markers).
    vocab_size_arm_d: u32,
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
    let mut arm_d: Vec<u8> = Vec::new();
    let mut arm_d_body: Vec<u8> = Vec::new();
    let mut term_graphs = String::new();
    let mut docs: Vec<DocRecord> = Vec::new();
    let mut total_c_tokens = 0usize;
    let mut total_d_tokens = 0usize;

    for path in &all_md {
        let rel = path.strip_prefix(&corpus_dir).unwrap_or(path);
        if !is_authored_prose(rel, DEFAULT_EXCLUDED_DIRS) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue; // skip non-UTF-8 / unreadable
        };
        // One walk feeds both arms (arm-C-lite boundaries + arm-D anonymized coref).
        let graph = walk(&text);
        let (ids, body) = encode_arm_c_lite(&text, &graph);
        for &id in &ids {
            arm_c.extend_from_slice(&id.to_le_bytes());
        }
        arm_c_body.extend_from_slice(&body);
        let (d_ids, d_body) = encode_arm_d_coref(&text, &graph);
        for &id in &d_ids {
            arm_d.extend_from_slice(&id.to_le_bytes());
        }
        arm_d_body.extend_from_slice(&d_body);
        // (b) downstream ground truth: this doc's salient-term graph, one JSON line.
        term_graphs.push_str(&serde_json::to_string(&term_graph_of(&graph)).expect("term graph"));
        term_graphs.push('\n');
        corpus_txt.push_str(&text);
        total_c_tokens += ids.len();
        total_d_tokens += d_ids.len();
        docs.push(DocRecord {
            path: rel.to_string_lossy().replace('\\', "/"),
            bytes: text.len(),
            c_tokens: ids.len(),
            d_tokens: d_ids.len(),
        });
    }

    let val_doc_count = if docs.is_empty() { 0 } else { (docs.len() / 10).max(1) };
    let manifest = Manifest {
        vocab_size_arm_c: LITE_VOCAB,
        vocab_size_arm_d: ARM_D_VOCAB,
        doc_count: docs.len(),
        total_bytes: corpus_txt.len(),
        total_c_tokens,
        val_doc_count,
        docs,
    };

    fs::write(out_dir.join("arm_c.u16"), &arm_c)?;
    fs::write(out_dir.join("arm_c.body"), &arm_c_body)?;
    fs::write(out_dir.join("arm_d.u16"), &arm_d)?;
    fs::write(out_dir.join("arm_d.body"), &arm_d_body)?;
    fs::write(out_dir.join("term_graphs.jsonl"), term_graphs.as_bytes())?;
    fs::write(out_dir.join("corpus.txt"), corpus_txt.as_bytes())?;
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
    )?;

    println!(
        "exported {} docs · {} text bytes · {} arm-C-lite tokens (vocab {}) · \
         {} arm-D coref tokens (vocab {}) → {}",
        manifest.doc_count,
        manifest.total_bytes,
        manifest.total_c_tokens,
        LITE_VOCAB,
        total_d_tokens,
        ARM_D_VOCAB,
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

    // ---- arm D (anonymized coref) — Phase 11 (a) / ADR-00019 -----------------

    #[test]
    fn arm_d_body_recovers_source_and_leaks_no_text() {
        // The masking invariant: body positions reproduce the source EXACTLY, every
        // source byte appears exactly once, and the coref markers carry no surface bytes.
        let doc = "The dragon flew over the keep. The dragon roared. Then the dragon slept.\n";
        let graph = walk(doc);
        let (ids, body) = encode_arm_d_coref(doc, &graph);
        assert_eq!(ids.len(), body.len());
        let recovered: Vec<u8> = ids
            .iter()
            .zip(&body)
            .filter(|(_, b)| **b == 1)
            .map(|(id, _)| *id as u8)
            .collect();
        assert_eq!(recovered, doc.as_bytes(), "body positions reproduce the source");
        assert_eq!(
            body.iter().filter(|b| **b == 1).count(),
            doc.len(),
            "each source byte appears exactly once — no duplication/leak"
        );
        // No-leak, stated as the dual: reading ONLY the marker (non-body) tokens yields
        // no source byte (every marker id is >= 256, an anonymized slot, not a byte).
        assert!(
            ids.iter()
                .zip(&body)
                .filter(|(_, b)| **b == 0)
                .all(|(id, _)| *id >= COREF_BASE),
            "marker positions contain no literal source byte"
        );
    }

    #[test]
    fn arm_d_markers_are_anonymized_coref_slots_in_vocab() {
        let doc = "The dragon flew. The wizard watched the dragon. The wizard fled. The dragon roared.";
        let graph = walk(doc);
        let (ids, body) = encode_arm_d_coref(doc, &graph);
        for (id, b) in ids.iter().zip(&body) {
            assert!((u32::from(*id)) < ARM_D_VOCAB, "id in arm-D vocab");
            if *b == 0 {
                assert!(
                    *id >= COREF_BASE && *id < COREF_BASE + MAX_COREF_SLOTS,
                    "markers are anonymized coref slots in [256, 256+64)"
                );
            } else {
                assert!(*id < COREF_BASE, "body bytes are < 256");
            }
        }
    }

    #[test]
    fn arm_d_encodes_coref_recurrence_distinct_per_entity() {
        // The non-redundant signal arm-C-lite lacked: a recurring entity is marked by the
        // SAME anonymized slot at each occurrence (coreference), and distinct entities get
        // distinct slots — all without ever emitting the surface word.
        let doc = "The dragon flew. The wizard watched the dragon. The wizard fled. The dragon roared.";
        let graph = walk(doc);
        let slots = coref_slots(&graph);
        let dragon = COREF_BASE + *slots.get("dragon").expect("dragon is a salient term");
        let wizard = COREF_BASE + *slots.get("wizard").expect("wizard is a salient term");
        assert_ne!(dragon, wizard, "distinct entities get distinct anonymized slots");

        let (ids, body) = encode_arm_d_coref(doc, &graph);
        let count = |m: u16| {
            ids.iter()
                .zip(&body)
                .filter(|(id, b)| **b == 0 && **id == m)
                .count()
        };
        assert!(
            count(dragon) >= 2,
            "the recurring entity's SAME slot marks each occurrence (coref recurrence)"
        );
        assert!(count(wizard) >= 2, "the second entity's coref recurrence is also encoded");
    }

    #[test]
    fn term_graph_has_nodes_and_cooccurrence_edges() {
        // Two terms that recur together produce term nodes + an unordered co-occ edge,
        // labels only (no "term:" prefix), matching the Python scorer's target shape.
        let doc = "The dragon guards the gold. The dragon hoards the gold. The dragon counts the gold.";
        let tg = term_graph_of(&walk(doc));
        assert!(tg.nodes.contains(&"dragon".to_string()));
        assert!(tg.nodes.contains(&"gold".to_string()));
        assert!(tg.nodes.iter().all(|n| !n.starts_with("term:")), "labels, not ids");
        assert!(
            tg.edges.iter().any(|[a, b]| {
                (a == "dragon" && b == "gold") || (a == "gold" && b == "dragon")
            }),
            "dragon and gold co-occur, so there is an undirected edge: {:?}",
            tg.edges
        );
        // edges are canonical (sorted within the pair: a <= b)
        assert!(tg.edges.iter().all(|[a, b]| a <= b), "each pair is sorted");
    }

    #[test]
    fn arm_d_slot_count_is_capped() {
        // Anonymized slots never exceed the bounded vocabulary, whatever the doc.
        let doc = "alpha bravo charlie delta echo foxtrot golf hotel india juliet. \
                   alpha bravo charlie delta echo foxtrot golf hotel india juliet again.";
        let slots = coref_slots(&walk(doc));
        assert!(slots.values().all(|&s| s < MAX_COREF_SLOTS), "every slot is within the cap");
    }
}
