# ADR-00018 — Tokenized-file round-trip (export → import → decode → diff)

- **Status:** Accepted (2026-06-05)
- **Phase:** 10 (#1)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-00013](ADR-00013-reversible-graph-tokenizer.md),
  [ADR-0007](ADR-0007-graph-persistence-save-load.md)
- **Supersedes / superseded by:** none

## Context

The reversible tokenizer (ADR-00013) is lossless, but today it only round-trips
**in memory**: the `Reconstruct` button re-encodes the live `(document, graph)` pair
and projects it back. The user wants the token stream to be a **real, portable file**
that round-trips **through disk**:

> upload a doc → visualize the tree → **tokenize to a FILE** → **import the tokenized
> file** → convert it back to tree or doc → **diff** the original doc vs the
> reconstructed one. "It's fine if this is all automated."

Every current surfacing (`reconstructDocument`, `reconstructGraph`, `tokenizeGraph`,
`reconstruct_document`, `tokenize_stats`) takes `(text, graph)` and **re-encodes**.
None **decodes from a raw token stream** — which is exactly what reading a token file
requires. So two things are missing: a **token-file format** (a contract, since files
outlive a session) and a **decode-from-tokens** path.

## Decision

### 1. Token-file format — `*.dttok.json`

A small, inspectable JSON document (the research artifact is meant to be read):

```json
{ "format": "doctree-tokens/v1", "vocabSize": 296, "tokens": 1234,
  "docBytes": 5678, "ids": [ … u32 token ids … ] }
```

`ids` is the payload (the `doctree_core::tokenizer::encode` stream); the rest is
metadata for display and a sanity check on import. Versioned (`format`) for
forward-compatibility. JSON over a binary blob because the file is meant to be
opened and studied (agenticmd.org testbench), and it parses trivially in both Rust
(serde) and the browser.

### 2. Decode-from-tokens path (the new engine capability)

Add a path that takes **raw ids** and projects both ways, mirroring the existing
`*_impl`/`*_json` pattern:

- **Tauri:** `tokenize_document(text, graph) -> { ids, tokens, docBytes, vocabSize }`
  (the desktop side currently exposes only `tokenize_stats`, no ids — needed to write
  the file) and `decode_tokens(ids) -> { text, graph, tokens }` (build `Tokens(ids)`,
  then `decode_text` + `decode_graph`).
- **WASM:** `tokenizeGraph` already returns ids; add `decodeTokens(ids) -> { text,
  graph }`. Same `doctree-core` engine, so desktop and browser behave identically.

Both wrap the existing `decode_text` / `decode_graph` — no new core logic, only a new
*entry point* (decode from an external stream rather than a re-encoded live pair).

### 3. Surfacing (the user's flow, automated)

- **Export tokens** — from the live `(document, graph)`, write `<name>.dttok.json`
  (Blob download on web; the established Export pattern). *This is "tokenize to a FILE".*
- **Import tokens** — pick a `.dttok.json`, `decodeTokens(ids)` → load the recovered
  **graph as the live tree** (the restore path, ADR-0007) and the recovered **document**.
  *This is "import → convert to tree or doc".*
- **Round-trip (one action)** — on the loaded doc: tokenize → download the file →
  decode the same ids → **diff** the original vs the recovered document (reusing the
  existing byte-diff) → show it. *This is the whole pipeline automated; the diff reads
  `identical ✓` on a clean round-trip and pinpoints any divergence otherwise.*

## Alternatives considered

1. **Reuse the existing in-memory `Reconstruct` only.** Rejected — it never produces or
   reads a file, so it cannot prove a *file* round-trips (the user's explicit ask).
2. **Binary `.dttok` (u16/u32 LE).** More compact, but opaque; the file is meant to be
   inspectable. (The research subproject's `arm_c.u16` is the binary form for training;
   the app's artifact is the readable JSON.)
3. **Embed the original text in the file to diff against on import.** Rejected — the
   tokens *are* the document (losslessly); storing the original too is redundant and
   would let the file disagree with itself. The diff compares against the in-session
   original (round-trip) or simply shows the recovered doc on a cold import.
4. **A decode that re-derives the graph from text via the walker.** Rejected — that
   would reconstruct an *approximation*, not the exact graph the file encodes;
   `decode_graph` returns the precise graph by construction.

## Consequences

- (+) The token stream becomes a **real, portable, inspectable file** that provably
  round-trips through disk to the **byte-exact** document and the **exact** graph.
- (+) **One-click automated** pipeline (upload → … → diff) as requested, with the file
  genuinely written and re-read.
- (+) Reuses the lossless core, the restore path (ADR-0007), and the byte-diff —
  contained additions, no new core logic.
- (−) The decode path trusts the file's `ids`; a hand-corrupted/edited file may fail to
  decode (`decode_*` returns a `TokenizeError`) — surfaced as a clear error, and the
  diff would show the divergence if it decodes to different bytes.
- (−) Large documents → large `ids` arrays → multi-MB JSON files (interleaving is not
  compression — ADR-00013). Acceptable for an analysis artifact.

## Invariant (pinned)

The file round-trip is pinned native-free (no model needed — pure tokenizer):

- **`decode_tokens(tokenize(doc, walk(doc)).ids)` reproduces the document byte-exact
  AND the graph exactly** — i.e. `decode_text` == `doc` and `decode_graph` ==
  `walk(doc)`, decoding from the *ids alone* (the file payload), not a re-encoded pair.
- A malformed id stream yields a `TokenizeError`, surfaced (not a panic).

Pinned test paths: `crates/doctree-wasm/src/lib.rs` (`decode_tokens_json` round-trip)
and `src-tauri/src/lib.rs` (`decode_tokens_impl` round-trip), both native-free; the
core `decode_text`/`decode_graph` invariants (ADR-00013) underwrite them.

## Anchors

- `crates/doctree-wasm/src/lib.rs` — `tokenizeGraph` (ids) + new `decodeTokens`.
- `src-tauri/src/lib.rs` — new `tokenize_document` (ids) + `decode_tokens` commands.
- `src/doc-source.ts` — `tokenizeDocument` / `decodeTokens` bridges.
- `src/main.ts` + `index.html` — Export tokens / Import tokens / Round-trip + the diff.
- `docs/plans/PLAN-phase10-tokenized-file-roundtrip.md` — milestones.
