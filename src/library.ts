// Phase 5 #4 (ADR-0007) — frontend data layer for the saved-graph library.
//
// The Rust `library` module persists each graph as an opaque JSON document; this
// module is the typed mirror of that contract plus thin wrappers over the five
// Tauri commands. It is **desktop-only**: every call goes through `tauriInvoke`,
// so callers must gate the UI on `isTauri()` (the browser/WASM build has no disk
// to write to). Keeping the wrappers here — not in `doc-source.ts` — keeps the
// build/walk path and the persistence path as separate concerns.

import { tauriInvoke } from "./doc-source";
import type { Routing } from "./doc-source";
import type { GraphNode, GraphEdge } from "./types";

// Lightweight per-graph metadata for the Library list — mirrors `SavedMeta` in
// `src-tauri/src/library.rs` (serde camelCase). The backend computes the counts
// from the stored arrays, so they're authoritative, not self-reported.
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

// The full saved document. Persisted opaquely by the backend (which only peeks at
// the meta fields), so this shape is owned here on the frontend. It carries
// everything needed to restore the view with **no re-walk and no re-layout**: the
// source `text`, the complete `nodes`/`edges`, each node's `positions`, and the
// `layout` mode that was active. `id` is absent on first save (the backend mints
// one) and present on every re-save (so it updates in place).
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

/** Persist a graph (mints an id on first save, updates in place on re-save). */
export function saveDoc(doc: SavedDoc): Promise<SavedMeta> {
  return tauriInvoke<SavedMeta>("save_doc", { doc });
}

/** List saved graphs, newest-first. */
export function listDocs(): Promise<SavedMeta[]> {
  return tauriInvoke<SavedMeta[]>("list_docs", {});
}

/** Load one saved graph in full (for instant restore). */
export function loadDoc(id: string): Promise<SavedDoc> {
  return tauriInvoke<SavedDoc>("load_doc", { id });
}

/** Delete one saved graph. */
export function deleteDoc(id: string): Promise<void> {
  return tauriInvoke<void>("delete_doc", { id });
}

/** Rename one saved graph; returns the refreshed meta. */
export function renameDoc(id: string, name: string): Promise<SavedMeta> {
  return tauriInvoke<SavedMeta>("rename_doc", { id, name });
}
