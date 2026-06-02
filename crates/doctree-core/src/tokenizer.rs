//! The reversible graph tokenizer — Phase 7 #4 / [ADR-00013].
//!
//! Encodes a `(document, graph)` pair into one **interleaved integer-id token
//! stream** with two lossless projections:
//!
//! * [`decode_text`] → the **original document, byte-exact** (the byte floor
//!   guarantees this for any input);
//! * [`decode_graph`] → the **full [`Graph`]** (order-stable via stored ordinals).
//!
//! ## Why it must carry the source bytes
//!
//! The structure walker's spans do **not tile** the document (it trims
//! whitespace, drops blank lines, the clause-cut comma, …), and the semantic
//! layer adds nodes with no byte position at all. So the graph alone cannot
//! reconstruct the original text — the *token stream* is the lossless container,
//! with the graph's spans indexing into the bytes it carries.
//!
//! ## Stream layout
//!
//! ```text
//! BOS
//!   body: every document byte, in order, with spanned-node OPEN/CLOSE markers
//!         emitted at their byte boundaries (markers carry an explicit ordinal,
//!         so crossing spans — e.g. a quote that straddles a clause cut — and
//!         arbitrary input order both round-trip)
//!   TRAILER
//!   spanless nodes (the semantic layer + Term nodes) then all edges
//! EOS
//! ```
//!
//! ## Vocabulary
//!
//! `u32` ids: a **byte floor** `0..=255` (losslessness), then one id per
//! [`NodeKind`] / [`EdgeKind`] / [`Provenance`], then the structural control
//! tokens. Every non-byte id is `>= 256`, so a byte run never collides with a
//! delimiter — framing needs no escaping.
//!
//! The pinned invariants live in the tests below:
//! `decode_text(encode(doc, g)) == doc` and `decode_graph(encode(doc, g)) == g`.
//!
//! [ADR-00013]: ../../../docs/adr/ADR-00013-reversible-graph-tokenizer.md

use std::collections::BTreeMap;
use std::fmt;

use crate::schema::{Edge, EdgeKind, Graph, Node, NodeKind, Provenance, Span};

// --------------------------------------------------------------------------
// Vocabulary layout
// --------------------------------------------------------------------------

/// Literal byte tokens occupy `0..=255`. The lossless floor.
const BYTE_CEIL: u32 = 256;

/// One id per [`NodeKind`] (13 variants), starting here.
const KIND_BASE: u32 = BYTE_CEIL; // 256
const KIND_COUNT: u32 = 13;

/// One id per [`EdgeKind`] (11 variants).
const EDGEKIND_BASE: u32 = KIND_BASE + KIND_COUNT; // 269
const EDGEKIND_COUNT: u32 = 11;

/// One id per [`Provenance`] (3 variants).
const PROV_BASE: u32 = EDGEKIND_BASE + EDGEKIND_COUNT; // 280
const PROV_COUNT: u32 = 3;

/// Structural control tokens.
const CTRL_BASE: u32 = PROV_BASE + PROV_COUNT; // 283
const T_BOS: u32 = CTRL_BASE;
const T_EOS: u32 = CTRL_BASE + 1;
const T_NODE_OPEN: u32 = CTRL_BASE + 2;
const T_NODE_CLOSE: u32 = CTRL_BASE + 3;
const T_NODE_SPANLESS: u32 = CTRL_BASE + 4;
const T_EDGE: u32 = CTRL_BASE + 5;
const T_TRAILER: u32 = CTRL_BASE + 6;
const T_STR_END: u32 = CTRL_BASE + 7;
const T_TEXT_EQ_SLICE: u32 = CTRL_BASE + 8;
const T_TEXT_PRESENT: u32 = CTRL_BASE + 9;
const T_TEXT_ABSENT: u32 = CTRL_BASE + 10;
const T_OPT_SOME: u32 = CTRL_BASE + 11;
const T_OPT_NONE: u32 = CTRL_BASE + 12;

