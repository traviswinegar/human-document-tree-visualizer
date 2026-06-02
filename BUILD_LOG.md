# BUILD_LOG — human-document-tree (working-memory ledger)

> _"The document is a hypothesis. The code is truth."_
> When they disagree, the code wins and this log gets corrected to match.

This is the **COMPACTION_RECOVERY ledger** for the autonomous build run started
2026-06-01. The user authorized an uninterrupted overnight build ("don't stop,
plow on through, I'll test when finished") and will test at the end. Maintain
this file religiously: update **Current Position** optimistically at the start of
each step; append a **verification triple** (commit hash + test path + source
`file:line`) only after the commit lands.

---

## Hard constraints (do NOT violate, even running autonomously)

1. **Commit locally, never push.** The user owns all publishing. No `git push`,
   no PR, no tags.
2. **Do not modify the `momusdev_llm` sibling crate** (`E:\Development\momusdev-packages\momusdev_llm`).
   It is shared by the user's other apps. If a change there is needed (e.g. the
   `complete_with_grammar` visibility item from ADR-0001), route around it / gate
   behind a feature flag and log it under Catch-all for the user's sign-off.
3. **Test-first per fix; atomic commits with a `Phase N #M:` subject.**
4. **Decouple:** the default build must never depend on native LLM/GPU/vectordb.
   The frontend + deterministic pipeline build and test with zero native deps.

---

## Resolved environment facts (Phase 0 spike — detected 2026-06-01)

- **Rust** 1.93.1, **cargo** 1.93.1. **Node** v22.13.0, **npm** 11.3.0 (no pnpm).
- **Tauri CLI not installed** at run start (scaffolding manually / via npm).
- **GPU backend = CUDA.** NVIDIA RTX 3060 Ti (8 GB), driver 595.97, **CUDA
  Toolkit 13.1** (`nvcc` present). CUDA 13 is very new — if `llama-cpp-2 0.1`
  fails to build against it, fall back to CPU-only `inference` (no `cuda`
  feature) or Vulkan. Resolves ADR-0001 open item #3's GPU choice.
