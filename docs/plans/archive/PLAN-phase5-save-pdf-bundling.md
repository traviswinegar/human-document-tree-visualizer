# Phase 5 — Persistence, ingestion, readability

> **Status:** ARCHIVED (shipped 2026-06-02). All four work items shipped and their
> triples are recorded in `BUILD_LOG.md`: #1 clear-on-open + elapsed timer (`#46`,
> commit `36b3a56`); #3 PDF ingestion / ADR-0006 (`#48`, commit `7ec0dcb`); #4
> save/load/CRUD library / ADR-0007 (`#50`, commit `bc07a81`); #2 edge bundling
> first cut (`#52`, commit `b5e99a6`). The deferred follow-up — true hierarchical-LCA
> merged-geometry bundling + its ADR — remains in the Phase 5 backlog (BUILD_LOG
> Catch-all), to be opened as a fresh plan when the user prioritizes it.

Four user-driven asks, landed after the #43 left-stats panel, while the user was
running the desktop app on a ~650 KB / 400-page novel (18 437 nodes / 46 477
edges, narrative · hybrid LLM). Verbatim:

1. "When a new document is opened, we should immediately clear the screen and
   data. I want to see a timer somewhere so that it's clearly work is being done
   and time is actually elapsing."
2. "Edges should be bundled for better visual routing. Not perfectly bundled, but
   very close to one another until they need to trail off to their own world."
3. "Yes, please implement PDF support."
4. "We need a SAVE function, to save the graph and whatever else is associated
   with it. I don't want to regraph every time. (So full file CRUD)."

Plus a capability note: the app already graphs a 650 KB novel successfully;
graph *navigation* lags at that scale ("nice to have", not blocking).

---

## Sequencing (by risk + dependency, not by the user's numbering)

| # | Item | ADR? | Surface | Risk |
|---|------|------|---------|------|
| 1 | Clear-on-open + elapsed timer | no (UX polish) | frontend | low |
| 3 | PDF ingestion | **ADR-0006** | frontend (`pdf.js`) | low–med |
| 4 | Save / load / CRUD library | **ADR-0007** | Tauri cmds + frontend | med–high |
| 2 | Edge bundling | plan-doc now; ADR when the heavy version lands | frontend render | high (perf) |

Each ships as its own atomic `Phase 5 #N` commit + a `#N+1` ledger commit, the
two-commit pattern the log already uses. Verification per item below.

---

### #1 — Clear-on-open + elapsed timer  *(ADR-free UX polish)*

**Problem.** `startBuild` clears `graphData` only *after* the walk resolves, so
during a slow desktop semantic walk the previous document's graph stays on
screen with no sign of progress — reads as a hang.

**Change (frontend only).**
- `loadAndBuild` clears the canvas + sidebar + stats panel *before* the await, so
  the old doc vanishes the instant a new one is opened.
- A live elapsed-time readout (`#elapsed`) ticks from open through walk → build →
  semantic-weave, then freezes at the total. It must keep running through the
  background `pendingDelta` (the genuinely slow CPU phase), stopping in its
  `.finally`; on the structural path it stops when the build reports `done`.

**Verify.** `npx tsc --noEmit` (0) + `npm run build` (0). Perceptual: open a doc →
old graph clears at once, number ticks up, freezes when ready.

---

### #3 — PDF ingestion  *(ADR-0006)*

**Decision.** Extract text in the **frontend** via `pdfjs-dist` (Mozilla pdf.js),
**dynamically imported** so it stays out of the initial bundle. `ingestFile`
detects PDF by extension/MIME and routes through `src/pdf.ts:extractPdfText` →
text → the existing `loadAndBuild` path. Pages joined by blank lines so the
walker sees paragraph breaks. One implementation serves both the desktop and
browser-WASM paths; the Rust default build is untouched (still native-free).

**Verify.** `tsc` + `build` (0); the bundle code-splits the pdf worker. Perceptual:
drop a real PDF → text extracted → graph builds. Documented limit: scanned/image
PDFs (no OCR) yield little text.

---

### #4 — Save / load / CRUD library  *(ADR-0007)*

**Decision.** Desktop-only. New Tauri commands store each saved graph as JSON in
`app_data_dir/library/{id}.doctree.json`, with an `index.json` of lightweight
metas for fast listing. The Rust layer is **schema-agnostic** — it persists the
doc as an opaque `serde_json::Value` and reads only meta fields, so persistence
never couples to the evolving graph schema (core types need no `Deserialize`).

**Saved doc** = `{ schemaVersion, id, name, createdAt, updatedAt, origin,
routing?, text, nodes, edges, positions?, layout? }`. Positions + layout let a
reopen restore the exact view with **no re-walk and no re-simulation**
(`cooldownTicks(0)` on load) — the core of "don't regraph every time" and a
direct mitigation of the navigation lag.

**CRUD surface (frontend).** A "Save" action (serialize current graph+text+meta+
positions → `save_doc`) and a "Library" modal listing saved graphs with Open /
Rename / Delete. Both gated on `isTauri()` (browser export/import to a file is a
backlog item).

**Verify.** `cargo test -p doctree-tauri` — pure helpers (`extract_meta`,
`upsert_index`, `doc_filename`, id/slug gen) unit-tested test-first. `tsc` +
`build` (0). Perceptual: save → reopen restores instantly, no re-walk.

---

### #2 — Edge bundling  *(plan-doc now; ADR when the merged-geometry version lands)*

**Target look (user's words).** "very close to one another until they need to
trail off to their own world" = a bundle shares a *trunk* and frays at the
*leaves*. The document already carries the tree this needs: the `part_of`
containment hierarchy (term ▸ … ▸ section). Route each edge through the path to
its endpoints' **lowest common ancestor** (LCA) in that tree → edges between
nearby regions share ancestor control points and visually merge; edges crossing
the document diverge.

**Perf reality.** At 46 477 edges, per-frame custom curve geometry (46k Three
objects + per-tick rebuilds) is infeasible — the user already sees navigation
lag. So:
- **This session (first cut):** a cheap, always-available **curvature-based**
  bundling toggle — assign each edge a `linkCurvature` keyed by its structural
  grouping so co-routed edges bow together into trails, with straight rendering
  retained as the default. Reversible styling; no geometry rebuild.
- **Follow-up (ADR-000x):** true hierarchical bundling — compute LCA control
  points once the force layout **settles**, render *all* bundled edges as a
  single merged `BufferGeometry` (one draw call, not 46k objects), recompute on
  demand / on settle rather than per frame. Static saved graphs (which don't
  simulate) are the ideal case for this.

**Verify (first cut).** `tsc` + `build` (0). Perceptual: toggle on → edges form
trails; large graph stays at least as fluid as straight edges.

---

## Backlog discovered while planning (catch-all)

- Animate / replay a **saved** graph (v1 loads it static, no player).
- Browser-path persistence (IndexedDB) + export/import to an arbitrary path.
- OCR for scanned PDFs.
- Full merged-geometry hierarchical bundling (the #2 follow-up above) + its ADR.
- Drag bypasses the file-picker `accept` filter — now that PDFs are real, the
  drop path validates type before ingest (folded into #3).
