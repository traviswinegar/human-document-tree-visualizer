//! The deterministic structure walker — Phase 2 / the graph **spine**.
//!
//! Given a document string it produces a [`Graph`] of structural nodes/edges
//! with **zero** randomness or model involvement: the same input always yields
//! the same output (pinned by [`tests::walk_is_deterministic`]). The pipeline is:
//!
//! 1. **segment** into sections (headings) → paragraphs (blank-line runs) →
//!    sentences (terminator scan with an abbreviation guard);
//! 2. **clause-split** each sentence on `;`, comma+coordinator, and a small set
//!    of subordinators (only when every resulting clause clears a word floor);
//! 3. **detect** quoted spans and citation/reference markers within sentences;
//! 4. **terms**: tokenize → stopword/length filter → frequency; salient repeated
//!    terms become `Term` nodes with `mentions` and weighted `co_occurs_with`
//!    edges.
//!
//! Spans are **byte offsets** into the source (UTF-8; equal to char offsets for
//! ASCII). Determinism is enforced by emitting structural nodes in document
//! order and term/co-occurrence output in sorted order (no hash-iteration order
//! leaks into the result).

use std::collections::BTreeMap;

use crate::schema::{Edge, EdgeKind, Graph, Node, NodeKind, Provenance, Span};

/// Tunables for the walk. Defaults are conservative; the live build / Tauri
/// layer can override.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// A term must occur at least this many times to become a `Term` node.
    pub min_term_freq: usize,
    /// Minimum word count for a clause; shorter fragments merge into a neighbor.
    pub min_clause_words: usize,
    /// Cap on the number of `Term` nodes (kept by frequency desc, then name asc).
    pub max_terms: usize,
}

impl Default for WalkOptions {
    fn default() -> Self {
        WalkOptions {
            min_term_freq: 2,
            min_clause_words: 2,
            max_terms: 100,
        }
    }
}

/// Walk a document into its deterministic spine graph using [`WalkOptions::default`].
pub fn walk(doc: &str) -> Graph {
    walk_with(doc, &WalkOptions::default())
}

