//! Phase 5 #4 (ADR-0007) — graph persistence: a small on-disk library of saved
//! document graphs, so the user can reopen a graph without re-walking it.
//!
//! **Schema-agnostic by design.** Each saved graph is persisted as an opaque
//! [`serde_json::Value`]; this module deserializes only the lightweight meta
//! fields it needs ([`SavedMeta`]) and never the graph itself. So persistence
//! does *not* couple to the evolving node/edge schema, and `doctree-core`'s types
//! need no `Deserialize` impl — the frontend owns the document shape. New deps:
//! none beyond `serde_json` + `std::fs` + `tauri::Manager` (ADR-0001 holds).
//!
//! Layout under the app data dir:
//! ```text
//! {app_data_dir}/library/
//!   index.json            # Vec<SavedMeta> — lightweight, for fast listing
//!   {id}.doctree.json     # one full opaque doc per saved graph
//! ```
//!
//! The fs-touching `*_impl` bodies are thin wrappers over the pure helpers
//! ([`extract_meta`], [`upsert_index`], [`doc_filename`], [`make_id`]) + `std::fs`,
//! and are exercised headlessly against a temp dir in this module's tests — no
//! Tauri runtime required (`cargo test -p doctree-tauri`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Lightweight metadata for one saved graph — everything the Library list needs
/// without loading (or deserializing) the full document. Serializes camelCase for
/// the frontend; `class` is optional because not every origin carries a routing
/// verdict (e.g. a plain in-browser structural walk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMeta {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub class: Option<String>,
}

// --- Pure helpers (unit-tested test-first; no fs, no Tauri) -----------------

/// Read the meta fields out of an opaque saved-doc value, robust to missing keys.
/// Counts are taken from the actual `nodes`/`edges` array lengths (authoritative),
/// not a self-reported count; `class` comes from the embedded routing verdict if
/// present. Never deserializes the graph — only peeks at the fields it names.
pub fn extract_meta(doc: &Value) -> SavedMeta {
    let s = |k: &str| doc.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let len = |k: &str| doc.get(k).and_then(Value::as_array).map_or(0, Vec::len);
    let class = doc
        .get("routing")
        .and_then(|r| r.get("class"))
        .and_then(Value::as_str)
        .or_else(|| doc.get("class").and_then(Value::as_str))
        .map(str::to_string);
    SavedMeta {
        id: s("id"),
        name: s("name"),
        created_at: s("createdAt"),
        updated_at: s("updatedAt"),
        node_count: len("nodes"),
        edge_count: len("edges"),
        origin: s("origin"),
        class,
    }
}

/// Insert `meta` into the index: replace any existing entry with the same id,
/// then place it at the front so the list reads newest-saved-first (a save bumps
/// the doc to the top, which is what the user just touched).
pub fn upsert_index(mut index: Vec<SavedMeta>, meta: SavedMeta) -> Vec<SavedMeta> {
    index.retain(|m| m.id != meta.id);
    index.insert(0, meta);
    index
}

/// Map a doc id to its on-disk filename. Sanitizes to `[A-Za-z0-9_-]` (every other
/// char, including `.` and any path separator, becomes `_`), so a hostile or stale
/// id can never escape the library dir via `..` or a slash — defense in depth on
/// top of the safe ids `make_id` produces.
pub fn doc_filename(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("{safe}.doctree.json")
}

/// Lowercase, hyphenated slug of `name`, ASCII-alphanumeric only, capped at 40
/// chars — the human-readable tail of an id.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.chars().take(40).collect()
}

/// Generate a stable, filesystem-safe, collision-resistant id: an epoch-millis
/// prefix (monotonic enough to disambiguate saves) plus a slug of the name. Pure
/// given `millis`, so it's deterministically testable.
pub fn make_id(name: &str, millis: u128) -> String {
    let slug = slugify(name);
    if slug.is_empty() {
        format!("{millis}")
    } else {
        format!("{millis}-{slug}")
    }
}

/// Wall-clock epoch millis (0 if the clock is before the epoch — never panics).
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn es<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// --- fs layer (thin wrappers over the helpers + std::fs) --------------------

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

