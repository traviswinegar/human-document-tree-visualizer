"""(b) downstream task — text -> graph GENERATION (Phase 11 / ADR-00019 M3).

The compression metric (BPB) said graph-awareness does not help *predict prose*. This
milestone tests the axis where it might pay: **producing structure**. A small decoder-only
model is trained, per input tokenization, to read a document's text and *generate* its
salient-term graph as JSON; we score the generation against the deterministic walker graph
with `graphmatch` — valid-output rate + node/edge F1.

    python -m tokbench.graphgen <data_dir> --arm byte|gpt2 [--steps N] [--block B] ...

Sequence per example:  <text tokens> <SEP> <graph-json tokens> <EOS>
The training loss is **masked to the target region** (graph-json + EOS) — the model
conditions on the text but is scored only on generating the graph (ADR-00019's conditional
setup). Greedy generation from the `<SEP>` boundary on the held-out docs; the recovered
string is parsed + scored. Arms differ ONLY in the tokenizer (byte vs gpt2 sub-word), so a
difference is attributable to the tokenization — the same control as the BPB arms.

Ground truth comes from `corpus-export`'s `term_graphs.jsonl` (one term graph per doc, in
manifest order); per-doc text is sliced from `corpus.txt` by the manifest byte counts. The
deterministic train/val doc split is reused (last `val_doc_count` docs = val).
"""

from __future__ import annotations

import argparse
import json
import os

from . import data as D
from . import graphmatch as GM

# torch / model are imported lazily inside the functions that need them, so the pure
# helpers (tokenizers, build_example, load_pairs) are unit-testable without a GPU stack.


# --- tokenizers ------------------------------------------------------------------

class ByteTok:
    """Byte tokenizer: ids 0..255 are literal UTF-8 bytes."""

    name = "byte"
    base = 256

    def encode_text(self, b: bytes) -> list[int]:
        return list(b)

    def encode_str(self, s: str) -> list[int]:
        return list(s.encode("utf-8"))

    def decode(self, ids: list[int]) -> str:
        return bytes(i for i in ids if 0 <= i < 256).decode("utf-8", "replace")


class GPT2Tok:
    """GPT-2 sub-word tokenizer (the strongest small published BPE in the bench)."""

    name = "gpt2"

    def __init__(self):
        import tiktoken

        self.enc = tiktoken.get_encoding("gpt2")
        self.base = self.enc.n_vocab  # 50257

    def encode_text(self, b: bytes) -> list[int]:
        return self.enc.encode_ordinary(b.decode("utf-8", "replace"))

    def encode_str(self, s: str) -> list[int]:
        return self.enc.encode_ordinary(s)

    def decode(self, ids: list[int]) -> str:
        return self.enc.decode([i for i in ids if 0 <= i < self.base])


def make_tok(arm: str):
    if arm == "byte":
        return ByteTok()
    if arm == "gpt2":
        return GPT2Tok()
    raise ValueError(f"unknown arm {arm!r} (expected 'byte' or 'gpt2')")


# --- data ------------------------------------------------------------------------

def load_pairs(data_dir: str):
    """`(pairs, val_doc_count)` — pairs are `(doc_text_bytes, {nodes, edges})` in
    manifest order. Text is sliced from corpus.txt by per-doc byte counts."""
    manifest = D.load_manifest(data_dir)
    text = D.load_text_bytes(data_dir)
    graph_path = os.path.join(data_dir, "term_graphs.jsonl")
    with open(graph_path, encoding="utf-8") as f:
        graphs = [json.loads(line) for line in f if line.strip()]
    docs = manifest["docs"]
    if len(docs) != len(graphs):
        raise ValueError(f"docs ({len(docs)}) and term_graphs ({len(graphs)}) misaligned")
    pairs = []
    off = 0
    for d, g in zip(docs, graphs):
        nb = d["bytes"]
        pairs.append((text[off : off + nb], g))
        off += nb
    return pairs, manifest["val_doc_count"]


def build_example(pair, tok, block: int, min_nodes: int):
    """`(ids, sep_pos)` for a pair, or None if it has too few nodes or won't fit `block`.
    `sep_pos` is the index of the SEP token; the target region is everything after it."""
    text_bytes, g = pair
    nodes = g.get("nodes", [])
    if len(nodes) < min_nodes:
        return None
    edges = [tuple(e) for e in g.get("edges", [])]
    target = GM.graph_json(nodes, edges)  # canonical truth string
    sep, eos = tok.base, tok.base + 1
    xt = tok.encode_text(text_bytes)
    yt = tok.encode_str(target)
    ids = xt + [sep] + yt + [eos]
    if len(ids) > block:
        return None
    return ids, len(xt)


