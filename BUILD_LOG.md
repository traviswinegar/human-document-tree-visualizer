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

**All build-order steps complete — #29 (frontend wiring) just landed; only
explicitly-deferred / blocked items remain.** The autonomous "finish the rest of
your known plans and deferred items" mandate has reached its natural floor: every
walk-plan unit (A1–A8, B1–B5, C1, D1–D4) plus the dual-pipeline benchmark (#20)
plus the deferred frontend-integration pass (#29) is shipped and verified. What's
left in the backlog is, by construction, either **blocked on user sign-off** (GPU
acceleration) or **future-product-surface scope** explicitly deferred by an ADR
(cross-document LanceDB → ADR-0004; LLM confirmation of low-confidence
classifications → ADR-0005). None can proceed autonomously without a decision.

**#29 just landed:** the frontend now consumes the capability-aware backend it
already had. On the Tauri path it classifies on load via `classify_document`,
dispatches to the **resolved** build command the classifier names
(`semantic_build_steps` / `embedded_build_steps` / `build_steps`), and gracefully
falls back to the structural spine if the richer command fails at runtime (feature
compiled but model file missing). A capability-gated "find by meaning" box
(`semantic_search`, shown only when `routing.capabilities.vectordb`) shares the
literal search's highlight channel, and a `#routing` HUD line surfaces the class +
confidence + resolved pipeline, with a warm hint on downgrade/fallback. Frontend
only — the default build stays native-free. Flagged **desktop-only-untested**: the
classify/dispatch *logic* type-checks and the browser/WASM fallback renders the
structural walk ("structural · in-browser walk", meaning box hidden), but the live
model/embedder dispatch + meaning search are the user's desktop end test (the Tauri
window can't launch headlessly).

**#20 (the prior step):** the measuring tape for "does the expensive LLM path earn
its keep." Pure, native-free `doctree-core::metrics` (`graph_metrics` →
`{nodes, edges, valid, edge_density, node_kinds, edge_kinds, provenance}`;
`PipelineRun::measured`; `compare_pipelines` → `{node_delta, edge_delta,
latency_ratio}`) computes the per-stage comparison headlessly, and the
`#[ignore]`d `--features llm` harness `src-tauri/tests/benchmark_roundtrip.rs`
runs the real head-to-head — walk (tier1) vs prompt→extract→merge (hybrid) —
timing each stage and printing the per-stage report.

**Post-ship UX polish (on user request, ad-hoc — frontend-only, native-free):**
With the build order done, the user is now driving look-and-feel passes. **#30
(commit `31a3d7a`)** is the first: a high-contrast, hue-varied node palette + an
UnrealBloom glow pass, because the all-blue structural palette read as a muddy
navy cloud that vanished on a large graph ("everything is SO DARK… get some
COLOR"). These are styling commits, not architecture — no ADR, verified the
D-stream way (tsc + vite build + dev-handle render).

**Workflow switch → desktop from now on (#33, commit `67758f1`).** The user asked
to drop the browser and run the full desktop app ("No more browser unless we can
bridge the gap in the future"), after learning the semantic layer (LLM
character/place/concept/event nodes + similarity edges + meaning search) is
desktop-only by design (a browser tab can't load a multi-GB local GGUF; the core
pipeline makes no cloud calls). `scripts/dev-desktop.cmd` is the repeatable
launcher (vcvars64 → DOCTREE_MODEL_PATH → `tauri dev --features llm,vectordb`).
**This crossed a long-standing barrier: the first verified full-desktop launch +
native-stack init from the agent context** — gated build compiled clean (1m26s),
`doctree-tauri.exe` linked + ran, qwen3-4b loaded (398 tensors, q4_K, 36 CPU
layers), `llama_context` built, KV/compute buffers reserved, warmed up. The model
loading *at launch* is evidence the #29 auto-classify chain fired
(classify→`semantic_build_steps`→`ensure_loaded`). Still the user's interactive
end test: the **visible** semantic-node render + live classify/meaning-search
round-trip (the Tauri webview can't be introspected like the browser preview MCP).
See the #33 entry for the full triple.

**Remaining deferred / blocked items (need a user decision — do NOT start
autonomously):**
- **GPU acceleration** — blocked by the CUDA 13.1 ✗ VS 2026 / Vulkan ✗ MSVC 14.50
  toolchain mismatch; needs the user to pick a fix path (see Catch-all). CPU is the
  working path for every LLM acceptance.
- **Cross-document / persistent vector search (LanceDB)** — deferred by ADR-0004;
  only relevant once search spans a corpus and must survive restarts. Single-doc
  in-memory cosine is the current product surface.
- **LLM confirmation for low-confidence classifications** — deferred by ADR-0005;
  the deterministic classifier separates the fixtures cleanly without a model.

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
> native-free. Verified live via dev handles (see D1–D4 entries below). _(B2 and
> B3 have since landed; B4 is now the next *build-order* step — see Current
> Position.)_

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
- **B3** — LLM semantic layer (grammar-constrained) merged onto the spine.
  Commit `5521c23` · tests `crates/doctree-core/src/schema.rs::tests::{prune_dangling_edges_drops_only_unwired_edges, prune_is_a_noop_on_a_valid_graph}` + `crates/doctree-llm/src/lib.rs::tests::{extraction_prompt_anchors_spine_and_lists_kinds, extraction_prompt_respects_the_doc_budget}` + `src-tauri/src/llm.rs::tests::merge_semantic_onto_spine_is_authoritative_and_valid` (all on the default native-free build) + ignored end-to-end `src-tauri/tests/llm_roundtrip.rs::hybrid_extraction_merges_onto_the_spine` · src `crates/doctree-core/src/schema.rs` (`Graph::prune_dangling_edges`), `crates/doctree-llm/src/lib.rs` (`build_extraction_prompt`, `PROMPT_DOC_BUDGET_BYTES`), `src-tauri/src/llm.rs` (pure `merge_semantic_onto_spine`; gated `ensure_loaded`/`extract_blocking`/async `semantic_build_steps`; native-free `semantic_build_steps` stub), `src-tauri/src/lib.rs` (`generate_handler!` + `llm::semantic_build_steps`).
  Verified: default `cargo test` green (core 36, llm 7, tauri 13) with ZERO native deps (`cargo tree -p doctree-tauri` shows no `momusdev`/`llama`/`lance`); gated `cargo test --no-run -p doctree-tauri --features llm` compiles clean under MSVC (my crates warning-free; only upstream `momusdev_*` warn) and builds the ignored hybrid round-trip binary. The merge is spine-authoritative: `Graph::merge` keeps the deterministic spine on id collisions, appends semantic edges, then `prune_dangling_edges` drops any the model wired to ids it didn't ground — the hybrid graph is valid by construction and orders into the existing `BuildStep` stream (no new frontend render path).
  Note: the **live extraction merge** (walk → prompt → grammar-constrained model → fragment → spine-union → build steps) is the user's desktop end test — run as the B2 note above. The orchestration's pure pieces (prompt builder, spine-union, prune) are unit-tested headlessly; the model round-trip is not runnable in this env.
- **B4** — embedding similarity edges + semantic search (gated `vectordb`) + ADR-0004.
  Commit `58aeae6` · tests `crates/doctree-llm/src/lib.rs::tests::{cosine_similarity_handles_identical_orthogonal_and_degenerate, rank_by_similarity_orders_best_first_and_truncates, similarity_edges_link_only_pairs_above_threshold, similarity_edges_respect_top_k, graph_embedding_inputs_selects_content_nodes, embed_cache_dir_prefers_the_env_var}` + `src-tauri/src/llm.rs::tests::{attach_similarity_edges_appends_valid_weighted_links, to_search_hits_attaches_label_and_kind_and_serializes_camel_case}` (all default native-free) + ignored live `src-tauri/tests/embedding_roundtrip.rs::{embedder_returns_a_384d_vector, related_text_is_closer_than_unrelated_text, similarity_edges_augment_the_spine}` · src `crates/doctree-llm/src/lib.rs` (`cosine_similarity`, `SimilarityOptions`, `similarity_edges`, `rank_by_similarity`, `SearchHit`, `is_embeddable_kind`, `graph_embedding_inputs`, `embed_cache_dir`, `EMBED_CACHE_ENV`; gated `Embedder` over momusdev's `FastEmbedder`), `src-tauri/src/llm.rs` (pure `attach_similarity_edges`, `SearchHitDto`, `to_search_hits`; gated `ensure_embedder_loaded`/`embed_batch_blocking`/async `embedded_build_steps`+`semantic_search`; native-free stubs + `no_vectordb_error`), `src-tauri/src/lib.rs` (two-slot `LlmState` gated `any(llm,vectordb)`; `generate_handler!` + `embedded_build_steps`/`semantic_search`), `crates/doctree-llm/Cargo.toml` + `src-tauri/Cargo.toml` (`vectordb` feature decoupled from `llm`), ADR `docs/adr/ADR-0004-embedding-similarity-layer.md`.
  Verified: default `cargo test --workspace` **72 green, native-free** (core 36 + integration 2 + llm 13 + tauri 15 + wasm 6; `cargo tree -p doctree-tauri` shows no `momusdev`/`llama`/`lancedb`/`fastembed`/`arrow`); `--features llm` still compiles clean under MSVC after the `LlmState` two-slot refactor (regression check); gated `cargo test --no-run -p doctree-tauri --features vectordb` compiles clean under MSVC in ~2m44s (my crates warning-free; only 2 upstream `momusdev_llm` warnings) and builds the ignored `embedding_roundtrip` binary — and notably did **not** compile llama.cpp, proving the decoupling (the embedder builds with no C++ toolchain). Per ADR-0004 similarity is in-memory cosine (no LanceDB at single-document scale) and `vectordb` is independent of `llm` (embeddings build with no C++ toolchain — fastembed downloads a prebuilt ONNX runtime).
  Note: the **live embedding round-trip** (ONNX model load → real 384-d vectors → similarity edges) is the user's desktop end test — the model can't load in this headless env. Run with `cargo test -p doctree-tauri --features vectordb -- --ignored --nocapture` (optionally `set DOCTREE_EMBED_CACHE=…` to a pre-populated cache for offline use). The pure similarity math (cosine/top-k/ranking/edge derivation) is unit-tested headlessly.
- **B5** — document-type detection runtime gate (deterministic classifier + capability-aware routing) + ADR-0005.
  Commit `d95e1be` · tests `crates/doctree-core/src/classify.rs::tests::{classify_is_deterministic, narrative_prose_classifies_as_narrative, technical_prose_classifies_as_expository, heading_list_code_doc_classifies_as_structured, tiny_or_empty_doc_is_unknown, a_single_heading_does_not_make_prose_structured, signals_are_bounded_and_confidence_in_unit_range, dialogue_and_pronouns_are_measured}` (8) + `src-tauri/src/lib.rs::tests::{narrative_routing_degrades_with_capabilities, expository_routing_needs_only_the_embedder, structural_only_always_resolves_to_the_spine, resolved_commands_are_actually_registered, classify_routes_narrative_and_flags_downgrade_on_a_lean_build, classify_document_serializes_camel_case_with_snake_case_tags}` (6) — all default native-free, **no `#[ignore]`d live test** (the gate consults no model) · src `crates/doctree-core/src/classify.rs` (`classify_document`, `Classification`, `ClassificationSignals`, `DocumentClass`, `RecommendedPipeline`; word-weighted structure + dialogue + pronoun + past-tense signals → decision tree), `crates/doctree-core/src/lib.rs` (re-exports), `src-tauri/src/lib.rs` (`Capabilities`/`Capabilities::compiled`, `ResolvedPipeline`/`command`, pure `resolve_pipeline`, `Routing`, `classify_document_impl` + `classify_document` command + `generate_handler!` registration), ADR `docs/adr/ADR-0005-document-type-detection-runtime-gate.md`.
  Verified: default `cargo test --workspace` **86 green, native-free** (core 44 + integration 2 + llm 13 + tauri 21 + wasm 6; `cargo tree -p doctree-tauri` shows no `momusdev`/`llama`/`lancedb`/`fastembed`/`arrow`); both gated builds regression-clean under MSVC — `cargo check -p doctree-tauri --features llm` (2.95 s incremental) and `--features vectordb` (1m06s) compile with my crates warning-free (only pre-existing upstream `momusdev_llm` warnings). Per ADR-0005 the classifier is deterministic + native-free (so it's fully headlessly verified, unlike B2/B3/B4): the class→ideal-pipeline mapping is pure `doctree-core` logic; the capability reconciliation (`cfg!(feature=…)` → which registered command can actually run) lives in the command layer, degrading gracefully (hybrid → similarity → spine) and flagging the downgrade.
  Note: the gate adds one always-registered IPC command (`classify_document`) returning the verdict + the command to call; **wiring the frontend to call it** (classify on load → dispatch to the resolved build command → surface class/downgrade) is the deferred B3/B4/B5 frontend-integration pass. An optional LLM *confirmation* for low-confidence classifications is deferred (ADR-0005) — the surface features separate the fixtures cleanly without a model.
- **#20** — dual-pipeline benchmark harness (deterministic Tier-1 vs gated LLM hybrid, per-stage). _No ADR — benchmarking is a tool, not load-bearing architecture._
  Commit `c5742fa` · tests `crates/doctree-core/src/metrics.rs::tests::{metrics_count_kinds_and_provenance, metrics_are_deterministic, edge_density_is_edges_per_node_and_empty_is_zero, comparison_reports_deltas_and_latency_ratio, latency_ratio_floors_a_sub_millisecond_baseline, metrics_serialize_camel_case_with_snake_case_histogram_keys, comparison_serializes_camel_case}` (7, all default native-free) + ignored live harness `src-tauri/tests/benchmark_roundtrip.rs::benchmarks_tier1_against_hybrid_per_stage` (`--features llm`) · src `crates/doctree-core/src/metrics.rs` (`GraphMetrics`, `graph_metrics`, `PipelineRun`/`PipelineRun::measured`, `PipelineComparison`, `compare_pipelines`), `crates/doctree-core/src/lib.rs` (re-exports), `crates/doctree-core/src/schema.rs` (`PartialOrd`/`Ord` on `NodeKind`/`EdgeKind`/`Provenance` so the histograms key into a deterministic `BTreeMap`), `src-tauri/tests/benchmark_roundtrip.rs` (the gated live head-to-head).
  Verified: default `cargo test --workspace` **93 green, native-free** (core 51 incl. 7 new `metrics` + integration 2 + llm 13 + tauri 21 + wasm 6; `cargo tree -p doctree-tauri` shows no `momusdev`/`llama`/`lancedb`/`fastembed`/`arrow`); gated `cargo test -p doctree-tauri --features llm --no-run` compiles clean under MSVC (17.5 s incremental, llama.cpp cached) and **builds the `benchmark_roundtrip` binary** — proving the harness compiles against the live `Engine`/`build_extraction_prompt`/`merge_semantic_onto_spine` APIs. The comparison machinery is pure `doctree-core` (`graph_metrics` → counts + per-kind/per-provenance `BTreeMap` histograms + validity + edges-per-node; `compare_pipelines` → node/edge deltas + a latency ratio whose baseline is floored at 1 ms so a sub-millisecond Tier-1 never divides by zero), serialized camelCase with snake_case enum histogram keys for the frontend.
  Note: the **live head-to-head** (walk the spine → build the anchored prompt → grammar-constrained extraction on a real CPU model → merge → time each stage → per-stage report) is the user's desktop end test — run with `set DOCTREE_MODEL_PATH=…\qwen3-4b-q4km.gguf` then `cargo test -p doctree-tauri --features llm --test benchmark_roundtrip -- --ignored --nocapture`. No Tauri command was added: the benchmark is a developer measurement (exercised via the ignored harness), not a runtime feature — the metrics surface can back a UI benchmark in the deferred frontend pass if wanted. Embeddings (B4) are a possible third lane but kept out of #20's scope (the user named "LLM vs Tier-1").
- **#29** — frontend integration pass (B3/B4/B5): classify on load → dispatch to the resolved build command + "find by meaning" search + capability-aware UI. _No ADR — frontend wiring of an already-decided backend; the load-bearing decisions live in ADR-0004 (embeddings) and ADR-0005 (routing)._
  Commit `c350174` · test `npx tsc --noEmit` (exit 0) + `npm run build` (tsc && vite build, exit 0 — wasm rebuilt, 393 modules transformed, `dist/` emitted: ~1,350 kB `index.js` + 183 kB wasm chunk); browser/WASM path verified to render the structural walk with the routing line reading "structural · in-browser walk" and the meaning box hidden (capabilities.vectordb absent off-Tauri) · src `src/doc-source.ts` (`BuildSource` gains `routing?`/`command?`/`fellBack?`; B5 routing DTOs `DocumentClass`/`RecommendedPipeline`/`ResolvedPipeline`/`ClassificationSignals`/`Capabilities`/`Routing` + `MeaningHit`; cached `tauriInvoke`; `loadTauriBuildSource` classifies then dispatches to `routing.command` with a `build_steps` fallback on runtime failure; `searchByMeaning` wraps `semantic_search`), `src/main.ts` (`renderRouting` HUD line — class · confidence · pipeline + warm downgrade/fallback hint; `runMeaningSearch` sharing the literal search's highlight/dimming channel; `PIPELINE_LABEL`; `currentText` tracked for re-query; meaning box toggled on `source.routing?.capabilities.vectordb` in `startBuild`), `index.html` (`#routing` line under `#stats`; capability-gated `#meaning-search` input + `#meaning-count`).
  Verified: both frontend gates exit 0 and the browser path renders unchanged (the default build stays native-free — no new native dep, the Tauri-only calls are dynamic-imported behind `isTauri()`). Flagged **desktop-only-untested**: the classify→dispatch chain and the embedding-backed meaning search require the live model/embedder, which can't load in this headless env — they are the user's desktop end test (`npm run tauri dev`, then classify a narrative doc and try "find by meaning"). The classifier itself is native-free so the *routing* always resolves; only the model-backed `semantic_build_steps`/`embedded_build_steps`/`semantic_search` need a desktop model, and the frontend degrades to the structural spine (with a visible hint) when they fail.
- **#30** — high-contrast node palette + bloom glow (user: "everything is SO DARK… get some COLOR"). _No ADR — visual styling, not load-bearing architecture (cf. the D-stream, also ADR-free)._
  Commit `31a3d7a` · test `npx tsc --noEmit` (exit 0) + `npm run build` (exit 0 — 397 modules, +4 for the bloom/output passes; `dist/` emitted) + browser dev-handle introspection (reload renders the wasm-walk: `__doctreeSource.origin === "wasm-walk"`, 54 nodes, `#graph canvas` present, **zero console warnings/errors** ⇒ both `composer.addPass()` calls ran at module init without throwing) · src `src/colors.ts` (`NODE_COLORS` reworked to a bright, hue-varied palette — structural = cool arc violet→teal, semantic = warm arc + vivid outliers, de-collided + lifted in lightness; `edgeColor` lifted off near-black), `src/main.ts` (`UnrealBloomPass` + `OutputPass` appended to `graph.postProcessingComposer()` with named `BLOOM_*` tunables; `nodeOpacity` 0.95→1.0; dim floors node 0.06→0.16 / link 0.04→0.08).
  Root cause: the old palette was all muddy near-blue (structural kinds only span the cool family, and the browser/WASM walk renders *only* structural kinds), so a large graph collapsed into an indistinct navy cloud against the `#05070d` background. The fix attacks it three ways — brighter/more-varied base colors, an additive bloom pass so bright spheres glow, and a higher dim floor so search/selection context stays legible. Frontend-only; default build stays native-free (the ESM `3d-force-graph` externalises `three`, so the bloom addons share its single `three` instance — no module-duplication hazard).
  Note: the **glow intensity** is the user's perceptual call — `preview_screenshot` times out against the continuously-animating WebGL canvas (the documented A5–D4 tooling limitation), so the render is verified by dev-handle introspection (loads clean, renders) rather than by image. The `BLOOM_STRENGTH`/`BLOOM_RADIUS`/`BLOOM_THRESHOLD` constants at the top of the bloom block in `main.ts` are the tuning knobs if the user wants more/less.
  **Follow-up (commit `85a1534`):** the first values (strength 0.85 / radius 0.55 / threshold 0.08) blew nodes out into pure-light orbs — retuned to **strength 0.4 / radius 0.3 / threshold 0.2** (subtle rim, spheres keep their shape). Same verification (tsc + vite build + clean browser reload).
- **#33** — `scripts/dev-desktop.cmd`: repeatable one-command launcher for the FULL desktop app (structural spine + semantic LLM/embedding layer). _No ADR — tooling/workflow, not load-bearing architecture (cf. the D-stream + #30, also ADR-free)._ User: "let's close this browser and open the desktop app. No more browser unless we can bridge the gap in the future."
  Commit `67758f1` · "test" = the live desktop launch itself (no automated test for a launcher) · src `scripts/dev-desktop.cmd` (calls `vcvars64.bat` for the MSVC env the gated llama.cpp backend needs, defaults `DOCTREE_MODEL_PATH` to the local qwen3-4b GGUF with an overridable env var + missing-file warning, runs `npm run tauri -- dev --features llm,vectordb`).
  **Milestone — first verified full-desktop launch from the agent context.** Every prior B-stream entry (B2/B3/B4/#20) flagged the live model load as "the user's desktop end test — the Tauri window cannot be launched/exercised in this headless env." That barrier is now partially crossed: running the launcher in the background, the build output shows the complete chain succeed — `Finished dev profile in 1m 26s` (gated `--features llm,vectordb` compiled clean; only the 4 pre-existing upstream `momusdev_llm` warnings) → `Running target\debug\doctree-tauri.exe` (native binary linked + launched) → `llama_model_loader` loaded the qwen3-4b GGUF (398 tensors, q4_K, 36 layers, all `dev = CPU` as expected since GPU is blocked) → `llama_context` built → KV cache 576 MiB + CPU compute buffer 306.75 MiB reserved → warmup `reserve took 72.54 ms`. The model loading **at launch** (not lazily) is itself evidence the #29 auto-classify-on-load chain fired: frontend loaded → `classify_document` → routed to `semantic_build_steps` (llm feature compiled + model present) → `ensure_loaded`.
  Note: what's verified is **launch + native-stack init** (compile→link→load GGUF→build context→reserve buffers→ready), proving the gated stack actually initializes end-to-end on this machine. What's still the user's interactive end test is the **visible GUI render of semantic nodes** + the live classify/meaning-search round-trip — the Tauri webview can't be introspected the way the browser preview MCP introspects the WASM build, so the perceptual confirmation (warm character/place/event nodes appear; "find by meaning" returns hits) is the user looking at their own screen. CPU-only (GPU blocked, see Catch-all); a GPU flag-flip awaits the user's toolchain decision.

---

## Catch-all backlog (off-topic discoveries — provenance noted, never fixed inline)

- **✅ DONE (#20, commit `c5742fa`) — Dual-pipeline benchmarking: run the full
  pipeline for BOTH the LLM path and the deterministic "Tier 1" path, instrumented
  to compare them at every stage.** _(raised by the user 2026-06-02 while A8 was
  finishing.)_ Intent: once the LLM wiring exists (B2+), run a document through
  both extraction strategies and benchmark them at each level of the process
  (segmentation, entity/relationship extraction, final graph quality, latency),
  not just at the end. **Shipped as designed:** the comparison machinery is pure,
  native-free `doctree-core::metrics` (`graph_metrics`, `PipelineRun`,
  `compare_pipelines` → per-kind/per-provenance histograms + node/edge deltas +
  latency ratio), and the live head-to-head over a real model is the `#[ignore]`d
  `--features llm` harness `src-tauri/tests/benchmark_roundtrip.rs`
  (`benchmarks_tier1_against_hybrid_per_stage`) that times each stage and prints a
  per-stage report. See the #20 completed entry for the full triple.

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

- **Cross-document / persistent vector search (LanceDB).** _(scoped out of B4 by
  [ADR-0004](docs/adr/ADR-0004-embedding-similarity-layer.md).)_ B4 computes
  embedding similarity **in-memory** over a single document's node vectors — at
  that scale (tens–hundreds of nodes) brute-force cosine beats building an index.
  The LanceDB store ADR-0001 named is the right tool only once search spans a
  **corpus** of documents and must survive restarts (true RAG). When that arrives:
  persist node embeddings to `momusdev_llm`'s LanceDB store (already pulled in by
  the `vectordb` feature, currently unused), key by document + node id, and route
  `semantic_search` through an ANN query instead of the in-memory rank. Not
  started — single-document similarity is the current product surface.

- **✅ DONE (#29, commit `c350174`) — Frontend wiring for the semantic B-stream
  (B3/B4/B5) — desktop-only, untested in this env.** _(deferred while landing the
  B2–B4 backend, like the B3 trigger.)_ The gated commands (`semantic_build_steps`,
  `embedded_build_steps`, `semantic_search`) were registered and headlessly
  compile-verified, but the frontend called only the structural `build_steps`/wasm
  path. **Shipped as designed:** `src/doc-source.ts` now classifies on load via
  `classify_document` and dispatches to the **resolved** command the classifier
  names (richer than the original "prefer when `llm_status` reports the capability"
  — the B5 router subsumes the capability check and degrades hybrid → similarity →
  spine), with a runtime fallback to `build_steps` if the model-backed command
  fails; `semantic_search` backs a capability-gated "find by meaning" box that
  complements the literal search; a `#routing` HUD line surfaces the class +
  resolved pipeline + any downgrade. See the #29 completed entry for the full
  triple. Flagged desktop-only-untested as planned (the live model/embedder run is
  the user's end test — the Tauri window can't be exercised headlessly).

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