/// Read the index, tolerating a missing or corrupt file (→ empty): the index is
/// a cache, rebuildable from the doc files (see [`list_docs_impl`]).
pub fn read_index(dir: &Path) -> Vec<SavedMeta> {
    match std::fs::read_to_string(index_path(dir)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn write_index(dir: &Path, index: &[SavedMeta]) -> Result<(), String> {
    let s = serde_json::to_string_pretty(index).map_err(es)?;
    std::fs::write(index_path(dir), s).map_err(es)
}

/// Scan the dir for `*.doctree.json` and rebuild the index from each doc's meta —
/// the repair path when `index.json` is missing or desynced. Sorted newest-first
/// by `updatedAt` (ISO-8601 sorts lexically).
fn rebuild_index(dir: &Path) -> Vec<SavedMeta> {
    let mut metas = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let is_doc = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".doctree.json"));
            if !is_doc {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<Value>(&s) {
                    metas.push(extract_meta(&v));
                }
            }
        }
    }
    metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    metas
}

/// Persist `doc` (opaque) and upsert its meta into the index. Mints an id from the
/// name when the doc carries none (first save), writing it back into the stored
/// value so the file, the meta, and the returned id all agree.
pub fn save_doc_impl(dir: &Path, mut doc: Value, millis: u128) -> Result<SavedMeta, String> {
    if !doc.is_object() {
        return Err("document must be a JSON object".into());
    }
    std::fs::create_dir_all(dir).map_err(es)?;

    let id = doc
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let name = doc.get("name").and_then(Value::as_str).unwrap_or("");
            make_id(name, millis)
        });
    doc["id"] = Value::String(id.clone());

    let body = serde_json::to_string(&doc).map_err(es)?;
    std::fs::write(dir.join(doc_filename(&id)), body).map_err(es)?;

    let meta = extract_meta(&doc);
    let index = upsert_index(read_index(dir), meta.clone());
    write_index(dir, &index)?;
    Ok(meta)
}

/// List saved graphs (newest-first). Falls back to rebuilding from the doc files
/// when the index is empty/missing, so a desynced or deleted index self-heals.
pub fn list_docs_impl(dir: &Path) -> Vec<SavedMeta> {
    let index = read_index(dir);
    if index.is_empty() {
        rebuild_index(dir)
    } else {
        index
    }
}

