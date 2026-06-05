# PLAN — Phase 10: tokenized-file round-trip

> Implementation plan for [ADR-00018](../adr/ADR-00018-tokenized-file-roundtrip.md).
> Goal: upload → tree → **tokenize to a `.dttok.json` file** → **import it** → decode
> back to doc + tree → **diff** original vs reconstructed, automatable in one action.
> Archived to `docs/plans/archive/` when it ships.

## Standing invariants
- Pure tokenizer — the new decode-from-tokens path is **native-free** (no model), so
  the file round-trip is unit-tested under `cargo test --workspace`.
- Reuse the lossless core (`decode_text`/`decode_graph`), the restore path (ADR-0007),
  and the existing byte-diff. No new core logic — only a new decode *entry point*.
- Test-first for the engine pieces; atomic `Phase 10 #M:` commits; two-commit ledger.

## Milestones

### M1 — Engine: decode-from-tokens (test-first)
- **WASM:** `decodeTokens(ids_json) -> { text, graph }` (build `Tokens(ids)`, then
  `decode_text` + `decode_graph`). `tokenizeGraph` already returns ids.
- **Tauri:** `tokenize_document(text, graph) -> { ids, tokens, docBytes, vocabSize }`
  (ids for the file) + `decode_tokens(ids) -> { text, graph, tokens }`.
- **Pinned tests** (native-free, both crates): `decode_tokens(tokenize(doc).ids)`
  reproduces the document **byte-exact** and the graph **exactly**, decoding from the
  ids alone; a malformed id stream returns a `TokenizeError` (no panic).
- **Done when:** the round-trip-from-ids tests pass native-free.

### M2 — Frontend bridges + Export tokens
- `doc-source.ts`: `tokenizeDocument(text, graph)` and `decodeTokens(ids)` (Tauri or
  WASM, by `isTauri()`), with a `TokenFile` type.
- **Export tokens** button → `<name>.dttok.json` Blob download (the established Export
  pattern). *"tokenize to a FILE".*
- **Done when:** a loaded doc exports a valid, re-parseable `.dttok.json`.

### M3 — Import tokens → tree + doc
- **Import tokens** file picker (`.dttok.json`) → parse `ids` → `decodeTokens` →
  restore the recovered **graph as the live tree** (ADR-0007 restore path) + set the
  recovered **document** (sidebar / reconstruct view).
- **Done when:** importing a file produced by M2 rebuilds the same tree + document.

### M4 — Automated round-trip + diff
- **Round-trip** action: tokenize the live pair → download the `.dttok.json` → decode
  the same ids → **diff** original vs recovered document (reuse `computeByteDiff`) →
  show the diff (`identical ✓` on success; first divergence otherwise) + offer to load
  the recovered tree. One click, file produced, diff shown.
- **Done when:** one action runs the whole pipeline and the diff appears.

### M5 — Verify + ship
- Native-free gates green; `tsc`/`vite` green; rebuild the full exe. The live UX (file
  download/upload dialogs, tree reload) is the user's end test (webview not headless).
- Two-commit ledger; archive this plan.

## Risks (and mitigations)
| Risk | Mitigation |
|------|------------|
| Large docs → multi-MB id arrays / JSON | Acceptable for an analysis artifact (ADR-00013: not compression); note it in the UI. |
| Corrupted/edited `.dttok.json` | `decode_*` returns `TokenizeError` → surfaced as a clear error, not a panic. |
| Import has no in-session original to diff | Show the recovered doc/tree; diff only when an original is loaded (round-trip case). |
| Browser can't auto-read its own download | The automated round-trip decodes the in-memory ids it just wrote; Export/Import cover the true cross-session file path. |