- **C++ toolchain:** Visual Studio Community 2026, MSVC 14.50.35717 + bundled
  CMake at `E:\Program Files\Visual Studio\`. NOT on the default PATH — native
  builds must run from a VS Developer environment (vcvars64) or with cmake/cl
  added to PATH.
- **GGUF models already on disk** (Qwen3-class, what `momusdev_llm` expects):
  - `qwen3-4b-q4km.gguf` (2.33 GB) — **default for the spike** (speed/quality).
    Path: `C:\Users\travi\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\com.example\webforge\webforge\models\qwen3-4b-q4km.gguf`
  - `qwen2.5-0.5b-q4km.gguf` (0.46 GB) — fast smoke-test fallback (same dir).
  - `qwen3.5-9b-q4km.gguf` (5.29 GB) — best quality, tighter VRAM fit.
  - (`ggml-vocab-*.gguf` under DewLogic are tokenizer-only test files — NOT usable
    for completion.)
  Model path is configurable via env `DOCTREE_MODEL_PATH`; not hardcoded as the
  permanent answer (it currently lives under another app's AppData).
- **momusdev_llm** crate confirmed at
  `E:\Development\momusdev-packages\momusdev_llm\Cargo.toml`.

---

## Architecture decided for the build (so it's decoupled + testable)

Cargo **workspace** at repo root:

- `crates/doctree-core/` — pure Rust, **zero native deps**: graph schema
  (node/edge types), the GBNF grammar text, the deterministic structure walker,
  fixtures. Fully `cargo test`-able anywhere.
- `crates/doctree-llm/` — `momusdev_llm` integration, **optional**, gated. Only
  compiled when the `llm` feature is on. Isolates llama-cpp-sys-2 / CUDA so a
  failed native build never blocks the rest.
- `src-tauri/` — Tauri v2 app crate. Depends on `doctree-core` always, on
  `doctree-llm` only under feature `llm`. Exposes commands + streaming events.
- Frontend: **Vite + TypeScript + `3d-force-graph`** (wraps three.js) in `src/`.

---

## Walk plan (build order — Stream A is native-free, runs first/always)

| # | Phase | Unit | Acceptance (test) |
|---|-------|------|-------------------|
| A1 | 0 | Cargo workspace + `doctree-core` skeleton + first failing test | `cargo test -p doctree-core` runs |
| A2 | 1 | Graph schema (node/edge enums + JSON) + ADR-0002 | schema (de)serializes; fixtures validate |
| A3 | 1 | GBNF grammar for `{nodes,edges}` (text + compile check) | grammar parses; rejects malformed by construction |
| A4 | 2 | Deterministic structure walker (tokenize→segment→clause→reference→co-occur) | fixture doc → deterministic spine; same in→same out |
| A5 | 0/3 | Frontend scaffold: Vite+TS+3d-force-graph | dev server boots; renders fixture graph |
| A6 | 3 | 3D render: color-by-type, force layout, orbit/zoom/pan, search, drag, animated edges, reset | all interactions work on fixture |
| A7 | 4 | Live animated build + replay (Tauri events stream spine; record→replay) | watch build; replay reproduces ordered build; explore after |
| A8 | 0/4 | Tauri command runs walker on a real doc and streams to frontend (no LLM) | end-to-end doc→animated graph, no LLM |
| B1 | 0 | `doctree-llm` crate: momusdev_llm optional dep; resolve `complete_with_grammar` visibility (READ); attempt CUDA build (background) | crate builds under `--features llm`; `cargo test` green w/o it |
| B2 | 0 | Tauri command calls `complete_chat_direct` w/ qwen3-4b → returns text to frontend | ADR-0001 acceptance: inference round-trip |
| B3 | 5 | LLM semantic layer: grammar-constrained extraction merged onto spine | fixture fiction → characters/events/edges; schema-valid |
| B4 | 5 | Embeddings → similarity edges + search (vectordb, gated separately) | similarity edges added; search works |
| B5 | 6 | Document-type detection runtime gate (classify → route to narrative) | classifies narrative vs not on fixtures; routes |

Interleave: kick the long CUDA compile (B1) in the background early; do Stream A
while it compiles.

---

## Current Position

**B1 — stand up `crates/doctree-llm`: optional `momusdev_llm` dependency behind a
`llm` feature (cuda/vulkan/vectordb sub-features), a thin `InferenceEngine`
wrapper, and a default workspace build that stays green with zero native deps.**
Stream A native-free core complete (A1–A4); kicking the long CUDA compile in the
background per the interleave plan.

---

## Completed entries (verification triples — append only after commit)

- **A1** — Cargo workspace + `doctree-core` skeleton.
  Commit `1fe0130` · test `crates/doctree-core/src/lib.rs::tests::crate_builds_and_test_harness_runs` (`cargo test -p doctree-core`) · src `crates/doctree-core/src/lib.rs:10` (`CRATE_NAME`).
- **A2** — graph schema (narrative ontology) + shared fixture.
  Commit `d050b88` · tests `crates/doctree-core/src/schema.rs::tests::*` (8) + `crates/doctree-core/tests/fixture_loads.rs::sample_narrative_fixture_is_schema_valid` · src `crates/doctree-core/src/schema.rs` (`Graph`, `NodeKind`, `EdgeKind`, `Provenance`), fixture `fixtures/sample-narrative.graph.json`.
  Note: ADR-0002 deferred to A3 close so it covers schema + grammar as one immutable record.
- **A3** — GBNF extraction grammar (semantic subset) + deterministic linter + ADR-0002.
  Commit `1de780d` · tests `crates/doctree-core/src/grammar.rs::tests::*` (7, incl. `grammar_is_subset_of_schema`, `lint_*`) · src `crates/doctree-core/src/grammar.rs` (`GRAPH_GBNF`, `graph_grammar`, `lint_gbnf`), ADR `docs/adr/ADR-0002-graph-schema-and-gbnf-grammar.md`.
  Open risk: `inference.rs` warns the GBNF parser rejects ~5+ alternatives in one production; the 6-alt `nodekind`/`edgekind` rules must be re-verified at first real grammar compile (B2).
- **A4** — deterministic structure walker (the graph spine).
  Commit `81805da` · tests `crates/doctree-core/src/walker.rs::tests::*` (13, incl. `walk_is_deterministic`, `spine_graph_is_referentially_valid`) + `crates/doctree-core/tests/fixture_loads.rs::walking_the_sample_text_yields_a_valid_nontrivial_spine` · src `crates/doctree-core/src/walker.rs` (`walk`, `walk_with`, `WalkOptions`).

---

## Catch-all backlog (off-topic discoveries — provenance noted, never fixed inline)

- _(none yet)_

---

## Recovery protocol (for the post-compaction self)

On any session start: read [`CLAUDE.md`](CLAUDE.md), then this file top to bottom.
Find **Current Position**. Walk back every prior **Completed entry**: confirm the
commit exists (`git log --oneline | grep <id>`), the named test passes
(`cargo test <path>` / frontend runner), and the source `file:line` matches the
claim. The first entry that fails is the real position — discard the in-progress
assumption and restart that step from scratch (reset-point rule: restart the whole
step, never a guessed midpoint). If all prior entries verify clean, restart the
named Current Position step. Full protocol:
[`docs/agent-protocols/COMPACTION_RECOVERY.md`](docs/agent-protocols/COMPACTION_RECOVERY.md).
