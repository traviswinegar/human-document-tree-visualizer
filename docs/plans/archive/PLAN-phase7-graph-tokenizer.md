# Implementation Plan — Phase 7 #4: reversible graph tokenizer

> Short-lived plan doc (AgentDNA: plan-before-implementation). Archive when the
> work ships. Decision + invariants are pinned in
> [ADR-00013](../adr/ADR-00013-reversible-graph-tokenizer.md); ledger lives in
> [`BUILD_LOG.md`](../../BUILD_LOG.md).

## Goal

Tokenize the document-graph into one **ordered integer-id token list** that a
model can consume, where the same list is **reversible** to *either* (a) the
**original document, byte-exact** or (b) the **full `Graph`**, and a button in the
app **reconstructs the document from the graph**. Fidelity is **lossless**;
purpose is a **research testbench** for intelligent (graph-aware) tokenization vs.
sub-word schemes (agenticmd.org). See ADR-00013 for the *why* and the alternatives.

## Design recap (full detail in ADR-00013)

One interleaved stream:

```
BOS · [ body: every document byte in order, with spanned-node OPEN/CLOSE
        markers nested inline at span boundaries ] · TRAILER ·
[ spanless semantic nodes ] · [ all edges ] · EOS
```

- **Vocabulary:** byte floor `0..=255` (losslessness) + control tokens (`≥256`) +
  one id per `NodeKind`/`EdgeKind`/`Provenance`. Control ids never collide with
  bytes, so byte runs need no escaping; a `SEP` control token frames fields.
- **`decode_text`** = body byte tokens only → original bytes (independent of graph
  correctness). **`decode_graph`** = markers + trailer, reordered by stored
  `ordinal` → exact `Graph` (order-stable for arbitrary input).
- **Bit-exact fields:** `weight: Option<f32>` stored as its 4 IEEE-754 LE bytes
  (text formatting would not round-trip `==`); `text` stored as `TEXT_EQ_SLICE`
  (flag only) when it equals the span slice, else `TEXT_PRESENT` + a byte run, else
  `TEXT_ABSENT`.

## Steps (test-first; native-free `doctree-core` throughout)

### S1 — `tokenizer.rs` skeleton + vocabulary + failing round-trip tests
- New `crates/doctree-core/src/tokenizer.rs`; `pub mod tokenizer;` + re-exports in
  `lib.rs`. Define `Tok` ids (byte floor, control, kind/provenance), `Tokens`
  (wraps `Vec<u32>`), and `kind↔id` / `provenance↔id` maps (exhaustive `match`, so
  adding a `NodeKind` is a compile error until handled — mirrors `tag()`).
- Write the two **pinned invariant tests first** (failing): `text_roundtrip_is_byte_exact`,
  `graph_roundtrip_is_exact` (over `walk(doc)` + a hybrid merged fixture), plus
  `byte_floor_covers_all_256` and a nesting unit (`clause ⊂ sentence ⊂ paragraph`).
- **Acceptance:** tests compile and *fail* (red) — proving they exercise real code.

### S2 — `encode(doc, graph) -> Tokens`
- Compute open/close events from spanned nodes (those with `span.is_some()`),
  bucketed by byte offset; nest via a stack (open by `span.start` asc; close by
  `span.end` desc). Emit `BOS`, then for each byte position flush opens, emit the
  byte token, flush closes; then `TRAILER`, spanless nodes, edges, `EOS`. Store
  `ordinal` (index into `graph.nodes`/`graph.edges`) in every marker.
- **Acceptance:** `text_roundtrip_is_byte_exact` goes green (text projection is the
  simplest, validate it first).

### S3 — `decode_text(&Tokens) -> String` and `decode_graph(&Tokens) -> Graph`
- `decode_text`: collect body byte tokens (skip markers, stop at `TRAILER`) →
  `String::from_utf8` (lossless; doc is valid UTF-8 by `&str` origin).
- `decode_graph`: replay markers to rebuild spanned nodes (span from running byte
  count; text from slice or stored run); read trailer for spanless nodes + edges;
  sort all by `ordinal`.
- **Acceptance:** both invariant tests green; full `cargo test -p doctree-core`.

### S4 — WASM export (ADR-0003 pattern)
- In `crates/doctree-wasm/src/lib.rs`: `#[wasm_bindgen(js_name = reconstructDocument)]`
  taking a graph JSON string + the (already in-hand) document, returning the
  reconstructed text; private host-testable `*_json` helper + a host test. Also a
  `tokenStats` helper (token count vs. byte count) for the research readout.
- **Acceptance:** host-side `*_json` test green; `cargo test -p doctree-wasm`.

### S5 — Tauri command + frontend button
- `src-tauri/src/lib.rs`: `reconstruct_document_impl(doc, graph) -> String` (+ a
  `tokenize_stats_impl`), thin `#[tauri::command]` wrappers, registered in
  `generate_handler!`. Pure-helper unit tests (`cargo test -p doctree-tauri --lib`).
- Frontend: a **"Reconstruct document"** button (desktop + web) that calls the
  command/WASM and shows the byte-exact reconstruction (and the token/byte stat).
  Gated like the rest; native-free default unaffected.
- **Acceptance:** `tsc --noEmit` exit 0 + `vite build` exit 0.

### S6 — Verify + commit
- **Gate:** `cargo test --workspace` (no features) green native-free; `tsc --noEmit`;
  `vite build`. Confirm no new native dep crept into the default build.
- **Commit (two-commit ledger pattern):** code commit `Phase 7 #72:` + ledger
  commit `Phase 7 #73:` with the verification triple. Then archive this plan.

## Risks / watch-items
- **f32 weight equality** — must store raw bytes, not formatted text (handled in
  design). A round-trip test with a weighted edge guards it.
- **Crossing (non-nesting) spans** from a hand-built graph are out of scope v1;
  assert the walker's nesting precondition in a test rather than handle it.
- **Stream size** — interleaving makes the stream larger than the doc; that is
  expected (this is a fidelity/analysis tool, not a codec — ADR-00013 Consequences).

## Backlog / catch-all (this work)
- Measure intelligent-tokenization token efficiency against a real BPE baseline
  (Qwen) — a research output, its own task once the stream exists.
- A learned/merged vocabulary (vs. the fixed bespoke one) — future work the
  testbench may motivate.

## Ledger pointer
Current Position + verification triples live in
[`BUILD_LOG.md`](../../BUILD_LOG.md) per
[`COMPACTION_RECOVERY.md`](../agent-protocols/COMPACTION_RECOVERY.md).
