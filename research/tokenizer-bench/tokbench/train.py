"""M2 trainer (Phase 8 / ADR-00015): one nanoGPT-style model per arm, identical
architecture across arms (only the vocabulary differs), wired to the text-position
bits-per-byte eval.

    python -m tokbench.train <data_dir> <A|B|C> [--steps N] [--eval-every K]

Logs `{step, train_loss, val_loss, val_bpb}` to `runs/arm_<X>.jsonl` and checkpoints
to `runs/arm_<X>.pt`. Run all three arms with the same flags for a fair comparison.
"""

from __future__ import annotations

import argparse
import json
import os

import numpy as np
import torch
import torch.nn.functional as F

from . import data as D
from .bpb import bits_per_byte
from .model import GPT, GPTConfig


def get_batch(ids: np.ndarray, block: int, batch: int, device: str):
    ix = torch.randint(len(ids) - block - 1, (batch,))
    x = torch.stack([torch.from_numpy(ids[i : i + block].astype("int64")) for i in ix])
    y = torch.stack([torch.from_numpy(ids[i + 1 : i + 1 + block].astype("int64")) for i in ix])
    return x.to(device), y.to(device)


@torch.no_grad()
def eval_arm(model, val_ids, arm: str, val_bytes: int, block: int, device: str,
             val_body=None, eval_batch: int = 64):
    """Full-coverage batched val eval: every target in [1, len) scored exactly once.
    Returns (bits_per_byte, mean_val_loss). For arm C only the *document body* targets
    (`val_body == 1`) count toward BPB — the structural markers are context, never
    scored, so no marker/label text is credited. The denominator is the true
    source-byte count (`val_bytes`) for every arm — what makes the vocabularies
    comparable."""
    model.eval()
    n = len(val_ids)
    starts = list(range(0, n - 1, block))
    total_nats = total_loss = 0.0
    total_tok = 0
    for i in range(0, len(starts), eval_batch):
        chunk = starts[i : i + eval_batch]
        xs = torch.zeros(len(chunk), block, dtype=torch.long)
        ys = torch.zeros(len(chunk), block, dtype=torch.long)
        keep = torch.zeros(len(chunk), block, dtype=torch.bool)
        textm = torch.zeros(len(chunk), block, dtype=torch.bool)
        for j, s in enumerate(chunk):
            e = min(s + block, n - 1)
            ln = e - s
            xs[j, :ln] = torch.from_numpy(val_ids[s:e].astype("int64"))
            ys[j, :ln] = torch.from_numpy(val_ids[s + 1 : e + 1].astype("int64"))
            keep[j, :ln] = True
            textm[j, :ln] = (
                torch.from_numpy(val_body[s + 1 : e + 1].astype(bool))
                if arm == "C"
                else True
            )
        xs, ys = xs.to(device), ys.to(device)
        keep, textm = keep.to(device), textm.to(device)
        logits, _ = model(xs)
        per = F.cross_entropy(
            logits.view(-1, logits.size(-1)), ys.view(-1), reduction="none"
        ).view(len(chunk), block)
        text = keep & textm
        total_nats += float(per[text].sum())
        total_loss += float(per[keep].sum())
        total_tok += int(keep.sum())
    model.train()
    return bits_per_byte(total_nats, val_bytes), total_loss / max(total_tok, 1)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("data_dir")
    ap.add_argument("arm", choices=["A", "B", "C", "gpt2", "cl100k", "o200k", "llama"])
    ap.add_argument("--steps", type=int, default=200)
    ap.add_argument("--batch", type=int, default=32)
    ap.add_argument("--block", type=int, default=256)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--eval-every", type=int, default=100)
    # Eval batch is separate + small by default: large-vocab industry tokenizers
    # (cl100k ~100k, o200k ~200k) make the logits tensor huge, so a big eval batch OOMs.
    ap.add_argument("--eval-batch", type=int, default=16)
    ap.add_argument("--dropout", type=float, default=0.0)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--seed", type=int, default=1337)
    args = ap.parse_args()

    torch.manual_seed(args.seed)
    train_ids, val_ids, vocab, val_bytes, val_body = D.build_arm(args.data_dir, args.arm)
    cfg = GPTConfig(vocab_size=vocab, block_size=args.block, dropout=args.dropout)
    model = GPT(cfg).to(args.device)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, betas=(0.9, 0.95), weight_decay=0.1)

    runs = os.path.normpath(os.path.join(args.data_dir, os.pardir, "runs"))
    os.makedirs(runs, exist_ok=True)
    print(
        f"arm {args.arm}: vocab {vocab}, {model.num_params() / 1e6:.2f}M params, "
        f"train {len(train_ids):,} tok, val {len(val_ids):,} tok, val_bytes {val_bytes:,}, "
        f"device {args.device}"
    )

    log_path = os.path.join(runs, f"arm_{args.arm}.jsonl")
    best_bpb = float("inf")
    best_step = 0
    best_ckpt = os.path.join(runs, f"arm_{args.arm}_best.pt")
    with open(log_path, "w", encoding="utf-8") as logf:
        for step in range(1, args.steps + 1):
            x, y = get_batch(train_ids, args.block, args.batch, args.device)
            _, loss = model(x, y)
            opt.zero_grad(set_to_none=True)
            loss.backward()
            opt.step()
            if step % args.eval_every == 0 or step == args.steps:
                bpb, vloss = eval_arm(model, val_ids, args.arm, val_bytes, args.block, args.device,
                                      val_body=val_body, eval_batch=args.eval_batch)
                if bpb < best_bpb:  # best-val = the fair point under a fixed compute budget
                    best_bpb, best_step = bpb, step
                    torch.save({"model": model.state_dict(), "cfg": cfg.__dict__, "arm": args.arm,
                                "step": step, "val_bpb": bpb}, best_ckpt)
                rec = {"step": step, "train_loss": loss.item(), "val_loss": vloss,
                       "val_bpb": bpb, "best_bpb": best_bpb}
                logf.write(json.dumps(rec) + "\n")
                logf.flush()
                print(f"  step {step:>5}  train {loss.item():.3f}  val {vloss:.3f}  bpb {bpb:.3f}")

    print(f"  arm {args.arm} BEST val bpb {best_bpb:.4f} @ step {best_step}  ->  {best_ckpt}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