/// Total vocabulary size: every id `encode` can emit is `< VOCAB_SIZE`.
pub const VOCAB_SIZE: u32 = CTRL_BASE + 13; // 296

// kind / provenance <-> id (exhaustive matches: adding a variant is a compile
// error here until handled — the same lock `NodeKind::tag()` uses).

fn node_kind_id(k: NodeKind) -> u32 {
    KIND_BASE
        + match k {
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

fn node_kind_from_id(id: u32) -> Option<NodeKind> {
    Some(match id.checked_sub(KIND_BASE)? {
        0 => NodeKind::Section,
        1 => NodeKind::Paragraph,
        2 => NodeKind::Sentence,
        3 => NodeKind::Clause,
        4 => NodeKind::Quote,
        5 => NodeKind::Reference,
        6 => NodeKind::Term,
        7 => NodeKind::Character,
        8 => NodeKind::Place,
        9 => NodeKind::Concept,
        10 => NodeKind::Event,
        11 => NodeKind::Object,
        12 => NodeKind::Group,
        _ => return None,
    })
}

fn edge_kind_id(k: EdgeKind) -> u32 {
    EDGEKIND_BASE
        + match k {
            EdgeKind::PartOf => 0,
            EdgeKind::Precedes => 1,
            EdgeKind::Mentions => 2,
            EdgeKind::References => 3,
            EdgeKind::Quotes => 4,
            EdgeKind::CoOccursWith => 5,
            EdgeKind::InteractsWith => 6,
            EdgeKind::LocatedIn => 7,
            EdgeKind::RelatesTo => 8,
            EdgeKind::Causes => 9,
            EdgeKind::SimilarTo => 10,
        }
}

fn edge_kind_from_id(id: u32) -> Option<EdgeKind> {
    Some(match id.checked_sub(EDGEKIND_BASE)? {
        0 => EdgeKind::PartOf,
        1 => EdgeKind::Precedes,
        2 => EdgeKind::Mentions,
        3 => EdgeKind::References,
        4 => EdgeKind::Quotes,
        5 => EdgeKind::CoOccursWith,
        6 => EdgeKind::InteractsWith,
        7 => EdgeKind::LocatedIn,
        8 => EdgeKind::RelatesTo,
        9 => EdgeKind::Causes,
        10 => EdgeKind::SimilarTo,
        _ => return None,
    })
}

fn provenance_id(p: Provenance) -> u32 {
    PROV_BASE
        + match p {
            Provenance::Structural => 0,
            Provenance::Semantic => 1,
            Provenance::Embedding => 2,
        }
}

fn provenance_from_id(id: u32) -> Option<Provenance> {
    Some(match id.checked_sub(PROV_BASE)? {
        0 => Provenance::Structural,
        1 => Provenance::Semantic,
        2 => Provenance::Embedding,
        _ => return None,
    })
}

// --------------------------------------------------------------------------
// Tokens + error
// --------------------------------------------------------------------------

/// A token stream: a flat list of `u32` ids drawn from the fixed vocabulary
/// (`< `[`VOCAB_SIZE`]). This is what would be fed to a model and what both
/// projections decode from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens(pub Vec<u32>);

impl Tokens {
    /// The raw token ids.
    pub fn ids(&self) -> &[u32] {
        &self.0
    }
    /// Number of tokens.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// `true` when there are no tokens.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What `decode_*` can reject on a malformed stream. `encode` is infallible, so
/// these only arise from externally-supplied / corrupted token lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenizeError {
    /// Ran off the end of the stream mid-structure.
    UnexpectedEnd,
    /// A token id appeared where the grammar did not allow it.
    UnexpectedToken(u32),
    /// A LEB128 varint was malformed (non-byte payload or overlong).
    BadVarint,
    /// A byte run was not valid UTF-8.
    BadUtf8,
    /// A spanned node was missing its open or close boundary.
    MissingSpanBoundary(u64),
}

impl fmt::Display for TokenizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenizeError::UnexpectedEnd => write!(f, "unexpected end of token stream"),
            TokenizeError::UnexpectedToken(t) => write!(f, "unexpected token id {t}"),
            TokenizeError::BadVarint => write!(f, "malformed varint"),
            TokenizeError::BadUtf8 => write!(f, "byte run was not valid UTF-8"),
            TokenizeError::MissingSpanBoundary(ord) => {
                write!(f, "spanned node ordinal {ord} missing an open/close boundary")
            }
        }
    }
}

