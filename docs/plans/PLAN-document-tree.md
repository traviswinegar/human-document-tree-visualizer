# Implementation Plan — human-document-tree

> Short-lived plan doc (AgentDNA: plan-before-implementation). Archive when the
> work ships. The kernel + discipline live in [`CLAUDE.md`](../../CLAUDE.md);
> stack rationale in [ADR-0001](../adr/ADR-0001-stack-tauri-rust-web3d.md).

## Goal

Feed in a human-written document; starting at word one, determine its
clauses/ideas/characters/concepts/references/events and their relationships, and
render them as a navigable, searchable 3D graph that **animates as it grows**,
then becomes **explorable**, with **replay**. Extraction is hybrid: deterministic
structure-walk + local-LLM semantic layer (grammar-constrained JSON). Phase-1
genre = **narrative/fiction**.

## Sequencing note (runtime-first vs build-first)

The user wants the app to **detect document type first** at runtime and route to a
genre extractor. In *build* order we still implement one genre pipeline
(narrative) before the router, so the router has something to route to. Doc-type
detection lands once the narrative pipeline exists, then becomes the runtime
entry gate.

## Cross-repo caution

`momusdev_llm` is a **shared sibling package** used by the user's other apps. Any
change to it (e.g. adding a generic `extract_with_grammar`) is a cross-repo edit —
confirm with the user before modifying it, and keep additions minimal/additive.

---

## Phases

### Phase 0 — Engine integration spike + scaffold
Resolve the ADR-0001 open verification items and get one inference round-trip end
to end.
- Tauri app skeleton (Rust backend + web frontend), committed.
- Add `momusdev_llm` as a Cargo dependency with `features = ["vectordb",
  "inference", <gpu>]`; confirm it builds on Windows (llama-cpp-sys-2 + GPU
  toolchain — CUDA or Vulkan).
- Resolve: (1) `complete_with_grammar` visibility; (2) engine boot path
  (`init_global` vs `new_llm_only`, embedder construction, model acquisition via
  `asset_manager`/`catalog`); (3) pick + load a small GGUF model from disk.
- **Acceptance:** a Tauri command runs `complete_chat_direct` and returns text to
  the frontend; `cargo test` green; GPU backend decided and building.

### Phase 1 — Graph schema + GBNF grammar (narrative ontology)
- Define node types (Character, Place, Concept, Event, Passage/Clause, Reference,
  …) and edge types (mentions, interacts-with, located-in, references,
  precedes/temporal, part-of, …) for fiction.
- Define the extraction JSON schema and author the **GBNF grammar** that
  constrains `{nodes:[…], edges:[…]}` to it. (Schema + grammar → ADR-0002.)
- **Acceptance:** grammar compiles via `complete_with_grammar`; a fixture passage
  round-trips into schema-valid `{nodes,edges}` JSON; grammar rejects malformed
  output by construction.

### Phase 2 — Deterministic structure walker (no LLM)
- Tokenize → sentence/paragraph/section segmentation → rule-based clause splitting
  → reference/citation/quote detection → repeated-term + co-occurrence. Produces
  the reproducible structural **spine** of the graph.
- **Acceptance:** a fixture doc yields a deterministic spine graph (same input →
  same output); unit tests cover segmentation, clause splitting, reference
  detection.

### Phase 3 — 3D graph rendering (frontend, against fixtures)
- three.js / `3d-force-graph`: nodes/edges colored by type, force layout,
  navigate (orbit/zoom/pan), search, drag-to-reorder, animated edges, reset view.
- Built against fixture node/edge JSON — no LLM in the loop (decoupling per
  ADR-0001).
- **Acceptance:** fixture graph renders; all listed interactions work; acceptable
  perf at target node counts (set a number in Phase 3).

### Phase 4 — Live animated build + replay
- Backend streams nodes/edges as it walks the document (Tauri events, using
  `complete_chat_streaming` + incremental structural walk). Frontend animates
  growth; the build event sequence is recorded for **replay**. Explore mode after
  build completes.
- **Acceptance:** watch a doc build into the graph; replay reproduces the same
  ordered build; explore mode active post-build.

### Phase 5 — LLM semantic layer (narrative/fiction)
- Grammar-constrained extraction adds characters/concepts/events/relationships and
  coreference-style linking on top of the structural spine; merge into the graph.
  Embeddings → similarity edges + graph-wide search.
- **Acceptance:** a fixture fiction doc yields characters/events/relationships as
  nodes/edges; results reproducible enough (cached/seeded) and schema-validated.

### Phase 6 — Document-type detection (the runtime "preface")
- Detect genre (LLM-classify + cheap heuristics) and route to the genre extractor.
  Narrative is the only route initially; later genres (legal/academic/general)
  plug in here.
- **Acceptance:** classifier identifies narrative vs non-narrative on fixtures and
  routes correctly; becomes the runtime entry gate.

---

## Backlog / catch-all
Off-topic discoveries get logged here with provenance (`discovered while Phase X
#N`), never fixed inline.

- _(none yet)_

## Working-memory ledger
Once Phase 0 starts, maintain Current Position + completed-step verification
triples here (or in a dedicated `BUILD_LOG.md`) per
[`docs/agent-protocols/COMPACTION_RECOVERY.md`](../agent-protocols/COMPACTION_RECOVERY.md).

- **Current Position:** substrate scaffolded; awaiting user go-ahead to start
  Phase 0.
