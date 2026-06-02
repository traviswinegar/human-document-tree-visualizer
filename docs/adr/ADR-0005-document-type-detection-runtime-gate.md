# ADR-0005 — Document-type detection: deterministic classifier in core, capability-aware routing in the command layer

- **Status:** Accepted
- **Date:** 2026-06-02
- **Deciders:** Travis (user), agent
- **Supersedes:** —
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md), [ADR-0002](ADR-0002-graph-schema-and-gbnf-grammar.md), [ADR-0004](ADR-0004-embedding-similarity-layer.md)

## Context

Phase 1's semantic ontology (ADR-0002 — `Character`, `Place`, `Event`,
`Object`, `Group`, `Concept`) is **narrative-specific**. The deterministic spine
(Stream A) is genre-agnostic — any text segments into sections/sentences/clauses
/terms — but the two semantic layers are not equally safe on every document:

- The **LLM layer** (B3) is prompted to extract a *narrative* graph. Run it over
  a code-heavy README or a present-tense API reference and it will hallucinate
  "characters" and "events" that aren't there — force-fitting the wrong ontology.
- The **embedding layer** (B4) is ontology-agnostic: cosine similarity between
  node texts is meaningful for any prose, narrative or not.

B5 is the runtime gate that decides, per document, *which* of these layers should
run — so a non-narrative document takes a degraded (or purely structural) path
rather than being mis-extracted. Two questions fall out:

1. **Deterministic or model-based classification?** We could ask the LLM "is this
   fiction?". But narrative-vs-not is a coarse decision the *surface features*
   answer well (dialogue, personal pronouns, past-tense density, heading/list/code
   structure), and a deterministic classifier keeps the gate native-free,
   instant, and headlessly testable — consistent with the rest of `doctree-core`.
2. **Where does routing live?** The *ideal* pipeline follows from the document
   class alone. But the *runnable* pipeline depends on which native features this
   particular build compiled in (`llm`, `vectordb` — independent per ADR-0004).
   Those are two different concerns and belong in two different layers.

## Decision

**Classify deterministically in `doctree-core`; reconcile the recommendation with
the build's compiled capabilities in the Tauri command layer.**

### Deterministic classifier (`doctree-core/src/classify.rs`)

`classify_document(text) -> Classification` measures four `[0,1]` signals in a
single pass and returns `{ class, confidence, signals }`:

- `structure_ratio` — **word-weighted** share of words sitting on heading / list
  / code-fence / table lines. Word-weighted (not line-weighted) so a lone
  `# Chapter One` over wrapped prose stays low while a bullet/code-dominated
  README scores high — line-weighting broke on unwrapped single-line prose.
- `dialogue_ratio` — share of non-blank lines carrying a double-quote.
- `pronoun_ratio` — first/third-person personal pronouns per word. **Second-person
  `you` is excluded** — it signals instructions, not narration.
- `past_tense_ratio` — `-ed` verbs (len ≥ 4) plus common irregular past verbs
  per word.

The verdict is a small decision tree: too few words → `Unknown`; structure
dominates (`≥ 0.5`) → `Structured`; otherwise a weighted **narrative score**
(dialogue 0.40 / pronoun 0.35 / past 0.25, each saturating at a fiction-typical
density) decides `Narrative` (`≥ 0.5`) vs `Expository`. Every threshold is a
named const. The class maps to a `RecommendedPipeline`: `Narrative →
NarrativeHybrid`, `Expository → StructuralPlusSimilarity`, `Structured`/`Unknown`
→ `StructuralOnly`.

### No LLM in the gate — deferred, not rejected

The classifier consults **no model**. An optional LLM *confirmation* for
low-confidence documents (ask the model only when the deterministic score sits
near 0.5) is a sensible future enhancement, but it is deliberately **deferred**:
it would couple the always-on routing surface to the gated engine, and the
surface features already separate the fixtures cleanly. Logged in the BUILD_LOG
backlog, not built here.

### Capability-aware routing (`src-tauri/src/lib.rs`)

`resolve_pipeline(recommended, caps) -> ResolvedPipeline` degrades the *ideal*
pipeline to what `caps` (a `{ llm, vectordb }` snapshot of the compiled features
via `cfg!`) can actually run:

- `NarrativeHybrid` → `SemanticBuild` if `llm`, else `EmbeddedBuild` if
  `vectordb`, else `StructuralBuild`.
- `StructuralPlusSimilarity` → `EmbeddedBuild` if `vectordb`, else
  `StructuralBuild`. (No `llm` fallback — the narrative LLM ontology is the
  *wrong* shape for expository prose, so we never route there.)
- `StructuralOnly` → `StructuralBuild`, always.

