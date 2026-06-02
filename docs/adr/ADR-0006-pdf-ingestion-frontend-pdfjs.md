# ADR-0006 — PDF ingestion via frontend pdf.js (dynamic import)

- **Status:** Accepted (2026-06-02)
- **Phase:** 5 (#3)
- **Supersedes / superseded by:** none

## Context

Until now the app ingests only `.txt` / `.md`: `ingestFile` reads the file with
`FileReader.readAsText` and hands the UTF-8 string to the walker (the comment in
`src/main.ts` even says "Phase 1 only ingests plain text / markdown, so a naive
`readAsText` is exactly right"). PDFs aren't in the picker's `accept` list, and
*dragging* one in (drag-drop bypasses `accept`) would `readAsText` the binary
container and feed the walker mojibake — no crash, just a nonsense graph.

The user asked for real PDF support. PDF is a binary container (object tables,
xref, FlateDecode-compressed streams, font→Unicode maps); extracting readable
text is a genuine parsing problem.

Two hard constraints frame the choice:
- **ADR-0001 decoupling:** the default build must stay native-free (no new
  native/GPU/LLM dependency on the structural path).
- **Two render paths:** desktop (Tauri webview) *and* public browser build
  (`doctree-core` → WASM). Ingestion should behave the same on both.

## Decision

Extract PDF text in the **frontend** using **`pdfjs-dist`** (Mozilla pdf.js),
loaded by **dynamic `import()`** so the ~MB of pdf.js + its worker stay out of
the initial bundle and load only when a PDF is actually opened (the same
lazy-load pattern already used for the Tauri `invoke` and the WASM glue).

- New module `src/pdf.ts` exposes `isPdf(file)` and
  `extractPdfText(file): Promise<string>` — it lazily imports pdf.js, reads the
  file as an `ArrayBuffer`, walks every page's text content, and joins pages with
  a blank line so the walker sees paragraph breaks.
- `ingestFile` branches: `isPdf(file)` → `extractPdfText` → existing
  `loadAndBuild(text, name)`; otherwise the unchanged `readAsText` path.
- The picker `accept` gains `.pdf` / `application/pdf`; the drop path validates
  the type before ingest.

The Rust workspace is untouched — the default build remains native-free, and the
**same** extraction serves desktop and browser.

## Alternatives considered

1. **Rust-side in `doctree-core` (`pdf-extract` / `lopdf`).** Co-locates
   extraction with the walk, no JS bundle cost. **Rejected:** `wasm32`
   compatibility is uncertain (most PDF crates pull non-wasm deps), so it would
   break the browser walker or force a *second* implementation; and it adds a
   heavy dependency to the path we deliberately keep lean. *(This reverses an
   earlier verbal lean toward Rust-side, after weighing cross-path uniformity and
   the native-free invariant.)*
2. **Rust-side in `src-tauri` only.** Desktop-only; diverges the browser path and
   still adds the dep. **Rejected** for the same divergence reason.
3. **Server/cloud extraction.** Violates the no-cloud core. **Rejected.**

## Consequences

- (+) One implementation, both paths; Rust default build unchanged.
- (+) Lazy-loaded — zero cost until a PDF is opened.
- (−) Bundle grows when pdf.js loads (mitigated: code-split, on demand).
- (−) Reading order can be imperfect on complex layouts (acceptable — it is the
  same class of text the walker already tolerates from any source).
- (−) Scanned / image-only PDFs yield little/no text (no OCR — documented limit,
  backlog item).

## Invariant (pinned)

The frontend has no JS test runner (every A5–D4 / #29–#43 frontend unit was
verified by the standing gate), so the pinned check is:

- **Gate:** `npx tsc --noEmit` (exit 0) + `npm run build` (exit 0, pdf worker
  code-split into its own chunk).
- **Behavioral pin:** `src/pdf.ts` — `isPdf` routes `.pdf`/`application/pdf`
  through `extractPdfText`; `ingestFile` (`src/main.ts`) calls it instead of
  `readAsText`. If PDF support regresses, the ingest branch is where it shows.
