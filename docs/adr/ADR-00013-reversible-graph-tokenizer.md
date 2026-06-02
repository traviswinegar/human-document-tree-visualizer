# ADR-00013 — Reversible graph tokenizer (dual-projection, byte-lossless)

- **Status:** Accepted (2026-06-02)
- **Phase:** 7 (#4)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-0002](ADR-0002-graph-schema-and-gbnf-grammar.md),
  [ADR-0003](ADR-0003-doctree-core-to-wasm-browser-walker.md)
- **Supersedes / superseded by:** none

## Context

The user wants to **tokenize the graph**: order its nodes into a token list that
can be fed to a model, where the token list is **reversible** back to *either*
the graph *or* the plain text of the **original** document, and a button in the
app **reconstructs the document from the graph**. When asked, the user fixed two
constraints and delegated the third:

- **Fidelity = lossless / byte-exact.** Reconstruction must return the original
  document's *exact bytes*, not a readable approximation.
- **Token format = "pick the best."** Delegated to the agent.
- **Purpose = a research testbench.** The user has invented a spec —
  **agenticmd.org**, the protocol this project already runs under — and is
  testing the thesis that *models are better served by **intelligent**,
  graph-aware tokenization than by existing sub-word schemes (BPE/SentencePiece).*
  So the tokenizer is not a serializer we hide; it is the artifact under study.

**The feasibility wall (verified in `walker.rs`).** The structural walker's spans
do **not tile** the document. `next_non_space` / `trim_end_idx` trim leading and
trailing whitespace; blank lines (paragraph separators), inter-sentence spaces,
leading indentation, and the comma at a clause cut are all dropped from every
span. `Sentence`/`Clause`/`Quote`/`Reference`/`Section` nodes carry exact `text`;
`Paragraph` nodes carry a `span` but **no** `text`; the six semantic node kinds
(`Character`/`Place`/`Concept`/`Event`/`Object`/`Group`) carry **neither** —
they are LLM-inferred and have no byte position at all. Therefore:

> Concatenating node `text` in document order yields a *readable* document, **not
> a byte-identical one.** Lossless reconstruction is impossible from the graph
> alone; the reversible artifact must itself carry the original source bytes, with
> the graph's spans indexing into them.

This is *why* the user chose byte-exact: it forces the token stream — not the
graph — to be the lossless container. That reframes the design from "serialize a
graph" to "**carry the document and its graph in one interleaved stream that
projects to both.**"

Constraints (standing):
- **ADR-0001 decoupling.** The tokenizer is pure logic over `Graph` + `&str`; it
  must live in **`doctree-core`** (zero native deps), be fully `cargo test`-able
  with no model/GPU, and be WASM-exposable like the walker (ADR-0003).
- It must round-trip **arbitrary** valid graphs over their source, not only
  pristine walker output — the semantic layer merges LLM nodes onto the spine
  (`merge_semantic_onto_spine`), so the real input is a hybrid graph.

## Decision

Add a pure module **`crates/doctree-core/src/tokenizer.rs`** that encodes a
`(document: &str, graph: &Graph)` pair into one **interleaved integer-id token
stream** with **two lossless projections**.

### 1. Integer-id vocabulary with a 256-id byte floor

Tokens are `u32` ids drawn from a fixed vocabulary:

- **Byte floor — ids `0..=255`.** The 256 literal byte values. **This is the
  losslessness guarantee:** any document is representable byte-for-byte as a run
  of byte tokens, independent of how (or whether) nodes cover it. UTF-8, control
  chars, BOMs, CRLF — all survive because we round-trip *bytes*, not characters.
- **Structural control tokens** (ids `≥ 256`): `BOS`, `EOS`, `NODE_OPEN`,
  `NODE_CLOSE`, `NODE_SPANLESS`, `EDGE`, `TRAILER`, a field delimiter `SEP`, and
  the text markers `TEXT_EQ_SLICE` / `TEXT_PRESENT` / `TEXT_ABSENT`. Because every
  control id is `≥ 256` and every literal byte is `≤ 255`, **a byte run can hold
  any byte without ever colliding with a delimiter** — framing is unambiguous with
  no escaping and no length prefix.
- **Kind / provenance tokens:** one id per `NodeKind` (13), one per `EdgeKind`
  (11), one per `Provenance` (3). The model sees node/edge *types* as first-class
  atoms, not spelled-out strings — the core of the "intelligent tokenization"
  claim.

This is the **BEST** of the formats considered (see Alternatives) for the
research framing: integer ids are exactly what an embedding table consumes, the
byte floor makes the scheme total (never fails on any input), and graph structure
is lifted into the vocabulary instead of being smeared across sub-word pieces.

### 2. One interleaved stream that is *simultaneously* the document and its graph

```
BOS
  ── body (document order) ──
  for every byte of the document, in order:
      • at each spanned node's span.start → NODE_OPEN, kind, ordinal,
        id, label, provenance, text-marker          (markers nest on a stack)
      • the literal byte                             (as a byte token 0..=255)
      • at each spanned node's span.end   → NODE_CLOSE
  ── trailer ──
  TRAILER
  for each spanless (semantic) node → NODE_SPANLESS, kind, ordinal, id,
                                      label, text, provenance
  for each edge                     → EDGE, kind, ordinal, id, src, tgt,
                                      label, weight, provenance
EOS
```

- **Every document byte appears exactly once**, in order, in the body. Bytes that
  belong to *no* node (blank lines, inter-sentence spaces, the dropped clause
  comma) are still emitted — they are document filler outside any marker.
- **Spanned nodes** (the structural ones) are marked **inline** at their byte
  boundaries. Nesting (`clause ⊂ sentence ⊂ paragraph ⊂ section`, `quote ⊂
  sentence`) is handled with a stack: open in ascending `span.start`, close in
  descending `span.end`.
- **Spanless nodes and all edges** have no byte position, so they live in the
  **trailer** — mirroring `build_sequence`'s "flush danglers last."
- **`ordinal`** = the node's / edge's original index in `graph.nodes` /
  `graph.edges`. Decode reorders by ordinal, so the round-trip preserves the
  input's exact `Vec` order (and thus derive-`PartialEq`) **for any graph**, even
  when its node order is not span-sorted.
- **`text` markers** keep the stream compact without sacrificing generality:
  `TEXT_EQ_SLICE` when a node's `text` equals its span slice (the walker's common
  case — store nothing), `TEXT_PRESENT` + a byte run when it differs,
  `TEXT_ABSENT` for `None` (e.g. `Paragraph`).

