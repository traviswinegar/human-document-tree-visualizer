# ADR-00019 — Fair tests of graph-awareness (conditioning, downstream, scale)

- **Status:** Accepted (2026-06-05)
- **Phase:** 11 (#1)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-00015](ADR-00015-graph-tokenizer-training-experiment.md),
  [ADR-00013](ADR-00013-reversible-graph-tokenizer.md)

## Context

The ADR-00015 benchmark answered its question and the answer was **no**: on
language-modeling **bits-per-byte**, our graph tokenizer (arm-C-lite) is the *worst*
of the field — tied with raw bytes (2.74 vs 2.71) and well behind every sub-word
tokenizer, industry or home-trained (~2.0). Two reasons it almost had to come out
that way, in hindsight:

1. **arm-C-lite carries only sparse structural *boundaries*** (`NODE_OPEN<kind>` /
   `NODE_CLOSE`), ~5% of the stream, and those boundaries are **redundant with the
   text's own punctuation** — the byte model already infers "sentence ends at the
   period." Near-zero new predictive signal.
2. **The leak fix removed the only non-redundant part.** The original `encode`
   carried each node's *label* (its surface text / the entities) — which is exactly
   what leaked — so we stripped it. What remained tests the *weakest* form of the
   thesis (boundaries), not the interesting one (entities/relations).

And **bits-per-byte is a compression metric**; the agenticmd.org value proposition
is really agentic/structured/retrieval use, which BPB does not measure.

So the negative result is real but narrow. This ADR designs three **fair** tests of
whether graph-awareness has value — without the leak, and not only on BPB.

## Decision

### (a) Non-leaky semantic-graph conditioning

Stop interleaving graph tokens in the *scored* stream. Instead, prepend the
document's **semantic graph as a loss-masked conditioning prefix** the model attends
to but is **never scored on**, and have it predict the text bytes (scored) given that
prefix. The prefix must **not contain the surface text** (that's the leak), so it
carries the graph's **skeleton**:

- node **kinds** (character/place/concept/event/…),
- **anonymized, stable entity ids** (`entity_0`, `entity_1`, …) — so *coreference*
  ("this passage is about the same entity as that one") is available **without** the
  name, and
- **relations** (`entity_0 interacts_with entity_1`, `entity_2 located_in place_0`).

The test: does an anonymized entity/relation skeleton lower **text** BPB versus the
same model with **no** prefix (the byte baseline)? If knowing the cast and their
relations helps predict the prose — even without the names — BPB drops. If not,
structure-without-surface doesn't help LM. Either way it's an honest answer, and it
tests the *real* claim (the entities/relations), not boundary markers. *(Anonymized
coref ids correlate with text positions — that's legitimate graph signal, not a
verbatim copy; surface labels would be the copy and are excluded.)*

### (b) Downstream task — text → graph generation

Replace the compression metric with **the task the product actually does**: from the
document text, generate the `{nodes, edges}` graph. Train a small conditional model
per tokenization and score **graph match** — valid-JSON rate + node/edge F1 against
the walker's graph (the deterministic ground truth). This measures whether a
graph-aware representation helps a model *produce structure*, which is where the
agentic value would live — orthogonal to bits-per-byte.

### (c) Scale

The byte-tie may be a small-data/small-model artifact. Re-run the **core** comparison
(ours vs byte vs one strong sub-word) on a **larger corpus** (the Vault augmented with
public-domain text) and a **bigger model + more steps**, within the 3060 Ti's budget,
to see whether the gap holds or structure starts to matter with capacity and data.

**Order:** (a) first — most direct test of the thesis, and it reuses the existing
trainer (add a masked prefix). Then (c) — mechanically simple (more data/steps),
compute-bound. Then (b) — a new conditional-generation task, the most new code.

## Alternatives considered

1. **Park the thesis on the BPB result alone.** Rejected — that result tested the
   thinnest version (boundaries) on the metric least favorable to structure; it
   doesn't actually falsify "graph-awareness helps."
2. **Put the surface labels back for a "richer" arm C.** Rejected — that is the leak;
   any test including the verbatim text is meaningless.
3. **Only scale (c).** Rejected — scaling a redundant encoding won't reveal value;
   (a)'s non-leaky conditioning is the conceptual fix, (c) is the magnitude check.
4. **Embedding/hashed entity prefix instead of anonymized ids.** Deferred — anonymized
   ids are the simplest principled non-leak; learned-embedding conditioning is a later
   refinement if (a) shows signal.

## Consequences

- (+) Tests the **real** thesis (entities/relations, not boundaries) and on metrics
  where graph-awareness could plausibly pay (conditioning, structured generation),
  not just compression.
- (+) (a) reuses the trainer (prefix + loss mask); (b)/(c) are bounded additions.
- (+) Honest by construction: anonymized prefix can't leak surface text; graph-match
  is against the deterministic walker ground truth.
- (−) Still small-corpus / small-model on a single GPU — results stay *indicative*.
- (−) (a)'s anonymization may strip so much that even genuine structure can't help —
  a null here means "structure-sans-surface doesn't help LM," not "graphs are useless."
- (−) (b) is a new training setup (conditional generation) — real code + its own
  failure modes (invalid JSON, etc.).

## Invariant (pinned)

The integrity machinery is what must be correct (the metric, not the outcome):

- **(a) loss masking:** the conditioning prefix contributes **zero** to the loss and
  the BPB denominator (only text-byte positions are scored) — unit-tested on a fixture
  (prefix tokens masked → identical loss to a no-prefix run on the same text).
- **(a) no-leak:** the prefix contains **no** byte-run of the source text (asserted:
  the anonymized prefix shares no surface label with the document).
- **(b) graph-match:** node/edge F1 + valid-JSON computed against the walker graph,
  unit-tested on a hand-checked fixture.

Pinned test paths: `research/tokenizer-bench/` tests (the masking + graph-match
fixtures). The shipped-app native-free gate is untouched (research subproject only).

## Anchors

- `research/tokenizer-bench/tokbench/` — conditioning prefix builder + masked-loss
  training (a); graph-match scorer + conditional trainer (b); scale configs (c).
- `crates/corpus-export` / `doctree-core` — source of the anonymized semantic graph
  for the prefix.
- `docs/plans/PLAN-phase11-fair-tests.md` — milestones.
