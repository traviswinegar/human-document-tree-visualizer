# ADR-00010 — Cross-document persistent RAG over momusdev_llm's LanceDB store

- **Status:** Accepted (2026-06-02)
- **Phase:** 6 (#7)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md), [ADR-0004](ADR-0004-embedding-similarity-layer.md)
- **Supersedes / superseded by:** none (delivers the cross-document/RAG milestone
  that ADR-0004 explicitly deferred)

## Context

ADR-0004 scoped B4 deliberately to **in-memory, single-document** similarity:
cosine over one graph's node vectors, no index, no persistence. It named the
follow-on out loud — *"LanceDB persistence is deferred to a future cross-document
/ RAG milestone (searching a corpus, surviving restarts)"* — and parked it in the
BUILD_LOG backlog. This ADR is that milestone.

What changes at corpus scale:

1. **The unit of work is no longer one document.** B4 ranks the nodes of the
   document currently on screen. A library (Phase 5/6 save-load + IndexedDB) holds
   many documents; a user wants *"where, across everything I've ingested, is this
   idea?"* — a query that spans documents and must **survive restarts**.
2. **Brute-force cosine stops being free.** ADR-0004's reasoning ("an index would
   be slower at a few hundred vectors") inverts once the vector set is the whole
   corpus. An on-disk ANN index is the right tool there.
3. **The stack already carries the store.** ADR-0001 named "embeddings + a LanceDB
   vector store (RAG)" via `momusdev_llm`. The `vectordb` feature already pulls in
   `momusdev_llm`'s `VectorStore` (LanceDB) — it was simply **unused** after B4
   chose the in-memory path. The embedder (all-MiniLM-L6-v2, 384-dim) that feeds it
   is the same one B4 already uses.

The load-bearing invariant from ADR-0001 still rules: the **default build is
native-free**, and nothing here may pull a native dep into it.

## Decision

**Persist node embeddings into `momusdev_llm`'s on-disk LanceDB `VectorStore`,
keyed by `(document, node)`, and route a corpus search through its ANN query —
all behind the existing `vectordb` feature, consuming the store read-only.**

### What gets persisted, and how it's keyed

- Each indexed document is walked into its deterministic spine, its content nodes
  are embedded (the **same** `graph_embedding_inputs` selection as B4 — span text
  else label; structural sub-units skipped), and each `(node)` becomes one stored
  row.
- The row's primary id is a **corpus-unique composite key**:
  `rag_storage_key(doc_id, node_id) = "{doc_id}\u{1f}{node_id}"`. The ASCII Unit
  Separator (`0x1f`) cannot occur in a document id or a walker node id (both
  printable), so the same `node_id` (`char:mara`) stays **distinct across
  documents** and the key `split`s back unambiguously. `doc_id`, `node_id`, and the
  node-kind tag are *also* stored in the row's metadata (`source_id` / `extra` /
  `source_type`), so a hit names its document, node, and kind without re-parsing
  the key — the split is only a fallback.

### The shape logic is native-free; only the store is gated

Mirroring ADR-0004's split exactly: everything that decides *what we persist and
what a search returns* is plain data and lives in `doctree-llm` **native-free**,
so the default `cargo test` exercises it like the B4 similarity logic —

- `RAG_DB_ENV` / `rag_db_dir()` — on-disk corpus location (env var, else a stable
  per-user temp subdir so the corpus persists without configuration).
- `rag_storage_key` / `split_rag_key` — the corpus-unique key and its inverse.
- `RagRecord` (doc_id, node_id, kind, text, vector) and
  `graph_rag_records(graph, doc_id, embedded)` — the `(graph, embeddings) →
  records` join (skips a vector whose node is absent or whose text is empty).
- `RagHit` (doc_id, node_id, kind, text, score) — the result DTO.

Only the **store wrapper** is gated. `RagStore` (under `vectordb`) is a thin,
*sync* adapter over `momusdev_llm::storage::VectorStore` (async, LanceDB on Tokio):
it maps our native-free `RagRecord` → the sibling's `TaskProposal` on `index`, and
the sibling's `QueryResult` → our `RagHit` on `search`. It **owns a small Tokio
runtime** and drives the async store with `block_on`, so the rest of the codebase
(and the Tauri command) call it with plain sync methods — identical to how the
sync `Engine` and `Embedder` are consumed (one `spawn_blocking` on the caller's
side). The sibling crate is **consumed read-only** (ADR-0001) — we never touch it.

### `vectordb` already gates it; runtime must not nest

No new feature: `vectordb` already cascades `doctree-tauri → doctree-llm →
momusdev_llm/vectordb` (lancedb/arrow/fastembed). It only gained an **optional
`tokio`** dep (with `rt-multi-thread`) so `RagStore` can build its own runtime.
Because that runtime calls `block_on`, the Tauri commands run the whole
embed→open→index / embed→open→search sequence inside **one `spawn_blocking`** — a
plain blocking-pool thread, never Tauri's async worker (where `block_on` would
panic on a nested runtime). The store is opened **per operation** (a LanceDB
directory connection is cheap), not held in managed state.

### Two new IPC commands on the stable, always-registered surface

- `rag_index_document(doc_id, text, params)` → count of rows written.
- `rag_search(query, top_k)` → `RagHitDto[]` across the whole corpus.

On a build **without** `vectordb` both resolve to the actionable rebuild-hint
error stub (like the B4 `embedded_build_steps` / `semantic_search` stubs) — the
corpus is a `vectordb`-only capability with no deterministic fallback, so it
errors rather than no-ops (unlike `confirm_classification`, which degrades to the
deterministic verdict).