impl std::error::Error for TokenizeError {}

type Result<T> = std::result::Result<T, TokenizeError>;

/// Research readout: how the graph-aware stream compares to the raw document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenStats {
    /// Number of tokens in the stream.
    pub tokens: usize,
    /// Number of bytes in the source document.
    pub doc_bytes: usize,
    /// Size of the fixed vocabulary.
    pub vocab_size: u32,
}

/// Compute [`TokenStats`] for a document and its encoded stream.
pub fn stats(doc: &str, tokens: &Tokens) -> TokenStats {
    TokenStats {
        tokens: tokens.len(),
        doc_bytes: doc.len(),
        vocab_size: VOCAB_SIZE,
    }
}

// --------------------------------------------------------------------------
// Encode
// --------------------------------------------------------------------------

/// One write-side helper that appends to the token buffer.
struct Enc {
    out: Vec<u32>,
}

impl Enc {
    fn ctrl(&mut self, t: u32) {
        self.out.push(t);
    }
    fn byte(&mut self, b: u8) {
        self.out.push(b as u32);
    }
    /// LEB128, one `u32` per 7-bit group (each payload token is `0..=255`).
    fn varint(&mut self, mut v: u64) {
        loop {
            let mut byte = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
            }
            self.out.push(byte as u32);
            if v == 0 {
                break;
            }
        }
    }
    /// A byte run terminated by `STR_END` (run bytes are `0..=255`, the
    /// terminator is `>= 256`, so no escaping is needed).
    fn str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.out.push(b as u32);
        }
        self.out.push(T_STR_END);
    }
    fn opt_str(&mut self, s: Option<&str>) {
        match s {
            Some(v) => {
                self.ctrl(T_OPT_SOME);
                self.str(v);
            }
            None => self.ctrl(T_OPT_NONE),
        }
    }
    /// Exact IEEE-754 bytes — formatting-to-text would not round-trip `==`.
    fn f32(&mut self, v: f32) {
        for b in v.to_le_bytes() {
            self.out.push(b as u32);
        }
    }
    fn opt_f32(&mut self, v: Option<f32>) {
        match v {
            Some(f) => {
                self.ctrl(T_OPT_SOME);
                self.f32(f);
            }
            None => self.ctrl(T_OPT_NONE),
        }
    }
}

