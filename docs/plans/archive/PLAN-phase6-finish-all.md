# Phase 6 — Finish all backlog work

> **Status:** active (opened 2026-06-02). Archived when all seven work items ship
> and their triples are recorded in `BUILD_LOG.md`.

User direction (verbatim, 2026-06-02, after Phase 5 shipped):
1. "You make the GPU acceleration decision"
2. "Go ahead and make a plan to finish ALL work, then we'll test at the end"

So this plan clears the **entire** `BUILD_LOG` Catch-all backlog in one autonomous
run (the standing mandate: test at the end, commit locally, never push, never
modify the shared sibling crates `momusdev_llm`/`momusdev_met`, keep the default
build native-free per ADR-0001). The two truly-blocked items become unblocked or
decided here; the rest are deferred features the earlier ADRs/plans named.

---

## GPU acceleration decision (item #5 — DECIDED, delegated to me)

**Decision: enable CUDA, built against the co-installed VS 2019 BuildTools
toolchain. Keep `cuda` a feature flag (default build stays native-free). CPU
remains the guaranteed fallback.**

**Why this is now possible (the Catch-all note was stale).** The block was never
"CUDA can't build our code" — it was **nvcc refusing the VS 2026 host compiler**
(`CUDA 13.1 only supports VS 2019–2022`). Probing the machine 2026-06-02 found a
**VS 2019 BuildTools install already present** at
`C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools` with **MSVC
14.29.30133** (`cl.exe`), bundled **CMake** + **Ninja**, and a Windows SDK — i.e. a
CUDA-13.1-**accepted** host compiler is already on disk. So GPU unblocks with **no
install** and **no sibling-crate change** (the `cuda` feature already cascades
`doctree-llm/cuda → momusdev_llm/cuda → llama-cpp-2/cuda`): build the gated native
layer from the **VS 2019** developer environment (so `cl` on PATH is 14.29, which
nvcc accepts and CMake detects) instead of VS 2026.

**Alternatives rejected:** (a) `-allow-unsupported-compiler` to force VS 2026 —
the Catch-all already recorded it doesn't reach CMake's try-compile; (b) Vulkan —
the Vulkan SDK is on PATH but `vulkan-shaders-gen` failed under MSVC 14.50, and
CUDA is the better fit for the RTX 3060 Ti anyway; (c) install a fresh VS
2019–2022 toolset — unnecessary, one already exists; (d) stay CPU-only — leaves
the 3060 Ti idle and the ~28 s extraction slow when a free win is available.

**Scope of the GPU work:** validate the `cuda` build compiles under VS 2019 (a
long llama.cpp CUDA compile — kicked to the background first), confirm the model
offloads layers to the GPU, add `scripts/dev-desktop-gpu.cmd` (the VS-2019 +
`--features llm,vectordb,cuda` launcher), and write **ADR-0008** recording the
decision once the compile is verified. If the compile hits an unforeseen wall, the
decision degrades gracefully to CPU-only (documented), since `cuda` is opt-in.

---

## Sequencing (native-free frontend first while CUDA compiles in the background)

| # | Item | ADR? | Surface | Risk | Native? |
|---|------|------|---------|------|---------|
| 1 | Animate / replay a saved graph | no (UX) | frontend | low | no |
| 2 | Browser IndexedDB persistence + export/import | no (UX/infra) | frontend | med | no |
| 3 | OCR for scanned PDFs | no (extends ADR-0006) | frontend (`tesseract.js`) | med | no |
| 4 | Merged-geometry hierarchical bundling | **ADR-0009** | frontend render | high (perf) | no |
| 5 | GPU / CUDA acceleration | **ADR-0008** | build/native | med | gated |
| 6 | LLM confirm of low-confidence class | no (closes ADR-0005 follow-up) | Tauri cmd | med | gated |
| 7 | Cross-document LanceDB persistent RAG | **ADR-00010** (extends ADR-0004) | Tauri cmds + native | high | gated |