def build_split(pairs, val_doc_count, tok, block, min_nodes):
    cut = len(pairs) - val_doc_count
    train = [e for e in (build_example(p, tok, block, min_nodes) for p in pairs[:cut]) if e]
    # val keeps the parsed truth alongside the prefix so we can score generations.
    val = []
    for p in pairs[cut:]:
        ex = build_example(p, tok, block, min_nodes)
        if ex is None:
            continue
        ids, sep_pos = ex
        truth = (set(p[1].get("nodes", [])),
                 {frozenset(tuple(e)) for e in p[1].get("edges", []) if e[0] != e[1]})
        val.append((ids[: sep_pos + 1], truth))  # prefix = text + SEP
    return train, val


# --- batching + masked loss ------------------------------------------------------

def make_batch(examples, idx, block, eos, device):
    """Pad a set of (ids, sep_pos) examples to `block`; return x, y, loss-mask (1 only on
    the target region — graph-json + EOS — never on the text/SEP prefix or padding)."""
    import torch

    xs = torch.full((len(idx), block), eos, dtype=torch.long)
    for row, j in enumerate(idx):
        ids, _ = examples[j]
        xs[row, : len(ids)] = torch.tensor(ids, dtype=torch.long)
    x = xs[:, :-1].contiguous()
    y = xs[:, 1:].contiguous()
    mask = torch.zeros_like(y, dtype=torch.bool)
    for row, j in enumerate(idx):
        ids, sep_pos = examples[j]
        L = len(ids)
        # y index j' predicts ids[j'+1]; score target tokens (indices sep_pos+1 .. L-1)
        mask[row, sep_pos : L - 1] = True
    return x.to(device), y.to(device), mask.to(device)


def generate(model, prefix_ids, eos, block, device, max_new=256):
    import torch

    model.eval()
    ids = list(prefix_ids)
    with torch.no_grad():
        for _ in range(max_new):
            ctx = ids[-block:]
            x = torch.tensor([ctx], dtype=torch.long, device=device)
            logits, _ = model(x)
            nxt = int(torch.argmax(logits[0, -1]))
            if nxt == eos:
                break
            ids.append(nxt)
    model.train()
    return ids[len(prefix_ids):]  # only the generated continuation


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("data_dir")
    ap.add_argument("--arm", choices=["byte", "gpt2"], required=True)
    ap.add_argument("--steps", type=int, default=3000)
    ap.add_argument("--batch", type=int, default=16)
    ap.add_argument("--block", type=int, default=768)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--dropout", type=float, default=0.1)
    ap.add_argument("--min-nodes", type=int, default=2)
    ap.add_argument("--n-layer", type=int, default=4)
    ap.add_argument("--n-head", type=int, default=4)
    ap.add_argument("--n-embd", type=int, default=256)
    ap.add_argument("--max-new", type=int, default=256)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--seed", type=int, default=1337)
    args = ap.parse_args()

    import torch
    import torch.nn.functional as F

    from .model import GPT, GPTConfig

    torch.manual_seed(args.seed)
    tok = make_tok(args.arm)
    sep, eos = tok.base, tok.base + 1
    vocab = tok.base + 2

    pairs, val_doc_count = load_pairs(args.data_dir)
    train, val = build_split(pairs, val_doc_count, tok, args.block, args.min_nodes)
    if not train or not val:
        print(f"arm {args.arm}: too few qualifying docs (train {len(train)}, val {len(val)}) "
              f"— loosen --block / --min-nodes")
        return 1

    cfg = GPTConfig(vocab_size=vocab, block_size=args.block, n_layer=args.n_layer,
                    n_head=args.n_head, n_embd=args.n_embd, dropout=args.dropout)
    model = GPT(cfg).to(args.device)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, betas=(0.9, 0.95), weight_decay=0.1)
    print(f"arm {args.arm}: vocab {vocab}, {model.num_params() / 1e6:.2f}M params, "
          f"train {len(train)} docs, val {len(val)} docs, block {args.block}, device {args.device}")

    g = torch.Generator().manual_seed(args.seed)
    for step in range(1, args.steps + 1):
        idx = torch.randint(len(train), (min(args.batch, len(train)),), generator=g).tolist()
        x, y, mask = make_batch(train, idx, args.block, eos, args.device)
        logits, _ = model(x)
        per = F.cross_entropy(logits.view(-1, logits.size(-1)), y.reshape(-1),
                              reduction="none").view_as(y)
        loss = (per * mask).sum() / mask.sum().clamp(min=1)
        opt.zero_grad(set_to_none=True)
        loss.backward()
        opt.step()
        if step % max(args.steps // 10, 1) == 0 or step == args.steps:
            print(f"  step {step:>5}  target-loss {loss.item():.3f}")

    # Generate + score on the held-out docs.
    preds, truths = [], []
    for prefix, truth in val:
        gen = generate(model, prefix, eos, args.block, args.device, max_new=args.max_new)
        preds.append(tok.decode(gen))
        truths.append(truth)
    s = GM.score_predictions(preds, truths)
    print(f"  arm {args.arm} GEN  valid {s['valid_rate']:.3f}  "
          f"node_f1 {s['node_f1']:.3f}  edge_f1 {s['edge_f1']:.3f}  (n={s['n']})")
    # one sample for eyeballing
    if preds:
        print(f"  sample gen[0]: {preds[0][:200]!r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
