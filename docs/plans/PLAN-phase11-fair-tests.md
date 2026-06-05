# PLAN — Phase 11: fair tests of graph-awareness

> Implementation plan for [ADR-00019](../adr/ADR-00019-fair-tests-of-graph-awareness.md).
> The ADR-00015 BPB benchmark was a negative result for the *thin* version of the
> thesis (sparse boundary markers, on a compression metric). This phase tests the
> *real* thesis — does the entity/relation graph help — without the leak and not only
> on bits-per-byte. Archived when it reports.

## Standing invariants
- Research subproject only (`research/tokenizer-bench/`); the shipped-app native-free
  gate is untouched.
- **No leak:** the conditioning prefix never contains the source's surface text.
- Test-first for the integrity machinery (loss-masking, graph-match); the *results*
  are whatever they are — a null is a valid finding.
- Single 3060 Ti → small/indicative scale; report caveats, never silently truncate.

## Milestones

### M1 — (a) Non-leaky semantic-graph conditioning *(first — most direct)*
- Export the **anonymized** semantic graph per doc: node kinds + stable anonymized
  entity ids (`entity_0…`) + relations — **no surface labels**. (From the walker +
  semantic layer via `corpus-export` / a small Python step.)
- Trainer: prepend the serialized prefix, **mask its loss** (and exclude it from the
  BPB denominator); model predicts the text bytes given the prefix.
- **Tests (first):** prefix tokens contribute 0 to loss/denominator (masked-loss
  fixture); prefix shares no source byte-run (no-leak assertion).
- **Compare:** byte baseline (no prefix) vs byte+anonymized-graph-prefix, on text BPB.
- **Done when:** both run to convergence; BPB delta (prefix vs none) reported honestly.

### M2 — (c) Scale
- Build a larger corpus: the Vault + public-domain text (e.g. a Gutenberg book or two).
- Bigger model + more steps within the GPU budget; re-run the **core** comparison
  (ours vs byte vs one strong sub-word, e.g. cl100k) at scale.
- **Done when:** the at-scale BPB table is logged; does the byte-tie hold or move?

### M3 — (b) Downstream task: text → graph generation
- Conditional setup: text → `{nodes, edges}` JSON; ground truth = the walker graph.
- **Graph-match scorer (test-first):** valid-JSON rate + node/edge F1 vs ground truth,
  on a hand-checked fixture.
- Train a small model per tokenization; compare on graph-match (not BPB).
- **Done when:** the graph-match table across tokenizations is logged.

### M4 — Report
- One honest writeup folding in: the BPB benchmark (negative), (a) conditioning,
  (c) scale, (b) downstream — what graph-awareness does and doesn't buy, and on which
  metric. Feed back into the ADR-00013 research-output backlog.

## Risks (and mitigations)
| Risk | Mitigation |
|------|------------|
| Anonymization strips so much that (a) can't show signal | Frame a null as "structure-sans-surface doesn't help LM," not "graphs useless"; (b) tests a different axis. |
| Accidental leak in the prefix | The no-leak assertion (no shared source byte-run) is a pinned test; prefix is anonymized by construction. |
| Scale is compute-bound on one 3060 Ti | Keep models small-but-bigger; report indicative, name the budget. |
| (b) invalid-JSON dominates | Report valid-JSON rate separately from F1; grammar-constrain generation if needed. |
| Multi-week scope creep | One milestone at a time, each with its own logged result; stop early if (a)+(c) are conclusive. |

## Backlog (parked)
- Learned-embedding entity conditioning (vs anonymized ids) — ADR-00019 alt 4.
- Retrieval as a second downstream metric (graph-aware vs text embeddings).
