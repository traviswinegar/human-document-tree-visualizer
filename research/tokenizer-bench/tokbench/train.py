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

from . import data as D
from .bpb import BYTE_CEIL, bits_per_byte
from .model import GPT, GPTConfig


def get_batch(ids: np.ndarray, block: int, batch: int, device: str):
    ix = torch.randint(len(ids) - block - 1, (batch,))
    x = torch.stack([torch.from_numpy(ids[i : i + block].astype("int64")) for i in ix])
    y = torch.stack([torch.from_numpy(ids[i + 1 : i + 1 + block].astype("int64")) for i in ix])
    return x.to(device), y.to(device)


@torch.no_grad()
def eval_arm(model, val_ids, arm: str, val_bytes: int, block: int, device: str):
    """Full-coverage val eval: every target token scored exactly once. Returns
    (bits_per_byte, mean_val_loss). Arm C scores only byte-token targets (text
    positions); the denominator is source bytes for all arms."""
    model.eval()
    n = len(val_ids)
    total_nats = total_loss = 0.0
    total_tok = scored = 0
    s = 0
    while s + 1 < n:
        e = min(s + block, n - 1)
        x = torch.from_numpy(val_ids[s:e].astype("int64"))[None].to(device)
        y = torch.from_numpy(val_ids[s + 1 : e + 1].astype("int64"))[None].to(device)
        _, loss = model(x, y, reduction="none")
        loss = loss.view(-1)
        tgt = y.view(-1)
        total_loss += float(loss.sum())
        total_tok += loss.numel()
        if arm == "C":
            mask = tgt < BYTE_CEIL
            total_nats += float(loss[mask].sum())
            scored += int(mask.sum())
        else:
            total_nats += float(loss.sum())
            scored += loss.numel()
        s = e
    model.train()
    denom = val_bytes if arm == "A" else scored
    return bits_per_byte(total_nats, denom), total_loss / max(total_tok, 1)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("data_dir")
    ap.add_argument("arm", choices=["A", "B", "C"])
    ap.add_argument("--steps", type=int, default=200)
    ap.add_argument("--batch", type=int, default=32)
    ap.add_argument("--block", type=int, default=256)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--eval-every", type=int, default=100)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--seed", type=int, default=1337)
    args = ap.parse_args()

    torch.manual_seed(args.seed)
    train_ids, val_ids, vocab, val_bytes = D.build_arm(args.data_dir, args.arm)
    cfg = GPTConfig(vocab_size=vocab, block_size=args.block)
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
    with open(log_path, "w", encoding="utf-8") as logf:
        for step in range(1, args.steps + 1):
            x, y = get_batch(train_ids, args.block, args.batch, args.device)
            _, loss = model(x, y)
            opt.zero_grad(set_to_none=True)
            loss.backward()
            opt.step()
            if step % args.eval_every == 0 or step == args.steps:
                bpb, vloss = eval_arm(model, val_ids, args.arm, val_bytes, args.block, args.device)
                rec = {"step": step, "train_loss": loss.item(), "val_loss": vloss, "val_bpb": bpb}
                logf.write(json.dumps(rec) + "\n")
                logf.flush()
                print(f"  step {step:>5}  train {loss.item():.3f}  val {vloss:.3f}  bpb {bpb:.3f}")

    ckpt = os.path.join(runs, f"arm_{args.arm}.pt")
    torch.save({"model": model.state_dict(), "cfg": cfg.__dict__, "arm": args.arm}, ckpt)
    print(f"  saved {ckpt}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