/// Walk a document into its deterministic spine graph.
pub fn walk_with(doc: &str, opts: &WalkOptions) -> Graph {
    let mut g = Graph::new();

    let mut section_n = 0usize;
    let mut paragraph_n = 0usize;
    let mut sentence_n = 0usize;
    let mut quote_n = 0usize;
    let mut ref_n = 0usize;

    // current section context (every paragraph is part_of a section)
    let mut current_section: Option<String> = None;
    let mut prev_sentence_id: Option<String> = None;

    // term frequency (global) and per-sentence salient-term membership
    let mut term_freq: BTreeMap<String, usize> = BTreeMap::new();
    // (sentence_id, sorted unique tokens in that sentence)
    let mut sentence_tokens: Vec<(String, Vec<String>)> = Vec::new();

    for block in blocks(doc) {
        match block {
            Block::Heading(span) => {
                section_n += 1;
                let id = format!("sec:{section_n}");
                let text = slice(doc, span);
                let label = heading_label(text);
                g.push_node(
                    Node::structural(id.clone(), NodeKind::Section, label)
                        .with_text(text)
                        .with_span(span),
                );
                current_section = Some(id);
            }
            Block::Paragraph(span) => {
                // Ensure an implicit section exists if the doc had no headings.
                if current_section.is_none() {
                    section_n += 1;
                    let id = format!("sec:{section_n}");
                    g.push_node(Node::structural(id.clone(), NodeKind::Section, "Document"));
                    current_section = Some(id);
                }
                let section_id = current_section.clone().unwrap();

                paragraph_n += 1;
                let para_id = format!("para:{paragraph_n}");
                g.push_node(
                    Node::structural(para_id.clone(), NodeKind::Paragraph, format!("Paragraph {paragraph_n}"))
                        .with_span(span),
                );
                g.push_edge(Edge::new(para_id.clone(), section_id, EdgeKind::PartOf, Provenance::Structural));

                let para_text = slice(doc, span);
                for sent_span in sentence_ranges(para_text, span.start) {
                    sentence_n += 1;
                    let sent_id = format!("sent:{sentence_n}");
                    let sent_text = slice(doc, sent_span);
                    g.push_node(
                        Node::structural(sent_id.clone(), NodeKind::Sentence, sent_text)
                            .with_text(sent_text)
                            .with_span(sent_span),
                    );
                    g.push_edge(Edge::new(sent_id.clone(), para_id.clone(), EdgeKind::PartOf, Provenance::Structural));

                    if let Some(prev) = &prev_sentence_id {
                        g.push_edge(Edge::new(prev.clone(), sent_id.clone(), EdgeKind::Precedes, Provenance::Structural));
                    }
                    prev_sentence_id = Some(sent_id.clone());

                    // clauses
                    let clauses = clause_ranges(sent_text, sent_span.start, opts);
                    for (i, c) in clauses.iter().enumerate() {
                        let cid = format!("clause:{sentence_n}.{}", i + 1);
                        let ctext = slice(doc, *c);
                        g.push_node(
                            Node::structural(cid.clone(), NodeKind::Clause, ctext)
                                .with_text(ctext)
                                .with_span(*c),
                        );
                        g.push_edge(Edge::new(cid, sent_id.clone(), EdgeKind::PartOf, Provenance::Structural));
                    }

                    // quotes
                    for q in quote_ranges(sent_text, sent_span.start) {
                        quote_n += 1;
                        let qid = format!("quote:{quote_n}");
                        let qtext = slice(doc, q);
                        g.push_node(
                            Node::structural(qid.clone(), NodeKind::Quote, qtext)
                                .with_text(qtext)
                                .with_span(q),
                        );
                        g.push_edge(Edge::new(qid, sent_id.clone(), EdgeKind::PartOf, Provenance::Structural));
                    }

                    // references / citations
                    for r in reference_ranges(sent_text, sent_span.start) {
                        ref_n += 1;
                        let rid = format!("ref:{ref_n}");
                        let rtext = slice(doc, r);
                        g.push_node(
                            Node::structural(rid.clone(), NodeKind::Reference, rtext)
                                .with_text(rtext)
                                .with_span(r),
                        );
                        g.push_edge(Edge::new(rid, sent_id.clone(), EdgeKind::PartOf, Provenance::Structural));
                    }

                    // terms in this sentence
                    let mut toks: Vec<String> = tokenize_terms(sent_text);
                    for t in &toks {
                        *term_freq.entry(t.clone()).or_insert(0) += 1;
                    }
                    toks.sort();
                    toks.dedup();
                    sentence_tokens.push((sent_id, toks));
                }
            }
        }
    }

    emit_terms(&mut g, &term_freq, &sentence_tokens, opts);
    g
}

// --------------------------------------------------------------------------
// Blocks: headings vs paragraphs, in document order, with byte spans.
// --------------------------------------------------------------------------

enum Block {
    Heading(Span),
    Paragraph(Span),
}

/// Split a document into heading/paragraph blocks. Paragraphs are runs of
/// non-blank lines separated by blank-line runs; a block that is a single
/// heading line becomes [`Block::Heading`].
fn blocks(doc: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut line_start = 0usize;
    let mut para_start: Option<usize> = None;
    let mut para_end = 0usize;

    let flush = |out: &mut Vec<Block>, start: Option<usize>, end: usize| {
        if let Some(s) = start {
            if end > s {
                let span = Span::new(s, end);
                let text = &doc[s..end];
                if is_single_heading_line(text) {
                    out.push(Block::Heading(span));
                } else {
                    out.push(Block::Paragraph(span));
                }
            }
        }
    };

    for line in doc.split_inclusive('\n') {
        let len = line.len();
        let content_end = line_start + trimmed_end_len(line);
        let is_blank = line.trim().is_empty();
        if is_blank {
            flush(&mut out, para_start, para_end);
            para_start = None;
        } else {
            if para_start.is_none() {
                para_start = Some(line_start + leading_ws_len(line));
            }
            para_end = content_end;
        }
        line_start += len;
    }
    flush(&mut out, para_start, para_end);
    out
}

/// Is this block a single heading line? (`#` markdown heading, or a
/// chapter/part/book/section line.)
fn is_single_heading_line(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.contains('\n') {
        return false;
    }
    if t.starts_with('#') {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    for kw in ["chapter ", "part ", "book ", "section "] {
        if lower.starts_with(kw) {
            // followed by a roman numeral / digit / word, short line
            return t.len() <= 40;
        }
    }
    false
}

fn heading_label(text: &str) -> String {
    text.trim().trim_start_matches('#').trim().to_string()
}

// --------------------------------------------------------------------------
// Sentence segmentation
// --------------------------------------------------------------------------

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "fig", "no", "vol", "al",
];

