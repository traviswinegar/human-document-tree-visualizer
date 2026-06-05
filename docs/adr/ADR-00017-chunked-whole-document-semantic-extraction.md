# ADR-00017 — Chunked whole-document semantic extraction

- **Status:** Accepted (2026-06-05)
- **Phase:** 9 (#1)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-0002](ADR-0002-graph-schema-and-gbnf-grammar.md)
- **Resolves:** the long-doc **context-starvation** Catch-all (#71), opened in the
  Phase 7 #3 ledger entry.

## Context

The LLM semantic layer extracts **only the document's opening**. `build_extraction_prompt`
(`doctree-llm`) anchors the spine's Section/Sentence nodes as `[id] text` lines until
`PROMPT_DOC_BUDGET_BYTES` (~4 KB — bounded in #78 so the prompt stays under
`momusdev_llm`'s hard 2048-token decode batch, which we cannot change), then emits
`[...document truncated...]`. `semantic_build_steps` runs that **one** prompt through
**one** grammar-constrained call and merges the fragment onto the spine.

The result: on any real document (the 400-page novel, the Anunnaki corpus) the graph
gets the full structural spine + key terms but **no Character / Place / Concept /
Event layer beyond the first few pages** — the user observed exactly this (the
"hybrid (LLM)" graph had zero semantic-entity nodes). This ADR makes the semantic
layer read the **whole** document.

Constraints (standing):
- **Cannot modify `momusdev_llm`** (read-only). The 2048-token batch ceiling is fixed,
  so *each* extraction call's prompt must stay under `PROMPT_DOC_BUDGET_BYTES`. Whole-
  document coverage therefore means **many bounded calls**, not one big one.
- **ADR-0001 decoupling.** The chunking and cross-chunk merge are **pure logic** and
  must live native-free (testable with no model); only the per-chunk LLM calls are
  gated behind `llm`.

## Decision

Replace the single bounded prompt with **chunked extraction**: split the document
(via its spine) into batch-safe windows, extract from each, and merge all fragments —
unifying entities that recur across chunks.

### 1. Chunk the spine, in document order

A native-free chunker splits the **anchorable** nodes (Section/Sentence, in `span`
order — the same nodes `build_extraction_prompt` uses) into contiguous windows, each
whose `[id] text` lines fit `PROMPT_DOC_BUDGET_BYTES`. Every anchorable node lands in
exactly one chunk, in order — **no gaps, no overlap**. `build_extraction_prompt` is
refactored to build a prompt from an arbitrary node subset; the chunker yields one
prompt per window. Deterministic and model-free.

### 2. Per-chunk grammar-constrained extraction (gated)

Each chunk's prompt runs through the existing `extract_graph_json` flow, yielding a
fragment of semantic nodes + edges referencing **that chunk's** spine anchors. A
chunk that fails (bad JSON, inference error) is **skipped with a logged warning** —
one bad window never sinks the whole pass.

### 3. Cross-chunk entity resolution via id canonicalization (native-free)

Independent chunk calls mint inconsistent ids for the same entity (`char:kuk-turu`
in one chunk, `kuk` in another). Before merging, each fragment's **semantic-node ids
are rewritten to a deterministic slug of `(kind, normalized-label)`** — e.g. both
become `character:kuk-turu` — and the fragment's edge endpoints are rewritten to
match. Then the existing `Graph::merge` (dedup **by id**, existing wins) **unifies the
same entity across chunks and unions its edges**, with zero new merge machinery. Only
the six semantic kinds are canonicalized (prefixing by kind: `character:` / `place:` /
`concept:` / `event:` / `object:` / `group:`), so canonical ids never collide with
spine ids (`sec:`/`para:`/`sent:`/…). Spine nodes stay authoritative.

### 4. Streaming + a bounded-cost cap

N chunks = N CPU inference calls, so a long document is a **minutes-to-long**
background job. Two mitigations: (a) per-chunk fragments **stream** onto the live
graph via the existing `pendingDelta` path, so entities appear as each window
finishes (chunk *i*/*N* progress); (b) a configurable **`max_chunks` / extraction
budget** bounds interactive runs (default covers the whole doc; a smaller cap gives a
fast partial pass). The latency/completeness trade-off is explicit, not hidden.

## Alternatives considered

1. **Keep the single bounded prompt.** Rejected — it *is* the starvation.
2. **Raise the batch ceiling / use a bigger context.** Rejected — `momusdev_llm` is
   read-only and the 2048 batch is fixed; and a 650 KB novel exceeds any single
   context regardless.
3. **Byte-window chunking ignoring structure.** Rejected — the spine already gives
   clean sentence/section boundaries; chunk on those so anchors stay meaningful.
4. **Embedding-based entity coreference** (cluster similar mentions). Deferred —
   deterministic slug dedup is the native-free, testable v1; embedding coref is a
   later refinement for spelling/alias variants.
5. **Map-reduce summarize-then-extract.** Deferred — heavier and lossy; direct
   per-chunk extraction is simpler and preserves spans.

## Consequences

- (+) **The semantic layer finally covers the whole document** — characters, places,
  events across every chapter, not just the opening.
- (+) **Contained change**: reuses anchoring, the byte budget, `Graph::merge`, and the
  `pendingDelta` streaming. The new logic (chunker, id canonicalization) is **pure /
  native-free / unit-tested**.
- (+) **Robust**: a failed chunk degrades to a partial-but-valid graph.
- (−) **Latency**: N bounded calls; long docs are long (background) runs. Mitigated by
  streaming + the `max_chunks` cap.
- (−) **Heuristic resolution**: slug dedup may merge two distinct same-named entities
  or miss spelling variants. Acceptable for v1; the embedding-coref refinement (alt 4)
  is the follow-up.

## Invariant (pinned)

Both native-free pieces are unit-tested with **no model**:

- **Chunker:** for any spine, the chunks (a) each fit `PROMPT_DOC_BUDGET_BYTES`,
  (b) partition the anchorable nodes **exactly once in document order** (no gap, no
  overlap), and (c) are deterministic. A single oversized anchor still yields its own
  chunk (never dropped).
- **Entity resolution + merge:** two fragments naming the same entity (same kind +
  label, *different* minted ids) merge into **one** node carrying the **union** of
  their edges; spine nodes remain authoritative; canonical semantic ids never collide
  with spine ids.

Pinned test paths: `crates/doctree-llm/src/lib.rs` (the chunker + per-subset prompt)
and `crates/doctree-core` / `src-tauri/src/llm.rs` (canonicalization + cross-chunk
merge). The standing `cargo test --workspace` (no features) stays green native-free;
the live multi-chunk run over a long document on the real engine is the user's
desktop end test (the webview/model isn't headlessly introspectable — the A5–D4
tooling limit).

## Anchors

- `crates/doctree-llm/src/lib.rs` — `build_extraction_prompt` (refactored to a node
  subset) + the new `chunk_spine` / per-chunk prompt builder.
- `crates/doctree-core` — entity-id canonicalization helper (slug of kind+label) over
  a fragment; reused by the merge.
- `src-tauri/src/llm.rs` — `semantic_build_steps` loops chunks → extract → canonicalize
  → `merge_semantic_onto_spine`, with the `max_chunks` cap + per-chunk skip-on-error.
- `src/` — chunk *i*/*N* progress + per-chunk streamed deltas (existing `pendingDelta`).
- `docs/plans/PLAN-phase9-whole-document-semantics.md` — milestones M1–M5.
