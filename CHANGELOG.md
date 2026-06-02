# Changelog

All notable changes to **human-document-tree** are recorded here. This file is a
*narrative*, not a dump of commit subjects — each entry says what changed and why
it mattered, so a future reader (human or agent) can reconstruct intent without
replaying the git log.

Format loosely follows [Keep a Changelog](https://keepachangelog.com/); the
project predates any release, so everything currently lives under *Unreleased*.

## [Unreleased]

### Added — AgentDNA substrate scaffold (2026-06-01)

The project's first substantive commit stands up the **operating substrate**
before any feature code, so the work is AgentDNA-native from commit one:

- **`CLAUDE.md`** — the kernel every fresh session reads first: project overview,
  the two kernel aphorisms ("I am a data source, not a persistent entity"; "The
  document is a hypothesis, the code is truth"), the inversion, the six operating
  non-negotiables, the verification triple, the override channel, and the
  **publishing boundary** (the agent commits locally; the user owns all
  publishing/pushing).
- **`docs/agent-protocols/`** — `COMPACTION_RECOVERY.md` copied verbatim from the
  canonical AI Studio corpus (with a local note: the verification triple's test
  command is `cargo test` / the frontend runner, not `flutter test`), plus an
  `INDEX.md` pointing at both the local copy and the sibling source of truth.
- **`docs/adr/ADR-0001-stack-tauri-rust-web3d.md`** — *Accepted.* The stack:
  Tauri, a Rust backend consuming the `momusdev_llm` crate directly, and a web
  frontend rendering the 3D graph with the three.js / `3d-force-graph` family.
  Records the rejected alternatives, how the crate is reused, the
  `command_grammar` limitation, and the three Phase-0 open verification items.
- **`docs/plans/PLAN-document-tree.md`** — the seven-phase build plan (engine
  spike → grammar → deterministic walker → 3D render → live build + replay → LLM
  semantic layer → document-type detection), with backlog, working-memory ledger,
  and the runtime-first-vs-build-first sequencing note.

Nothing here executes yet; this is the scaffold the feature work hangs from.