/// Sentence byte spans within `text`, offset by `base` to absolute document
/// positions.
fn sentence_ranges(text: &str, base: usize) -> Vec<Span> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut spans = Vec::new();
    let mut start = next_non_space(text, 0);

    while start < n {
        let mut j = start;
        let mut end = n;
        while j < n {
            let c = bytes[j];
            if c == b'.' || c == b'!' || c == b'?' {
                // consume a run of terminators (handles "?!", "...")
                let mut k = j + 1;
                while k < n && matches!(bytes[k], b'.' | b'!' | b'?') {
                    k += 1;
                }
                // single '.' immediately after a known abbreviation: not a boundary
                if c == b'.' && k == j + 1 && is_abbrev_before(text, j) {
                    j = k;
                    continue;
                }
                // terminator must be followed by whitespace/EOL to be a boundary
                // (guards decimals like "3.14" and "U.S.A.")
                if k < n && !matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
                    j = k;
                    continue;
                }
                end = k;
                break;
            }
            j += 1;
        }
        let trimmed = trim_end_idx(text, start, end);
        if trimmed > start {
            spans.push(Span::new(base + start, base + trimmed));
        }
        start = next_non_space(text, end);
    }
    spans
}

fn is_abbrev_before(text: &str, dot: usize) -> bool {
    let bytes = text.as_bytes();
    let mut i = dot;
    while i > 0 {
        let c = bytes[i - 1];
        if c.is_ascii_alphabetic() {
            i -= 1;
        } else {
            break;
        }
    }
    if i == dot {
        return false;
    }
    let word = text[i..dot].to_ascii_lowercase();
    ABBREVIATIONS.contains(&word.as_str())
}

// --------------------------------------------------------------------------
// Clause splitting
// --------------------------------------------------------------------------

/// Coordinators that follow a comma (cut at the comma).
const COMMA_COORDINATORS: &[&str] = &[", and ", ", but ", ", or ", ", nor ", ", yet ", ", so "];
/// Subordinators (cut before the leading space).
const SUBORDINATORS: &[&str] = &[
    " because ",
    " although ",
    " though ",
    " while ",
    " whereas ",
    " which ",
    " who ",
    " when ",
    " where ",
];

/// Clause byte spans within a sentence. Returns empty if the sentence does not
/// split into more than one clause that clears the word floor.
fn clause_ranges(text: &str, base: usize, opts: &WalkOptions) -> Vec<Span> {
    // collect (cut_at, resume_at) split points relative to `text`
    let mut cuts: Vec<(usize, usize)> = Vec::new();

    // semicolons
    let b = text.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b';' {
            cuts.push((i, i + 1));
        }
    }
    // comma coordinators: cut at the comma
    for marker in COMMA_COORDINATORS {
        let mut from = 0;
        while let Some(p) = find_ci(text, marker, from) {
            cuts.push((p, p + 1)); // comma at p; resume after comma
            from = p + 1;
        }
    }
    // subordinators: cut before the word (at the leading space)
    for marker in SUBORDINATORS {
        let mut from = 0;
        while let Some(p) = find_ci(text, marker, from) {
            cuts.push((p, p + 1)); // leading space at p; resume after it
            from = p + 1;
        }
    }

    if cuts.is_empty() {
        return Vec::new();
    }
    cuts.sort_by_key(|c| c.0);

    // build raw segments
    let mut segs: Vec<(usize, usize)> = Vec::new();
    let mut seg_start = next_non_space(text, 0);
    for (cut_at, resume_at) in &cuts {
        if *cut_at > seg_start {
            let e = trim_end_idx(text, seg_start, *cut_at);
            if e > seg_start {
                segs.push((seg_start, e));
            }
            seg_start = next_non_space(text, *resume_at);
        }
    }
    let last_end = trim_end_idx(text, seg_start, text.len());
    if last_end > seg_start {
        segs.push((seg_start, last_end));
    }

    // merge segments below the word floor into a neighbor (keeps contiguous text)
    let merged = merge_short_segments(text, segs, opts.min_clause_words);
    if merged.len() <= 1 {
        return Vec::new();
    }
    merged
        .into_iter()
        .map(|(s, e)| Span::new(base + s, base + e))
        .collect()
}