Each ships as its own atomic `Phase 6 #N` commit + a `#N+1` ledger commit (the
two-commit pattern). The CUDA compile (#5) runs in the background from the start so
the native-free frontend items (#1–#4) land while it churns — the same interleave
B1 used.

---

### #1 — Animate / replay a saved graph  *(ADR-free UX)*

**Problem.** `restoreSavedDoc` (Phase 5 #50) sets `player = null` and presents the
chip as a finished build, so a reopened graph is **static** — no scrub/replay.

**Change (frontend only).** Rebuild a `BuildPlayer` from the saved `nodes`/`edges`:
their stored order is already a valid `BuildStep` sequence (each edge's endpoints
precede it). A "Replay" on a saved graph re-streams the growth from empty; opening
still lands instantly on the saved layout, with replay available on demand. Seeded
positions are reused so the replay relaxes toward the saved shape, not a new one.

**Verify.** `tsc` + `build` (0). Perceptual: open a saved graph → it appears at
once; Replay re-animates its growth.

---

### #2 — Browser persistence (IndexedDB) + export / import  *(ADR-free infra)*

**Problem.** The Phase 5 library is desktop-only (`isTauri()`, app-data files); the
public browser/WASM build can't save, and neither path can move a graph between
machines.

**Change (frontend only).** A small storage abstraction (`Library` interface) with
two backends: the existing Tauri commands (desktop) and an **IndexedDB** store
(browser), chosen by `isTauri()`. The Library modal becomes available on both
paths. Plus **export** (download the opaque `*.doctree.json`) and **import** (file
picker → validate `schemaVersion` → restore/save), so a graph is portable. The
opaque-`Value` format (ADR-0007) is already file-portable, so this is plumbing.

**Verify.** `tsc` + `build` (0). Perceptual: in the browser, save → reload →
Library lists it → reopen; export a graph → import it elsewhere.

---

### #3 — OCR for scanned PDFs  *(extends ADR-0006, no new ADR)*

**Problem.** ADR-0006's `extractPdfText` reads the PDF **text layer**; a scanned /
image-only PDF has none, so it reports "no extractable text".

**Change (frontend only).** When the extracted text is empty/near-empty, fall back
to OCR: render each page to a canvas (pdf.js already can) and run **`tesseract.js`**
over the rasters, **dynamically imported** so the (heavy) OCR engine + traineddata
stay out of the initial bundle (the pdf.js code-split precedent). A progress beat
in the elapsed/status line (OCR is slow). The Rust default build is untouched
(native-free).

**Verify.** `tsc` + `build` (0) with `tesseract.js` proven to code-split into its
own lazy chunk. Perceptual: drop a scanned PDF → OCR runs → text → graph builds.

---

### #4 — Merged-geometry hierarchical bundling  *(ADR-0009)*

**Problem.** Phase 5 #52 shipped the cheap **curvature** first cut. The user's
target ("very close … until they trail off to their own world") and the
large-graph **navigation lag** both want the heavy version: true hierarchical
bundling where edges sharing ancestry merge into one trunk.

**Decision (ADR-0009).** Compute control points along each edge's path through the
`part_of` tree to its endpoints' **lowest common ancestor**, and render **all**
bundled edges as a single merged `BufferGeometry` — **one draw call** instead of
~46k line objects — recomputed **on layout settle / on demand**, never per frame.
Static saved graphs (no simulation) are the ideal case. The straight + curvature
modes stay; this is a third "bundle: trails" mode. Load-bearing (a new render path
that replaces 3d-force-graph's per-link lines), hence its own ADR.

**Verify.** `tsc` + `build` (0). Perceptual + perf: on the 650 KB novel the merged
geometry holds interactive frame rates where 46k straight lines lagged.

---

### #5 — GPU / CUDA acceleration  *(ADR-0008 — see decision above)*

Validate the `cuda` build under VS 2019, confirm GPU offload, ship
`scripts/dev-desktop-gpu.cmd`, write ADR-0008. Default build stays native-free.

**Verify.** `cargo build -p doctree-tauri --features llm,vectordb,cuda` compiles
under the VS 2019 env; launch shows the GGUF layers offloaded to the 3060 Ti
(`llama_model_loader … dev = CUDA`, not all-CPU) and extraction wall-time drops.
The live GPU run is the user's desktop end test; the compile + offload log is the
agent-side proof.

---

### #6 — LLM confirm of low-confidence classifications  *(closes the ADR-0005 follow-up)*

**Problem.** ADR-0005 deferred an optional model **confirmation** for low-confidence
classifier verdicts. The classifier is deterministic + native-free and separates
the fixtures cleanly, but a borderline real document may be misrouted.

**Change.** When `Classification.confidence` is below a threshold (and `llm` is
compiled + a model is loadable), prompt the model for a one-shot class verdict
(grammar-constrained to the four classes) and let it confirm/correct before
routing. Gated; the native-free default is unaffected (no model → keep the
deterministic verdict). The threshold + the merge logic (when to trust the model)
are pure functions, unit-tested **test-first**.

**Verify.** `cargo test -p doctree-tauri` (pure logic) + `tsc`/`build` (0). The
live confirm is the user's desktop end test.

---

### #7 — Cross-document LanceDB persistent RAG  *(ADR-00010, extends ADR-0004)*

**Problem.** ADR-0004 scoped B4 to **in-memory, single-document** cosine. The
deferred goal is true RAG: search **across a corpus** of saved documents, surviving
restarts.

**Decision (ADR-00010, extends ADR-0004).** Persist node embeddings to
`momusdev_llm`'s **LanceDB** store (already pulled by the `vectordb` feature,
currently unused), keyed by `document id + node id`, and add a cross-document
`semantic_search` that runs an **ANN query** over the corpus instead of the
in-memory rank. Single-document search keeps the in-memory fast path (no index
needed at tens–hundreds of nodes); the LanceDB path activates for corpus search.
Wired to the Phase 6 #2 saved-graph library so "search all my graphs" works.

**Hard constraint.** `momusdev_llm` is **read-only**. I consume its LanceDB API as
published; if it doesn't expose persistence/query I need, I **flag + document**
(Catch-all, for the user) and ship the largest correct subset — I do **not** patch
the crate. This is the biggest/last item and the one most likely to surface a
read-only-crate boundary.

**Verify.** Pure pieces (key scheme, query→hit mapping, single-vs-corpus routing)
unit-tested **test-first** on the default native-free build; the live LanceDB
round-trip is a gated `#[ignore]` harness + the user's desktop end test.

---

## Notes / invariants carried from prior phases

- **Default build native-free (ADR-0001).** Items #5/#6/#7 are all behind
  `llm`/`vectordb`/`cuda` features; `cargo test --workspace` (no features) must
  stay green with zero native deps after every commit.
- **Never modify `momusdev_llm` / `momusdev_met`.** Consume read-only; flag, don't
  patch.
- **Frontend gate** = `npx tsc --noEmit` (0) + `npm run build` (0); **Rust gate** =
  `cargo test -p doctree-tauri` (or `--lib` if a running app holds the bin lock).
- **Desktop-only-untested** flag stands for every live model/GPU/LanceDB path (the
  Tauri webview isn't headlessly introspectable) — the agent proves compile + pure
  logic + headless round-trips; the user does the perceptual/live end test.