/// Encode a `(document, graph)` pair into the interleaved token stream.
///
/// Infallible: the byte floor represents any document, and any graph whose spans
/// lie within the document round-trips. Spans beyond the document length are
/// clamped (a degenerate, referentially-invalid input); the document text is
/// reproduced byte-exact regardless.
pub fn encode(doc: &str, graph: &Graph) -> Tokens {
    let bytes = doc.as_bytes();
    let len = bytes.len();
    let mut enc = Enc { out: Vec::new() };
    enc.ctrl(T_BOS);

    // Boundary events for spanned nodes, keyed by byte offset. A node may share
    // a boundary with others; ordinals on the markers keep them distinct.
    let mut opens_at: Vec<Vec<usize>> = vec![Vec::new(); len + 1];
    let mut closes_at: Vec<Vec<usize>> = vec![Vec::new(); len + 1];
    let mut spanless: Vec<usize> = Vec::new();
    for (idx, node) in graph.nodes.iter().enumerate() {
        match node.span {
            Some(span) => {
                let s = span.start.min(len);
                let e = span.end.min(len).max(s);
                opens_at[s].push(idx);
                closes_at[e].push(idx);
            }
            None => spanless.push(idx),
        }
    }

    for i in 0..=len {
        // Closes first, then opens, then the byte. Order among markers at one
        // boundary is irrelevant to the recovered spans (decode records the byte
        // position by ordinal), but a fixed order keeps encode deterministic.
        for &idx in &closes_at[i] {
            enc.ctrl(T_NODE_CLOSE);
            enc.varint(idx as u64);
        }
        for &idx in &opens_at[i] {
            let node = &graph.nodes[idx];
            enc.ctrl(T_NODE_OPEN);
            enc.varint(idx as u64);
            enc.ctrl(node_kind_id(node.kind));
            enc.ctrl(provenance_id(node.provenance));
            enc.str(&node.id);
            enc.str(&node.label);
            emit_spanned_text(&mut enc, node, doc);
        }
        if i < len {
            enc.byte(bytes[i]);
        }
    }

    enc.ctrl(T_TRAILER);
    for &idx in &spanless {
        let node = &graph.nodes[idx];
        enc.ctrl(T_NODE_SPANLESS);
        enc.varint(idx as u64);
        enc.ctrl(node_kind_id(node.kind));
        enc.ctrl(provenance_id(node.provenance));
        enc.str(&node.id);
        enc.str(&node.label);
        match &node.text {
            Some(t) => {
                enc.ctrl(T_TEXT_PRESENT);
                enc.str(t);
            }
            None => enc.ctrl(T_TEXT_ABSENT),
        }
    }
    for (idx, edge) in graph.edges.iter().enumerate() {
        enc.ctrl(T_EDGE);
        enc.varint(idx as u64);
        enc.ctrl(edge_kind_id(edge.kind));
        enc.ctrl(provenance_id(edge.provenance));
        enc.opt_str(edge.id.as_deref());
        enc.str(&edge.source);
        enc.str(&edge.target);
        enc.opt_str(edge.label.as_deref());
        enc.opt_f32(edge.weight);
    }
    enc.ctrl(T_EOS);

    Tokens(enc.out)
}

/// Emit a spanned node's text marker: `EQ_SLICE` when text equals the span slice
/// (the walker's common case — store nothing), else `PRESENT` + the bytes, else
/// `ABSENT`.
fn emit_spanned_text(enc: &mut Enc, node: &Node, doc: &str) {
    match &node.text {
        None => enc.ctrl(T_TEXT_ABSENT),
        Some(t) => {
            let eq = node
                .span
                .and_then(|s| doc.get(s.start..s.end))
                .map(|slice| slice == t.as_str())
                .unwrap_or(false);
            if eq {
                enc.ctrl(T_TEXT_EQ_SLICE);
            } else {
                enc.ctrl(T_TEXT_PRESENT);
                enc.str(t);
            }
        }
    }
}

// --------------------------------------------------------------------------
// Decode
// --------------------------------------------------------------------------

/// Read-side cursor over a token slice.
struct Cur<'a> {
    t: &'a [u32],
    i: usize,
}

