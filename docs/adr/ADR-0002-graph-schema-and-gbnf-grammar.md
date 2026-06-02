# ADR-0002 — Document-graph schema + GBNF extraction grammar (narrative ontology)

- **Status:** Accepted
- **Date:** 2026-06-01
- **Deciders:** Travis (user), agent
- **Supersedes:** —
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md)

## Context

The app renders a document as a `{ nodes, edges }` graph built by a **hybrid**
pipeline: a deterministic structure walker produces the spine, and a local LLM
produces the fuzzy semantic layer (ADR-0001). Two artifacts must be pinned before
either producer is built, because both producers *and* the frontend depend on
them:

1. a **graph schema** — the node/edge vocabulary (the narrative ontology) and its
   JSON encoding; and
2. a **GBNF grammar** — the deterministic contract that forces the LLM's output
   to be schema-valid *by construction* rather than "usually parseable" (the
   linchpin called out in ADR-0001).

Phase-1 genre is narrative/fiction, so the ontology is scoped to that.

## Decision

### Schema (`crates/doctree-core/src/schema.rs`)

A `Graph` is `{ nodes: [Node], edges: [Edge] }`. Nodes and edges are **flat
structs with a string discriminant** (`kind`), not Rust enums-with-data.

- **`NodeKind`** (serde `snake_case`):
  - *structural* (deterministic spine): `section`, `paragraph`, `sentence`,
    `clause`, `quote`, `reference`, `term`;
  - *semantic* (LLM, narrative): `character`, `place`, `concept`, `event`,
    `object`, `group`.
- **`EdgeKind`**: `part_of`, `precedes`, `mentions`, `references`, `quotes`,
  `co_occurs_with`, `interacts_with`, `located_in`, `relates_to`, `causes`,
  `similar_to`.
- **`Provenance`**: `structural` | `semantic` | `embedding` — drives
  frontend coloring (deterministic spine vs inferred vs similarity) and defaults
  to `semantic` (the only partial JSON we deserialize is LLM output).
- **`Node`**: `id`, `kind`, `label`, optional `text`, optional `span`
  (`{start,end}` char range into the source — provenance for navigation),
  `provenance`.
- **`Edge`**: optional `id`, `source`, `target`, `kind`, optional `label`,
  optional `weight` (for `co_occurs_with` / `similar_to`), `provenance`.
- **`Graph::validate`** returns dangling edges (endpoints with no node);
  **`Graph::merge`** unions a fragment, deduping nodes by `id` with the existing
  node winning — so an LLM fragment that references a spine id does not clobber
  the authoritative structural node.

Optional fields use `#[serde(default)]`, so a *partial* fragment (`id`+`kind`+
`label` for nodes; `source`+`target`+`kind` for edges) deserializes cleanly. This
is what lets the grammar emit a minimal shape while the deterministic layer fills
the rich fields.

### Grammar (`crates/doctree-core/src/grammar.rs`, `GRAPH_GBNF`)

A hand-authored GBNF grammar constrains the LLM to emit exactly
`{ "nodes":[…], "edges":[…] }` where:

- it covers the **semantic subset only** — node `kind` ∈ {character, place,
  concept, event, object, group}; edge `kind` ∈ {mentions, interacts_with,
  located_in, relates_to, causes, precedes}. Structural nodes come from the
  walker, not the model.
- each node is `{id, kind, label}` and each edge is `{source, target, kind}` —
  the **minimal** shape; Rust fills the rest. Ids are unconstrained strings so a
  semantic edge may reference a spine node id (coreference linking); referential
  integrity is checked in Rust via `Graph::validate`, which a grammar cannot
  express.
- it uses only the portable GBNF subset (`* + ?`, `|`, `(...)`, `[...]`,
  `"..."`); the unicode escape is spelled out as four `hex` refs rather than
  `{4}`, for maximum llama.cpp-version compatibility.

It is fed to the model via `InferenceEngine::complete_with_grammar` (ADR-0001),
**not** the narrow `build_command_grammar` (which is capped at ≤4 commands / one
arg — unusable here).

## Pinned invariant (the contract that must not silently drift)

- **Grammar ⊆ schema:** every `kind` the grammar can emit deserializes into the
  schema enums. Pinned by
  `crates/doctree-core/src/grammar.rs::tests::grammar_kinds_are_valid_schema_variants`
  and `…::grammar_enumerates_every_semantic_kind`.
- **Grammar is well-formed:** no reference to an undefined rule; a `root` exists.
  Pinned by `…::grammar_lints_clean` (via `lint_gbnf`).
- **Schema round-trips and validates:** `schema.rs::tests::*` and
  `tests/fixture_loads.rs::sample_narrative_fixture_is_schema_valid`.

Actual grammar *compilation* against llama.cpp's GBNF parser is exercised later in
the gated LLM crate (B-stream) via `complete_with_grammar`; the deterministic
linter guards it in the meantime so `cargo test -p doctree-core` needs no model.

## Alternatives considered

1. **Rust enums-with-data per node kind** (e.g. `Character { name, … }`). Pro:
   stronger typing. Con: produces nested, kind-dependent JSON that is far harder
   to GBNF-constrain and to evolve; the flat discriminant maps 1:1 onto a tiny
   grammar. Rejected.
2. **Generate the grammar from a JSON Schema.** Pro: single source of truth. Con:
   general schema→GBNF generators don't exist in this stack (the crate's
   generator is the narrow command-bar one), and a hand-authored grammar is small
   and lets us emit a *minimal* shape distinct from the full struct. Rejected for
   now; the lockstep tests recover most of the safety.
3. **Put the entire schema (all fields, structural kinds) in the grammar.** Con:
   forces the model to emit spans/provenance it cannot know and structural kinds
   it should not produce; bloats the grammar. Rejected — the grammar is the
   *semantic* contract; the walker owns structure.

## Consequences

- The frontend, walker, and LLM layer all compile against one schema crate; the
  grammar and schema can't drift without a red test.
- Adding a later genre (legal/academic) means a new ontology + grammar variant;
  this ADR is narrative-scoped and would be superseded/extended by a new ADR, not
  edited.
- `weight` + `similar_to` + `embedding` provenance are present in the schema now
  but only populated once the embeddings layer (B4) lands — reserved, not dead.

## Anchors

- `crates/doctree-core/src/schema.rs` — `Graph`, `Node`, `Edge`, `NodeKind`,
  `EdgeKind`, `Provenance`, `Span`, `validate`, `merge`.
- `crates/doctree-core/src/grammar.rs` — `GRAPH_GBNF`, `lint_gbnf`,
  `SEMANTIC_NODE_KINDS`, `SEMANTIC_EDGE_KINDS`.
- `fixtures/sample-narrative.graph.json` — the schema-valid render fixture.