fn merge_short_segments(text: &str, segs: Vec<(usize, usize)>, min_words: usize) -> Vec<(usize, usize)> {
    if segs.is_empty() {
        return segs;
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (s, e) in segs {
        let words = text[s..e].split_whitespace().count();
        if words < min_words && !out.is_empty() {
            // merge into previous (extend end; spans contiguous in original text)
            let last = out.last_mut().unwrap();
            last.1 = e;
        } else {
            out.push((s, e));
        }
    }
    // if the first segment is short, merge it forward
    if out.len() >= 2 {
        let first_words = text[out[0].0..out[0].1].split_whitespace().count();
        if first_words < min_words {
            let first = out.remove(0);
            out[0].0 = first.0;
        }
    }
    out
}

// --------------------------------------------------------------------------
// Quote + reference detection
// --------------------------------------------------------------------------

/// Spans of the *inner* text of double-quoted segments (straight `"` and curly
/// `“ ”`). Single quotes are intentionally skipped to avoid apostrophe false
/// positives.
fn quote_ranges(text: &str, base: usize) -> Vec<Span> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start_byte, c) = chars[i];
        let opener = c == '"' || c == '\u{201C}'; // " or “
        if opener {
            let want_close: &[char] = if c == '"' { &['"'] } else { &['\u{201D}', '"'] };
            // inner starts after opener
            let inner_start = start_byte + c.len_utf8();
            let mut j = i + 1;
            while j < chars.len() {
                let (cb, cc) = chars[j];
                if want_close.contains(&cc) {
                    if cb > inner_start {
                        out.push(Span::new(base + inner_start, base + cb));
                    }
                    i = j; // continue after closer
                    break;
                }
                j += 1;
            }
        }
        i += 1;
    }
    out
}

/// Spans of citation/reference markers: `[12]` (bracketed numbers) and
/// parenthetical groups containing a 4-digit year, e.g. `(Bennet, 1813)`.
fn reference_ranges(text: &str, base: usize) -> Vec<Span> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let n = b.len();

    // [digits]
    let mut i = 0;
    while i < n {
        if b[i] == b'[' {
            let mut j = i + 1;
            while j < n && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < n && b[j] == b']' {
                out.push(Span::new(base + i, base + j + 1));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }

    // ( ... DDDD ... ) with a 4-digit year inside
    let mut i = 0;
    while i < n {
        if b[i] == b'(' {
            if let Some(close) = memfind(b, b')', i + 1) {
                let inner = &text[i + 1..close];
                if contains_4digit_run(inner) {
                    out.push(Span::new(base + i, base + close + 1));
                }
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }

    out.sort_by_key(|s| s.start);
    out
}

// --------------------------------------------------------------------------
// Terms + co-occurrence
// --------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was",
    "one", "our", "out", "day", "get", "has", "him", "his", "how", "man", "new", "now", "old",
    "see", "two", "way", "who", "boy", "did", "its", "let", "put", "say", "she", "too", "use",
    "that", "this", "with", "they", "have", "from", "were", "been", "their", "them", "then",
    "than", "your", "what", "when", "which", "would", "could", "should", "there", "here", "into",
    "upon", "over", "such", "some", "very", "more", "most", "much", "many", "also", "about",
    "after", "before", "because", "while", "where", "though",
];

fn tokenize_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '\'' {
            cur.push(ch);
        } else {
            push_term(&mut out, &mut cur);
        }
    }
    push_term(&mut out, &mut cur);
    out
}

