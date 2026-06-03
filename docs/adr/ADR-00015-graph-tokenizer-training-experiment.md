# ADR-00015 — Train a model on our graph tokenizer (the agenticmd.org thesis test)

- **Status:** **Proposed** (2026-06-02) — *one open decision (training stack) flagged
  for the user before this flips to Accepted.*
- **Phase:** 8 (#1)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-0002](ADR-0002-graph-schema-and-gbnf-grammar.md),
  [ADR-00013](ADR-00013-reversible-graph-tokenizer.md)
- **Supersedes / superseded by:** none

## Context

The user's question: *"We have a local LLM, and we can replace its tokenizer with
OUR OWN, right?"* — and, having heard the constraint, chose to **train a model on
our tokens.**

**Why you cannot hot-swap a tokenizer into the local qwen model (the constraint
that forces this ADR).** A pretrained LLM is not "a model + a detachable
tokenizer." The tokenizer fixes a vocabulary, and the model's **embedding table
has one learned row per token id**; every weight in every layer was trained
against those exact ids over trillions of tokens. Token id *N* is meaningful to
qwen *only* because embedding row *N* was shaped to mean it. Our vocabulary
(`VOCAB_SIZE = 296`: a 256-byte floor + node/edge/provenance ids + control
tokens, ADR-00013) assigns completely different meanings to those integers.
Pointing qwen at our ids indexes its learned embeddings with numbers that mean
nothing to it → noise. `momusdev_llm` / llama.cpp is moreover an **inference**
engine for pretrained GGUF files (vocab + merges baked in); there is no path to
host a foreign tokenizer and keep the weights meaningful, and feeding raw ids
directly only relabels the same problem. **The tokenizer and the weights are
co-trained.** To genuinely use our tokenization, a model must be **trained on it
from scratch** — which is this ADR.

This squarely tests ADR-00013's research thesis: *models are better served by
intelligent, graph-aware tokenization than by sub-word schemes (BPE/SentencePiece).*

Constraints (standing):
- **ADR-0001 decoupling is untouched.** This is a **separate research subproject**
  (training, not inference). The shipped Tauri app's native-free default build
  (`cargo test --workspace`, no features) is not affected; nothing here is wired
  into the desktop app or its CI gate.
- **Do not modify the `momusdev_llm` / `momusdev_met` siblings.** They are
  inference-only and irrelevant to a training run; this subproject does not touch
  them.
- **The tokenizer stays in `doctree-core`** (ADR-00013, native-free, tested). It is
  the *data source* for the experiment's graph arm; the training code consumes its
  output, never reimplements it.

## Decision

Stand up a research subproject that trains small language models from scratch on
three tokenizations of the **same corpus** and compares them on a **fair,
cross-tokenizer metric**, isolating the contribution of *graph structure*.

### 1. Three arms (controls the confound)

Our scheme changes **two** things versus BPE at once — byte-level text **and**
added graph-structure tokens. A two-arm "ours vs BPE" test cannot attribute a win
to the *graph* (the actual thesis) rather than to the byte-level switch. So:

| Arm | Tokenization | Isolates |
|-----|--------------|----------|
| **A · BPE** | a sub-word BPE trained on the corpus (the incumbent) | the baseline |
| **B · Byte** | raw byte-level tokens over the same text, no graph | byte-vs-subword, *without* graph |
| **C · Graph** | our `doctree_core::tokenizer::encode(doc, graph)` stream (byte text + inline structural markers + graph trailer) | the **thesis**: graph structure on top of B |

A→B isolates the byte-level effect; **B→C isolates the graph's contribution** —
the number the thesis lives or dies on.

### 2. Fair metric — bits-per-byte (BPB), not perplexity

Per-token perplexity is **not comparable across different vocabularies** (a token
covers a different amount of text in each arm). The comparison metric is
**bits-per-byte**: `BPB = (total eval cross-entropy in nats / ln 2) / (UTF-8 bytes
of the original eval text)`. Normalizing by *bytes of source text* — the shared,
tokenization-independent denominator — makes all three arms directly comparable.
This is the standard cross-tokenizer measure from the byte-level-LM literature.
Also reported: **tokens-per-byte** (raw efficiency), loss curves, and — for arm C
only — whether the model can be prompted to emit *valid graph structure*, a
capability arms A/B structurally cannot have (a finding in itself).

*Honest nuance recorded:* arm C also spends capacity modeling the graph, so it is
"does jointly modeling text+graph help the text?" not a pure text-only contest.
A refinement arm (graph markers in the input but **masked from the loss**) is
listed in the plan to sharpen this if the headline result warrants it.

### 3. Model — small, same across arms

A small decoder-only transformer (nanoGPT-scale: ~10–30M params, context
512–1024). **Identical architecture, depth, width, and compute budget across all
three arms** — only the tokenizer, vocabulary, and the (vocab-sized) embedding /
output head differ. The vocab-driven parameter delta is small and reported, not
hidden. This fits the 8 GB RTX 3060 Ti comfortably; runs are minutes-to-hours per
arm. Absolute model quality is **not** the goal — *relative* tokenization
efficiency is.

### 4. Corpus — documents that produce real graphs

Arm C needs `(document, graph)` pairs, so the corpus must be documents the
deterministic walker handles well (narrative / structured prose). Start with the
user's **Birth of the Anunnaki** agenticmd corpus plus any other library
documents; the comparison is *relative*, so modest size is acceptable (with noise
caveats, §Consequences). Graphs are generated by the **deterministic walker**
(reproducible, fast) — not the slow/shallow LLM semantic layer — so the dataset is
regenerable bit-for-bit. *(Corpus selection is a user input, not a hard decision:
point me at the document set; the Anunnaki corpus is the default seed.)*

### 5. Stack — **recommended: PyTorch; flagged for the user**

This is the **one decision I want confirmed before committing weeks of work.**

- **Recommended — Python / PyTorch + a nanoGPT-style trainer.** Fastest path to a
  *correct* comparison: a proven ~300-line reference, mature CUDA, trivial BPB and
  metric code, the deepest debugging ecosystem. For a research result, iteration
  speed and correctness dominate. Cost: it is Python, outside the Rust workspace,
  and a new toolchain on the machine.
- **Alternative — Rust / `candle` (or `burn`).** Keeps everything in one Rust
  workspace, matches this project's AgentDNA-native ethos and the CUDA toolchain
  already set up for llama.cpp. Cost: training ergonomics are less mature, more
  code to get right, slower iteration, higher risk of a subtle, result-invalidating
  bug in exactly the place correctness matters most.

Either way the subproject lives in its **own directory** (e.g.
`research/tokenizer-bench/`), isolated from the shipped app. **If the user prefers
Rust-native, this ADR is superseded by an Accepted revision naming `candle`** — the
rest of the design (arms, BPB, model scale, corpus) is stack-agnostic.

## Alternatives considered

1. **Swap our tokenizer into the pretrained qwen GGUF.** *Impossible* without
   retraining (embedding table + all weights are co-trained against qwen's vocab;
   llama.cpp bakes the vocab into the GGUF). This impossibility is precisely why we
   train from scratch — it is the motivating context, not a viable option.
2. **Compare via perplexity / loss-per-token.** Rejected: non-comparable across
   vocabularies. BPB is the fair denominator.
3. **Two arms only (ours vs BPE).** Rejected: confounds byte-level representation
   with graph structure; the byte-only control (arm B) is what lets us credit the
   *graph*.
4. **A large / general model.** Rejected for v1: 8 GB VRAM, and absolute quality is
   not the question — relative tokenization efficiency is. Small models give the
   signal at a fraction of the cost.
5. **Rust `candle`/`burn` as the default stack.** Considered and kept as the
   alternative (see §5); PyTorch recommended for speed-to-signal, pending user call.

## Consequences

- (+) **Actually tests the thesis** the whole tokenizer (ADR-00013) was built to
  test — with a design that can *attribute* a result to graph structure, not an
  artifact of byte-vs-subword or an unfair metric.
- (+) **App untouched.** Separate subproject; the native-free desktop build, the
  `momusdev_llm` siblings, and ADR-0001 are all unaffected.
- (+) **Reproducible dataset.** Arm C's graphs come from the deterministic walker
  in the tested `doctree-core`, so the training corpus regenerates bit-for-bit.
- (−) **Uncertain payoff / possible null result.** Graph tokenization may *not*
  beat BPE on BPB. That is a legitimate, publishable-grade finding — the experiment
  is designed to be able to say "no."
- (−) **Small-corpus noise.** A few documents is little for LM training; results
  are *relative and indicative*, not a generalization claim. Mitigated by holding
  arch/compute fixed across arms and by the user expanding the corpus.
- (−) **Arm C models extra information** (the graph), so the headline is "joint
  text+graph modeling," with the loss-masking refinement arm available to sharpen
  the text-only attribution.
- (−) **New stack on the machine** (PyTorch *or* a Rust ML crate + its CUDA build).

## Invariant (pinned)

The experiment's *fairness machinery* is what must be correct, so it is what gets
pinned:

- **Bits-per-byte is computed identically and correctly for every arm** — a unit
  test on a hand-checkable fixture (known logits → known nats → known BPB over a
  known byte count), independent of the model.
- **Arm C's training data is exactly `doctree_core::tokenizer::encode` output** —
  the dataset exporter round-trips through the *tested* tokenizer (the ADR-00013
  `decode_text`/`decode_graph` invariants still hold on the exported streams), so
  the graph arm trains on the real artifact, not a drifted copy.

Pinned test path: the subproject's test suite (e.g.
`research/tokenizer-bench/tests/`) — the BPB fixture test and the
encode-exporter round-trip — plus the standing `doctree-core` tokenizer tests it
depends on. The shipped-app gate `cargo test --workspace` (no features) stays green
and native-free, unaffected by this subproject.

## Anchors

- `research/tokenizer-bench/` — the subproject (scaffolded in Phase 8 #2): tokenizer
  adapters for the three arms, the model + training loop, the BPB evaluator, the
  run configs, and the analysis/report.
- `crates/doctree-core/src/tokenizer.rs` — `encode` (arm C data source) + the
  ADR-00013 round-trip invariants the exporter relies on.
- `docs/plans/PLAN-phase8-tokenizer-training.md` — milestones M0–M5.
