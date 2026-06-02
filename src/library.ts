// Phase 5 #4 (ADR-0007) + Phase 6 #2 — frontend data layer for the saved-graph library.
//
// Two backends behind one `Library` interface, chosen at runtime by `isTauri()`:
//   - **desktop (Tauri):** the Rust `library` module persists each graph as an
//     opaque JSON file under app-data (ADR-0007), reached via 5 Tauri commands.
//   - **browser / WASM:** an **IndexedDB** store in the page's origin, so the
//     public web build can save / list / reopen too (Phase 6 #2). No disk, no
//     server — and nothing leaves the machine.
//
// The opaque, file-portable `SavedDoc` shape (ADR-0007) is identical on both
// paths, so **export / import** (a downloaded `*.doctree.json`) moves a graph
// between backends and between machines. `SavedMeta` is *derived* from the stored
// doc on both paths (counts authoritative), so a listing looks the same either way.

import { tauriInvoke, isTauri } from "./doc-source";
import type { Routing } from "./doc-source";
import type { GraphNode, GraphEdge } from "./types";

// Lightweight per-graph metadata for the Library list — mirrors `SavedMeta` in
// `src-tauri/src/library.rs` (serde camelCase). The counts come from the stored
// arrays, so they're authoritative, not self-reported.
export interface SavedMeta {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  nodeCount: number;
  edgeCount: number;
  origin: string;
  /** Document class (e.g. "narrative"), when the save carried a routing verdict. */
  class?: string;
}

/** A node's resting position, captured so a reopened graph restores its exact
 *  layout instead of re-simulating from scratch. */
export interface SavedPosition {
  x: number;
  y: number;
  z: number;
}

// The full saved document. Persisted opaquely by both backends, so this shape is
// owned here on the frontend. It carries everything needed to restore the view
// with **no re-walk and no re-layout**: the source `text`, the complete
// `nodes`/`edges`, each node's `positions`, and the `layout` mode that was active.
// `id` is absent on first save (the backend mints one) and present on every
// re-save (so it updates in place).
export interface SavedDoc {
  schemaVersion: 1;
  id?: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  origin: string;
  routing?: Routing;
  text: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  positions?: Record<string, SavedPosition>;
  layout?: string;
}

// The storage contract both backends satisfy (mirrors the 5 Tauri commands).
export interface Library {
  saveDoc(doc: SavedDoc): Promise<SavedMeta>;
  listDocs(): Promise<SavedMeta[]>;
  loadDoc(id: string): Promise<SavedDoc>;
  deleteDoc(id: string): Promise<void>;
  renameDoc(id: string, name: string): Promise<SavedMeta>;
}

// Derive the listing metadata from a full doc, exactly as the Rust `extract_meta`
// does (counts from the arrays, class from the routing verdict). Used by the
// IndexedDB backend (the desktop backend computes its own meta in Rust). Exported
// so import-validation can reuse it.
export function extractMeta(doc: SavedDoc): SavedMeta {
  const meta: SavedMeta = {
    id: doc.id ?? "",
    name: doc.name,
    createdAt: doc.createdAt,
    updatedAt: doc.updatedAt,
    nodeCount: Array.isArray(doc.nodes) ? doc.nodes.length : 0,
    edgeCount: Array.isArray(doc.edges) ? doc.edges.length : 0,
    origin: doc.origin,
  };
  if (doc.routing?.class) meta.class = doc.routing.class;
  return meta;
}

// --- Tauri (desktop) backend: the ADR-0007 commands, unchanged. --------------
const tauriLibrary: Library = {
  saveDoc: (doc) => tauriInvoke<SavedMeta>("save_doc", { doc }),
  listDocs: () => tauriInvoke<SavedMeta[]>("list_docs", {}),
  loadDoc: (id) => tauriInvoke<SavedDoc>("load_doc", { id }),
  deleteDoc: (id) => tauriInvoke<void>("delete_doc", { id }),
  renameDoc: (id, name) => tauriInvoke<SavedMeta>("rename_doc", { id, name }),
};

// --- Browser backend: IndexedDB, mirroring the desktop semantics. ------------
const DB_NAME = "doctree-library";
const STORE = "docs";

let dbReady: Promise<IDBDatabase> | null = null;
function openDb(): Promise<IDBDatabase> {
  if (!dbReady) {
    dbReady = new Promise<IDBDatabase>((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, 1);
      req.onupgradeneeded = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains(STORE)) {
          db.createObjectStore(STORE, { keyPath: "id" });
        }
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }
  return dbReady;
}

// Run one request in its own transaction, resolving with the request's result
// only **after the transaction commits** (so a following read sees the write).
function runTx<T>(
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => IDBRequest
): Promise<T> {
  return openDb().then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const t = db.transaction(STORE, mode);
        const req = run(t.objectStore(STORE));
        let result: T;
        req.onsuccess = () => {
          result = req.result as T;
        };
        t.oncomplete = () => resolve(result);
        t.onerror = () => reject(t.error ?? req.error);
        t.onabort = () => reject(t.error ?? req.error);
      })
  );
}

// Filesystem-free id: time-ordered (so the default sort is chronological) + a
// slug of the name + a short random suffix for collision-resistance. Mirrors the
// intent of the Rust id (timestamp + slug) without needing to match its bytes.
function mintId(name: string): string {
  const slug =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40) || "graph";
  const rand = Math.random().toString(36).slice(2, 8);
  return `${Date.now().toString(36)}-${slug}-${rand}`;
}

const idbLibrary: Library = {
  async saveDoc(doc) {
    const toStore: SavedDoc = {
      ...doc,
      id: doc.id ?? mintId(doc.name),
      updatedAt: new Date().toISOString(),
    };
    await runTx("readwrite", (s) => s.put(toStore));
    return extractMeta(toStore);
  },
  async listDocs() {
    const all = await runTx<SavedDoc[]>("readonly", (s) => s.getAll());
    return all
      .map(extractMeta)
      .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : a.updatedAt > b.updatedAt ? -1 : 0));
  },
  async loadDoc(id) {
    const doc = await runTx<SavedDoc | undefined>("readonly", (s) => s.get(id));
    if (!doc) throw new Error(`no saved graph with id ${id}`);
    return doc;
  },
  async deleteDoc(id) {
    await runTx("readwrite", (s) => s.delete(id));
  },
  async renameDoc(id, name) {
    const doc = await idbLibrary.loadDoc(id);
    doc.name = name;
    doc.updatedAt = new Date().toISOString();
    await runTx("readwrite", (s) => s.put(doc));
    return extractMeta(doc);
  },
};

/** The active backend for this runtime (desktop → Tauri, browser → IndexedDB). */
export function getLibrary(): Library {
  return isTauri() ? tauriLibrary : idbLibrary;
}

// Thin free-function facade so callers don't care which backend is live — each
// dispatches to `getLibrary()` (resolved per call, cheap).

/** Persist a graph (mints an id on first save, updates in place on re-save). */
export const saveDoc = (doc: SavedDoc): Promise<SavedMeta> => getLibrary().saveDoc(doc);
/** List saved graphs, newest-first. */
export const listDocs = (): Promise<SavedMeta[]> => getLibrary().listDocs();
/** Load one saved graph in full (for instant restore). */
export const loadDoc = (id: string): Promise<SavedDoc> => getLibrary().loadDoc(id);
/** Delete one saved graph. */
export const deleteDoc = (id: string): Promise<void> => getLibrary().deleteDoc(id);
/** Rename one saved graph; returns the refreshed meta. */
export const renameDoc = (id: string, name: string): Promise<SavedMeta> =>
  getLibrary().renameDoc(id, name);
