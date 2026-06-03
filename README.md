# human-document-tree

**Watch a document become a mind-map.** A desktop (and browser) app that reads a
human-written document from word one, extracts its structure and meaning, and
renders them as a navigable, **animated 3D graph** that grows as it parses — then
becomes explorable, searchable, and replayable.

Extraction is a **hybrid**: a deterministic structure walker builds the graph's
spine (sections → paragraphs → sentences → clauses, quotes, references, key terms),
and an **optional local LLM** adds a grammar-constrained semantic layer (characters,
places, concepts, events, and their relationships). **No cloud calls** — when the
semantic layer is enabled, the model runs entirely on your machine.

> **Status:** active development, pre-release (`0.1.0`). Built in the open,
> *substrate-first* (see [Development](#development)).

## What it does

- **Animated 3D graph** — a force-directed [three.js](https://threejs.org) graph
  (`3d-force-graph`); nodes and edges stream in as the document is walked, then you
  orbit / zoom / search / select and explore. The build is recorded and **replayable**.
- **Hybrid extraction** — a deterministic spine plus an optional local-LLM semantic
  overlay, merged onto one graph.
- **Document-aware routing** — classifies the document (narrative / expository /
  structured) and picks the matching pipeline.
- **Ingestion** — plain text, Markdown, and **PDF** (with **OCR** for scanned pages).
- **A library** — save / load / export / import graphs (desktop files, or browser
  IndexedDB on the web build).
- **Find by meaning** — embedding-similarity search over the graph (optional).
- **A reversible graph tokenizer** — encodes the `(document, graph)` pair into one
  integer-id token stream that losslessly reconstructs *either* the **byte-exact
  original document** *or* the **exact graph**, with a "Reconstruct" view + a
  byte-level diff.
- **Two front-ends, one engine** — a single double-click desktop executable (Tauri),
  or the browser (the same Rust walker compiled to **WebAssembly**).

## How it works

A Rust workspace + a web frontend, deliberately decoupled so the **default build has
zero native / LLM / GPU dependencies** and compiles and tests anywhere:

| Crate / directory | Role |
|---|---|
| `crates/doctree-core` | Pure Rust: graph schema, the deterministic structure walker, the reversible tokenizer. No native deps. |
| `crates/doctree-llm` | The optional local-LLM semantic layer + embeddings — **feature-gated** (`llm`, `vectordb`, `cuda`/`vulkan`). |
| `crates/doctree-wasm` | The core walker compiled to WebAssembly for the browser build. |
| `crates/corpus-export` | Research tool: export a corpus into the tokenizer-bench datasets. |
| `src-tauri/` | The Tauri (desktop) app — commands + streamed build events. |
| `src/` | The web frontend — Vite + TypeScript + `3d-force-graph` (three.js). |
| `research/tokenizer-bench/` | The graph-aware-tokenization training experiment (Python / PyTorch). |

The optional local model is the `momusdev_llm` Rust crate (GGUF inference via
`llama-cpp-2`, grammar-constrained JSON decoding, embeddings + a LanceDB vector
store). It is **optional and not bundled here** — the default build never touches it.

## Build & run

Prerequisites: **Rust** (stable) and **Node 18+**. The default build needs nothing else.

```bash
npm install
npm run dev          # web build (browser): Vite dev server; walks via WebAssembly
npm run tauri dev    # desktop app (Tauri): structural pipeline
```

Tests + production build (native-free, runs anywhere):

```bash
cargo test --workspace   # Rust: walker, tokenizer, schema, …
npm run build            # tsc + Vite production build
```

### Optional: the local-LLM semantic layer

The semantic layer and embeddings are **feature-gated** and require the
`momusdev_llm` crate (a separate, not-yet-public dependency — see the note below)
plus a C++ toolchain (MSVC + CMake) to compile `llama.cpp`:

```bash
cargo build -p doctree-tauri --features llm,vectordb   # CPU inference + embeddings
# Windows: scripts/build-desktop.cmd builds a standalone full-capability .exe
```

A GGUF model is discovered automatically next to the executable, in
`<exe>/models/`, or in the app's data dir (see `docs/adr/ADR-00014`); no env var
required.

> **Note for external builders:** `doctree-llm` currently path-depends on a local
> `momusdev_llm` crate that is not included in this repository. Only the **default,
> native-free** build is reproducible from a fresh clone today; the gated `llm` /
> `vectordb` features need that crate present.

## Reversible tokenizer & research

The tokenizer ([ADR-00013](docs/adr/ADR-00013-reversible-graph-tokenizer.md)) orders
the `(document, graph)` pair into one integer-id stream over a fixed vocabulary with
a 256-value **byte floor** that guarantees losslessness — it reconstructs the
byte-exact document *or* the exact graph from the same stream.

That tokenizer is also the subject of an ongoing experiment in
[`research/tokenizer-bench/`](research/tokenizer-bench/README.md): *does graph-aware
tokenization help a small language model more than sub-word BPE?* Three arms — **BPE**,
**raw byte**, and **byte + graph markers** — are trained from scratch on the same
corpus and compared fairly in **bits-per-byte**. *Preliminary, honest finding:* with
the text-copy leak removed, graph-structural markers give byte-level modeling only a
**marginal** edge over raw bytes, and BPE still leads. A clean, modest result — which
is exactly what the experiment was built to be able to report. (See
[`docs/plans/PLAN-phase8-tokenizer-training.md`](docs/plans/PLAN-phase8-tokenizer-training.md).)

## Repository layout

```
crates/doctree-core/      pure-Rust engine (walker, schema, tokenizer)
crates/doctree-llm/       optional local-LLM + embeddings (feature-gated)
crates/doctree-wasm/      browser build of the walker (WebAssembly)
crates/corpus-export/     research dataset exporter
src-tauri/                Tauri desktop app
src/                      web frontend (3D graph, TypeScript)
research/tokenizer-bench/ graph-tokenization training experiment (Python)
docs/adr/                 Architecture Decision Records (immutable)
docs/plans/               implementation plans
CHANGELOG.md              narrative changelog (the "why")
```

## Development

This project is built **substrate-first**: load-bearing decisions are captured as
immutable **ADRs** (`docs/adr/`), fixes begin with a failing test, commits are atomic
with phase/issue ids, and the **[CHANGELOG](CHANGELOG.md)** is a narrative of *why*
things changed rather than a dump of commit subjects. The default
`cargo test --workspace` is a standing native-free gate. See `CLAUDE.md` for the full
operating discipline.

## License

[MIT](LICENSE) © 2026 Travis Winegar.
