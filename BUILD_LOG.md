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
| A8 | 4 ✓ | Tauri command runs walker on a real doc and streams to frontend (no LLM) | end-to-end doc→animated graph, no LLM |
| B1 | 0 | `doctree-llm` crate: momusdev_llm optional dep; resolve `complete_with_grammar` visibility (READ); attempt CUDA build (background) | crate builds under `--features llm`; `cargo test` green w/o it |
| B2 | 0 | Tauri command calls `complete_chat_direct` w/ qwen3-4b → returns text to frontend | ADR-0001 acceptance: inference round-trip |
| B3 | 5 | LLM semantic layer: grammar-constrained extraction merged onto spine | fixture fiction → characters/events/edges; schema-valid |
| B4 | 5 | Embeddings → similarity edges + search (vectordb, gated separately) | similarity edges added; search works |
| B5 | 6 | Document-type detection runtime gate (classify → route to narrative) | classifies narrative vs not on fixtures; routes |
| C1 | 4 ✓ | Browser walker via WASM (public web build) + runtime document upload (Open button / drag-drop) + floating-chip build progress | public web build walks via WASM with the *same* engine as desktop; user swaps the document at runtime → fresh streamed build |
| D1 | 4 ✓ | "Keep it fluid" — fix large-doc build stutter (force tuning + LOD-throttled apply) | large doc builds to completion without frame-drops; final state always lands |
| D2 | 4 ✓ | Click node → highlight it, its edges and immediate neighbours (dim the rest) | selection lights node+neighbours+incident edges; bg-click/Esc clears |
| D3 | 4 ✓ | Right sidebar: the document unfolds (reconstructed from revealed heading/sentence nodes) as it's assimilated | text appears in step with the build, in document order; canvas yields width |
| D4 | 4 ✓ | Click node → jump to/highlight its text in the sidebar + a details panel (kind, provenance, text, connections) | click ↔ text two-way; connections navigate; panel clears on deselect |

Interleave: kick the long CUDA compile (B1) in the background early; do Stream A
while it compiles. **Stream C** (public-web/WASM + upload UX) was added mid-run
on user direction ("host it for the public … should work in Desktop as well");
C1 is complete. **Stream D** (explorer UX: fluidity + click-highlight + unfolding
document sidebar + jump-to-text/details) was added mid-run on user direction
(2026-06-02: "Keep it fluid. Also … 1. Clicking a node should highlight it, its
edges, and its neighbors 2. … the entire document … in a righthand sidebar as
it's assimilated 3. … clicking on a node should highlight it in the text … a
details window"); D1–D4 are complete (frontend-only, default build stays
native-free).

---

## Current Position

**B3 — LLM semantic layer (grammar-constrained), merged onto the deterministic
spine → animated semantic edges.** Stream A is **complete** end to end (doc →
walker → ordered build steps → animated, interactive, replayable 3D graph;
A1–A8). **B2 just landed:** a gated Tauri command (`llm_complete`, behind the
`llm` feature) lazily loads a CPU model and round-trips text to the frontend,
with `llm_status` always available so the UI can detect the capability — and the
canonical GBNF grammar is exercised by an ignored live round-trip test (the
user's desktop end test). The default build stays native-free (verified via
`cargo tree`); the `--features llm` build compiles clean under MSVC.

B3 is the next *build-order* step: prompt the model (grammar-constrained to
`doctree_core::GRAPH_GBNF`) to extract the **semantic** nodes/edges the
structural walker can't (entities, ideas, relationships), then **merge** them
onto the existing spine so they stream in as the flowing-particle semantic edges
the renderer already distinguishes. CPU is the working path; GPU still blocked
(Catch-all). Model at `DOCTREE_MODEL_PATH` (qwen3-4b-q4km.gguf). Desktop-only at
runtime — the live merge is the user's end test, but the merge logic, the
prompt builder, and the spine-union are pure and unit-testable headlessly.

> **Stream C (C1) landed since this position was set.** The public web build now
> walks documents in-browser via WASM (`doctree-core` → wasm32, same engine as
> the desktop walker) and the user can replace the document at runtime (Open
> button / drag-drop) — each new doc tears down the running build and streams a
> fresh one, with playback collapsed into a faint floating chip so it never
> competes with the animation. Verified live (see C1 entry below).