impl<'a> Cur<'a> {
    fn next(&mut self) -> Result<u32> {
        let v = *self.t.get(self.i).ok_or(TokenizeError::UnexpectedEnd)?;
        self.i += 1;
        Ok(v)
    }
    fn expect(&mut self, tok: u32) -> Result<()> {
        let v = self.next()?;
        if v == tok {
            Ok(())
        } else {
            Err(TokenizeError::UnexpectedToken(v))
        }
    }
    fn varint(&mut self) -> Result<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        loop {
            let b = self.next()?;
            if b > 0xff {
                return Err(TokenizeError::BadVarint);
            }
            if shift >= 64 {
                return Err(TokenizeError::BadVarint);
            }
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(result)
    }
    fn str(&mut self) -> Result<String> {
        let mut buf = Vec::new();
        loop {
            let tok = self.next()?;
            if tok == T_STR_END {
                break;
            }
            if tok > 0xff {
                return Err(TokenizeError::UnexpectedToken(tok));
            }
            buf.push(tok as u8);
        }
        String::from_utf8(buf).map_err(|_| TokenizeError::BadUtf8)
    }
    fn opt_str(&mut self) -> Result<Option<String>> {
        match self.next()? {
            T_OPT_SOME => Ok(Some(self.str()?)),
            T_OPT_NONE => Ok(None),
            other => Err(TokenizeError::UnexpectedToken(other)),
        }
    }
    fn f32(&mut self) -> Result<f32> {
        let mut b = [0u8; 4];
        for slot in &mut b {
            let v = self.next()?;
            if v > 0xff {
                return Err(TokenizeError::UnexpectedToken(v));
            }
            *slot = v as u8;
        }
        Ok(f32::from_le_bytes(b))
    }
    fn opt_f32(&mut self) -> Result<Option<f32>> {
        match self.next()? {
            T_OPT_SOME => Ok(Some(self.f32()?)),
            T_OPT_NONE => Ok(None),
            other => Err(TokenizeError::UnexpectedToken(other)),
        }
    }
    fn node_kind(&mut self) -> Result<NodeKind> {
        let v = self.next()?;
        node_kind_from_id(v).ok_or(TokenizeError::UnexpectedToken(v))
    }
    fn edge_kind(&mut self) -> Result<EdgeKind> {
        let v = self.next()?;
        edge_kind_from_id(v).ok_or(TokenizeError::UnexpectedToken(v))
    }
    fn provenance(&mut self) -> Result<Provenance> {
        let v = self.next()?;
        provenance_from_id(v).ok_or(TokenizeError::UnexpectedToken(v))
    }
}

/// Metadata recorded at a spanned node's OPEN (its boundaries are recorded
/// separately, since CLOSE can precede OPEN for a zero-length span).
struct SpannedMeta {
    kind: NodeKind,
    prov: Provenance,
    id: String,
    label: String,
    text: SpannedText,
}

enum SpannedText {
    Absent,
    EqSlice,
    Present(String),
}

