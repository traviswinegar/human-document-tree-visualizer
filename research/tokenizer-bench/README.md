# tokenizer-bench

**The agenticmd.org thesis test** ([ADR-00015](../../docs/adr/ADR-00015-graph-tokenizer-training-experiment.md),
[plan](../../docs/plans/PLAN-phase8-tokenizer-training.md)): does our graph-aware
tokenization (ADR-00013) help a model more than sub-word BPE?

A **standalone Streamlit GUI app** that trains small models from scratch on three
tokenizations of one corpus and compares them on a *fair* metric.

## The experiment

| Arm | Tokenization | Isolates |
|-----|--------------|----------|
| **A · BPE** | sub-word BPE trained on the corpus | the incumbent baseline |
| **B · byte** | raw UTF-8 bytes, no graph | byte-vs-subword, *without* graph |
| **C · byte+graph** | our `doctree_core::tokenizer::encode(doc, graph)` stream | the **thesis** |

- **Metric: bits-per-byte (BPB)** — loss normalized by *source text bytes*, the only
  denominator that is fair across different vocabularies (per-token perplexity is not).
- The number that matters is the **B→C gap**: the part attributable to *graph
  structure*, with the byte-vs-subword switch controlled out by arm B.
- A clean **null result** (graph does not beat BPE) is an acceptable finding.

## Corpus

The user's Obsidian **Writing Vault** (`C:\Writing Vault`), curated to authored prose
(~358K words): `_Books`, `_Short Stories`, `_Ideas`, `Archive`, `Campaigns`,
`Exercises`, `_Poetry`. Excluded: `_Gemini Chats` (AI logs), `_Templates`, Obsidian
internals. Entirely local.

## Setup (isolated venv — does not touch the global Python)

```powershell
python -m venv .venv
.venv\Scripts\activate
# CUDA build of torch FIRST (the default pip wheel is CPU-only):
pip install torch --index-url https://download.pytorch.org/whl/cu128
pip install -r requirements.txt
python cuda_check.py        # expect: cuda available: True
```

## Run

```powershell
streamlit run app.py
```

## Status

- **M0** — scaffold + Streamlit shell + CUDA env (this).
- **M1** — 3-arm datasets (from the Vault, via the tested `doctree-core` tokenizer) +
  the bits-per-byte evaluator (test-first). *next*
- **M2–M4** — model + training loop, runs, analysis/report. **M5** — ablations.