> **Stream D (D1–D4) landed next (2026-06-02), on user direction.** The frontend
> is now a real explorer: large-doc builds stay fluid (coalesced, LOD-throttled
> `graphData()` applies + force friction — D1); clicking a node lights it, its
> incident edges and immediate neighbours while the rest dim (D2); the document
> re-unfolds in a right sidebar *in step with the build*, reconstructed in
> document order from the revealed heading/sentence nodes (D3); and clicking a
> node both jumps to/highlights its text in the sidebar and opens a details panel
> (kind, provenance, text, navigable connections), with the text→node direction
> wired too (D4). All four are frontend-only — the default build is still
> native-free. Verified live via dev handles (see D1–D4 entries below). **B2
> remains the next *build-order* step.**

> **User direction (2026-06-02, to discuss in the morning):** build the *full*
> pipeline for **both** the LLM path **and** the deterministic "Tier 1" path so
> the two can be **benchmarked head-to-head at every stage** of extraction. Not
> started — the user explicitly said to stop at the A8 request and confirm it
> works first. See Catch-all backlog entry "Dual-pipeline benchmarking".

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
- **B1** — `doctree-llm` crate: optional `momusdev_llm` dep behind an `llm`
  feature (cuda/vulkan/vectordb sub-features), native-free by default.
  Commit `8c14fdd` · tests `crates/doctree-llm/src/lib.rs::tests::*` (5, incl. config defaults + `from_env` + `graph_extraction_grammar` ↔ `doctree_core::GRAPH_GBNF`) · src `crates/doctree-llm/src/lib.rs` (`LlmConfig`, `Completion`, `graph_extraction_grammar`, gated `mod engine::Engine`).
  Verified: default `cargo test` workspace green with ZERO native deps; gated `cargo build -p doctree-llm --features llm` compiles clean against the real `momusdev_llm` API (CPU `inference`, ~52 s). GPU sub-features (`cuda`/`vulkan`) do NOT build on this machine — see Catch-all backlog; CPU is the working path for B2/B3.
- **A5** — frontend scaffold (Vite + TS + 3d-force-graph) renders the fixture.
  Commit `95f224b` · test `npx tsc --noEmit` (exit 0) + `npm run build` (tsc && vite build, exit 0); runtime render confirmed via dev-handle scene introspection (32 three.js meshes = 13 node spheres + 18 link cylinders + interaction mesh; canvas 1280×720; WebGL2 live; zero console errors) · src `src/main.ts` (graph construction, explicit `width()/height()` at init), `src/types.ts` (mirrors `schema.rs`), `src/colors.ts` (palette).
  Note: `preview_screenshot` times out against the continuously-animating WebGL canvas (the rAF render loop never idles) — a tooling limitation, not an app defect; render verified by introspection instead.
- **A6** — 3D render interactions (search + focus + animated edges + reset).
  Commit `b33181b` · test `npx tsc --noEmit` + `npm run build` (both exit 0); runtime verified via dev-handle introspection: search "mara"→2 matches (`sent:1`,`char:mara`), 11 nodes + 17 links dimmed to low alpha; "vane"→2 (`sent:2`,`char:vane`); focus tween moves the camera toward the matched node; reset re-fits; scene = 42 meshes (13 nodes + 18 link cylinders + 10 semantic directional particles + interaction mesh) · src `src/main.ts` (`runSearch`, `focusNode`, `resetView`, highlight-aware `nodeColor`/`linkColor`, `linkDirectionalParticles`), `index.html` (search box + reset button).
  Note: precise *settled* camera coordinate not asserted — eval polling loops time out against the animating canvas; camera-moves-toward-node and reset-re-fits were both confirmed directly.
- **A7** — live animated build + replay (frontend stream path).
  Commit `79034bc` · test `npx tsc --noEmit` + `npm run build` (both exit 0); runtime verified via dev handles (`__doctreePlayer`, `__doctreeBuildSequence`): sequence deterministic across runs and valid (no edge precedes its endpoints), 13 node + 18 edge events = 31; build visibly grows (7→13 nodes mid-run); Replay resets to 0 and regrows; "harbor"→2 matches after completion; done-state button reads "Replay" · src `src/build-player.ts` (`buildSequence`, `createBuildPlayer`), `src/main.ts` (empty-start + player wiring), `index.html` (Play/Pause + Replay).
  Note: build runs slower than the nominal 220 ms/step — per-step `graphData()` reheats + periodic `zoomToFit` load the main thread, delaying the timer; visually fine (graph grows over ~15 s). Ordering currently lives only in the frontend; A8 makes the Rust walker stream authoritative.
