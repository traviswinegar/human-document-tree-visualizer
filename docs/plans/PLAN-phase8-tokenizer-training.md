# PLAN — Phase 8: train a model on our graph tokenizer

> Plan doc for the multi-week research subproject decided in
> [ADR-00015](../adr/ADR-00015-graph-tokenizer-training-experiment.md). Archived to
> `docs/plans/archive/` when the experiment reports.

## Goal

Answer one question with evidence: **does our graph-aware tokenization
(ADR-00013) help a model more than sub-word BPE?** Measured fairly
(bits-per-byte), with the *graph's* contribution isolated from the byte-level
switch (3 arms: BPE / byte / byte+graph). A clean null result is an acceptable
outcome.

## Decisions (settled)

- **Stack:** PyTorch + a nanoGPT-style trainer (ADR-00015 §5; Karpathy's reference).
- **Form:** a standalone **Streamlit** GUI app (ADR-00015 §6) — configure / launch /
  watch-live / compare / browse tokens, separate from the Tauri app.
- **Corpus:** the user's Obsidian Writing Vault `C:\Writing Vault`, curated to
  authored prose (~358K words; `_Books`/`_Short Stories`/`_Ideas`/`Archive`/
  `Campaigns`/`Exercises`/`_Poetry`), excluding `_Gemini Chats`, `_Templates`, and
  Obsidian internals (ADR-00015 §4).

## Standing invariants (do not violate)

- Separate subproject under `research/tokenizer-bench/`. **The shipped Tauri app,
  its native-free `cargo test --workspace` gate, and the `momusdev_llm`/`momusdev_met`
  siblings are untouched.**
- The tokenizer is **not** reimplemented — arm C's data comes from
  `doctree_core::tokenizer::encode`, via an exporter that round-trips through the
  ADR-00013-tested decode invariants.
- Commit locally only (never push). Atomic commits, `Phase 8 #M:` subjects, the
  two-commit ledger pattern, test-first where there is logic to pin (the BPB
  evaluator, the exporters).

## Milestones

### M0 — Subproject scaffold + reproducibility spine
- `research/tokenizer-bench/` (Python): `requirements.txt` (torch+CUDA, streamlit,
  tokenizers), README stating the question / 3 arms / BPB / corpus, a fixed RNG-seed
  + config convention, `.gitignore` for checkpoints/datasets (artifacts never
  committed), and a minimal **Streamlit** app shell.
- Verify the environment: Python + PyTorch import, **CUDA visible on the 3060 Ti**,
  Streamlit launches.
- **Done when:** `streamlit run` opens the app shell and a "torch sees CUDA" check
  passes on the 3060 Ti.

### M1 — Datasets for the three arms + the BPB evaluator (test-first)
- **Corpus loader:** read `C:\Writing Vault` `.md` files with the ADR-00015 §4
  include/exclude curation (authored prose in; `_Gemini Chats`/`_Templates`/Obsidian
  internals out); the exclude-list is config, not hard-coded.
- **Exporter (in/near `doctree-core`, native-free):** walk each corpus document →
  `(doc, graph)` → emit, per document: (C) the `encode` token-id stream, (B) the raw
  UTF-8 byte stream, and the raw text for (A). Pin a round-trip test: the exported
  arm-C stream `decode_text`s back to the source bytes (ADR-00013 invariant on real
  corpus data).
- **Arm A tokenizer:** train a BPE (e.g. 8–16k vocab) on the corpus text; record
  vocab + merges as a frozen artifact.
- **BPB evaluator:** the shared metric. **Test-first** on a hand-computed fixture
  (known logits → nats → BPB over a known byte count) before any model exists.
- **Done when:** all three datasets generate deterministically from the corpus, and
  the BPB unit test passes.

### M2 — Model + training loop (needs the stack decision)
- One small decoder-only transformer (nanoGPT-scale), **config-driven so arch/depth/
  width/compute are identical across arms**; only vocab/embedding/head size vary
  (delta logged). Train/val split fixed and shared (by source bytes, so the split is
  identical across arms).
- **Done when:** each arm trains for a few steps and the loss decreases; checkpoints
  + metrics log to disk.

### M3 — Run the three arms to convergence
- Same step/compute budget per arm; log loss curves, tokens-per-byte, and **eval
  BPB** per arm. Multiple seeds if cheap, to gauge noise.
- **Done when:** all three arms have converged runs with logged BPB + curves.

### M4 — Analysis + report (the deliverable)
- BPB table (A vs B vs C) with the **B→C delta** front and center; tokens-per-byte;
  loss curves; sample generations; and arm C's graph-emission capability probe.
- Honest write-up incl. the null-result case and the small-corpus caveat. Feed the
  headline numbers back as the **ADR-00013 research output** (the "measure
  intelligent vs sub-word tokenization" backlog item).
- **Done when:** a committed report answers the goal question with evidence.

### M5 — Stretch / ablations (only if M4 warrants)
- Loss-masking arm (graph markers in input, excluded from loss) to sharpen the
  text-only attribution; larger corpus / model; structural-only vs LLM-semantic
  graphs for arm C; vocabulary-size controls.

## Key risks (and mitigations)

| Risk | Mitigation |
|------|------------|
| Tiny corpus → noisy, non-general results | Hold arch/compute fixed across arms; report *relative*; user expands corpus; multi-seed. |
| Unfair comparison (perplexity / vocab size) | BPB over source bytes; identical arch; report the vocab param delta. |
| Arm C "wins" only by modeling extra info (the graph) | State the nuance; M5 loss-masking arm to isolate text-only. |
| Stack churn / training bugs | Recommend PyTorch+nanoGPT (proven); pin the BPB metric with a fixture test before trusting any run. |
| Scope creep into "build a good model" | Goal is *relative tokenization efficiency*, not model quality — small + fast on purpose. |

## Backlog (parked, from this subproject)

- Learned/merged graph-aware vocabulary (vs the fixed bespoke one) — ADR-00013
  noted this; it becomes live only if the from-scratch comparison is promising.
- A "graph-aware prompting" comparison (feed the *existing* qwen smarter input) as a
  cheaper, no-training cousin of arm C — relevant to the long-doc semantic layer
  (#71) regardless of how training turns out.
