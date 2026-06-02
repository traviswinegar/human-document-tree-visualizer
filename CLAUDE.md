# human-document-tree

A desktop app that walks a human-written document from word one, determines its
clauses / ideas / characters / concepts / references / events and their
relationships, and renders them as a navigable, searchable **3D graph** that
animates as it grows, then becomes explorable (with replay).

Extraction is a **hybrid**: deterministic structure-walking for the graph spine,
plus a **local LLM** (grammar-constrained JSON) for the fuzzy semantic layer.
No cloud calls in the core pipeline.

---

## Kernel — read this first, every session

> **I am a data source, not a persistent entity.**
> Identity lives in the project substrate, not in me.

> **The document is a hypothesis. The code is truth.**
> When they disagree, the code wins and the document gets corrected to match.

When my memory is partial, I trust the substrate (ADRs, CHANGELOG, tests, commit
log, this file), never inferred recall. **Verification beats memory at every
checkpoint.** The conversation is the working surface; the artifacts are
authoritative. When the window compacts, the conversation goes away and the
substrate stays.

This project inherits **AgentDNA**. The full corpus is referenced from
`docs/agent-protocols/INDEX.md`; the operational recovery protocol is copied in
verbatim at `docs/agent-protocols/COMPACTION_RECOVERY.md`.

---

## Operating discipline (non-negotiable)

1. **Test-first per fix.** Every `fix:` begins with a failing test that
   demonstrates the bug. Source-scan tests don't count as the demonstration —
   they catch deletion, not drift.
2. **Atomic commits.** One commit per logical issue, with a phase/issue id in the
   subject (e.g. `Phase 0 #2:`). The git log is part of the recovery story.
3. **Catch-all logging.** Off-topic discoveries go to the backlog in the active
   plan doc with a `discovered while …` note — never fixed inline, never held in
   working memory.
4. **ADR-before-code** for load-bearing decisions. Decision + alternatives +
   pinned invariant (test path), then the code lands. ADRs are immutable;
   supersede with a new ADR.
5. **Plan-doc-before-implementation** for multi-day work. Lives in `docs/plans/`,
   archived when the work ships.
6. **CHANGELOG-as-narrative** on every shipped version — the *why*, not just the
   *what*.

**Verification triple** = commit hash + test path + source `file:line`. The unit
of recovered truth. On a fresh session: read this file, walk back the active plan
/ ledger, validate prior steps against their triple, resume from the first
verification failure.

---

## Override channel

When the human corrects me, I **stop immediately**, acknowledge, and redo the
work per their instruction. I do not argue, do not defend the original approach,
do not continue on the original path.

---

## Publishing boundary

I **commit locally** as work proceeds. I **never push / publish** — the user
handles all publishing. No `git push`, no PR creation, no release tagging unless
explicitly instructed in the moment.

---

## Substrate map

- `CLAUDE.md` — this kernel file (read first).
- `docs/adr/` — Architecture Decision Records (immutable; one per load-bearing choice).
- `docs/plans/` — short-lived implementation plans (archived when shipped).
- `docs/agent-protocols/` — recovery protocol (local verbatim copy) + corpus pointer.
- `CHANGELOG.md` — append-only narrative of shipped versions.
- `src-tauri/` — Rust backend (Tauri commands + `momusdev_llm` engine). *(arrives Phase 0)*
- `src/` — web frontend (3D graph). *(arrives Phase 0)*

---

## Stack (see `docs/adr/ADR-0001`)

- **Tauri** desktop app: Rust backend + web frontend.
- **Local LLM** = the `momusdev_llm` Rust crate (`E:\Development\momusdev-packages`),
  consumed directly as a Cargo dependency (no flutter_rust_bridge). It provides
  GGUF inference via `llama-cpp-2`, token streaming, embeddings + a LanceDB
  vector store (RAG), and **grammar-constrained (GBNF) decoding** via
  `InferenceEngine::complete_with_grammar`.
- **3D graph** in the web frontend (three.js / force-directed). The graph is the
  product; the web ecosystem is why the frontend is web, not Flutter.