fn push_term(out: &mut Vec<String>, cur: &mut String) {
    if cur.is_empty() {
        return;
    }
    let token = cur.trim_matches('\'').to_lowercase();
    cur.clear();
    if token.len() < 3 {
        return;
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    if STOPWORDS.contains(&token.as_str()) {
        return;
    }
    out.push(token);
}

fn emit_terms(
    g: &mut Graph,
    term_freq: &BTreeMap<String, usize>,
    sentence_tokens: &[(String, Vec<String>)],
    opts: &WalkOptions,
) {
    // salient terms: freq >= threshold, kept by (freq desc, name asc), capped
    let mut salient: Vec<(String, usize)> = term_freq
        .iter()
        .filter(|(_, &f)| f >= opts.min_term_freq)
        .map(|(t, &f)| (t.clone(), f))
        .collect();
    salient.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    salient.truncate(opts.max_terms);

    let salient_set: std::collections::BTreeSet<&str> =
        salient.iter().map(|(t, _)| t.as_str()).collect();

    // term nodes (sorted by name for deterministic ordering)
    let mut names: Vec<&str> = salient_set.iter().copied().collect();
    names.sort_unstable();
    for name in &names {
        g.push_node(Node::structural(format!("term:{name}"), NodeKind::Term, *name));
    }

    // mentions edges (sentence -> term) and co-occurrence counts
    let mut cooc: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (sent_id, toks) in sentence_tokens {
        let present: Vec<&str> = toks
            .iter()
            .map(|s| s.as_str())
            .filter(|t| salient_set.contains(t))
            .collect();
        for t in &present {
            g.push_edge(Edge::new(
                sent_id.clone(),
                format!("term:{t}"),
                EdgeKind::Mentions,
                Provenance::Structural,
            ));
        }
        // unordered pairs (a < b)
        for a in 0..present.len() {
            for b in (a + 1)..present.len() {
                let (x, y) = if present[a] <= present[b] {
                    (present[a], present[b])
                } else {
                    (present[b], present[a])
                };
                if x != y {
                    *cooc.entry((x.to_string(), y.to_string())).or_insert(0) += 1;
                }
            }
        }
    }

    let max_co = cooc.values().copied().max().unwrap_or(1).max(1);
    for ((a, b), count) in &cooc {
        let weight = *count as f32 / max_co as f32;
        g.push_edge(
            Edge::new(
                format!("term:{a}"),
                format!("term:{b}"),
                EdgeKind::CoOccursWith,
                Provenance::Structural,
            )
            .with_weight(weight),
        );
    }
}

// --------------------------------------------------------------------------
// Small text helpers (byte-offset, char-boundary safe)
// --------------------------------------------------------------------------

fn slice(doc: &str, span: Span) -> &str {
    &doc[span.start..span.end]
}

/// Index of the first non-whitespace byte at or after `from` (a char boundary).
fn next_non_space(text: &str, from: usize) -> usize {
    let b = text.as_bytes();
    let mut i = from;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// End index of `[start, end)` with trailing ASCII whitespace removed.
fn trim_end_idx(text: &str, start: usize, end: usize) -> usize {
    let b = text.as_bytes();
    let mut e = end.min(b.len());
    while e > start && matches!(b[e - 1], b' ' | b'\t' | b'\n' | b'\r') {
        e -= 1;
    }
    e
}

/// Number of leading-whitespace bytes in a line.
fn leading_ws_len(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Length of the line up to the end of its trimmed content (excludes trailing
/// whitespace incl. the newline).
fn trimmed_end_len(line: &str) -> usize {
    line.trim_end().len()
}

/// ASCII case-insensitive substring search.
fn find_ci(haystack: &str, needle_lower: &str, from: usize) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle_lower.as_bytes();
    if n.is_empty() || from + n.len() > h.len() {
        return None;
    }
    'outer: for i in from..=(h.len() - n.len()) {
        for k in 0..n.len() {
            if h[i + k].to_ascii_lowercase() != n[k] {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

fn memfind(h: &[u8], target: u8, from: usize) -> Option<usize> {
    (from..h.len()).find(|&i| h[i] == target)
}

fn contains_4digit_run(s: &str) -> bool {
    let b = s.as_bytes();
    let mut run = 0;
    for &c in b {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 4 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(g: &Graph, kind: NodeKind) -> Vec<String> {
        g.nodes.iter().filter(|n| n.kind == kind).map(|n| n.id.clone()).collect()
    }

    #[test]
    fn walk_is_deterministic() {
        let doc = "Mara found a letter. Vane arrived at the harbor. They argued about the ship.\n\nThe ship was gone. The harbor was empty.";
        let a = walk(doc);
        let b = walk(doc);
        assert_eq!(a, b, "same input must yield identical graph");
    }

    #[test]
    fn segments_sentences_in_a_paragraph() {
        let doc = "First sentence here. Second sentence here. Third one too.";
        let g = walk(doc);
        let sents = ids(&g, NodeKind::Sentence);
        assert_eq!(sents.len(), 3, "expected 3 sentences, got {sents:?}");
    }

    #[test]
    fn abbreviation_does_not_split_sentence() {
        let doc = "Dr. Vane studied the map carefully today.";
        let g = walk(doc);
        assert_eq!(ids(&g, NodeKind::Sentence).len(), 1, "Dr. must not end a sentence");
    }

    #[test]
    fn blank_lines_separate_paragraphs() {
        let doc = "Para one sentence.\n\nPara two sentence.";
        let g = walk(doc);
        assert_eq!(ids(&g, NodeKind::Paragraph).len(), 2);
    }

    #[test]
    fn markdown_heading_becomes_section() {
        let doc = "# Chapter One\n\nThe keeper lit the lamp.";
        let g = walk(doc);
        let secs: Vec<&Node> = g.nodes.iter().filter(|n| n.kind == NodeKind::Section).collect();
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].label, "Chapter One");
    }

    #[test]
    fn clause_split_on_comma_coordinator() {
        let doc = "Mara lit the lamp, and Vane watched the dark sea.";
        let g = walk(doc);
        let clauses = ids(&g, NodeKind::Clause);
        assert_eq!(clauses.len(), 2, "comma+and should split into 2 clauses: {clauses:?}");
    }

    #[test]
    fn no_clause_nodes_when_sentence_does_not_split() {
        let doc = "The keeper lit the lamp.";
        let g = walk(doc);
        assert!(ids(&g, NodeKind::Clause).is_empty());
    }

    #[test]
    fn detects_double_quote() {
        let doc = "Vane said \"the ship is lost\" to the keeper today.";
        let g = walk(doc);
        let quotes: Vec<&Node> = g.nodes.iter().filter(|n| n.kind == NodeKind::Quote).collect();
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].text.as_deref(), Some("the ship is lost"));
    }

    #[test]
    fn detects_bracket_and_year_references() {
        let doc = "The theory was proposed earlier [12] and refined later (Bennet, 1813).";
        let g = walk(doc);
        let refs: Vec<&Node> = g.nodes.iter().filter(|n| n.kind == NodeKind::Reference).collect();
        let texts: Vec<&str> = refs.iter().map(|n| n.text.as_deref().unwrap()).collect();
        assert!(texts.contains(&"[12]"), "missing bracket ref in {texts:?}");
        assert!(texts.iter().any(|t| t.contains("1813")), "missing year ref in {texts:?}");
    }

    #[test]
    fn salient_terms_and_cooccurrence() {
        let doc = "The harbor held a ship. The ship left the harbor. A storm took the ship.";
        let g = walk(doc);
        let terms: Vec<String> = ids(&g, NodeKind::Term);
        assert!(terms.contains(&"term:ship".to_string()), "ship recurs → term node: {terms:?}");
        assert!(terms.contains(&"term:harbor".to_string()), "harbor recurs → term node: {terms:?}");
        // ship & harbor co-occur in sentence 1 and 2
        let cooc = g
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::CoOccursWith
                && ((e.source == "term:harbor" && e.target == "term:ship")
                    || (e.source == "term:ship" && e.target == "term:harbor")));
        assert!(cooc, "expected harbor↔ship co_occurs_with edge");
        // mentions edges exist
        assert!(g.edges.iter().any(|e| e.kind == EdgeKind::Mentions && e.target == "term:ship"));
    }

    #[test]
    fn spine_graph_is_referentially_valid() {
        let doc = "# Chapter One\n\nMara lit the lamp, and Vane watched. The ship was gone.\n\nThe harbor was empty. Mara searched the harbor for the ship.";
        let g = walk(doc);
        let dangling = g.validate();
        assert!(dangling.is_empty(), "spine must have no dangling edges: {dangling:?}");
        // structural-only: no semantic nodes from the deterministic walker
        assert!(g.nodes.iter().all(|n| n.kind.is_structural()));
        assert!(g.nodes.iter().all(|n| n.provenance == Provenance::Structural));
    }

    #[test]
    fn empty_document_yields_empty_graph() {
        let g = walk("   \n\n  \t  ");
        assert!(g.nodes.is_empty());
        assert!(g.edges.is_empty());
    }

    #[test]
    fn sentence_spans_slice_back_to_source() {
        let doc = "First here. Second there.";
        let g = walk(doc);
        for n in g.nodes.iter().filter(|n| n.kind == NodeKind::Sentence) {
            let span = n.span.unwrap();
            assert_eq!(&doc[span.start..span.end], n.text.as_deref().unwrap());
        }
    }
}