### 3. The two projections (the product)

- **`decode_text(&tokens) -> String`** — keep the body's byte tokens, drop every
  marker and the whole trailer, concatenate → **the original document, byte-exact.**
  This holds *by construction* and does **not** depend on the graph being correct.
- **`decode_graph(&tokens) -> Graph`** — read inline markers to rebuild spanned
  nodes (`span` from byte position, `text` from the slice or the stored run,
  `label`/`kind`/`provenance`/`id` from the marker); read the trailer for spanless
  nodes and edges; reorder everything by `ordinal` → **the original graph.**

### 4. Surfacing (mirrors existing patterns)

- **Tauri:** a pure `reconstruct_document_impl(...)` + a thin `#[tauri::command]`
  wrapper registered in `generate_handler!` (the established `*_impl` pattern),
  driving a **"Reconstruct document"** button.
- **WASM:** a `#[wasm_bindgen]` export returning a JSON string, with a private
  host-testable `*_json` helper (the ADR-0003 pattern), so the browser build gets
  the same engine.
- **Research stat:** expose token-count vs. byte-count (and, later, vs. a BPE
  baseline) so the testbench can measure the thesis.

## Alternatives considered

1. **A readable text DSL** (e.g. an indented outline / Markdown-ish dump of the
   graph). **Rejected:** it is the *readable, not byte-exact* failure mode the
   user explicitly ruled out, and a string blob is the opposite of "a token list
   for a model."
