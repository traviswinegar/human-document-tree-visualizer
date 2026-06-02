# ADR-0004 — Embedding-similarity layer: in-memory cosine, gated independently of inference

- **Status:** Accepted
- **Date:** 2026-06-02
- **Deciders:** Travis (user), agent
- **Supersedes:** —
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md), [ADR-0002](ADR-0002-graph-schema-and-gbnf-grammar.md)

## Context

B4 adds the third extraction layer: **embedding similarity**. After the
deterministic spine (Stream A) and the grammar-constrained LLM semantic layer
(B3), we want to link nodes that are *conceptually* close even when no syntactic
or LLM-asserted relationship connects them — echoed sentences, related terms,
thematically adjacent entities — and to offer **semantic search** (a free-text
query jumps to the most relevant node, beyond literal substring match).

ADR-0001 named the stack's embedding capability as "embeddings + a LanceDB
vector store (RAG)" via `momusdev_llm`. Two questions fall out of actually
building B4:

1. **Do we need LanceDB now?** The unit of work is *one document's graph* — tens
   to a few hundred content nodes. That is a trivially small vector set.
2. **Must embeddings ride on the same feature as llama.cpp inference?** The
   `momusdev_llm` crate gates the embedder (`fastembed`/ONNX) behind its
   `vectordb` feature, entirely separately from `inference` (llama.cpp). Our B1
   scaffolding had `doctree-llm/vectordb` *imply* `llm`, coupling them.

The load-bearing invariant from ADR-0001 still rules: the **default build is
native-free**, and a failed native build of any semantic layer must never block
the deterministic Stream-A pipeline.

## Decision

**Compute embedding similarity in memory, over a single document's node vectors,
and gate the embedder independently of inference.**

### In-memory cosine similarity, not LanceDB (for now)

The whole similarity computation is plain arithmetic over the document's node
vectors:

- `cosine_similarity(a, b)` — degenerate-safe (length mismatch / zero vector → 0,
  never `NaN`).
- `similarity_edges(embedded, opts)` — pairwise cosine over the node set; keep
  each node's strongest `top_k` neighbours above `min_similarity`; emit one
  undirected `EdgeKind::SimilarTo` edge per surviving pair, weighted by the
  cosine and tagged `Provenance::Embedding`. Bounds the edge count to ~`top_k·n`
  so the graph stays legible instead of becoming an `n²` hairball.
- `rank_by_similarity(query, embedded, top_k)` — semantic search: rank nodes by
  cosine to the query vector, best first.

No ANN index, no LanceDB, no persistence. At this scale an index would be
slower (build cost) and add a heavyweight stateful dependency for no recall
benefit. **LanceDB persistence is deferred** to a future cross-document / RAG
milestone (searching a *corpus*, surviving restarts) — logged in the BUILD_LOG
backlog, not built here.

### The pure logic is native-free; only the model is gated

Mirroring B3's prompt builder: everything above lives in `doctree-llm` as
**native-free** functions, exercised by the default `cargo test`. Only the
embedding *model* — `Embedder`, wrapping `momusdev_llm`'s `FastEmbedder`
(all-MiniLM-L6-v2, 384-dim ONNX) — is compiled under a feature. `which nodes to
embed and with what text` (`graph_embedding_inputs`, `is_embeddable_kind`) is
also pure: sections/sentences embed their span text, terms/semantic entities
their label; structural sub-units (paragraph/clause/quote/reference) are skipped
as noise.

### `vectordb` is decoupled from `llm`

`doctree-llm/vectordb = ["dep:momusdev_llm", "momusdev_llm/vectordb"]` — it does
**not** imply `llm`. Consequences:

- A `--features vectordb` build gets embeddings **without compiling llama.cpp**
  (no C++/CMake toolchain needed — `fastembed` downloads a prebuilt ONNX
  runtime). So "spine + similarity edges + semantic search" is a legitimate
  lighter build that runs where the CUDA/Vulkan/llama path can't.
- `--features llm` and `--features vectordb` are orthogonal; combine them for
  similarity over the full hybrid (spine + LLM semantic) graph.
- The Tauri managed state (`LlmState`) carries two independently `#[cfg]`-gated
  lazy slots (`engine` under `llm`, `embedder` under `vectordb`); it is only
  managed, and only exists, when at least one feature is on.

### Reuses the schema and the renderer — no new shapes