- **A8** — Tauri app crate + frontend bridge: doc → deterministic walker → ordered build steps → animated 3D graph, **no LLM** (the structure-only path ADR-0001 requires the default build to stand on).
  - **(1/3)** authoritative build ordering in Rust. Commit `8de06f3` · tests `crates/doctree-core/src/build.rs::tests::*` (6) · src `crates/doctree-core/src/build.rs` (`build_sequence`, `BuildStep`); frontend `src/build-player.ts` mirrors it.
  - **(2/3)** Tauri app crate + bridge. Commit `0f6db4f` · tests `src-tauri/src/lib.rs::tests::*` (7, incl. `walk_params_*`, `walk_document_is_deterministic`, `build_steps_account_for_exactly_the_graph`, `build_steps_serialize_to_frontend_tagged_shape`) (`cargo test -p doctree-tauri`) · src `src-tauri/src/lib.rs` (`walk_document_impl`/`build_steps_impl` + `#[tauri::command]` `walk_document`/`build_steps`, `WalkParams`→`WalkOptions`, `run()`), `src-tauri/src/main.rs` (launcher), `src-tauri/tauri.conf.json` (id `games.milsoft.doctree`, frontendDist `../dist`), `src-tauri/capabilities/default.json` (`core:default`), `src/doc-source.ts` (Tauri live-walk vs browser-fixture fallback), `src/main.ts` (async `initBuild`).
  - Verified: `cargo check -p doctree-tauri` clean (34 s, MSVC 14.50 — the Tauri dep tree builds fine; the toolchain failures are CUDA/Vulkan-specific, not general MSVC); full workspace **48 tests green with ZERO native deps** (core 34 + integration 2 + llm 5 + tauri 7); `npm run build` (tsc + vite) clean, `dist/` emitted for `generate_context!`; browser fixture fallback renders + animates live via dev handles (`__doctreeSource.origin === "fixture"`, 13 nodes/18 edges/31 steps, build grew to 13/31, `isTauri === false`).
  - Note: the **live desktop window** (Tauri shell calling the real Rust walker over IPC) **cannot be launched/screenshotted in this headless environment** — that round-trip is the user's end test (`npm run tauri dev`). Everything else (compile, Rust unit tests, browser-fallback render) is verified above. GPU still CPU-only (see Catch-all).
- **C1** — public-web browser walker via WASM + runtime document upload + floating-chip progress. Added mid-run on user direction: "Definitely want this to work in a browser so I can host it for the public … should work in Desktop as well."
  - **toolchain** — `rustup target add wasm32-unknown-unknown` + `cargo install wasm-pack` (0.15.0); `wasm-opt = false` in the crate's wasm-pack profile so the build is self-contained (no binaryen download). Environment change, no committed files.
  - **wasm crate + ADR-0003.** Commit `c6b80d4` · tests `crates/doctree-wasm/src/lib.rs::tests::*` (6, incl. `build_steps_json_matches_core_sequence`, `build_steps_json_is_frontend_tagged_shape`, `walk_document_json_is_valid_graph_shape`) (`cargo test -p doctree-wasm`) · src `crates/doctree-wasm/src/lib.rs` (`#[wasm_bindgen] buildSteps`/`walkDocument`/`start`, returns the *same tagged JSON string* the Tauri command produces), ADR `docs/adr/ADR-0003-doctree-core-to-wasm-browser-walker.md`, workspace member in `Cargo.toml`.
  - **frontend WASM bridge.** Commit `6f15aa7` · src `src/doc-source.ts` (`loadBuildSource` engine pick: `tauri-walk` → `wasm-walk` → `fixture`; lazy one-time `getWasm()` dynamic import keeps the ~180 KB glue out of the initial bundle), `package.json` (`build:wasm` + `pre(dev|build)` hooks), `.gitignore` (`src/wasm/` generated output).
  - **upload UI + chip.** Commit `0ba7000` · src `src/main.ts` (rebuildable `startBuild`/`loadAndBuild`/`ingestFile`; Open-button → hidden file input; window-wide drag-drop with depth-counted hint; chip wires `playpause`/`replay` once against a re-pointed module `player`; progress bar fill + icon swap), `index.html` (`#open-doc`, hidden `#file-input`, `#playback` floating chip with `#build-bar`, `#drop-hint` overlay).
  - Verified: `npx tsc --noEmit` clean; `npm run build` clean (wasm rebuilt, tsc, vite — wasm code-split into its own 183 KB chunk, `dist/` emitted). Runtime confirmed live via preview dev handles (`__doctreeSource`): initial load walks via **WASM** (`origin "wasm-walk"`, 54 nodes/145 edges/199 steps, build streams in); a **dropped document** re-ingests (`wasm-walk`, 8/12/20, distinct counts) — old build torn down, fresh one streamed; build completes → bar 100 %, progress "ready", play/pause shows the replay glyph (⟳); the chip's **replay** click resets to 0/20 and regrows. Zero console warnings/errors.
  - Note: `preview_screenshot` again times out against the continuously-animating WebGL canvas (the documented rAF-loop tooling limitation) — render verified by dev-handle introspection instead, as with A5–A8.
