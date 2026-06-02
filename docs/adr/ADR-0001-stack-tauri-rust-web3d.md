# ADR-0001 — Stack: Tauri (Rust backend + web 3D frontend)

- **Status:** Accepted
- **Date:** 2026-06-01
- **Deciders:** Travis (user), agent
- **Supersedes:** —

## Context

`human-document-tree` walks a human-written document and renders its
clauses / characters / concepts / references / events and their relationships as a
navigable, searchable **3D graph** that animates as it grows, then becomes
explorable with replay.

Two hard requirements shape the stack:

1. **The 3D graph is the product.** It must be navigable, searchable, with
   animated edges, drag-to-reorder, and reset-view — i.e. an interactive
   force-directed graph at non-trivial node counts.
2. **A local LLM does the fuzzy semantic extraction.** The user supplied one: the
   `momusdev_llm` crate in `E:\Development\momusdev-packages`. It is a Rust crate
   providing on-device GGUF inference (via `llama-cpp-2`), token streaming,
   embeddings + a LanceDB vector store (RAG), and **grammar-constrained (GBNF)
   decoding**. Extraction is a hybrid: deterministic structure-walking for the
   spine, LLM (grammar-constrained JSON) for the semantic layer.

The tension: the local LLM is Rust, and the rest of the user's `momusdev`
ecosystem is Flutter — but Flutter's 3D story is weak relative to the web's
mature force-directed-graph ecosystem (three.js, `3d-force-graph`), and the graph
is the core feature.

## Decision

Build the app with **Tauri**: a **Rust backend** that consumes the `momusdev_llm`
crate directly as a Cargo dependency, and a **web frontend** that renders the 3D
graph with the three.js / `3d-force-graph` family.

- The Rust backend exposes Tauri commands (and Tauri events for streaming) that
  wrap `momusdev_llm`'s `TaskEngine`.
- We do **not** use `momusdev_bridge` / flutter_rust_bridge — that exists to bind
  the engine to Flutter. In Tauri we call the crate's Rust API directly.
- The frontend owns rendering, layout, interaction, and the live-build/replay
  animation; the backend owns parsing + inference and streams nodes/edges out.

## Alternatives considered

1. **Flutter + Rust FFI (reuse `momusdev_bridge`).** Pro: consistent with the
   user's other `momusdev` apps; the bridge already exists. Con: the core feature
   (interactive 3D graph) is the weakest thing to build in Flutter — would likely
   end up embedding a WebView running three.js anyway, i.e. the web stack with
   extra layers. Rejected: friction concentrated exactly on the product's core.
2. **Web UI + standalone local Rust server (HTTP/WebSocket).** Pro: maximal
   decoupling; graph can be developed in isolation against a mock. Con: two
   processes to ship, launch, and version; IPC and lifecycle overhead for what is
   fundamentally one desktop app. Rejected for shipping; the dev-time decoupling
   benefit is recovered under Tauri by keeping the command surface clean.
3. **Tauri (chosen).** Single desktop binary; Rust crate compiles straight in;
   web frontend gets the best 3D ecosystem; streaming maps cleanly onto Tauri
   events. Accepted.

## How `momusdev_llm` is reused

Verified by reading the crate (anchors below):

- **Inference + lifecycle:** `TaskEngine::load_llm(path, ctx, n_threads,
  n_gpu_layers)` loads a GGUF model from disk; GPU backend is a build-time feature
  (`cuda` / `vulkan` / `metal` / `rocm`), mutually exclusive.
- **Chat:** `complete_chat` (RAG-enriched), `complete_chat_direct` (no RAG, with
  metrics), `complete_chat_streaming` (token pieces over an mpsc channel → maps to
  Tauri events for the live build).
- **Grammar-constrained JSON:** the reusable primitive is
  `InferenceEngine::complete_with_grammar(prompt, grammar, max_bytes)`, which
  accepts an **arbitrary** GBNF grammar. We will author our own grammar for the
  document-graph schema (`{nodes:[…], edges:[…]}`).
- **Embeddings + vector store** (feature `vectordb`): semantic-similarity edges
  and graph-wide search, local.

### Important limitation (do not misuse)

The crate's `command_grammar::build_command_grammar` is **not** a general JSON-schema
grammar generator — it is specialized for the weather app's command bar: it emits
a `{"command","args"}` envelope, supports **at most one arg per command**, and is
capped at **≤ 4 commands** (a GBNF-parser ceiling noted in its source). We will
**not** use it for node/edge extraction. We use the lower-level
`complete_with_grammar` primitive with a grammar we write.

## Consequences

- Frontend and backend are separately testable: the graph renderer can be
  developed against fixture node/edge JSON with no LLM in the loop; the extraction
  pipeline can be tested headless with `cargo test`.
- The GBNF grammar for our schema becomes a load-bearing artifact — it is the
  deterministic contract that tames the non-deterministic model. It deserves its
  own ADR when designed.
- A GGUF model file is **not** bundled in `momusdev_llm`; it is loaded from disk at
  runtime. Model selection/acquisition is a Phase-0 concern (the crate has
  `asset_manager` + `catalog` modules for downloads/integrity).
- GPU backend is a build-time choice. On this Windows machine that's CUDA (if
  NVIDIA present) or Vulkan (cross-vendor). To be settled in Phase 0.

## Open verification items (Phase 0 spike — resolve before relying on these)

1. Is `InferenceEngine::complete_with_grammar` `pub` (callable from an external
   crate), or only `pub(crate)`? If the latter, add a thin generic
   `TaskEngine::extract_with_grammar(system_prompt, user_message, grammar,
   max_bytes)` to `momusdev_llm` (a small, upstreamable addition).
2. Engine boot path from a non-Flutter host: `init_global` (with `vectordb`)
   requires an `EmbedText` embedder + a db path; `new_llm_only` skips vectordb.
   Confirm the embedder construction (fastembed) and the model-download flow via
   `asset_manager` / `catalog`.
3. Confirm the crate builds on Windows with `features = ["vectordb", "inference",
   <gpu>]` (llama-cpp-sys-2 native build + chosen GPU toolchain).

## Anchors

- `E:\Development\momusdev-packages\momusdev_llm\Cargo.toml` — features `inference`,
  `vectordb`, `cuda|vulkan|metal|rocm`; `llama-cpp-2` dependency.
- `…\momusdev_llm\src\engine.rs` — `load_llm`, `complete_chat`,
  `complete_chat_direct`, `complete_chat_streaming`, `extract_command`
  (calls `complete_with_grammar`).
- `…\momusdev_llm\src\command_grammar.rs` — `build_command_grammar` (the narrow,
  command-bar-specific generator we will NOT reuse for node/edge output).
- `…\momusdev_llm\src\lib.rs` — re-exports `TaskEngine`, `InferenceEngine`.
- `…\momusdev_bridge\src\chat.rs` — the flutter_rust_bridge surface we are NOT
  using under Tauri.