/// Parse the whole stream once, recovering both the document bytes and the
/// graph. Both public projections are thin views over this.
fn decode_all(tokens: &Tokens) -> Result<(Vec<u8>, Graph)> {
    let mut cur = Cur { t: &tokens.0, i: 0 };
    cur.expect(T_BOS)?;

    let mut doc_bytes: Vec<u8> = Vec::new();
    let mut meta: BTreeMap<u64, SpannedMeta> = BTreeMap::new();
    let mut starts: BTreeMap<u64, usize> = BTreeMap::new();
    let mut ends: BTreeMap<u64, usize> = BTreeMap::new();

    // Body: literal bytes interleaved with OPEN/CLOSE markers, until TRAILER.
    loop {
        let tok = cur.next()?;
        if tok == T_TRAILER {
            break;
        }
        if tok < BYTE_CEIL {
            doc_bytes.push(tok as u8);
            continue;
        }
        match tok {
            T_NODE_OPEN => {
                let ord = cur.varint()?;
                let kind = cur.node_kind()?;
                let prov = cur.provenance()?;
                let id = cur.str()?;
                let label = cur.str()?;
                let text = match cur.next()? {
                    T_TEXT_ABSENT => SpannedText::Absent,
                    T_TEXT_EQ_SLICE => SpannedText::EqSlice,
                    T_TEXT_PRESENT => SpannedText::Present(cur.str()?),
                    other => return Err(TokenizeError::UnexpectedToken(other)),
                };
                starts.insert(ord, doc_bytes.len());
                meta.insert(
                    ord,
                    SpannedMeta {
                        kind,
                        prov,
                        id,
                        label,
                        text,
                    },
                );
            }
            T_NODE_CLOSE => {
                let ord = cur.varint()?;
                ends.insert(ord, doc_bytes.len());
            }
            other => return Err(TokenizeError::UnexpectedToken(other)),
        }
    }

    // Trailer: spanless nodes + edges, until EOS. Keyed by ordinal so the
    // recovered `Vec` order matches the input exactly.
    let mut nodes_by_ord: BTreeMap<u64, Node> = BTreeMap::new();
    let mut edges_by_ord: BTreeMap<u64, Edge> = BTreeMap::new();
    loop {
        let tok = cur.next()?;
        if tok == T_EOS {
            break;
        }
        match tok {
            T_NODE_SPANLESS => {
                let ord = cur.varint()?;
                let kind = cur.node_kind()?;
                let prov = cur.provenance()?;
                let id = cur.str()?;
                let label = cur.str()?;
                let text = match cur.next()? {
                    T_TEXT_ABSENT => None,
                    T_TEXT_PRESENT => Some(cur.str()?),
                    other => return Err(TokenizeError::UnexpectedToken(other)),
                };
                nodes_by_ord.insert(
                    ord,
                    Node {
                        id,
                        kind,
                        label,
                        text,
                        span: None,
                        provenance: prov,
                    },
                );
            }
            T_EDGE => {
                let ord = cur.varint()?;
                let kind = cur.edge_kind()?;
                let prov = cur.provenance()?;
                let id = cur.opt_str()?;
                let source = cur.str()?;
                let target = cur.str()?;
                let label = cur.opt_str()?;
                let weight = cur.opt_f32()?;
                edges_by_ord.insert(
                    ord,
                    Edge {
                        id,
                        source,
                        target,
                        kind,
                        label,
                        weight,
                        provenance: prov,
                    },
                );
            }
            other => return Err(TokenizeError::UnexpectedToken(other)),
        }
    }

    // Finalize spanned nodes from their recorded boundaries.
    for (ord, m) in meta {
        let start = *starts.get(&ord).ok_or(TokenizeError::MissingSpanBoundary(ord))?;
        let end = *ends.get(&ord).ok_or(TokenizeError::MissingSpanBoundary(ord))?;
        let text = match m.text {
            SpannedText::Absent => None,
            SpannedText::Present(s) => Some(s),
            SpannedText::EqSlice => {
                let slice = doc_bytes
                    .get(start..end)
                    .ok_or(TokenizeError::MissingSpanBoundary(ord))?;
                Some(String::from_utf8(slice.to_vec()).map_err(|_| TokenizeError::BadUtf8)?)
            }
        };
        nodes_by_ord.insert(
            ord,
            Node {
                id: m.id,
                kind: m.kind,
                label: m.label,
                text,
                span: Some(Span::new(start, end)),
                provenance: m.prov,
            },
        );
    }

    let nodes = nodes_by_ord.into_values().collect();
    let edges = edges_by_ord.into_values().collect();
    Ok((doc_bytes, Graph { nodes, edges }))
}

/// Project the stream back to the **original document, byte-exact**.
pub fn decode_text(tokens: &Tokens) -> Result<String> {
    let (bytes, _graph) = decode_all(tokens)?;
    String::from_utf8(bytes).map_err(|_| TokenizeError::BadUtf8)
}

