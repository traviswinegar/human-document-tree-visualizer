// Phase 5 #3 (ADR-0006) — PDF text extraction in the frontend via pdf.js.
// Phase 6 #3 — OCR fallback for scanned / image-only PDFs via tesseract.js.
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
// When a PDF carries little or no embedded text (a scan, or pages that are just
// page-images), pdf.js extraction comes back near-empty. In that case we fall
// back to **OCR**: render each page to a canvas and read the glyphs with
// tesseract.js. tesseract.js (its worker, the WASM core, and the language data)
// is itself a *dynamic* import that loads only on that fallback path, so it
// never weighs down the initial bundle or a normal text-PDF open.
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

// Collapse the spaces/newlines pdf.js / tesseract sprinkle between fragments into
// a clean single-space / single-newline so the walker isn't fed runs of
// whitespace. Shared by the text and OCR paths.
function tidy(s: string): string {
  return s
    .replace(/[ \t]+/g, " ")
    .replace(/ *\n */g, "\n")
    .trim();
}

export interface PdfExtractOptions {
  /** Progress callback during the (slow) OCR fallback: page `done` of `total`. */
  onOcrProgress?: (done: number, total: number) => void;
  /** Total embedded-text length (chars) below which OCR kicks in. Defaults to a
   *  per-page minimum (≈16 chars/page) so a genuine scan triggers OCR but a real
   *  text PDF never pays for it. */
  ocrThreshold?: number;
}

// Extract the document's text, page by page, joining pages with a blank line so
// the downstream walker sees a paragraph break between pages. Within a page,
// fragments are joined on spaces and pdf.js's end-of-line hints become newlines.
//
// If the embedded text comes back below `ocrThreshold` (a scanned / image-only
// PDF), fall back to OCR (#3) and return whichever yields more text.
//
// Limits (documented, acceptable — the same class of text the walker already
// tolerates from any source): reading order can be imperfect on complex
// multi-column layouts; OCR accuracy depends on scan quality; and OCR fetches
// its engine + English language data on first use (so the very first OCR needs
// network — cached by the browser/webview thereafter).
export async function extractPdfText(
  file: File,
  opts: PdfExtractOptions = {}
): Promise<string> {
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
      pageText = tidy(pageText);
      if (pageText) pages.push(pageText);
      page.cleanup();
    }
    const embedded = pages.join("\n\n");
    const threshold = opts.ocrThreshold ?? 16 * doc.numPages;
    if (embedded.length >= threshold) return embedded;

    // Near-empty: treat as a scan and OCR every page. Return whichever path
    // produced more text (OCR can still come up short on a noisy scan).
    const ocr = await ocrPdfDoc(doc, opts.onOcrProgress);
    return ocr.length > embedded.length ? ocr : embedded;
  } finally {
    await loadingTask.destroy();
  }
}

// Render every page of an already-open PDF to a canvas and OCR it with
// tesseract.js. tesseract.js is dynamically imported here so it only loads when
// a scan is actually encountered. One worker is created up front and reused
// across pages (worker spin-up is the expensive part), then terminated.
async function ocrPdfDoc(
  doc: import("pdfjs-dist").PDFDocumentProxy,
  onProgress?: (done: number, total: number) => void
): Promise<string> {
  const Tesseract = await import("tesseract.js");
  const worker = await Tesseract.createWorker("eng");
  try {
    const pages: string[] = [];
    const total = doc.numPages;
    for (let i = 1; i <= total; i++) {
      const page = await doc.getPage(i);
      // 2× scale: legible glyphs for OCR without ballooning canvas memory.
      const viewport = page.getViewport({ scale: 2 });
      const canvas = document.createElement("canvas");
      canvas.width = Math.ceil(viewport.width);
      canvas.height = Math.ceil(viewport.height);
      const ctx = canvas.getContext("2d");
      if (ctx) {
        await page.render({ canvasContext: ctx, canvas, viewport }).promise;
        const {
          data: { text },
        } = await worker.recognize(canvas);
        const clean = tidy(text);
        if (clean) pages.push(clean);
      }
      page.cleanup();
      canvas.width = 0; // release the backing store
      canvas.height = 0;
      onProgress?.(i, total);
    }
    return pages.join("\n\n");
  } finally {
    await worker.terminate();
  }
}
