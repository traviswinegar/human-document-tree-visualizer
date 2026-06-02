// Phase 5 #3 (ADR-0006) — PDF text extraction in the frontend via pdf.js.
//
// A PDF is a binary container (object tables, xref, FlateDecode-compressed
// streams, font→Unicode maps); pulling readable text out of it is a genuine
// parse, so we lean on Mozilla's pdf.js (`pdfjs-dist`) rather than reinvent it.
// The heavy library + its worker (~MB) are pulled in by a *dynamic* import
// inside extractPdfText, so they stay out of the initial bundle and load only
// when a PDF is actually opened — the same lazy pattern as the Tauri `invoke`
// and the WASM glue. `isPdf` is a cheap extension/MIME check that needs none of
// that, so callers can branch *before* paying the load cost.
//
// One implementation serves both render paths (the desktop Tauri webview and the
// public browser-WASM build); the Rust workspace is untouched, so the default
// build stays native-free (ADR-0001 holds).

// Detect a PDF by MIME type or extension. The drag-drop path bypasses the file
// picker's `accept` filter, so this is also the guard the drop handler uses
// before deciding how to ingest a file.
export function isPdf(file: File): boolean {
  return file.type === "application/pdf" || /\.pdf$/i.test(file.name);
}

// Extract the document's text, page by page, joining pages with a blank line so
// the downstream walker sees a paragraph break between pages. Within a page,
// fragments are joined on spaces and pdf.js's end-of-line hints become newlines.
//
// Limits (documented, acceptable — the same class of text the walker already
// tolerates from any source): reading order can be imperfect on complex
// multi-column layouts, and scanned / image-only PDFs yield little or no text
// (no OCR — a backlog item).
export async function extractPdfText(file: File): Promise<string> {
  // Lazy load: the ~MB of pdf.js + its worker are fetched only now, on the first
  // PDF open, and Vite code-splits them into their own chunks.
  const pdfjs = await import("pdfjs-dist");
  const workerUrl = (await import("pdfjs-dist/build/pdf.worker.min.mjs?url")).default;
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

  const data = await file.arrayBuffer();
  // Keep the loading task: in pdf.js v6 the worker is torn down via the task's
  // destroy(), not the document proxy's.
  const loadingTask = pdfjs.getDocument({ data });
  const doc = await loadingTask.promise;
  try {
    const pages: string[] = [];
    for (let i = 1; i <= doc.numPages; i++) {
      const page = await doc.getPage(i);
      const content = await page.getTextContent();
      let pageText = "";
      for (const item of content.items) {
        // TextMarkedContent items carry no `str`; only real TextItems do.
        if ("str" in item) {
          pageText += item.str;
          pageText += item.hasEOL ? "\n" : " ";
        }
      }
      // Collapse the spaces/newlines pdf.js sprinkles between fragments into a
      // clean single-space / single-newline so the walker isn't fed runs of
      // whitespace.
      pageText = pageText
        .replace(/[ \t]+/g, " ")
        .replace(/ *\n */g, "\n")
        .trim();
      if (pageText) pages.push(pageText);
      page.cleanup();
    }
    return pages.join("\n\n");
  } finally {
    await loadingTask.destroy();
  }
}