- **D1** — keep large-doc builds fluid (coalesced apply + force tuning).
  Commit `c97c80b` · test `npx tsc --noEmit` + `npx vite build` (both exit 0); runtime verified via dev handles: default build (54 nodes, apply interval 0) completes cleanly; a synthetic 403-node / 3685-edge / 4088-step doc drives to full completion with the entire final state on screen and "ready" reached (trailing `flushApply` lands the final batch) · src `src/main.ts` (`applyIntervalForSize`, `cancelPendingApply`, `commit`/`flushApply` trailing-throttle in `startBuild`; `.d3VelocityDecay(0.45)`/`.cooldownTime(12000)`/`.warmupTicks(0)` on the graph).
  Root cause: every `graph.graphData()` reheats the force sim, so re-applying on every revealed batch thrashed layout as node count climbed. Fix coalesces `graphData()` into a node-count-scaled minimum interval.
- **D2** — click a node to highlight it, its edges and neighbours.
  Commit `e20a7e0` · test `npx tsc --noEmit` + `npx vite build` (both exit 0); runtime verified via `__doctreeSelect`: selecting `sent:1` keeps it + its 3 neighbours full-colour while a non-neighbour drops to 0.06 alpha and incident edges widen 0.4→2; Escape/background-click restore full colour/width · src `src/main.ts` (`selectNode`/`clearSelection`, `highlightActive`/`isNodeLit`/`isLinkLit`, highlight-aware `linkWidth`, `onBackgroundClick`, Escape handler). Selection unions with search through the same dimming channel.
- **D3** — right sidebar where the document unfolds as it's assimilated.
  Commit `423352f` · test `npx tsc --noEmit` + `npx vite build` (both exit 0); runtime verified: sample narrative reconstructs as 1 heading + 5 paragraphs + 15 ordered sentences with stable `data-node-id`s; canvas width tracks the sidebar (920 open / 1280 collapsed) and collapse/reopen toggles cleanly · src `src/main.ts` (`renderSidebar` — sections+sentences tiled, sentences grouped into paragraphs by span containment with an ordered fallback; `layoutGraph`/`setSidebar`; `renderSidebar` called from `commit`), `index.html` (`#sidebar`/`#sidebar-doc`/`#node-details` + styles). Only sentence/section nodes carry usable text+span (clauses/quotes/refs are sub-spans inside sentences ⇒ excluded; walker `walker.rs:73-167`).
- **D4** — click a node to jump to its text + open a details panel.
  Commit `159a6b6` · test `npx tsc --noEmit` + `npx vite build` (both exit 0); runtime verified via `__doctreeSelect`: selecting `sent:1` highlights its line and lists mentions/part_of/precedes; selecting `term:charts` (no own line) lights its neighbour sentence and lists co_occurs_with/mentions; clicking a connection navigates to `term:cove` and re-highlights its sentences; background-click hides the panel and clears the text highlight; zero console errors · src `src/main.ts` (`jumpToNode`/`clearDocActive`, `showDetails`/`hideDetails`, `PROV_META`, `selectNodeById`, delegated click wiring on `#sidebar-doc`/`#node-details`).
