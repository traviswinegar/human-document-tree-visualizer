"""Dataset assembly for tokenizer-bench (M1).

Reads the `corpus-export` output (`manifest.json` + `arm_c.u16` + `corpus.txt`) and
builds the per-arm token streams, with a shared, deterministic doc-based train/val
split (the last `val_doc_count` docs are validation). Also trains the arm-A BPE and
reports **tokens-per-byte** per arm — the efficiency half of the comparison.

    python tokbench/data.py <data_dir> [bpe_vocab]
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass

import numpy as np

BYTE_CEIL = 256


@dataclass
class Split:
    train_bytes: int
    val_bytes: int
    train_c_tokens: int
    val_c_tokens: int


def load_manifest(data_dir: str) -> dict:
    with open(os.path.join(data_dir, "manifest.json"), encoding="utf-8") as f:
        return json.load(f)


def split_boundaries(manifest: dict) -> Split:
    """Cumulative byte / arm-C-token counts for the train and val doc ranges."""
    docs = manifest["docs"]
    val_n = manifest["val_doc_count"]
    cut = len(docs) - val_n
    train, val = docs[:cut], docs[cut:]
    return Split(
        train_bytes=sum(d["bytes"] for d in train),
        val_bytes=sum(d["bytes"] for d in val),
        train_c_tokens=sum(d["c_tokens"] for d in train),
        val_c_tokens=sum(d["c_tokens"] for d in val),
    )


def load_arm_c(data_dir: str) -> np.ndarray:
    """Arm-C-lite token ids (little-endian u16), concatenated in manifest order."""
    return np.fromfile(os.path.join(data_dir, "arm_c.u16"), dtype="<u2")


def load_arm_c_body(data_dir: str) -> np.ndarray:
    """Arm-C-lite body mask (u8: 1 = document body byte scored for BPB, 0 = marker)."""
    return np.fromfile(os.path.join(data_dir, "arm_c.body"), dtype=np.uint8)


def load_text_bytes(data_dir: str) -> bytes:
    with open(os.path.join(data_dir, "corpus.txt"), "rb") as f:
        return f.read()


def arm_b_tokens(text_bytes: bytes) -> np.ndarray:
    """Arm B: each source byte is a token id 0..255."""
    return np.frombuffer(text_bytes, dtype=np.uint8).astype(np.uint16)


def train_arm_a_bpe(train_text: str, vocab_size: int, save_dir: str | None = None):
    """Arm A: train a byte-level BPE on the *train* text only (no val leakage)."""
    from tokenizers import ByteLevelBPETokenizer

    tok = ByteLevelBPETokenizer()
    tok.train_from_iterator(
        [train_text], vocab_size=vocab_size, min_frequency=2, special_tokens=["<eos>"]
    )
    if save_dir:
        os.makedirs(save_dir, exist_ok=True)
        tok.save(os.path.join(save_dir, "bpe.json"))
    return tok


def tokens_per_byte_summary(data_dir: str, bpe_vocab: int = 8192) -> dict:
    """Report tokens-per-byte for each arm over the validation split.

    Arm B is 1.0 by construction; arm C comes straight from the export; arm A is the
    trained BPE applied to the held-out val text. (Tokens-per-byte is the *efficiency*
    half of the comparison — the *quality* half is bits-per-byte after training.)
    """
    manifest = load_manifest(data_dir)
    split = split_boundaries(manifest)
    text = load_text_bytes(data_dir)
    train_text = text[: split.train_bytes].decode("utf-8", errors="replace")
    val_text = text[split.train_bytes :].decode("utf-8", errors="replace")

    bpe = train_arm_a_bpe(train_text, bpe_vocab, save_dir=os.path.join(data_dir, "bpe"))
    a_val_tokens = len(bpe.encode(val_text).ids)

    val_bytes = split.val_bytes
    return {
        "val_bytes": val_bytes,
        "arm_A_bpe": {
            "vocab": bpe_vocab,
            "val_tokens": a_val_tokens,
            "tokens_per_byte": a_val_tokens / val_bytes,
        },
        "arm_B_byte": {"vocab": 256, "val_tokens": val_bytes, "tokens_per_byte": 1.0},
        "arm_C_graph": {
            "vocab": manifest["vocab_size_arm_c"],
            "val_tokens": split.val_c_tokens,
            "tokens_per_byte": split.val_c_tokens / val_bytes,
        },
    }


def build_arm(data_dir: str, arm: str, bpe_vocab: int = 8192):
    """Return `(train_ids, val_ids, vocab_size, val_text_bytes, val_body)` for an arm.

    1-D numpy token-id arrays, split at the shared doc-based train/val boundary.
    `val_text_bytes` is the source-byte denominator for bits-per-byte. `val_body` is
    the arm-C-lite body mask (1 = document byte scored for BPB) aligned to `val_ids`;
    `None` for arms A and B (every position is text).
    """
    manifest = load_manifest(data_dir)
    split = split_boundaries(manifest)
    text = load_text_bytes(data_dir)
    if arm == "B":
        ids = arm_b_tokens(text)
        return ids[: split.train_bytes], ids[split.train_bytes :], 256, split.val_bytes, None
    if arm == "C":
        ids = load_arm_c(data_dir)
        body = load_arm_c_body(data_dir)
        cut = split.train_c_tokens
        return ids[:cut], ids[cut:], int(manifest["vocab_size_arm_c"]), split.val_bytes, body[cut:]
    if arm == "A":
        train_text = text[: split.train_bytes].decode("utf-8", errors="replace")
        val_text = text[split.train_bytes :].decode("utf-8", errors="replace")
        bpe = train_arm_a_bpe(train_text, bpe_vocab, save_dir=os.path.join(data_dir, "bpe"))
        train = np.asarray(bpe.encode(train_text).ids, dtype=np.int64)
        val = np.asarray(bpe.encode(val_text).ids, dtype=np.int64)
        return train, val, bpe.get_vocab_size(), split.val_bytes, None
    raise ValueError(f"unknown arm {arm!r} (expected 'A', 'B', or 'C')")


def _main(argv: list[str]) -> int:
    data_dir = argv[1] if len(argv) > 1 else "data"
    bpe_vocab = int(argv[2]) if len(argv) > 2 else 8192
    s = tokens_per_byte_summary(data_dir, bpe_vocab)
    print(f"val bytes: {s['val_bytes']:,}\n")
    print(f"{'arm':<14}{'vocab':>8}{'val tokens':>14}{'tokens/byte':>14}")
    for key, label in [("arm_A_bpe", "A (BPE)"), ("arm_B_byte", "B (byte)"), ("arm_C_graph", "C (byte+graph)")]:
        a = s[key]
        print(f"{label:<14}{a['vocab']:>8}{a['val_tokens']:>14,}{a['tokens_per_byte']:>14.3f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(_main(sys.argv))
