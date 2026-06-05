# PLAN — Phase 9: whole-document semantic extraction

> Implementation plan for [ADR-00017](../adr/ADR-00017-chunked-whole-document-semantic-extraction.md),
> resolving the long-doc context-starvation Catch-all (#71). Archived to
> `docs/plans/archive/` when it ships.

## Goal

Make the LLM semantic layer read the **whole** document, not just the opening ~4 KB:
chunk the spine into batch-safe windows, extract from each, and merge — unifying
entities that recur across chunks — so a long document yields characters, places,
and events across **every** chapter.

## Standing invariants

- **`momusdev_llm` is read-only** — each chunk's prompt must stay under
  `PROMPT_DOC_BUDGET_BYTES` (the 2048-token batch ceiling, #78).
- **Native-free default** — the chunker and entity resolution are pure logic, tested
  under `cargo test --workspace` (no features); only the per-chunk LLM calls are gated.
- Test-first for the pure pieces; atomic commits with `Phase 9 #M:` subjects; the
  two-commit (code + ledger) pattern.

## Milestones

### M1 — Native-free spine chunker (test-first)
- Refactor `build_extraction_prompt` to build a prompt from an **arbitrary node
  subset** (so a chunk's window can be prompted).
- Add `chunk_spine(spine) -> Vec<Chunk>`: partition the anchorable (Section/Sentence)
  nodes, in `span` order, into windows each fitting `PROMPT_DOC_BUDGET_BYTES`.
- **Pinned tests:** chunks fit the budget; partition the anchors exactly once in
  order (no gap/overlap); deterministic; a single oversized anchor gets its own chunk.
- **Done when:** the chunker + per-subset prompt are green native-free.

### M2 — Cross-chunk entity resolution (test-first)
- A native-free helper that, given a semantic fragment, rewrites each semantic-node id
  to `slug(kind, normalized-label)` (kind-prefixed) and rewrites edge endpoints to
  match. Only the six semantic kinds; spine ids untouched.
- **Pinned tests:** two fragments naming the same entity (same kind+label, different
  ids) → after canonicalize + `merge`, **one** node with the **union** of edges; spine
  authoritative; canonical ids never collide with spine ids.
- **Done when:** canonicalize + merge unify cross-chunk entities, green native-free.

### M3 — Chunked extraction driver (gated)
- `semantic_build_steps` loops chunks → `extract_graph_json` per chunk → canonicalize
  → `merge_semantic_onto_spine`, accumulating onto the spine.
- A **failed chunk is skipped** (logged), never aborting the pass.
- A configurable **`max_chunks` / extraction-budget** cap (env or param) — default
  covers the whole document; a small cap gives a fast partial pass.
- **Done when:** a multi-chunk doc extracts across all windows and merges to one graph.

### M4 — Streaming + frontend progress
- Stream each chunk's fragment onto the live graph via the existing `pendingDelta`
  path; surface **chunk *i*/*N*** progress in the status/routing line.
- **Done when:** entities appear window-by-window during the build; progress reads.

### M5 — Verify + ship
- Native-free gate green; `tsc`/`vite` green. The **live multi-chunk run over a long
  document** (the Anunnaki corpus) on the real engine is the user's desktop end test —
  confirm Character/Event/Place nodes now span the whole doc.
- Two-commit ledger; archive this plan.

## Key risks (and mitigations)

| Risk | Mitigation |
|------|------------|
| Latency — N inference calls on a long doc | Stream per-chunk; `max_chunks` cap; full pass is opt-in/background. |
| Slug dedup merges distinct same-named entities / misses variants | Acceptable v1; ADR-00017 alt 4 (embedding coref) is the refinement. |
| A chunk's LLM output is malformed | Skip-on-error per chunk; partial-but-valid graph. |
| Edges crossing chunk boundaries (entity in ch.1, mention in ch.9) | Canonical ids make cross-chunk edges resolve to the same node; `prune_dangling_edges` drops any still-unresolved. |
| Cost balloons on huge corpora | Cap + the honest latency note; not a silent truncation. |

## Backlog (parked)

- Embedding-based entity coreference (alias/spelling variants) — ADR-00017 alt 4.
- Hierarchical extraction (per-section summaries → document-level entities) if flat
  chunking proves too local.