2. **Hybrid: wrap an existing BPE tokenizer** (e.g. Qwen's) for the literal text
   and add graph control tokens around it. **Rejected for v1:** it imports a
   native/large dependency into what must be a pure `doctree-core` module
   (ADR-0001), makes losslessness depend on the sub-word vocab's byte-fallback
   coverage, and — most importantly — it *presupposes* the very sub-word scheme
   the research is meant to test against. Kept as the **baseline to measure
   against**, not the design.
3. **Make the walker's spans tile the document** (emit filler nodes for dropped
   whitespace) so node `text` concatenation reconstructs the source. **Rejected:**
   it contorts the *extraction* truth to serve serialization, pollutes the graph
   (and the 3D view) with whitespace nodes, and still would not survive an
   arbitrary externally-built graph. Losslessness belongs in the token stream, not
   in the graph.
4. **Two separate artifacts** (a plain byte blob for text + a JSON graph dump).
   **Rejected:** it abandons the user's core ask — *one* ordered token list that
   is *both* — and the research thesis lives precisely in the interleaving.

## Consequences

- (+) **Total and lossless:** the byte floor means `encode` never fails and
  `decode_text` is byte-exact on *any* input, regardless of graph quality.
- (+) **Both projections from one stream:** the artifact *is* the document and the
  graph — directly serving "reversible to either" and the one-button reconstruct.
- (+) **Pure `doctree-core`, native-free, WASM-able** — same engine on desktop and
  web (ADR-0001/0003); fully unit-testable with no model.
- (+) **Order-stable for arbitrary graphs** via stored ordinals → clean derive-`==`
  round-trip, not just for pristine walker output.
- (+) **Research-ready:** integer ids + type-atoms + a byte baseline give the
  testbench something to measure intelligent vs. sub-word tokenization on.
- (−) **Not compression.** Interleaving bytes with markers makes the stream
  *larger* than the raw document; this is a fidelity/analysis tool, not a codec.
  (Token-efficiency is a *research output*, measured, not a design goal of v1.)
- (−) **Vocabulary is fixed/bespoke**, not a learned merge table — intentional for
  v1; a learned/merged vocabulary is future work the testbench may motivate.
- (−) **Span-nesting assumes well-formed nesting** (intervals nest or are disjoint,
  as the walker guarantees). Pathologically *crossing* spans from a hand-built
  graph are out of scope for v1 and asserted against in tests.

## Invariant (pinned)

Two round-trip properties, unit-tested in `doctree-core` (native-free, no model):

- **Text round-trip (byte-exact):**
  `tokenizer::decode_text(&tokenizer::encode(doc, &graph)) == doc`
  for every fixture — including the dense, dialogue-heavy multi-paragraph
  narrative from BUILD_LOG #69 — and for *any* graph over that `doc` (the property
  does not depend on graph correctness).
- **Graph round-trip (exact):**
  `tokenizer::decode_graph(&tokenizer::encode(doc, &graph)) == graph`
  for `graph = walker::walk(doc)` and for a hybrid
  `merge_semantic_onto_spine(walk(doc), fragment)` fixture (spanned + spanless
  nodes + edges), with node/edge order preserved via stored ordinals.

Pinned test path: **`crates/doctree-core/src/tokenizer.rs`** `#[cfg(test)]`
(`text_roundtrip_is_byte_exact`, `graph_roundtrip_is_exact`, plus byte-floor and
nesting unit tests). If reversibility ever breaks, it breaks here first. The
standing gate `cargo test --workspace` (no features) must stay green native-free.

## Anchors

- `crates/doctree-core/src/tokenizer.rs` — `Tokenizer` / `Tokens`, the vocabulary
  (byte floor + control + kind/provenance ids), `encode`, `decode_text`,
  `decode_graph`, and the pinned round-trip tests.
- `crates/doctree-core/src/lib.rs` — `pub mod tokenizer;` + re-exports.
- `crates/doctree-wasm/src/lib.rs` — the `#[wasm_bindgen]` reconstruct export +
  private `*_json` helper (ADR-0003 pattern).
- `src-tauri/src/lib.rs` — `reconstruct_document_impl` + `#[tauri::command]` +
  `generate_handler!` registration (the `*_impl` pattern).
- `docs/plans/PLAN-phase7-graph-tokenizer.md` — the implementation plan.