## Alternatives considered

1. **Keep B4's in-memory cosine, just iterate every saved document at query
   time.** Pro: no new store, no persistence semantics. Con: re-embeds (or re-loads
   + re-ranks) the entire corpus on every search, scaling linearly with the library
   and discarding the embeddings each time; no ANN. **Rejected** — this is exactly
   the corpus-scale case ADR-0004 said the index is *for*.
2. **A second/own vector DB (e.g. a hand-rolled flat file, or sqlite-vss).** Con:
   ADR-0001 already chose LanceDB via `momusdev_llm`, and the `vectordb` feature
   already links it; adding a *different* store would duplicate the dependency and
   contradict the named stack. **Rejected.**
3. **Store node ids bare (no doc prefix) and add a `doc_id` column only.** Con: the
   LanceDB primary id would collide across documents (`char:mara` in two novels →
   one row clobbers the other on upsert). The composite key makes the row id itself
   corpus-unique; the metadata columns are the convenience, not the uniqueness
   guarantee. **Rejected.**
4. **Hold an open `RagStore` (and its runtime) in Tauri managed state.** Con: a
   directory connection is cheap to open, and a long-lived owned Tokio runtime in
   managed state complicates the `#[cfg]` matrix for marginal benefit. **Rejected** —
   open per operation inside the existing `spawn_blocking`.
5. **Block on the async `VectorStore` directly from Tauri's async runtime.**
   **Forbidden** — `block_on` inside the async worker panics on a nested runtime.
   The owned runtime + `spawn_blocking` is the correct bridge.
6. **Patch `momusdev_llm` to expose a sync store API.** **Forbidden:** the sibling
   crate is read-only. The sync wrapper lives entirely on our side.

## Consequences

- (+) Search now spans the **whole ingested corpus** and **survives restarts** —
  the RAG capability ADR-0001 named and ADR-0004 deferred is delivered.
- (+) Reuses the existing embedder, the existing `vectordb` feature, and the
  already-linked LanceDB store; **zero sibling-crate changes** (read-only).
- (+) Default/public build untouched and still native-free (ADR-0001): the shape
  logic is pure and tested by the default suite; only the store wrapper is gated.
- (+) The frontend gains two commands on the stable IPC surface; without `vectordb`
  they return the actionable rebuild hint, like the other gated stubs.
- (−) `vectordb` pulls heavy deps (lancedb/arrow/fastembed) — but it already did
  for B4; this ADR adds only an optional `tokio`.
- (−) Per-ADR-0008, `vectordb` and `cuda` can't co-link on this machine (VS 2019
  ORT-STL conflict), so the corpus runs CPU-side under the normal launcher; the GPU
  path stays `llm,cuda`. Unchanged by this ADR.
- (−) Corpus invalidation is coarse: re-indexing a document upserts its rows by
  composite key (so a re-index overwrites in place), but deleting a document does
  not yet prune its rows. Logged as a future Catch-all, not built here.

## Invariant (pinned)

- **Shape logic is pure + native-free:** key, split, the `(graph, embeddings) →
  records` join, and the result DTO are tested by the default `cargo test`. Pinned
  by `crates/doctree-llm/src/lib.rs::tests::{rag_db_dir_prefers_the_env_var,
  rag_storage_key_is_corpus_unique_and_splits_back,
  graph_rag_records_join_nodes_with_their_vectors}` and
  `src-tauri/src/llm.rs::tests::{rag_commands_are_actionable_errors_without_vectordb,
  rag_hit_dto_maps_every_field_and_serializes_camel_case}`.
- **Round-trip across documents (gated):** the store persists two documents'
  `char:mara` as distinct rows and returns the nearer one first, skipping a
  wrong-width vector. Pinned by `crates/doctree-llm/src/lib.rs::tests::
  rag_store_round_trips_records_across_documents` (`#[cfg(feature = "vectordb")]`).
  This is the agent-side proof; the live embedder run (real ONNX vectors over a
  real library) is the user's desktop end test.
- **Native-free default build:** `cargo test --workspace` (no features) stays green
  with **zero** native deps; `vectordb` (and its `tokio`) must never enter the
  default build (ADR-0001).
- **Read-only siblings:** the persistent RAG required **no** edit to `momusdev_llm`
  / `momusdev_met`; `RagStore` consumes the published `VectorStore` /
  `TaskProposal` / `QueryResult` as-is. If a future RAG need can't be met by the
  published API, flag it under Catch-all — do not patch the crate.

## Anchors

- `crates/doctree-llm/src/lib.rs` — `RAG_DB_ENV`, `rag_db_dir`, `rag_storage_key`,
  `split_rag_key`, `RagRecord`, `graph_rag_records`, `RagHit` (native-free) +
  gated `mod rag` / `RagStore` (`open` / `index` / `search`) + tests.
- `crates/doctree-llm/Cargo.toml` — `vectordb` gains the optional `tokio` dep.
- `crates/doctree-core/src/schema.rs` — `NodeKind::tag()` (the snake_case kind tag
  stored as `source_type`) + its serde-lock test.
- `src-tauri/src/llm.rs` — `RagHitDto` + `From<doctree_llm::RagHit>` (pure) + gated
  `rag_index_document` / `rag_search` commands + native-free stubs.
- `src-tauri/src/lib.rs` — both commands registered in `generate_handler!`.