/// Project the stream back to the **full [`Graph`]** (order-stable).
pub fn decode_graph(tokens: &Tokens) -> Result<Graph> {
    let (_bytes, graph) = decode_all(tokens)?;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::walk;

    /// A dense, dialogue-heavy multi-paragraph narrative with em-dashes, curly
    /// quotes, CRLF, indentation, and named entities — the kind of prose that
    /// surfaced BUILD_LOG #69, exercising every span trim the walker performs.
    const NARRATIVE: &str = "# Chapter One — The Harbor\r\n\r\n\
        Mara Vane stood at the edge of the cove, watching the tide pull at the wreck. \
        \u{201C}You shouldn\u{2019}t have come,\u{201D} said Inspector Holloway. \
        She did not turn. \u{201C}The ship was mine,\u{201D} she answered.\n\n\
        \tHolloway studied the broken mast. The Harbor Guild had ruled it an \
        accident; Mara called it murder, and the ledger named every captain.";

    fn docs() -> Vec<&'static str> {
        vec![
            "",
            "Hello.",
            "First sentence here. Second sentence here. Third one too.",
            "# Chapter One\n\nThe keeper lit the lamp, and Vane watched the sea.\n\nThe ship was gone.",
            "Vane said \"the ship is lost, and the crew too\" to me at dawn.",
            "The theory was proposed earlier [12] and refined later (Bennet, 1813).",
            NARRATIVE,
        ]
    }

    // ----- pinned invariant 1: text round-trip is byte-exact -----

    #[test]
    fn text_roundtrip_is_byte_exact() {
        for doc in docs() {
            let g = walk(doc);
            let toks = encode(doc, &g);
            let back = decode_text(&toks).expect("decode_text");
            assert_eq!(back, doc, "text projection must be byte-exact");
        }
    }

    #[test]
    fn text_roundtrip_independent_of_graph() {
        // The text projection holds for *any* graph over the doc — even an empty
        // one — because every document byte is emitted regardless of coverage.
        for doc in docs() {
            let toks = encode(doc, &Graph::default());
            assert_eq!(decode_text(&toks).unwrap(), doc);
        }
    }

    // ----- pinned invariant 2: graph round-trip is exact -----

    #[test]
    fn graph_roundtrip_is_exact() {
        for doc in docs() {
            let g = walk(doc);
            let toks = encode(doc, &g);
            let back = decode_graph(&toks).expect("decode_graph");
            assert_eq!(back, g, "graph projection must reproduce the graph exactly");
        }
    }

    #[test]
    fn hybrid_graph_with_semantic_layer_roundtrips() {
        // walk() spine + a hand-merged semantic layer: spanless Character nodes
        // with text, and edges carrying every Option branch (id/label/weight).
        let doc = NARRATIVE;
        let mut g = walk(doc);
        g.push_node(
            Node::semantic("char:mara", NodeKind::Character, "Mara Vane")
                .with_text("the protagonist"),
        );
        g.push_node(Node::semantic("char:holloway", NodeKind::Character, "Inspector Holloway"));
        g.push_edge(
            Edge::new("char:mara", "char:holloway", EdgeKind::InteractsWith, Provenance::Semantic)
                .with_label("argues with")
                .with_weight(0.5),
        );
        g.push_edge(
            Edge {
                id: Some("e:custom".into()),
                source: "char:mara".into(),
                target: "char:holloway".into(),
                kind: EdgeKind::SimilarTo,
                label: None,
                weight: Some(1.0 / 3.0),
                provenance: Provenance::Embedding,
            },
        );

        let toks = encode(doc, &g);
        assert_eq!(decode_text(&toks).unwrap(), doc, "text still byte-exact with semantic layer");
        assert_eq!(decode_graph(&toks).unwrap(), g, "hybrid graph round-trips exactly");
    }

    // ----- robustness: crossing spans (walker really emits these) -----

    #[test]
    fn crossing_quote_and_clause_roundtrip() {
        // A comma-coordinator clause cut *inside* a quote makes the quote span
        // straddle a clause boundary — neither nests the other. The explicit
        // ordinals on OPEN/CLOSE markers handle this where a LIFO stack could not.
        let doc = "Vane said \"the ship is lost, and the crew too\" to me at dawn.";
        let g = walk(doc);
        let quote = g.nodes.iter().find(|n| n.kind == NodeKind::Quote).expect("a quote node");
        let clauses: Vec<Span> =
            g.nodes.iter().filter(|n| n.kind == NodeKind::Clause).filter_map(|n| n.span).collect();
        let q = quote.span.unwrap();
        let crosses = clauses.iter().any(|c| {
            // partial overlap with neither containing the other
            let overlaps = q.start < c.end && c.start < q.end;
            let q_in_c = c.start <= q.start && q.end <= c.end;
            let c_in_q = q.start <= c.start && c.end <= q.end;
            overlaps && !q_in_c && !c_in_q
        });
        assert!(crosses, "fixture must actually produce a crossing quote/clause: q={q:?} clauses={clauses:?}");

        let toks = encode(doc, &g);
        assert_eq!(decode_graph(&toks).unwrap(), g, "crossing spans must round-trip");
        assert_eq!(decode_text(&toks).unwrap(), doc);
    }

    // ----- vocabulary / byte-floor sanity -----

    #[test]
    fn byte_floor_covers_all_256_and_is_disjoint_from_controls() {
        assert_eq!(BYTE_CEIL, 256, "byte floor must be exactly 0..=255");
        assert!(KIND_BASE >= BYTE_CEIL, "kind ids must not collide with bytes");
        assert!(T_BOS >= BYTE_CEIL && T_EOS < VOCAB_SIZE);
        // Every kind/edge/provenance id maps round-trip.
        for raw in 0..KIND_COUNT {
            let id = KIND_BASE + raw;
            assert_eq!(node_kind_id(node_kind_from_id(id).unwrap()), id);
        }
        for raw in 0..EDGEKIND_COUNT {
            let id = EDGEKIND_BASE + raw;
            assert_eq!(edge_kind_id(edge_kind_from_id(id).unwrap()), id);
        }
        for raw in 0..PROV_COUNT {
            let id = PROV_BASE + raw;
            assert_eq!(provenance_id(provenance_from_id(id).unwrap()), id);
        }
    }

    #[test]
    fn all_emitted_ids_are_within_vocab() {
        let doc = NARRATIVE;
        let toks = encode(doc, &walk(doc));
        assert!(toks.ids().iter().all(|&t| t < VOCAB_SIZE), "no token may exceed the vocab");
        assert_eq!(toks.ids().first().copied(), Some(T_BOS));
        assert_eq!(toks.ids().last().copied(), Some(T_EOS));
    }

    #[test]
    fn weight_is_bit_exact_through_roundtrip() {
        let doc = "A and B share a theme here.";
        let mut g = walk(doc);
        g.push_node(Node::semantic("a", NodeKind::Concept, "A"));
        g.push_node(Node::semantic("b", NodeKind::Concept, "B"));
        for w in [0.0_f32, 1.0, 1.0 / 3.0, 0.123_456_79, f32::MIN_POSITIVE] {
            g.edges.clear();
            g.push_edge(
                Edge::new("a", "b", EdgeKind::SimilarTo, Provenance::Embedding).with_weight(w),
            );
            let back = decode_graph(&encode(doc, &g)).unwrap();
            assert_eq!(back.edges[0].weight, Some(w), "weight {w} must survive bit-exact");
        }
    }

    #[test]
    fn stream_is_larger_than_doc_not_a_codec() {
        // Documented consequence (ADR-00013): interleaving markers makes the
        // stream larger than the raw document; this is a fidelity tool.
        let doc = NARRATIVE;
        let s = stats(doc, &encode(doc, &walk(doc)));
        assert_eq!(s.doc_bytes, doc.len());
        assert_eq!(s.vocab_size, VOCAB_SIZE);
        assert!(s.tokens >= s.doc_bytes);
    }

    #[test]
    fn empty_doc_and_empty_graph() {
        let toks = encode("", &Graph::default());
        assert_eq!(decode_text(&toks).unwrap(), "");
        assert_eq!(decode_graph(&toks).unwrap(), Graph::default());
    }

    #[test]
    fn determinism_same_inputs_same_stream() {
        let doc = NARRATIVE;
        let g = walk(doc);
        assert_eq!(encode(doc, &g), encode(doc, &g));
    }
}
