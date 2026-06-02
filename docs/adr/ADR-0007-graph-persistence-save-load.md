# ADR-0007 — Graph persistence (save / load library) via schema-agnostic Tauri commands

- **Status:** Accepted (2026-06-02)
- **Phase:** 5 (#4)
- **Supersedes / superseded by:** none

## Context

Walking a large document is expensive — the structural spine is fast, but the
desktop **semantic / embedding** pass is CPU-bound and runs tens of seconds to
minutes (the user's 650 KB novel → 18 437 nodes / 46 477 edges, hybrid LLM). The
user asked to **save the graph and reopen it without re-graphing**, with "full
file CRUD".

The desktop shell (Tauri) has filesystem access; the public browser build does
not. So persistence is a **desktop-only** capability for v1.

## Decision

Add desktop Tauri commands that persist each graph as JSON in the app data
directory, managed as a small library:

```
{app_data_dir}/library/
  index.json            # Vec<SavedMeta> — lightweight, for fast listing
  {id}.doctree.json     # one full SavedDoc per saved graph
```

**Commands** (registered in `generate_handler!`, so no extra ACL grant — custom
app commands don't need one, unlike Tauri's own plugin APIs):

- `save_doc(doc: Value) -> SavedMeta`
- `list_docs() -> Vec<SavedMeta>`
- `load_doc(id: String) -> Value`
- `delete_doc(id: String) -> ()`
- `rename_doc(id: String, name: String) -> SavedMeta`

**The Rust layer is schema-agnostic.** It persists the document as an opaque
`serde_json::Value` and extracts only the meta fields it needs
(`id, name, createdAt, updatedAt, nodeCount, edgeCount, origin, class`). It never
deserializes the graph itself — so persistence does **not** couple to the
evolving node/edge schema, and `doctree-core`'s types need no `Deserialize`
impl. The frontend owns the document shape.

**SavedDoc (frontend-owned shape):**

```ts
{
  schemaVersion: 1,
  id: string,            // timestamp + slug, stable
  name: string,          // user-facing; defaults to the source filename
  createdAt, updatedAt,  // ISO strings
  origin: string,        // "tauri-walk" | "wasm-walk" | "saved" | …
  routing?: Routing,     // the B5 verdict, so the panel/routing line restores
  text: string,          // the source document (so meaning-search / re-walk work)
  nodes: GraphNode[],    // full graph — semantic layer already baked in
  edges: GraphEdge[],    // id-based source/target (d3 object refs stripped)
  positions?: Record<string, [number, number, number]>,  // saved layout
  layout?: LayoutMode,
}
```

`positions` + `layout` let a reopen **restore the exact view with no re-walk and
no re-simulation** (`cooldownTicks(0)` on load) — the heart of "don't regraph
every time" and a direct mitigation of the navigation lag at scale.

**Frontend CRUD** = a "Save" action + a "Library" modal (list with Open / Rename
/ Delete), both gated on `isTauri()`.

## Alternatives considered

1. **SQLite (`tauri-plugin-sql`).** Indexing, queries. **Rejected for v1:**
   heavier, plugin + ACL setup, overkill for a handful of opaque doc blobs;
   plain JSON files are inspectable and trivially CRUD-able.
2. **`tauri-plugin-fs` + `tauri-plugin-dialog`** for user-chosen save paths.
   **Rejected for v1:** ACL/permission complexity; a managed app-data library is
   simpler and matches the "library of my graphs" intent. Export/import to an
   arbitrary path is a backlog item.
3. **Typed Rust graph model (derive `Deserialize` on core types).** **Rejected:**
   couples persistence to a schema still in motion; the opaque-`Value` approach
   is forward-compatible by construction (`schemaVersion` guards migrations).
4. **Browser persistence (IndexedDB).** Deferred — desktop is the user's path.

## Consequences

- (+) Reopen is instant — no walk, no simulation; the lag complaint is sidestepped
  for saved graphs.
- (+) Persistence is decoupled from the graph schema; the format versions itself.
- (−) Library lives in app data, not a user-chosen folder (v1 scope).
- (−) `index.json` can desync from the files; mitigated by making it rebuildable
  from the `{id}.doctree.json` files (a `list_docs` repair path).
- (−) Large semantic graphs are multi-MB JSON (acceptable; gzip is a backlog
  option).
- (−) A loaded saved graph is **static** in v1 (no animated player); Replay /
  animate-from-saved is backlog.

## Invariant (pinned)

- **Test path:** `cargo test -p doctree-tauri` — the pure persistence helpers are
  unit-tested **test-first**: `extract_meta(&Value) -> SavedMeta` (counts +
  fields, robust to missing keys), `upsert_index(Vec<SavedMeta>, SavedMeta)`
  (replace-by-id or append, newest-first), `doc_filename(id)` / id generation
  (filesystem-safe, collision-resistant). The fs-touching command bodies stay
  thin wrappers over these helpers + `std::fs`.
- **Gate:** `cargo test -p doctree-tauri` (default, native-free build) green;
  frontend `tsc` + `build` (0).
- **Decoupling check:** the persistence module compiles with **zero** new
  dependencies beyond `serde_json` + `std::fs` + `tauri::Manager` — no native,
  no LLM, no GPU (ADR-0001 holds).
