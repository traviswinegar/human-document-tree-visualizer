"""tokenizer-bench — Phase 8 standalone GUI (ADR-00015).

A Streamlit dashboard for the graph-tokenizer training experiment: point it at the
corpus, configure a run, launch training, watch the three arms (A=BPE / B=byte /
C=byte+graph) live on bits-per-byte, compare them, and browse the tokenized output.

    streamlit run app.py

This is the M0 shell: layout + an environment banner that reports GPU readiness.
The dataset/evaluator (M1) and the training loop + live charts (M2-M4) wire in here.
"""

from __future__ import annotations

import streamlit as st

st.set_page_config(page_title="tokenizer-bench", layout="wide")

st.title("tokenizer-bench — does graph-aware tokenization beat BPE?")
st.caption(
    "ADR-00015 · three arms compared on bits-per-byte:  "
    "**A** = BPE subword · **B** = byte-only · **C** = byte+graph (our `encode`).  "
    "The **B→C gap** is the graph's contribution."
)

# Environment banner — the GUI itself reports GPU readiness (M0 acceptance).
try:
    import torch

    cuda = torch.cuda.is_available()
    device = torch.cuda.get_device_name(0) if cuda else "CPU only"
    (st.success if cuda else st.warning)(
        f"PyTorch {torch.__version__} · CUDA available: {cuda} · device: {device}"
    )
except Exception as exc:  # pragma: no cover - shell diagnostic
    st.error(f"PyTorch not importable yet: {exc}")

with st.sidebar:
    st.header("Run config")
    st.text_input("Corpus path", value=r"C:\Writing Vault", key="corpus_path")
    st.multiselect(
        "Arms",
        ["A · BPE", "B · byte", "C · byte+graph"],
        default=["A · BPE", "B · byte", "C · byte+graph"],
        key="arms",
    )
    st.number_input("Train steps", min_value=100, value=2000, step=100, key="steps")
    st.number_input("Context length", min_value=128, value=512, step=128, key="ctx")
    st.button("Launch run", disabled=True, help="Wires in at M2.")
    st.caption("Controls are inert in the M0 shell.")

left, right = st.columns(2)
with left:
    st.subheader("Live training")
    st.write("Loss + bits-per-byte curves per arm land here (M3).")
with right:
    st.subheader("Comparison")
    st.write("Converged BPB table (A vs B vs C) + the B→C delta land here (M4).")

st.divider()
st.subheader("Tokenized output")
st.write(
    "Browse any document's `encode` stream — the readable token view and the "
    'exported file (the "see the tokenized file" surface), wired at M1.'
)