`ResolvedPipeline::command()` names the **registered** Tauri command for that
path (`build_steps` / `semantic_build_steps` / `embedded_build_steps`). The
always-registered `classify_document` command returns a `Routing` that bundles
the class, confidence, signals, recommended + resolved pipeline, the command to
call, a `downgraded` flag (true when a fully-featured build would have resolved
differently), and the capability snapshot — everything the frontend needs to
route the build *and* explain a degradation ("rebuild with `--features llm` for
the richer extraction").

The split mirrors ADR-0004's reasoning: pure domain logic (which pipeline *suits*
a class) lives in core; the concern that needs to know what was compiled (which
pipeline *can run*) lives in the command layer.

## Pinned invariant (the contract that must not silently drift)

- **Classification is pure + native-free + deterministic:** same text → same
  `Classification`. Pinned by `crates/doctree-core/src/classify.rs::tests::{
  classify_is_deterministic, narrative_prose_classifies_as_narrative,
  technical_prose_classifies_as_expository,
  heading_list_code_doc_classifies_as_structured, tiny_or_empty_doc_is_unknown,
  a_single_heading_does_not_make_prose_structured,
  signals_are_bounded_and_confidence_in_unit_range}`.
- **Routing degrades gracefully and only names registered commands:** pinned by
  `src-tauri/src/lib.rs::tests::{narrative_routing_degrades_with_capabilities,
  expository_routing_needs_only_the_embedder,
  structural_only_always_resolves_to_the_spine,
  resolved_commands_are_actually_registered,
  classify_routes_narrative_and_flags_downgrade_on_a_lean_build,
  classify_document_serializes_camel_case_with_snake_case_tags}`.
- **Native-free default build:** `classify_document` is registered on every build
  and pulls in no native dep; `cargo test --workspace` stays green with zero
  native deps (ADR-0001).

## Alternatives considered

1. **Ask the LLM to classify.** Pro: handles ambiguous/mixed documents; one
   "understanding" path. Con: couples the always-on routing gate to the gated
   engine (a native-free build couldn't classify), adds multi-second latency to a
   coarse yes/no, and the surface features already separate the corpus. Deferred
   as an *optional confirmation* for low-confidence cases, not the primary path.
2. **Line-weighted `structure_ratio`.** Simpler to compute, but a document with
   one heading over a single unwrapped prose paragraph reads as 50 % structural
   and trips `Structured`. Word-weighting is robust to how prose is wrapped.
   Rejected (and caught by `a_single_heading_does_not_make_prose_structured`).
3. **Put the routing reconciliation in `doctree-core`.** Con: core is genre/
   capability-agnostic and must not know about Tauri features or command names;
   `cfg!(feature = "llm")` is meaningless there. The recommendation (class →
   ideal) is pure and lives in core; the capability reconciliation belongs with
   the commands. Rejected to keep the layer split clean.
4. **Route by forcing a single "best" pipeline with no fallback.** Con: a
   narrative doc on a native-free build would have nothing to run. Graceful
   degradation (hybrid → similarity → spine) means every build does the best it
   can and flags what it couldn't. Rejected.

## Consequences

- Non-narrative documents no longer get force-fit into the narrative ontology:
  expository prose routes to similarity-only, reference material to the spine.
- The gate is **deterministic and CPU-free** — it runs identically on the
  native-free default build, the desktop, and (potentially) the WASM web build,
  and is fully verified by the default test suite. No `#[ignore]`d live test is
  needed because no model is involved (unlike B2/B3/B4).
- The frontend gains one always-registered IPC command (`classify_document`) that
  hands back both the verdict and the concrete command to invoke. **Wiring the
  frontend to actually call it** (classify on load → dispatch to the resolved
  build command → surface the class/downgrade in the UI) is the deferred
  B3/B4/B5 frontend-integration item, landed as one pass after the backend
  B-stream completes.
- An LLM confirmation for low-confidence classifications remains a future
  enhancement; this ADR records that B5 intentionally shipped the deterministic
  gate alone.

## Anchors

- `crates/doctree-core/src/classify.rs` — `classify_document`, `Classification`,
  `ClassificationSignals`, `DocumentClass`, `RecommendedPipeline`, the signal
  measurement + decision tree + tests.
- `crates/doctree-core/src/lib.rs` — re-exports of the classify surface.
- `src-tauri/src/lib.rs` — `Capabilities`, `ResolvedPipeline`, `resolve_pipeline`,
  `Routing`, `classify_document_impl` + the `classify_document` command + tests.
- `crates/doctree-core/src/schema.rs` — the narrative ontology (ADR-0002) this
  gate protects from non-narrative input.