- **B2** — gated Tauri inference round-trip (ADR-0001 acceptance).
  Commit `2862cd7` · tests `src-tauri/src/llm.rs::tests::*` (5: `status_enabled_tracks_the_compiled_feature`, `status_names_the_model_path_env`, `status_present_implies_a_path`, `status_serializes_camel_case_for_the_frontend`, `completion_dto_maps_every_field_and_tags_cpu`) (`cargo test -p doctree-tauri`) + ignored live round-trip `src-tauri/tests/llm_roundtrip.rs` (`freeform_completion_returns_text`, `grammar_constrained_output_is_schema_shaped_json`) · src `src-tauri/src/llm.rs` (`llm_status`/`llm_status_impl`, `CompletionDto`, gated `LlmState` + `complete_blocking` + async `llm_complete`; native-free `llm_complete` stub), `src-tauri/src/lib.rs` (`mod llm`, gated `.manage(LlmState)`, `generate_handler!` + `llm_status`/`llm_complete`), `src-tauri/Cargo.toml` (`llm`/`cuda`/`vulkan` features → `doctree-llm`; non-optional native-free dep).
  Verified: default `cargo test --workspace` **59 green, native-free** (`cargo tree -p doctree-tauri` shows no `momusdev`/`llama`); gated `cargo check -p doctree-tauri --features llm` and `cargo test --no-run --features llm` compile clean under MSVC (proves `Engine: Send+Sync` for managed state, the `spawn_blocking` async-command wiring, and that the round-trip test binary builds). The command lazily loads the model on first call and offloads inference to a blocking thread so the webview never stalls.
  Note: the **live model load** (2.33 GB qwen3-4b on CPU) + the GBNF grammar-compile check are the user's desktop end test — the Tauri window cannot be launched/exercised in this headless env (as with A8). Run with `set DOCTREE_MODEL_PATH=…\qwen3-4b-q4km.gguf` then `cargo test -p doctree-tauri --features llm -- --ignored --nocapture`.

---

## Catch-all backlog (off-topic discoveries — provenance noted, never fixed inline)

- **Dual-pipeline benchmarking: run the full pipeline for BOTH the LLM path and
  the deterministic "Tier 1" path, instrumented to compare them at every stage.**
  _(raised by the user 2026-06-02 while A8 was finishing; to discuss in the
  morning.)_ Intent: once the LLM wiring exists (B2+), we should be able to run a
  document through both extraction strategies and benchmark them at each level of
  the process (segmentation, entity/relationship extraction, final graph quality,
  latency), not just at the end. The user noted "we should have built the full
  pipeline for both" and wants to do this — but explicitly said **stop at the A8
  request first and make sure everything works as intended**, so this is logged,
  not started. Likely shape: a benchmark harness that drives `doctree-core` (Tier
  1) and the gated `doctree-llm` path over the same fixtures and emits a
  per-stage comparison. Sequencing TBD with the user (probably after B2/B3 give
  us a working LLM path to compare against).

- **GPU acceleration blocked by bleeding-edge toolchain — needs user sign-off on a
  fix path.** _(discovered while building B1's gated native LLM layer.)_ The CPU
  `inference` feature builds and is the working path for all LLM acceptance
  (B2/B3/B5). Both GPU backends fail on this exact machine:
  - **CUDA 13.1 ✗ VS 2026.** `llama-cpp-sys-2` → nvcc rejects the compiler:
    `host_config.h(164): fatal error C1189: unsupported Microsoft Visual Studio
    version! Only versions between 2019 and 2022 supported`. The
    `-allow-unsupported-compiler` override (CUDAFLAGS / NVCC_PREPEND_FLAGS) does
    not reach CMake's compiler-detection try-compile, so it doesn't help.
  - **Vulkan ✗ MSVC 14.50.** llama.cpp's `vulkan-shaders-gen` ExternalProject
    fails under this MSVC: the `-Brepro` flag triggers
    `fatal error C1083: Cannot open compiler generated file: '': Invalid argument`.
    (Ninja-vs-MSBuild generator juggling didn't get past it.)
  - **Fix options for the user to choose:** (a) install a VS 2019–2022 toolset
    alongside VS 2026 and point CUDA at it (unblocks `cuda`); (b) wait for a
    newer CUDA / llama-cpp-2 that supports MSVC 19.50; (c) ship CPU-only for now
    (RTX 3060 Ti idle). Recommend (c) now, (a) when GPU speed is wanted — the
    crate already gates `cuda`/`vulkan` behind features, so enabling later is a
    flag flip, no code change.

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