/// Load one saved graph as its opaque value (the frontend re-hydrates the shape).
pub fn load_doc_impl(dir: &Path, id: &str) -> Result<Value, String> {
    let path = dir.join(doc_filename(id));
    let s = std::fs::read_to_string(&path).map_err(|e| format!("load {id}: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("parse {id}: {e}"))
}

/// Delete a saved graph: remove its file (if present) and drop it from the index.
pub fn delete_doc_impl(dir: &Path, id: &str) -> Result<(), String> {
    let path = dir.join(doc_filename(id));
    if path.exists() {
        std::fs::remove_file(&path).map_err(es)?;
    }
    let index: Vec<SavedMeta> = read_index(dir).into_iter().filter(|m| m.id != id).collect();
    write_index(dir, &index)
}

/// Rename a saved graph in place: rewrite its `name`, re-persist, refresh the
/// index meta. The graph payload is untouched.
pub fn rename_doc_impl(dir: &Path, id: &str, name: &str) -> Result<SavedMeta, String> {
    let mut doc = load_doc_impl(dir, id)?;
    if !doc.is_object() {
        return Err("stored document is not a JSON object".into());
    }
    doc["name"] = Value::String(name.to_string());
    let body = serde_json::to_string(&doc).map_err(es)?;
    std::fs::write(dir.join(doc_filename(id)), body).map_err(es)?;

    let meta = extract_meta(&doc);
    let index = upsert_index(read_index(dir), meta.clone());
    write_index(dir, &index)?;
    Ok(meta)
}

/// The `{app_data_dir}/library` directory, created on demand.
fn library_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(es)?.join("library");
    std::fs::create_dir_all(&dir).map_err(es)?;
    Ok(dir)
}

// --- Tauri commands (thin) — desktop-only persistence; registered in lib.rs --

/// Save the current graph (opaque doc) and return its meta. Mints an id on first
/// save; upserts on re-save.
#[tauri::command]
pub fn save_doc(app: tauri::AppHandle, doc: Value) -> Result<SavedMeta, String> {
    save_doc_impl(&library_dir(&app)?, doc, now_millis())
}

/// List saved graphs, newest-first.
#[tauri::command]
pub fn list_docs(app: tauri::AppHandle) -> Result<Vec<SavedMeta>, String> {
    Ok(list_docs_impl(&library_dir(&app)?))
}

/// Load one saved graph by id (opaque value).
#[tauri::command]
pub fn load_doc(app: tauri::AppHandle, id: String) -> Result<Value, String> {
    load_doc_impl(&library_dir(&app)?, &id)
}

/// Delete one saved graph by id.
#[tauri::command]
pub fn delete_doc(app: tauri::AppHandle, id: String) -> Result<(), String> {
    delete_doc_impl(&library_dir(&app)?, &id)
}

/// Rename one saved graph; returns the refreshed meta.
#[tauri::command]
pub fn rename_doc(app: tauri::AppHandle, id: String, name: String) -> Result<SavedMeta, String> {
    rename_doc_impl(&library_dir(&app)?, &id, &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// A unique scratch dir under the OS temp dir (no tempfile crate — zero new
    /// deps per ADR-0007). Best-effort cleanup via [`cleanup`].
    fn unique_dir(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("doctree-lib-{tag}-{}-{n}", now_millis()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn sample(name: &str) -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "name": name,
            "createdAt": "2026-06-02T10:00:00.000Z",
            "updatedAt": "2026-06-02T10:00:00.000Z",
            "origin": "tauri-walk",
            "routing": { "class": "narrative" },
            "text": "Some text.",
            "nodes": [ {"id":"a"}, {"id":"b"}, {"id":"c"} ],
            "edges": [ {"source":"a","target":"b"} ]
        })
    }
    fn meta(id: &str, name: &str) -> SavedMeta {
        SavedMeta {
            id: id.into(),
            name: name.into(),
            created_at: String::new(),
            updated_at: String::new(),
            node_count: 0,
            edge_count: 0,
            origin: String::new(),
            class: None,
        }
    }

    // --- pure helpers -------------------------------------------------------

    #[test]
    fn extract_meta_reads_counts_and_fields() {
        let m = extract_meta(&sample("My Novel"));
        assert_eq!(m.name, "My Novel");
        assert_eq!(m.node_count, 3, "counts from the nodes array length");
        assert_eq!(m.edge_count, 1);
        assert_eq!(m.origin, "tauri-walk");
        assert_eq!(m.created_at, "2026-06-02T10:00:00.000Z");
        assert_eq!(m.class.as_deref(), Some("narrative"));
    }

    #[test]
    fn extract_meta_is_robust_to_missing_keys() {
        let m = extract_meta(&serde_json::json!({}));
        assert_eq!(m.node_count, 0);
        assert_eq!(m.edge_count, 0);
        assert_eq!(m.name, "");
        assert_eq!(m.origin, "");
        assert!(m.class.is_none());
    }

    #[test]
    fn upsert_replaces_by_id_and_keeps_newest_first() {
        let idx = upsert_index(vec![], meta("1", "A"));
        let idx = upsert_index(idx, meta("2", "B"));
        assert_eq!(
            idx.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["2", "1"],
            "newest save sits at the front"
        );
        // Re-saving id 1 replaces (not duplicates) and bumps it to the front.
        let idx = upsert_index(idx, meta("1", "A2"));
        assert_eq!(idx.len(), 2);
        assert_eq!(idx[0].id, "1");
        assert_eq!(idx[0].name, "A2");
    }

    #[test]
    fn doc_filename_cannot_escape_the_library_dir() {
        assert_eq!(doc_filename("1717-my-doc"), "1717-my-doc.doctree.json");
        let f = doc_filename("../../etc/passwd");
        assert!(!f.contains('/') && !f.contains('\\') && !f.contains(".."), "got {f}");
    }

    #[test]
    fn make_id_is_safe_unique_and_slugged() {
        let a = make_id("My Great Novel!", 1000);
        assert_eq!(a, "1000-my-great-novel");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        // Different millis ⇒ different id (collision-resistant across saves).
        assert_ne!(a, make_id("My Great Novel!", 1001));
        // A name with no usable chars degrades to just the timestamp.
        assert_eq!(make_id("", 42), "42");
        assert_eq!(make_id("!!!", 42), "42");
    }

    #[test]
    fn saved_meta_serializes_camel_case_for_the_frontend() {
        let v = serde_json::to_value(extract_meta(&sample("X"))).unwrap();
        for k in ["id", "name", "createdAt", "updatedAt", "nodeCount", "edgeCount", "origin"] {
            assert!(v.get(k).is_some(), "missing JSON key {k}");
        }
    }

    // --- fs round-trips (thin wrappers, exercised against a temp dir) --------

    #[test]
    fn save_then_load_round_trips_the_opaque_value() {
        let dir = unique_dir("roundtrip");
        let m = save_doc_impl(&dir, sample("Doc One"), 1_717_322_400_000).unwrap();
        assert!(m.id.starts_with("1717322400000-"), "id minted from millis+slug: {}", m.id);
        assert_eq!(m.node_count, 3);
        let loaded = load_doc_impl(&dir, &m.id).unwrap();
        assert_eq!(loaded["text"], "Some text.");
        assert_eq!(loaded["id"], m.id, "minted id is written back into the stored doc");
        assert_eq!(loaded["nodes"].as_array().unwrap().len(), 3);
        cleanup(&dir);
    }

    #[test]
    fn list_reflects_saves_newest_first() {
        let dir = unique_dir("list");
        let _m1 = save_doc_impl(&dir, sample("First"), 1000).unwrap();
        let m2 = save_doc_impl(&dir, sample("Second"), 2000).unwrap();
        let list = list_docs_impl(&dir);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, m2.id, "most recent save first");
        cleanup(&dir);
    }

    #[test]
    fn delete_removes_the_file_and_the_index_entry() {
        let dir = unique_dir("delete");
        let m = save_doc_impl(&dir, sample("Doomed"), 1000).unwrap();
        delete_doc_impl(&dir, &m.id).unwrap();
        assert!(list_docs_impl(&dir).is_empty());
        assert!(load_doc_impl(&dir, &m.id).is_err(), "file is gone");
        cleanup(&dir);
    }

    #[test]
    fn rename_changes_the_name_and_keeps_the_graph() {
        let dir = unique_dir("rename");
        let m = save_doc_impl(&dir, sample("Old Name"), 1000).unwrap();
        let m2 = rename_doc_impl(&dir, &m.id, "New Name").unwrap();
        assert_eq!(m2.id, m.id);
        assert_eq!(m2.name, "New Name");
        let loaded = load_doc_impl(&dir, &m.id).unwrap();
        assert_eq!(loaded["name"], "New Name");
        assert_eq!(loaded["nodes"].as_array().unwrap().len(), 3, "graph payload intact");
        cleanup(&dir);
    }

    #[test]
    fn index_rebuilds_from_files_when_it_desyncs() {
        let dir = unique_dir("repair");
        let m = save_doc_impl(&dir, sample("Survivor"), 1000).unwrap();
        std::fs::remove_file(index_path(&dir)).unwrap(); // simulate a lost index
        let list = list_docs_impl(&dir);
        assert_eq!(list.len(), 1, "rebuilt from the {{id}}.doctree.json file");
        assert_eq!(list[0].id, m.id);
        cleanup(&dir);
    }

    #[test]
    fn save_rejects_a_non_object_payload() {
        let dir = unique_dir("nonobj");
        assert!(save_doc_impl(&dir, serde_json::json!([1, 2, 3]), 1000).is_err());
        cleanup(&dir);
    }
}