`EdgeKind::SimilarTo` and `Provenance::Embedding` were reserved in the A2 schema
(ADR-0002) and carry the cosine in the existing `Edge::weight`. Similarity edges
flow through the **same** `build_sequence` → `BuildStep` stream and the existing
weighted-edge render path, so the animated build needs no new frontend code, as
with B3.

## Pinned invariant (the contract that must not silently drift)

- **Similarity logic is pure + native-free:** cosine / top-k / ranking / edge
  derivation are tested by the default `cargo test`. Pinned by
  `crates/doctree-llm/src/lib.rs::tests::{cosine_similarity_handles_identical_orthogonal_and_degenerate,
  similarity_edges_link_only_pairs_above_threshold, similarity_edges_respect_top_k,
  rank_by_similarity_orders_best_first_and_truncates, graph_embedding_inputs_selects_content_nodes}`
  and `src-tauri/src/llm.rs::tests::{attach_similarity_edges_appends_valid_weighted_links,
  to_search_hits_attaches_label_and_kind_and_serializes_camel_case}`.
- **Native-free default build:** with default features `doctree-llm` pulls in no
  embedder; `cargo test --workspace` stays green with zero native deps (ADR-0001).
- **Augmentation preserves validity:** similarity edges only reference ids drawn
  from the graph being augmented, so `attach_similarity_edges` keeps the graph
  referentially valid by construction (no prune needed).

## Alternatives considered

1. **Use LanceDB now (as ADR-0001 envisioned).** Pro: matches the stated stack;
   path to corpus-scale RAG. Con: a heavyweight stateful store for a few hundred
   in-memory vectors — index build cost exceeds brute-force cosine at this scale,
   and persistence semantics (where does the DB live, when is it invalidated) are
   real complexity with zero payoff for single-document similarity. Deferred, not
   rejected — it returns when search spans documents.
2. **Keep `vectordb` implying `llm`.** Pro: one "semantic" switch. Con: forces a
   C++/llama.cpp build on anyone who only wants embeddings, and couples two
   independent native subsystems against ADR-0001's minimise-native-deps spirit.
   Rejected — the embedder builds with no compiler; don't chain it to one.
3. **Put the similarity math in `doctree-core`.** Pro: also native-free. Con:
   `doctree-core` is the *deterministic structural* layer; embeddings are a
   semantic concern that belongs with the other gated semantic code in
   `doctree-llm`. Keeping core purely structural preserves the clean A/B split.
4. **A new `EdgeKind` per similarity flavour / a dedicated render path.** Con:
   `SimilarTo` + `weight` + `Provenance::Embedding` already express it and render
   through the existing weighted-edge path. Rejected as needless surface.

## Consequences

- Similarity + search work on a **CPU-only, compiler-free** build
  (`--features vectordb`), widening where the semantic layer runs.
- The real B4 logic is verified headlessly by the default test suite; the live
  model run (ONNX load + real vectors) is the user's desktop acceptance test
  (`src-tauri/tests/embedding_roundtrip.rs`, `#[ignore]`d), matching the B2/B3
  pattern.
- Cross-document / persistent vector search (the LanceDB path ADR-0001 named)
  remains a future milestone; this ADR records that B4 intentionally scoped to
  in-memory single-document similarity.
- The frontend gains two new IPC commands (`embedded_build_steps`,
  `semantic_search`) on the stable, always-registered surface; on a build
  without `vectordb` they return an actionable error, like the `llm` stubs.

## Anchors

- `crates/doctree-llm/src/lib.rs` — `cosine_similarity`, `SimilarityOptions`,
  `similarity_edges`, `rank_by_similarity`, `SearchHit`, `is_embeddable_kind`,
  `graph_embedding_inputs`, `embed_cache_dir`, gated `Embedder` + tests.
- `crates/doctree-llm/Cargo.toml` — `vectordb` feature decoupled from `llm`.
- `src-tauri/src/llm.rs` — `attach_similarity_edges`, `SearchHitDto`,
  `to_search_hits` (pure) + gated `embedded_build_steps` / `semantic_search`
  commands + native-free stubs.
- `src-tauri/Cargo.toml` — `vectordb = ["doctree-llm/vectordb"]` (no `llm`).
- `src-tauri/tests/embedding_roundtrip.rs` — `#[ignore]`d live acceptance tests.
- `crates/doctree-core/src/schema.rs` — `EdgeKind::SimilarTo`,
  `Provenance::Embedding`, `Edge::weight` (reserved in ADR-0002).
